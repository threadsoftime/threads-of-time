"""FastAPI application + lifespan: open MCP clients, wire components, rehydrate active bots."""
from __future__ import annotations

import asyncio
import contextlib
import logging
import sys
from pathlib import Path

import httpx
from fastapi import FastAPI

from brain_sidecar import schema_builder
from brain_sidecar.api import make_router
from brain_sidecar.decide import Decider
from brain_sidecar.dispatch import Dispatcher
from brain_sidecar.llm_client import LlmClient
from brain_sidecar.logging_setup import JsonlDecisionLogWriter, setup_stdlib_logging
from brain_sidecar.loop import LoopSupervisor
from brain_sidecar.mcp_clients import open_mcp
from brain_sidecar.personality import PersonalityCache
from brain_sidecar.settings import get_settings
from brain_sidecar.state import StateStore
from brain_sidecar.triage import TriageGate


async def validate_sse_endpoint_or_raise(
    *,
    memory_url: str,
    bearer: str,
    probe_bot_id: str = "0",
) -> None:
    """Probe the memory-sidecar SSE endpoint at brain startup.

    Sends a short-lived GET to ``/v1/events/stream`` with a probe bot_id.
    Expects HTTP 200 + ``Content-Type: text/event-stream``.  Raises
    ``RuntimeError`` on any failure so the brain refuses a degraded start.

    Called from the lifespan when ``brain_sse_enabled=True``.
    """
    url = f"{memory_url}/v1/events/stream"
    params = {"bot_id": probe_bot_id, "prefixes": "received whisper"}
    headers = {
        "Authorization": f"Bearer {bearer}",
        "Accept": "text/event-stream",
    }
    # Use a split timeout: short connect (5 s) but long read (20 s).
    # The SSE stream sends no data until the first heartbeat (15 s interval)
    # or a matching memory row, so a flat 5-second timeout always fires before
    # any text arrives. We only need to verify the stream opens (HTTP 200 +
    # Content-Type: text/event-stream) — we do NOT wait for actual data.
    split_timeout = httpx.Timeout(connect=5.0, read=20.0, write=5.0, pool=5.0)
    try:
        async with httpx.AsyncClient(timeout=split_timeout) as c:
            async with c.stream("GET", url, params=params, headers=headers) as r:
                if r.status_code != 200:
                    raise RuntimeError(
                        f"SSE endpoint validation failed: HTTP {r.status_code} from {url}"
                    )
                ctype = r.headers.get("content-type", "")
                if "text/event-stream" not in ctype:
                    raise RuntimeError(
                        f"SSE endpoint validation failed: unexpected Content-Type {ctype!r} from {url}"
                    )
                # Headers are valid — stream confirmed open. Close immediately;
                # we do NOT need to read a data chunk for the gate to pass.
    except RuntimeError:
        raise
    except httpx.HTTPError as exc:
        raise RuntimeError(f"SSE endpoint validation failed: {exc}") from exc


