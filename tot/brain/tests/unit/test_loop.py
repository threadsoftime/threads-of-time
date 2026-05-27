"""Unit tests for the loop supervisor (C2). Integration coverage is in Task 11."""
from __future__ import annotations

import asyncio
from unittest.mock import AsyncMock, MagicMock

import pytest

from brain_sidecar.loop import LoopSupervisor
from brain_sidecar.models import Decision, DecisionKind, TickState, TriageResult


def _make_supervisor() -> tuple[LoopSupervisor, list[dict]]:
    """Construct a supervisor with all-mock collaborators. Returns (sup, recorded_logs)."""
    triage = AsyncMock()
    triage.evaluate.return_value = TriageResult(
        should_decide=False, reason="no_change", hot_inputs={}
    )
    decider = AsyncMock()
    decider.decide.return_value = (
        Decision(kind=DecisionKind.NO_OP, tool=None, args=None,
                 confidence=0.0, reasoning="x"),
        None,  # llm_latency_ms — None for no_op
        False,  # at_cap — default False for non-at-cap test cases
    )
    dispatcher = AsyncMock()
    dispatcher.dispatch.return_value = MagicMock(disposition="no_op", error="")
    state_store = MagicMock()
    state_store.append_decision = MagicMock()

    records: list[dict] = []
    writer = MagicMock()
    writer.write = lambda r: records.append(r)

    sup = LoopSupervisor(
        triage=triage, decider=decider, dispatcher=dispatcher,
        state_store=state_store, tick_interval_s=0.05,
        decision_log_writer=writer,
    )
    return sup, records


@pytest.mark.asyncio
async def test_start_idempotent_same_bot():
    sup, _ = _make_supervisor()
    sup.start(42)
    first_task = sup._tasks[42]
    sup.start(42)  # second call: must not spawn a new task
    assert sup._tasks[42] is first_task
    await sup.stop_all()


@pytest.mark.asyncio
async def test_stop_on_unknown_bot_does_not_raise():
    sup, _ = _make_supervisor()
    await sup.stop(99999)  # never started — must be a no-op


@pytest.mark.asyncio
async def test_list_active_includes_started_bots():
    sup, _ = _make_supervisor()
    sup.start(1); sup.start(2)
    assert set(sup.list_active()) == {1, 2}
    await sup.stop_all()
    assert sup.list_active() == []


@pytest.mark.asyncio
async def test_stop_releases_internal_dicts():
    sup, _ = _make_supervisor()
    sup.start(7)
    await asyncio.sleep(0.02)  # give it a moment
    await sup.stop(7)
    assert 7 not in sup._tasks
    assert 7 not in sup._locks
    assert 7 not in sup._stop


@pytest.mark.asyncio
async def test_first_tick_records_jsonl_with_triage_reason():
    sup, records = _make_supervisor()
    sup.triage.evaluate.return_value = TriageResult(
        should_decide=False, reason="no_change", hot_inputs={}
    )
    sup.start(1)
    await asyncio.sleep(0.10)  # ~2 ticks
    await sup.stop_all()
    assert len(records) >= 1
    assert all(set(r.keys()) >= {
        "event_id", "ts_ms", "bot_guid", "triage_reason",
        "llm_called", "llm_latency_ms", "decision_kind", "tool",
        "confidence", "dispatch_result", "error",
    } for r in records)
    assert any(r["triage_reason"] == "no_change" for r in records)


@pytest.mark.asyncio
async def test_loop_writes_at_cap_to_decisions_log():
    """When decide() returns at_cap=True, the JSONL record has at_cap: true."""
    sup, records = _make_supervisor()
    # Override triage to return should_decide=True so the LLM path fires.
    # Use "timed" reason to avoid the dedup fence that filters empty fresh_chat lists.
    sup.triage.evaluate.return_value = TriageResult(
        should_decide=True, reason="timed", hot_inputs={}
    )
    # Override decider to return a 3-tuple with at_cap=True
    sup.decider.decide.return_value = (
        Decision(kind=DecisionKind.NO_OP, tool=None, args=None,
                 confidence=0.0, reasoning="at cap idle"),
        42.0,   # llm_latency_ms
        True,   # at_cap
    )
    sup.start(1)
    await asyncio.sleep(0.10)  # ~2 ticks
    await sup.stop_all()

    llm_records = [r for r in records if r.get("llm_called")]
    assert llm_records, "Expected at least one llm_called=True record"
    assert all(r.get("at_cap") is True for r in llm_records), (
        f"Expected at_cap=True in all llm_called records; got: {llm_records}"
    )


