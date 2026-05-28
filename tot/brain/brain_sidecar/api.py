"""C1: HTTP API — /enroll, /release, /status. /healthz is owned by app.py (pre-lifespan)."""
from __future__ import annotations

import asyncio
import logging
import time
from typing import Annotated

from fastapi import APIRouter, Depends, Header, HTTPException
from pydantic import BaseModel

from brain_sidecar.models import PersonalityCard

log = logging.getLogger(__name__)


class EnrollRequest(BaseModel):
    bot_guid: int
    personality_seed: PersonalityCard


class EnrollResponse(BaseModel):
    bot_guid: int
    status: str
    enrolled_at: int


class ReleaseRequest(BaseModel):
    bot_guid: int


class ReleaseResponse(BaseModel):
    bot_guid: int
    status: str


def make_router(*, state_store, personality_cache, supervisor, brain_bearer: str, harness_mcp=None, llm_client=None, subset_gate=None) -> APIRouter:
    router = APIRouter()

    def _check_auth(authorization: Annotated[str | None, Header()] = None) -> None:
        if not brain_bearer:
            return  # auth disabled (dev mode)
        if not authorization or not authorization.startswith("Bearer "):
            raise HTTPException(401, "missing bearer")
        token = authorization.removeprefix("Bearer ").strip()
        if token != brain_bearer:
            raise HTTPException(403, "invalid bearer")

    @router.get("/status", dependencies=[Depends(_check_auth)])
    async def status():
        return {
            "active_bots": supervisor.list_active(),
        }

    @router.post("/enroll", response_model=EnrollResponse, status_code=201,
                 dependencies=[Depends(_check_auth)])
    async def enroll(req: EnrollRequest):
        now_ms = int(time.time() * 1000)
        existing = state_store.get_bot(req.bot_guid)
        if existing is not None and existing.status == "active":
            raise HTTPException(409, f"bot_guid {req.bot_guid} already enrolled")
        try:
            state_store.enroll(
                bot_guid=req.bot_guid,
                enrolled_at_ms=now_ms,
                personality_seed=req.personality_seed,
            )
        except ValueError as e:
            raise HTTPException(409, str(e))
        # Item C: auto-populate identity (name/race/class_) from obs.get_state.
        # ALL-OR-NOTHING: if any field is missing or the call fails, keep the seed
        # values unchanged.  Enrollment must not block on harness unavailability.
        if harness_mcp is not None:
            try:
                raw = await asyncio.wait_for(
                    harness_mcp.call("obs.get_state", {"target_guid": req.bot_guid}),
                    timeout=5.0,
                )
                # Unwrap {"ok": True, "result": {...}} harness envelope, then
                # the Tier0 digest nests identity under `self`:
                #   {"self": {"name": ..., "race": ..., "class": ..., ...}, "location": ..., ...}
                # (Tier0_StateDigest.cpp BuildDigestJson, j["self"] block).
                obs = raw.get("result", raw) if isinstance(raw, dict) else {}
                self_obj = obs.get("self", {}) if isinstance(obs, dict) else {}
                live_name = self_obj.get("name") or ""
                live_race = self_obj.get("race") or ""
                live_class = self_obj.get("class") or ""
                if live_name and live_race and live_class:
                    req.personality_seed.name = live_name
                    req.personality_seed.race = live_race
                    req.personality_seed.class_ = live_class
                else:
                    log.warning(
                        "enroll bot_guid=%d: obs.get_state missing identity fields "
                        "(name=%r race=%r class=%r); keeping seed values",
                        req.bot_guid, live_name, live_race, live_class,
                    )
            except Exception as exc:
                log.warning(
                    "enroll bot_guid=%d: obs.get_state failed (%s); keeping seed values",
                    req.bot_guid, exc,
                )
        # V3.6: morph personality v2 fields. If any v2 field is None,
        # random-seed + LLM-morph based on backstory. On LLM failure,
        # seed values are kept — enroll never fails on this path.
        if llm_client is not None:
            try:
                from brain_sidecar.morph import morph_personality
                req.personality_seed = await morph_personality(
                    req.personality_seed, llm_client,
                )
            except Exception as e:
                log.warning(
                    "enroll bot_guid=%d: morph_personality unexpectedly raised "
                    "(%s); proceeding with seed card; v2 will fill on next "
                    "cache miss.", req.bot_guid, e,
                )
        # Seed personality into memory MCP via cache (write-through).
        # Must happen before supervisor.start() so the loop's first tick can read
        # personality from the in-process cache without a cold MCP fetch.
        # On failure, roll back the state_store row to avoid an orphan `active`
        # entry with no personality (spec §5.1; I4 domain review fix).
        try:
            await personality_cache.seed(req.bot_guid, req.personality_seed)
        except Exception as e:
            state_store.set_status(req.bot_guid, "released")
            raise HTTPException(503, f"personality seed failed; memory MCP may be unavailable: {e}")
        supervisor.start(req.bot_guid)
        return EnrollResponse(bot_guid=req.bot_guid, status="active", enrolled_at=now_ms)

    @router.post("/release", response_model=ReleaseResponse,
                 dependencies=[Depends(_check_auth)])
    async def release(req: ReleaseRequest):
        existing = state_store.get_bot(req.bot_guid)
        if existing is None:
            raise HTTPException(404, "not enrolled")
        await supervisor.stop(req.bot_guid)
        state_store.set_status(req.bot_guid, "released")
        return ReleaseResponse(bot_guid=req.bot_guid, status="released")

    # ------------------------------------------------------------------
    # Admin endpoints — subset gate operator surface (Plan 3 T20, Spec §4.6)
    # ------------------------------------------------------------------

    @router.post("/admin/subset/pin/{bot_guid}", dependencies=[Depends(_check_auth)])
    async def pin_bot(bot_guid: int):
        if state_store.get_bot(bot_guid) is None:
            raise HTTPException(404, f"bot_guid {bot_guid} not in living_bots")
        state_store.set_pin(bot_guid, True)
        return {"ok": True, "bot_guid": bot_guid, "pinned": True}

    @router.post("/admin/subset/unpin/{bot_guid}", dependencies=[Depends(_check_auth)])
    async def unpin_bot(bot_guid: int):
        state_store.set_pin(bot_guid, False)
        return {"ok": True, "bot_guid": bot_guid, "pinned": False}

    @router.get("/admin/subset/snapshot", dependencies=[Depends(_check_auth)])
    async def subset_snapshot():
        config_payload: dict = {}
        if subset_gate is not None:
            config_payload = {
                "living_bot_count": subset_gate.config.living_bot_count,
                "recompute_interval_s": subset_gate.config.recompute_interval_s,
                "phase_b_enabled": subset_gate.config.phase_b_enabled,
            }
        return {
            "currently_enrolled": [
                {"bot_guid": row.bot_guid, "tier": state_store.get_tier(row.bot_guid)}
                for row in state_store.list_active()
            ],
            "pinned": list(state_store.list_pinned()),
            "config": config_payload,
        }

    @router.post("/admin/subset/recompute", dependencies=[Depends(_check_auth)])
    async def subset_recompute():
        if subset_gate is None:
            raise HTTPException(503, "SubsetGate not configured")
        decision = await subset_gate._recompute_and_apply()
        return {
            "target": sorted(decision.target_living_set),
            "to_enroll": sorted(decision.to_enroll),
            "to_release": sorted(decision.to_release),
            "to_full": sorted(decision.to_full),
            "to_reduced": sorted(decision.to_reduced),
        }

    return router