def create_app() -> FastAPI:
    settings = get_settings()
    setup_stdlib_logging("INFO")
    log = logging.getLogger("brain-sidecar")

    state_store = StateStore(settings.state_db_path)
    state_store.migrate()
    decision_log = JsonlDecisionLogWriter(settings.decisions_log_path)
    llm_client = LlmClient(
        base_url=settings.llm_base_url,
        model=settings.llm_model,
        timeout_s=settings.llm_timeout_s,
    )
    prompt_path = Path(__file__).resolve().parent.parent / "prompts" / "decide_v1.txt"
    prompt_template = prompt_path.read_text(encoding="utf-8")

    @contextlib.asynccontextmanager
    async def lifespan(app: FastAPI):
        # Spec §5.1: "Memory MCP unavailable → degraded mode, paused loops, retry
        # connection every 30 s." For MVP, both MCPs must be reachable at startup —
        # degraded-mode retry logic lives in C2/C3 (future), not here.
        async with open_mcp(settings.harness_mcp_url, settings.harness_bearer) as harness, \
                   open_mcp(settings.memory_mcp_url, settings.memory_bearer) as memory:
            # V3.2: validate the SSE endpoint before opening the decision loop.
            # Spec criterion #4: "Brain refuses to start if SSE schema validation
            # fails." On failure, log the reason then let the RuntimeError propagate
            # so uvicorn exits non-zero. Operator's recovery path is the
            # BRAIN_SSE_ENABLED=0 env override (spec §8 rollback playbook), which
            # skips validation entirely via the if-guard below.
            if settings.brain_sse_enabled:
                try:
                    await validate_sse_endpoint_or_raise(
                        memory_url=settings.memory_mcp_url.removesuffix("/mcp/mcp"),
                        bearer=settings.memory_bearer,
                    )
                    log.info("sse_endpoint_validated memory_url=%s", settings.memory_mcp_url)
                except RuntimeError as e:
                    log.error("sse_endpoint_validation_failed reason=%s", e)
                    raise

            # V3.1: fetch live tool schemas from both MCPs at boot. Fail-loud
            # on either MCP unreachable; systemd will retry per Quadlet policy.
            try:
                per_tool = await schema_builder.fetch_schemas(harness, memory)
            except Exception as e:
                log.error("schema_builder_fetch_failed error=%r", e)
                sys.exit(1)
            decision_schema = schema_builder.compose_oneof(per_tool)
            tools_summary = schema_builder.render_prompt_summary(per_tool)
            harness_count = sum(1 for t in per_tool.values() if t.source_mcp == "harness")
            memory_count = sum(1 for t in per_tool.values() if t.source_mcp == "memory")
            log.info(
                "schema_loaded harness_tools=%d memory_tools=%d oneof_branches=%d",
                harness_count, memory_count, len(decision_schema["oneOf"]),
            )
            personality_cache = PersonalityCache(
                memory_mcp=memory,
                ttl_s=settings.personality_ttl_s,
                capacity=32,
                llm_client=llm_client,
            )
            triage = TriageGate(harness_mcp=harness, memory_mcp=memory)
            decider = Decider(
                llm_client=llm_client,
                personality_cache=personality_cache,
                memory_mcp=memory,
                state_store=state_store,
                prompt_template=prompt_template,
                decision_schema=decision_schema,
                tools_summary=tools_summary,
                settings=settings,
            )
            dispatcher = Dispatcher(harness_mcp=harness, memory_mcp=memory)
            # Extract the HTTP base URL from the MCP URL for SSE (strip the /mcp/mcp suffix).
            memory_http_url = settings.memory_mcp_url.removesuffix("/mcp/mcp")
            supervisor = LoopSupervisor(
                triage=triage,
                decider=decider,
                dispatcher=dispatcher,
                state_store=state_store,
                tick_interval_s=settings.tick_interval_s,
                reduced_tick_interval_s=settings.reduced_tick_interval_s,
                decision_log_writer=decision_log,
                brain_sse_enabled=settings.brain_sse_enabled,
                memory_mcp_url=memory_http_url,
                memory_bearer=settings.memory_bearer,
                brain_sse_coalesce_ms=settings.brain_sse_coalesce_ms,
                brain_sse_dedup_capacity=settings.brain_sse_dedup_capacity,
            )

            # Rehydrate active bots from the living_bots table (Task 9 review note).
            # Per-row try/except ensures one bad row (e.g., corrupt personality JSON)
            # does not block subsequent rehydrations (domain-knowledge note #4).
            for row in state_store.list_active():
                try:
                    supervisor.start(row.bot_guid)
                    log.info("rehydrated bot_guid=%s", row.bot_guid)
                except Exception as e:
                    log.exception("rehydrate failed bot_guid=%s: %s", row.bot_guid, e)

            # V3.6: warm the personality cache for each enrolled bot so the
            # lazy v2 migration (PersonalityCache.get migration branch) fires
            # at boot rather than at first decide-tick. Gives S4 deterministic
            # timing. Per-bot try/except; one failure does not abort startup.
            for row in state_store.list_active():
                try:
                    await personality_cache.get(row.bot_guid)
                    log.info("personality_warmed bot_guid=%s", row.bot_guid)
                except Exception as e:
                    log.warning(
                        "personality_warm_failed bot_guid=%s err=%s", row.bot_guid, e,
                    )

            # Plan 3 T21: wire SubsetGate — starts after LoopSupervisor.
            # phase_b_enabled=False at this milestone; flipped in T30 (Phase B).
            from brain_sidecar.subset_gate import (
                SubsetGate, SubsetGateConfig,
                WorldSnapshot, PlayerSnapshot, BotSnapshot,
            )

            async def _snapshot_fetcher() -> WorldSnapshot:
                players_raw = await harness.call("obs.list_players", {})
                bots_raw = await harness.call("obs.list_bot_population", {})
                return WorldSnapshot(
                    players=tuple(
                        PlayerSnapshot(**p)
                        for p in (players_raw.get("players") or [])
                    ),
                    bots=tuple(
                        BotSnapshot(**b)
                        for b in (bots_raw.get("bots") or [])
                    ),
                )

            async def _enroll_via_api(bot_guid: int) -> None:
                supervisor.enroll_bot(bot_guid)

            async def _release_via_api(bot_guid: int) -> None:
                await supervisor.release_bot(bot_guid)

            subset_gate_config = SubsetGateConfig(
                living_bot_count=settings.living_bot_count,
                recompute_interval_s=settings.subset_recompute_interval_s,
                hysteresis_out_ticks=settings.subset_hysteresis_out_ticks,
                hysteresis_in_ticks=settings.subset_hysteresis_in_ticks,
                enroll_backoff_s=settings.subset_enroll_backoff_s,
                enabled=settings.subset_gate_enabled,
                phase_b_enabled=False,  # Flipped to True in Phase B (T30).
            )
            subset_gate = SubsetGate(
                state_store=state_store,
                snapshot_fetcher=_snapshot_fetcher,
                enroll_fn=_enroll_via_api,
                release_fn=_release_via_api,
                config=subset_gate_config,
            )
            subset_gate_task = asyncio.create_task(subset_gate.run(), name="subset_gate")
            app.state.subset_gate = subset_gate
            app.state.subset_gate_task = subset_gate_task

            # Expose components on app.state so the router and tests can reach them.
            app.state.supervisor = supervisor
            app.state.state_store = state_store
            app.state.personality_cache = personality_cache

            # Register routes AFTER MCPs are open (auth-gated routes: /status,
            # /enroll, /release). /healthz is pre-registered on the bare app above
            # so it responds immediately while MCPs are still connecting.
            app.include_router(make_router(
                state_store=state_store,
                personality_cache=personality_cache,
                supervisor=supervisor,
                brain_bearer=settings.brain_bearer,
                harness_mcp=harness,
                llm_client=llm_client,
                subset_gate=subset_gate,
            ))

            try:
                yield
            finally:
                # Cancel SubsetGate task first (it has no active bot state to flush).
                subset_gate_task.cancel()
                with contextlib.suppress(asyncio.CancelledError):
                    await subset_gate_task

                # PARALLEL teardown: override Task 9's serial stop_all() so a full
                # 15-bot allowlist (15 × 10 s timeout) doesn't block pod restart for
                # up to 150 s. asyncio.gather fires all stop() coroutines concurrently;
                # return_exceptions=True prevents one cancellation failure from aborting
                # the rest. Task 9's stop_all() is left intact for its unit-test contract.
                # Use supervisor.list_active() (public API) instead of _tasks.keys().
                active_guids = list(supervisor.list_active())
                await asyncio.gather(
                    *(supervisor.stop(g) for g in active_guids),
                    return_exceptions=True,
                )
                state_store.close()

    app = FastAPI(title="brain-sidecar", version="0.1.0", lifespan=lifespan)

    # C2: Pre-lifespan health endpoint — registered on the bare FastAPI before
    # lifespan opens MCPs. Responds 200 as soon as uvicorn binds the port,
    # regardless of whether MCP connections have completed.
    @app.get("/healthz")
    async def _healthz():
        return {"ok": True}

    return app