@pytest.mark.asyncio
async def test_no_duplicate_event_ids_per_n_ticks():
    """Regression for double-write bug (v0.1.3): each tick must emit exactly one JSONL record.

    Root cause: _one_tick called self._log(record) on the no-decide return path AND the
    finally block called it again — producing two identical records per tick.
    Fix: remove the early _log call; finally is the sole emission point.
    """
    sup, records = _make_supervisor()
    sup.triage.evaluate.return_value = TriageResult(
        should_decide=False, reason="no_change", hot_inputs={}
    )
    sup.start(1)
    await asyncio.sleep(0.20)  # ~4 ticks at 0.05s interval
    await sup.stop_all()

    event_ids = [r["event_id"] for r in records]
    unique_ids = list(dict.fromkeys(event_ids))  # preserves order, removes dupes
    assert len(event_ids) == len(unique_ids), (
        f"Duplicate event_ids detected — double-write bug: {event_ids}"
    )


# ---------------------------------------------------------------------------
# V3.7.1: wakeup_in_ms (delta) clamping + dual-field JSONL emission
# ---------------------------------------------------------------------------

def _make_decision_with_wakeup(wakeup_in_ms):
    """Build a Decision (no_op) with a specific wakeup_in_ms (delta) for clamp tests."""
    return Decision(
        kind=DecisionKind.NO_OP, tool=None, args=None,
        confidence=0.5, reasoning="test", wakeup_in_ms=wakeup_in_ms,
    )


@pytest.mark.asyncio
async def test_v37_supervisor_has_next_wakeup_ms_field():
    """LoopSupervisor exposes _next_wakeup_ms as a dict."""
    sup, _ = _make_supervisor()
    assert isinstance(sup._next_wakeup_ms, dict)
    assert sup._next_wakeup_ms == {}


@pytest.mark.asyncio
async def test_v37_one_tick_passes_next_wakeup_at_ms_into_triage():
    """_one_tick reads supervisor._next_wakeup_ms[bot_guid] and passes it
    as next_wakeup_at_ms kwarg to triage.evaluate. This stays ABSOLUTE — it
    is a loop→triage parameter, not the Decision.wakeup_in_ms delta."""
    sup, _ = _make_supervisor()
    sup._next_wakeup_ms[1003] = 999_999
    last_state = TickState(bot_guid=1003, last_tick_ms=1_000_000)
    await sup._one_tick(1003, last_state, sse_inputs=None)
    sup.triage.evaluate.assert_awaited_once()
    kwargs = sup.triage.evaluate.await_args.kwargs
    assert kwargs.get("next_wakeup_at_ms") == 999_999


@pytest.mark.asyncio
async def test_v37_one_tick_passes_triage_reason_into_decide():
    """When triage fires, decide() receives triage_reason kwarg."""
    sup, _ = _make_supervisor()
    sup.triage.evaluate.return_value = TriageResult(
        should_decide=True, reason="organic_wakeup", hot_inputs={},
    )
    sup.decider.decide.return_value = (
        _make_decision_with_wakeup(None), 10.0, False,
    )
    last_state = TickState(bot_guid=1003, last_tick_ms=1_000_000)
    await sup._one_tick(1003, last_state, sse_inputs=None)
    sup.decider.decide.assert_awaited_once()
    kwargs = sup.decider.decide.await_args.kwargs
    assert kwargs.get("triage_reason") == "organic_wakeup"


@pytest.mark.asyncio
async def test_v371_clamp_in_range_passes_through_as_delta():
    """Brain returns wakeup_in_ms=300_000 (5min) → kept; next_wakeup ≈ now+5min."""
    import time
    sup, _ = _make_supervisor()
    sup.triage.evaluate.return_value = TriageResult(
        should_decide=True, reason="organic_wakeup", hot_inputs={},
    )
    sup.decider.decide.return_value = (
        _make_decision_with_wakeup(300_000),  # +5min as DELTA
        10.0, False,
    )
    now_before = int(time.time() * 1000)
    last_state = TickState(bot_guid=1003, last_tick_ms=now_before - 1000)
    await sup._one_tick(1003, last_state, sse_inputs=None)
    saved = sup._next_wakeup_ms[1003]
    # Allow ±2s for clock drift between now_before and the _one_tick now_ms
    assert now_before + 298_000 <= saved <= now_before + 302_000


