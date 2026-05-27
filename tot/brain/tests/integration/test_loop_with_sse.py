"""Integration tests: LoopSupervisor spawns SseConsumer alongside 5s tick (B6).

Tests verify:
1. When BRAIN_SSE_ENABLED=True, an SSE event delivered between ticks fires
   the decide pipeline with fresh_chat in hot_inputs.
2. When BRAIN_SSE_ENABLED=False, no SSE task is created; polling-only mode.
3. Concurrent SSE + tick: if tick is in-flight when SSE fires, the SSE event
   is buffered in _pending_sse_inputs and drained on the next tick.
4. (v0.2.2) SSE delivery then poll of same memory_id decides exactly once
   (spec criterion #10).
"""
from __future__ import annotations

import asyncio
import json
import tempfile
import os
import pytest
from unittest.mock import AsyncMock, MagicMock, patch

from brain_sidecar.loop import LoopSupervisor
from brain_sidecar.models import PersonalityCard, Decision, DecisionKind
from brain_sidecar.state import StateStore
from brain_sidecar.triage import TriageGate
from brain_sidecar.decide import Decider
from brain_sidecar.dispatch import Dispatcher
from brain_sidecar.sse_consumer import SseConsumer
from brain_sidecar.dedup import SeenMemoryIds
from .mocks import FakeMcp, FakeLlm


def _card() -> PersonalityCard:
    return PersonalityCard(
        name="Casmina", race="Human", **{"class": "Paladin"},
        backstory="A tested knight.", talkativeness=0.5, courage=0.7,
        greed=0.2, attitude_to_master=0.8,
    )


def _no_op_decision() -> Decision:
    return Decision(kind=DecisionKind.NO_OP, tool=None, args=None,
                    confidence=0.0, reasoning="test no_op")


def _make_store(tmp_path):
    """Create a fresh StateStore with migrations applied."""
    db_path = os.path.join(tmp_path, "state.sqlite")
    store = StateStore(db_path)
    store.migrate()
    store.enroll(bot_guid=1003, enrolled_at_ms=1_700_000_000_000, personality_seed=_card())
    return store


class _StubDecisionLogWriter:
    def __init__(self):
        self.records = []

    def write(self, record):
        self.records.append(record)


@pytest.mark.asyncio
async def test_sse_disabled_no_consumer_created(tmp_path):
    """When BRAIN_SSE_ENABLED=False, supervisor._sse_tasks is empty."""
    store = _make_store(str(tmp_path))
    harness = FakeMcp()
    memory = FakeMcp()

    # Triage always returns no_change to keep the test fast
    harness.responder = lambda t, a: (
        {"ok": True, "result": {"state": "FOLLOW"}} if t == "obs.get_state"
        else {"ok": True, "result": {"events": []}}
    )
    memory.responder = lambda t, a: {"ok": True, "result": {"items": []}}

    triage = TriageGate(harness_mcp=harness, memory_mcp=memory)
    llm = FakeLlm()
    llm.response_json = {"kind": "no_op", "tool": None, "args": None,
                         "confidence": 0.0, "reasoning": "idle"}

    decider = MagicMock()
    decider.decide = AsyncMock(return_value=(_no_op_decision(), 100.0, False))
    dispatcher = MagicMock()
    dispatcher.dispatch = AsyncMock(return_value=MagicMock(disposition="no_op", error=None))

    log_writer = _StubDecisionLogWriter()
    supervisor = LoopSupervisor(
        triage=triage,
        decider=decider,
        dispatcher=dispatcher,
        state_store=store,
        tick_interval_s=0.1,
        decision_log_writer=log_writer,
        brain_sse_enabled=False,
        memory_mcp_url="http://test",
        memory_bearer="x",
    )

    supervisor.start(1003)
    await asyncio.sleep(0.05)
    await supervisor.stop(1003)
    store.close()

    # No SSE tasks created
    assert not any(supervisor._sse_tasks.values()), (
        "SSE disabled: expected no SSE task"
    )


