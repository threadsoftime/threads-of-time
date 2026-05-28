# SPDX-License-Identifier: GPL-2.0-or-later
"""Tests for tier-aware dispatch threading in LoopSupervisor (T28) and
variable tick interval per tier (T29)."""
from __future__ import annotations

import asyncio
from unittest.mock import AsyncMock, MagicMock, patch

import pytest

from brain_sidecar.loop import LoopSupervisor
from brain_sidecar.models import Decision, DecisionKind, TickState, TriageResult


def _make_supervisor(
    tick_interval_s: float = 0.05,
    reduced_tick_interval_s: float = 300.0,
    state_store_tier: str = "full",
) -> tuple[LoopSupervisor, list[dict], MagicMock]:
    """Construct a supervisor with all-mock collaborators.

    Returns (supervisor, recorded_log_entries, dispatcher_mock).
    """
    triage = AsyncMock()
    triage.evaluate.return_value = TriageResult(
        should_decide=True, reason="timed", hot_inputs={}
    )
    decider = AsyncMock()
    decider.decide.return_value = (
        Decision(kind=DecisionKind.NO_OP, tool=None, args=None,
                 confidence=0.0, reasoning="test"),
        10.0,   # llm_latency_ms
        False,  # at_cap
    )
    dispatcher = AsyncMock()
    dispatcher.dispatch.return_value = MagicMock(disposition="no_op", error="")

    state_store = MagicMock()
    state_store.append_decision = MagicMock()
    state_store.get_tier = MagicMock(return_value=state_store_tier)

    records: list[dict] = []
    writer = MagicMock()
    writer.write = lambda r: records.append(r)

    sup = LoopSupervisor(
        triage=triage,
        decider=decider,
        dispatcher=dispatcher,
        state_store=state_store,
        tick_interval_s=tick_interval_s,
        reduced_tick_interval_s=reduced_tick_interval_s,
        decision_log_writer=writer,
    )
    return sup, records, dispatcher


# ---------------------------------------------------------------------------
# T28: LoopSupervisor reads tier and passes it to Dispatcher.dispatch
# ---------------------------------------------------------------------------

@pytest.mark.asyncio
async def test_dispatcher_called_with_tier_reduced():
    """When state_store.get_tier returns 'reduced', dispatcher.dispatch receives tier='reduced'."""
    sup, records, dispatcher = _make_supervisor(state_store_tier="reduced")
    last_state = TickState(bot_guid=7, last_tick_ms=0)
    await sup._one_tick(7, last_state, sse_inputs=None)

    dispatcher.dispatch.assert_awaited_once()
    call_kwargs = dispatcher.dispatch.await_args.kwargs
    assert call_kwargs.get("tier") == "reduced", (
        f"Expected tier='reduced' in dispatch call kwargs; got: {call_kwargs}"
    )


@pytest.mark.asyncio
async def test_dispatcher_called_with_tier_full():
    """When state_store.get_tier returns 'full', dispatcher.dispatch receives tier='full'."""
    sup, records, dispatcher = _make_supervisor(state_store_tier="full")
    last_state = TickState(bot_guid=8, last_tick_ms=0)
    await sup._one_tick(8, last_state, sse_inputs=None)

    dispatcher.dispatch.assert_awaited_once()
    call_kwargs = dispatcher.dispatch.await_args.kwargs
    assert call_kwargs.get("tier") == "full", (
        f"Expected tier='full' in dispatch call kwargs; got: {call_kwargs}"
    )


@pytest.mark.asyncio
async def test_state_store_get_tier_called_per_tick():
    """state_store.get_tier must be called once per LLM-invoked tick."""
    sup, records, dispatcher = _make_supervisor(state_store_tier="reduced")
    last_state = TickState(bot_guid=9, last_tick_ms=0)
    await sup._one_tick(9, last_state, sse_inputs=None)
    sup.state_store.get_tier.assert_called_once_with(9)


# ---------------------------------------------------------------------------
# T29: Variable tick interval — REDUCED bots sleep for reduced_tick_interval_s
# ---------------------------------------------------------------------------

@pytest.mark.asyncio
async def test_reduced_tier_uses_reduced_tick_interval():
    """The _run loop must sleep for reduced_tick_interval_s when tier='reduced'."""
    sleep_calls: list[float] = []
    _real_wait_for = asyncio.wait_for

    async def fake_wait_for(coro, *, timeout: float) -> None:
        # Distinguish the tick-sleep path (coro is a coroutine, not a Task)
        # from stop()'s wait_for(task, timeout=10) call (coro is a Task).
        if asyncio.iscoroutine(coro):
            coro.close()
            sleep_calls.append(timeout)
            raise asyncio.TimeoutError
        # For Task objects (stop() path), use the real wait_for.
        return await _real_wait_for(coro, timeout=timeout)

    sup, records, dispatcher = _make_supervisor(
        tick_interval_s=0.05,
        reduced_tick_interval_s=300.0,
        state_store_tier="reduced",
    )

    with patch("brain_sidecar.loop.asyncio.wait_for", fake_wait_for):
        sup.start(42)
        # Give the loop exactly one iteration
        await asyncio.sleep(0.01)
        await sup.stop(42)

    # The sleep was called with the reduced interval, not the full interval.
    assert any(t == 300.0 for t in sleep_calls), (
        f"Expected reduced_tick_interval_s=300.0 in sleep calls; got: {sleep_calls}"
    )
    assert not any(t == 0.05 for t in sleep_calls), (
        f"FULL tick_interval_s=0.05 must NOT appear in sleep calls for REDUCED tier; got: {sleep_calls}"
    )


@pytest.mark.asyncio
async def test_full_tier_uses_tick_interval():
    """The _run loop must sleep for tick_interval_s when tier='full'."""
    sleep_calls: list[float] = []
    _real_wait_for = asyncio.wait_for

    async def fake_wait_for(coro, *, timeout: float) -> None:
        if asyncio.iscoroutine(coro):
            coro.close()
            sleep_calls.append(timeout)
            raise asyncio.TimeoutError
        return await _real_wait_for(coro, timeout=timeout)

    sup, records, dispatcher = _make_supervisor(
        tick_interval_s=0.05,
        reduced_tick_interval_s=300.0,
        state_store_tier="full",
    )

    with patch("brain_sidecar.loop.asyncio.wait_for", fake_wait_for):
        sup.start(43)
        await asyncio.sleep(0.01)
        await sup.stop(43)

    # The sleep was called with the full interval.
    assert any(t == 0.05 for t in sleep_calls), (
        f"Expected tick_interval_s=0.05 in sleep calls; got: {sleep_calls}"
    )
    assert not any(t == 300.0 for t in sleep_calls), (
        f"reduced_tick_interval_s=300.0 must NOT appear in sleep calls for FULL tier; got: {sleep_calls}"
    )


def test_loop_supervisor_has_reduced_tick_interval_field():
    """LoopSupervisor has reduced_tick_interval_s field with default 300.0."""
    sup, _, _ = _make_supervisor()
    assert hasattr(sup, "reduced_tick_interval_s")
    assert sup.reduced_tick_interval_s == 300.0
