"""C2: per-bot decision loop supervisor."""
from __future__ import annotations

import asyncio
import logging
import time
import uuid
from dataclasses import dataclass, field
from typing import Any, Optional, Protocol

from brain_sidecar.decide import Decider
from brain_sidecar.dedup import SeenMemoryIds
from brain_sidecar.dispatch import Dispatcher
from brain_sidecar.models import TickState, TriageResult
from brain_sidecar.state import StateStore
from brain_sidecar.triage import TriageGate, CHAT_PREFIXES

log = logging.getLogger(__name__)


class DecisionLogWriter(Protocol):
    def write(self, record: dict[str, Any]) -> None: ...


@dataclass
class LoopSupervisor:
    triage: TriageGate
    decider: Decider
    dispatcher: Dispatcher
    state_store: StateStore
    tick_interval_s: float
    decision_log_writer: DecisionLogWriter
    # SSE settings (V3.2)
    brain_sse_enabled: bool = False
    memory_mcp_url: str = ""
    memory_bearer: str = ""
    brain_sse_coalesce_ms: int = 200
    brain_sse_dedup_capacity: int = 100

    # Per-bot: periodic tick coroutines
    _tasks: dict[int, asyncio.Task] = field(default_factory=dict)
    _locks: dict[int, asyncio.Lock] = field(default_factory=dict)
    _stop: dict[int, asyncio.Event] = field(default_factory=dict)
    # Per-bot: last known TickState (shared between polling tick and SSE-triggered tick)
    _last_states: dict[int, TickState] = field(default_factory=dict)
    # Per-bot: SSE consumers + tasks
    _sse_tasks: dict[int, Optional[asyncio.Task]] = field(default_factory=dict)
    _sse_consumers: dict[int, Any] = field(default_factory=dict)
    # Per-bot: pending SSE items buffered while a tick is in-flight
    _pending_sse: dict[int, list[dict]] = field(default_factory=dict)
    # Per-bot: shared dedup set (v0.2.2) — fence between SSE and polling paths.
    # Capacity of 200 holds ~3-4 minutes of dense chat at 1 msg/s before evicting.
    _seen_memory_ids: dict[int, SeenMemoryIds] = field(default_factory=dict)
    # V3.7 / V3.7.1: per-bot organic-wakeup timer (Unix ms, absolute).
    # Refreshed after every LLM-called tick from the brain's wakeup_in_ms
    # (a delta — converted to absolute here and clamped). Empty on startup;
    # absence → triage's next first_tick path naturally fills.
    _next_wakeup_ms: dict[int, int] = field(default_factory=dict)

    def list_active(self) -> list[int]:
        return [g for g, t in self._tasks.items() if not t.done()]

    def start(self, bot_guid: int) -> None:
        if bot_guid in self._tasks and not self._tasks[bot_guid].done():
            return
        self._locks[bot_guid] = asyncio.Lock()
        self._stop[bot_guid] = asyncio.Event()
        self._last_states[bot_guid] = TickState(bot_guid=bot_guid, last_tick_ms=0)
        self._pending_sse[bot_guid] = []
        self._sse_tasks[bot_guid] = None
        self._sse_consumers[bot_guid] = None
        # v0.2.2: create the shared dedup set that SSE and polling paths both reference.
        self._seen_memory_ids[bot_guid] = SeenMemoryIds(capacity=200)
        self._tasks[bot_guid] = asyncio.create_task(
            self._run(bot_guid), name=f"brain-loop-{bot_guid}"
        )
        # Spawn the SSE consumer task alongside the polling tick.
        if self.brain_sse_enabled and self.memory_mcp_url:
            self._start_sse_consumer(bot_guid)

    def _start_sse_consumer(self, bot_guid: int) -> None:
        """Create and start the SseConsumer for this bot."""
        # Import lazily to avoid circular imports at module load time.
        from brain_sidecar.sse_consumer import SseConsumer

        consumer = SseConsumer(
            bot_guid=bot_guid,
            memory_url=self.memory_mcp_url,
            bearer=self.memory_bearer,
            state_store=self.state_store,
            on_events=self._make_sse_handler(bot_guid),
            prefixes=CHAT_PREFIXES,
            coalesce_window_ms=self.brain_sse_coalesce_ms,
            dedup_capacity=self.brain_sse_dedup_capacity,
            # v0.2.2: inject the supervisor's shared dedup set so SSE marks
            # memory_ids that the polling path then skips.
            dedup_set=self._seen_memory_ids.get(bot_guid),
        )
        self._sse_consumers[bot_guid] = consumer
        self._sse_tasks[bot_guid] = asyncio.create_task(
            consumer.run_until_complete(),
            name=f"brain-sse-{bot_guid}",
        )

    def _make_sse_handler(self, bot_guid: int):
        """Return the async callback that SseConsumer calls on coalesced events."""
        async def _on_sse_events(items: list[dict]) -> None:
            lock = self._locks.get(bot_guid)
            if lock is None:
                return  # bot was released
            if lock.locked():
                # A periodic tick is in-flight; buffer for it to drain.
                self._pending_sse.get(bot_guid, []).extend(items)
                log.debug(
                    "sse_buffered bot_guid=%s count=%d (tick in-flight)", bot_guid, len(items)
                )
                return
            # Lock is free — fire an out-of-band triage/decide cycle.
            async with lock:
                last_state = self._last_states.get(bot_guid) or TickState(
                    bot_guid=bot_guid, last_tick_ms=0
                )
                new_state = await self._one_tick(
                    bot_guid, last_state, sse_inputs={"fresh_chat": items}
                )
                self._last_states[bot_guid] = new_state

        return _on_sse_events

    async def stop(self, bot_guid: int) -> None:
        ev = self._stop.get(bot_guid)
        if ev is not None:
            ev.set()
        # Stop SSE consumer + task first.
        consumer = self._sse_consumers.pop(bot_guid, None)
        if consumer is not None:
            consumer.cancel()
        sse_task = self._sse_tasks.pop(bot_guid, None)
        if sse_task is not None and not sse_task.done():
            sse_task.cancel()
            try:
                await asyncio.wait_for(sse_task, timeout=2)
            except (asyncio.TimeoutError, asyncio.CancelledError, Exception):
                pass
        # Stop the polling tick task.
        task = self._tasks.get(bot_guid)
        if task is not None and not task.done():
            try:
                await asyncio.wait_for(task, timeout=10)
            except asyncio.TimeoutError:
                task.cancel()
        self._tasks.pop(bot_guid, None)
        self._locks.pop(bot_guid, None)
        self._stop.pop(bot_guid, None)
        self._last_states.pop(bot_guid, None)
        self._pending_sse.pop(bot_guid, None)
        self._seen_memory_ids.pop(bot_guid, None)

    async def stop_all(self) -> None:
        for guid in list(self._tasks.keys()):
            await self.stop(guid)

    async def _run(self, bot_guid: int) -> None:
        last_state = TickState(bot_guid=bot_guid, last_tick_ms=0)
        self._last_states[bot_guid] = last_state
        stop_ev = self._stop[bot_guid]
        lock = self._locks[bot_guid]
        log.info("brain-loop started bot_guid=%s", bot_guid)
        while not stop_ev.is_set():
            if lock.locked():
                # Previous tick still running; drop this tick and keep alive.
                self._log({"event_id": _eid(), "ts_ms": _now_ms(), "bot_guid": bot_guid,
                           "triage_reason": "tick_skipped_busy",
                           "llm_called": False, "llm_latency_ms": None,
                           "decision_kind": None,
                           "tool": None, "confidence": None,
                           "dispatch_result": None, "error": None})
            else:
                async with lock:
                    # Drain any pending SSE inputs accumulated while we were sleeping.
                    pending = self._pending_sse.get(bot_guid, [])
                    sse_inputs = {"fresh_chat": pending[:]} if pending else None
                    if pending:
                        pending.clear()
                    last_state = await self._one_tick(bot_guid, last_state, sse_inputs=sse_inputs)
                    self._last_states[bot_guid] = last_state
            try:
                await asyncio.wait_for(stop_ev.wait(), timeout=self.tick_interval_s)
            except asyncio.TimeoutError:
                continue
        log.info("brain-loop stopped bot_guid=%s", bot_guid)

    async def _one_tick(
        self,
        bot_guid: int,
        last_state: TickState,
        sse_inputs: Optional[dict[str, Any]] = None,
    ) -> TickState:
        now_ms = _now_ms()
        # Build the full telemetry record. All fields initialized so the finally block
        # always emits a complete record even if an exception interrupts mid-tick.
        record: dict[str, Any] = {
            "event_id": _eid(),
            "ts_ms": now_ms,
            "bot_guid": bot_guid,
            "triage_reason": None,
            "llm_called": False,
            "llm_latency_ms": None,
            "decision_kind": None,
            "tool": None,
            "confidence": None,
            "dispatch_result": None,
            "error": None,
            "wakeup_at_ms": None,    # absolute Unix ms (V3.7 continuity)
            "wakeup_in_ms": None,    # delta ms (V3.7.1 — equals wakeup_at_ms - ts_ms)
            # V3.2: tag SSE-triggered decides for telemetry
            "event_source": "sse" if (sse_inputs and sse_inputs.get("fresh_chat")) else "poll",
        }
        try:
            triage: TriageResult = await self.triage.evaluate(
                bot_guid=bot_guid, last_state=last_state, now_ms=now_ms,
                sse_inputs=sse_inputs,
                next_wakeup_at_ms=self._next_wakeup_ms.get(bot_guid),
            )
            record["triage_reason"] = triage.reason
            if not triage.should_decide:
                return TickState(bot_guid=bot_guid, last_tick_ms=now_ms,
                                 last_decision_id=last_state.last_decision_id)

            # v0.2.2: SSE/poll dedup fence — polling path only.
            #
            # When triage returns fresh_chat from the POLLING path (sse_inputs was
            # None or empty), check whether any of those items were already decided by
            # the SSE path.  If so, filter them out to prevent a double-decide.
            #
            # The SSE path does NOT need this filter: SseConsumer._handle_event already
            # marks each memory_id in the shared set before calling on_events, so the
            # consumer itself prevents duplicate SSE deliveries.  Applying the filter on
            # the SSE path would silently drop the first (and only) SSE decide.
            is_poll_fresh_chat = (
                triage.reason == "fresh_chat"
                and not (sse_inputs and sse_inputs.get("fresh_chat"))
            )
            if is_poll_fresh_chat:
                seen = self._seen_memory_ids.get(bot_guid)
                if seen is not None:
                    raw = triage.hot_inputs.get("fresh_chat") or []
                    filtered = [
                        item for item in raw
                        if not seen.seen(item.get("memory_id") or "")
                    ]
                    if not filtered:
                        # Every chat item was already dispatched via the SSE path.
                        # Treat this tick as a no_change so we don't call the LLM.
                        record["triage_reason"] = "no_change"
                        return TickState(bot_guid=bot_guid, last_tick_ms=now_ms,
                                         last_decision_id=last_state.last_decision_id)
                    # Mark survivors so a future poll doesn't re-decide them either.
                    for item in filtered:
                        mid = item.get("memory_id") or ""
                        if mid:
                            seen.mark(mid)
                    triage.hot_inputs["fresh_chat"] = filtered

            record["llm_called"] = True
            decision, llm_latency_ms, at_cap = await self.decider.decide(
                bot_guid=bot_guid, hot_inputs=triage.hot_inputs,
                triage_reason=triage.reason,
            )
            record["at_cap"] = at_cap

            # V3.7.1: clamp brain-returned wakeup_in_ms (a delta in ms) into
            # the safe window [60_000, 600_000] and convert to absolute. The
            # delta semantics match what the LLM emits (V3.7.1 prompt rewrite).
            # None / non-positive → 3-min default delta.
            proposed_delta = decision.wakeup_in_ms
            if not isinstance(proposed_delta, int) or proposed_delta <= 0:
                delta = 180_000  # default: 3 min
            else:
                delta = max(60_000, min(proposed_delta, 600_000))
            next_wakeup = now_ms + delta
            self._next_wakeup_ms[bot_guid] = next_wakeup
            record["wakeup_at_ms"] = next_wakeup     # absolute (log continuity)
            record["wakeup_in_ms"] = delta           # delta (V3.7.1 metric)
            record["decision_kind"] = decision.kind.value
            record["tool"] = decision.tool
            record["confidence"] = decision.confidence
            record["llm_latency_ms"] = llm_latency_ms

            result = await self.dispatcher.dispatch(bot_guid=bot_guid, decision=decision)
            record["dispatch_result"] = result.disposition
            if result.error:
                record["error"] = result.error

            # Persist to the ring buffer (synchronous SQLite → run_in_executor).
            running_loop = asyncio.get_running_loop()
            await running_loop.run_in_executor(
                None,
                self._append_decision_sync,
                bot_guid, now_ms, decision,
            )
        except Exception as e:
            log.exception("uncaught tick exception bot=%s", bot_guid)
            record["error"] = f"tick_exception: {e}"
        finally:
            self._log(record)
        return TickState(bot_guid=bot_guid, last_tick_ms=now_ms,
                         last_decision_id=record["event_id"])

    def _append_decision_sync(self, bot_guid: int, ts_ms: int, decision: Any) -> None:
        """Thin wrapper so run_in_executor can call state_store.append_decision without kwargs."""
        self.state_store.append_decision(bot_guid=bot_guid, ts_ms=ts_ms, decision=decision)

    def _log(self, record: dict[str, Any]) -> None:
        try:
            self.decision_log_writer.write(record)
        except Exception:
            log.exception("decision log write failed")


def _now_ms() -> int:
    return int(time.time() * 1000)


def _eid() -> str:
    return str(uuid.uuid4())