@pytest.mark.asyncio
async def test_sse_event_triggers_decide_out_of_band(tmp_path):
    """An SSE event fires the decide pipeline with fresh_chat in hot_inputs."""
    store = _make_store(str(tmp_path))
    harness = FakeMcp()
    memory = FakeMcp()

    # Triage: return no_change for polling ticks
    harness.responder = lambda t, a: (
        {"ok": True, "result": {"state": "FOLLOW"}} if t == "obs.get_state"
        else {"ok": True, "result": {"events": []}}
    )
    memory.responder = lambda t, a: {"ok": True, "result": {"items": []}}

    triage = TriageGate(harness_mcp=harness, memory_mcp=memory)

    hot_inputs_received = []

    async def fake_decide(bot_guid, hot_inputs, **kwargs):
        hot_inputs_received.append(dict(hot_inputs))
        return _no_op_decision(), 100.0, False

    decider = MagicMock()
    decider.decide = fake_decide
    dispatcher = MagicMock()
    dispatcher.dispatch = AsyncMock(return_value=MagicMock(disposition="no_op", error=None))

    log_writer = _StubDecisionLogWriter()
    supervisor = LoopSupervisor(
        triage=triage,
        decider=decider,
        dispatcher=dispatcher,
        state_store=store,
        tick_interval_s=10.0,   # long tick so SSE fires first
        decision_log_writer=log_writer,
        brain_sse_enabled=True,
        memory_mcp_url="http://test-memory:8090",
        memory_bearer="test-bearer",
    )

    supervisor.start(1003)

    # Wait for the first_tick decide to complete before injecting SSE
    await asyncio.sleep(0.2)
    hot_inputs_received.clear()

    # Inject an SSE event directly via the consumer
    consumer = supervisor._sse_consumers.get(1003)
    assert consumer is not None, "SSE consumer should be created when enabled"

    payload = {
        "row_id": 42,
        "memory_id": "uuid-abc-42",
        "bot_id": "1003",
        "text": "received whisper from Tebrack: follow me",
        "created_ts": 1716321547,
    }

    class _FakeEvent:
        type = "memory"
        id = "42"
        data = json.dumps(payload)

    await consumer._handle_event(_FakeEvent())

    # Wait for coalesce window (200ms default) + dispatch
    await asyncio.sleep(0.5)
    await supervisor.stop(1003)
    store.close()

    # At least one decide call should have fresh_chat from SSE
    sse_decides = [hi for hi in hot_inputs_received if hi.get("fresh_chat")]
    assert sse_decides, (
        f"Expected at least one decide call with fresh_chat from SSE; "
        f"got hot_inputs: {hot_inputs_received}"
    )
    assert sse_decides[0]["fresh_chat"][0]["row_id"] == 42


@pytest.mark.asyncio
async def test_sse_settings_plumb_through_to_consumer(tmp_path):
    """Regression test: BRAIN_SSE_COALESCE_MS + BRAIN_SSE_DEDUP_CAPACITY
    settings must reach the SseConsumer instance, not be silently ignored.
    """
    store = _make_store(str(tmp_path))
    harness = FakeMcp()
    memory = FakeMcp()
    harness.responder = lambda t, a: {"ok": True, "result": {}}
    memory.responder = lambda t, a: {"ok": True, "result": {"items": []}}

    triage = TriageGate(harness_mcp=harness, memory_mcp=memory)
    decider = MagicMock()
    decider.decide = AsyncMock(return_value=(_no_op_decision(), 100.0, False))
    dispatcher = MagicMock()
    dispatcher.dispatch = AsyncMock(return_value=MagicMock(disposition="no_op", error=None))

    supervisor = LoopSupervisor(
        triage=triage,
        decider=decider,
        dispatcher=dispatcher,
        state_store=store,
        tick_interval_s=10.0,    # don't actually tick during the test
        decision_log_writer=_StubDecisionLogWriter(),
        brain_sse_enabled=True,
        memory_mcp_url="http://test",
        memory_bearer="x",
        brain_sse_coalesce_ms=350,
        brain_sse_dedup_capacity=250,
    )

    # Patch the SseConsumer's run loop to prevent actual network attempts.
    with patch.object(SseConsumer, "run_until_complete", new=AsyncMock(return_value=None)):
        supervisor.start(1003)
        await asyncio.sleep(0.05)
        consumer = supervisor._sse_consumers[1003]
        assert consumer is not None
        assert consumer.coalesce_window_ms == 350, (
            f"Expected coalesce_window_ms=350; got {consumer.coalesce_window_ms}"
        )
        assert consumer.dedup_capacity == 250, (
            f"Expected dedup_capacity=250; got {consumer.dedup_capacity}"
        )
        await supervisor.stop(1003)
    store.close()