@pytest.mark.asyncio
async def test_v371_clamp_default_when_none():
    """Brain returns wakeup_in_ms=None → now+180_000 default."""
    import time
    sup, _ = _make_supervisor()
    sup.triage.evaluate.return_value = TriageResult(
        should_decide=True, reason="organic_wakeup", hot_inputs={},
    )
    sup.decider.decide.return_value = (
        _make_decision_with_wakeup(None), 10.0, False,
    )
    now_before = int(time.time() * 1000)
    last_state = TickState(bot_guid=1003, last_tick_ms=now_before - 1000)
    await sup._one_tick(1003, last_state, sse_inputs=None)
    saved = sup._next_wakeup_ms[1003]
    assert now_before + 178_000 <= saved <= now_before + 182_000


@pytest.mark.asyncio
async def test_v371_clamp_default_when_non_positive():
    """Brain returns wakeup_in_ms <= 0 (treated as invalid) → now+180_000 default."""
    import time
    sup, _ = _make_supervisor()
    sup.triage.evaluate.return_value = TriageResult(
        should_decide=True, reason="organic_wakeup", hot_inputs={},
    )
    sup.decider.decide.return_value = (
        _make_decision_with_wakeup(-5_000),
        10.0, False,
    )
    now_before = int(time.time() * 1000)
    last_state = TickState(bot_guid=1003, last_tick_ms=now_before - 1000)
    await sup._one_tick(1003, last_state, sse_inputs=None)
    saved = sup._next_wakeup_ms[1003]
    assert now_before + 178_000 <= saved <= now_before + 182_000


@pytest.mark.asyncio
async def test_v371_clamp_floor_when_too_soon():
    """Brain returns wakeup_in_ms < 60_000 → clamp to floor delta 60_000."""
    import time
    sup, _ = _make_supervisor()
    sup.triage.evaluate.return_value = TriageResult(
        should_decide=True, reason="organic_wakeup", hot_inputs={},
    )
    sup.decider.decide.return_value = (
        _make_decision_with_wakeup(10_000),  # only 10s delta
        10.0, False,
    )
    now_before = int(time.time() * 1000)
    last_state = TickState(bot_guid=1003, last_tick_ms=now_before - 1000)
    await sup._one_tick(1003, last_state, sse_inputs=None)
    saved = sup._next_wakeup_ms[1003]
    assert now_before + 58_000 <= saved <= now_before + 62_000


@pytest.mark.asyncio
async def test_v371_clamp_ceiling_when_too_far():
    """Brain returns wakeup_in_ms > 600_000 → clamp to ceiling delta 600_000."""
    import time
    sup, _ = _make_supervisor()
    sup.triage.evaluate.return_value = TriageResult(
        should_decide=True, reason="organic_wakeup", hot_inputs={},
    )
    sup.decider.decide.return_value = (
        _make_decision_with_wakeup(3_600_000),  # +1hr delta
        10.0, False,
    )
    now_before = int(time.time() * 1000)
    last_state = TickState(bot_guid=1003, last_tick_ms=now_before - 1000)
    await sup._one_tick(1003, last_state, sse_inputs=None)
    saved = sup._next_wakeup_ms[1003]
    assert now_before + 598_000 <= saved <= now_before + 602_000


@pytest.mark.asyncio
async def test_v371_jsonl_record_has_both_wakeup_fields():
    """After an LLM-called tick, decisions.jsonl record contains BOTH
    wakeup_at_ms (absolute, log continuity) AND wakeup_in_ms (delta, V3.7.1)."""
    import time
    sup, records = _make_supervisor()
    sup.triage.evaluate.return_value = TriageResult(
        should_decide=True, reason="organic_wakeup", hot_inputs={},
    )
    sup.decider.decide.return_value = (
        _make_decision_with_wakeup(300_000), 10.0, False,
    )
    now_before = int(time.time() * 1000)
    last_state = TickState(bot_guid=1003, last_tick_ms=now_before - 1000)
    await sup._one_tick(1003, last_state, sse_inputs=None)
    # Find the record that has wakeup_at_ms populated (the llm-called one)
    rec = next(r for r in records if isinstance(r.get("wakeup_at_ms"), int))
    # Absolute timestamp emitted (continuity with V3.7 logs)
    assert isinstance(rec["wakeup_at_ms"], int)
    assert rec["wakeup_at_ms"] > now_before
    # NEW V3.7.1: delta field emitted alongside
    assert isinstance(rec["wakeup_in_ms"], int)
    # Delta should be roughly 300_000 (within tolerance)
    assert 298_000 <= rec["wakeup_in_ms"] <= 302_000
    # And the two fields are consistent: wakeup_at_ms == ts_ms + wakeup_in_ms
    # (within tolerance)
    assert abs(rec["wakeup_at_ms"] - rec["ts_ms"] - rec["wakeup_in_ms"]) <= 50