@pytest.mark.asyncio
async def test_sse_then_poll_of_same_memory_decides_once(tmp_path):
    """Spec criterion #10: a single memory_id must produce exactly ONE decide call.

    Scenario:
      a. SSE delivers memory_id="m_abc" → decide fires (call #1).
      b. Polling tick runs; memory.search returns the SAME item (memory_id="m_abc").
      c. The shared SeenMemoryIds dedup set filters it out.
      d. Assert: decider.decide was called exactly once total.
    """
    store = _make_store(str(tmp_path))

    harness = FakeMcp()
    memory = FakeMcp()

    # obs.get_state and obs.get_combat_log return idle state (no combat events).
    harness.responder = lambda t, a: (
        {"ok": True, "result": {"state": "FOLLOW"}} if t == "obs.get_state"
        else {"ok": True, "result": {"events": []}}
    )

    # The chat item that SSE delivers and the poll would also return.
    shared_item = {
        "memory_id": "m_abc",
        "row_id": 99,
        "bot_id": "1003",
        "text": "received whisper from Tebrack: follow me",
        "created_ts": 1716321547,
        "ts": 9_999_999_999,   # far future so since_ts filter passes
    }

    # memory.search returns the same item on polling (simulates the bug scenario).
    memory.responder = lambda t, a: {
        "ok": True,
        "result": {"items": [shared_item]},
    }

    triage = TriageGate(harness_mcp=harness, memory_mcp=memory)

    decide_call_count = 0

    async def fake_decide(bot_guid, hot_inputs, **kwargs):
        nonlocal decide_call_count
        decide_call_count += 1
        return _no_op_decision(), 50.0, False

    decider = MagicMock()
    decider.decide = fake_decide
    dispatcher = MagicMock()
    dispatcher.dispatch = AsyncMock(return_value=MagicMock(disposition="no_op", error=None))

    log_writer = _StubDecisionLogWriter()
    supervisor = LoopSupervisor(
        triage=triage,
        decider=decider,
        dispatcher=dispatcher,
        state_store=store,
        tick_interval_s=10.0,   # very long — we drive ticks manually
        decision_log_writer=log_writer,
        brain_sse_enabled=True,
        memory_mcp_url="http://test-memory:8090",
        memory_bearer="test-bearer",
    )

    # Patch SseConsumer.run_until_complete so no real network is attempted.
    with patch.object(SseConsumer, "run_until_complete", new=AsyncMock(return_value=None)):
        supervisor.start(1003)

        # Wait for the mandatory first_tick decide to complete.
        await asyncio.sleep(0.15)
        pre_sse_count = decide_call_count  # may be 1 (first_tick)

        # ---- SSE path: inject the shared item via SseConsumer._handle_event ----
        consumer = supervisor._sse_consumers.get(1003)
        assert consumer is not None, "SSE consumer must be created when enabled"

        class _FakeEvent:
            type = "memory"
            id = "99"
            data = json.dumps(shared_item)

        await consumer._handle_event(_FakeEvent())
        # Wait for the coalesce window (default 200ms) + handler to run.
        await asyncio.sleep(0.5)

        post_sse_count = decide_call_count
        assert post_sse_count == pre_sse_count + 1, (
            f"Expected exactly 1 decide from SSE; "
            f"got {post_sse_count - pre_sse_count}"
        )

        # ---- Poll path: run a manual tick; memory.search returns the SAME item ----
        # Simulate the polling tick by calling _one_tick directly with no sse_inputs
        # (so triage falls through to memory.search, which returns shared_item).
        last_state = supervisor._last_states.get(1003)
        # Force last_tick_ms to a real (non-zero) value so triage doesn't short-circuit
        # via first_tick logic.
        from brain_sidecar.models import TickState
        last_state = TickState(bot_guid=1003, last_tick_ms=1)
        supervisor._last_states[1003] = last_state

        await supervisor._one_tick(1003, last_state, sse_inputs=None)
        post_poll_count = decide_call_count

        assert post_poll_count == post_sse_count, (
            f"Poll tick must NOT re-decide for an already-seen memory_id; "
            f"decide called {post_poll_count - post_sse_count} extra time(s)"
        )

        # Verify the dedup set contains the id.
        seen_set = supervisor._seen_memory_ids.get(1003)
        assert seen_set is not None, "_seen_memory_ids must exist for enrolled bot"
        assert seen_set.seen("m_abc"), "m_abc should be marked in the supervisor's SeenMemoryIds"

        await supervisor.stop(1003)
    store.close()
