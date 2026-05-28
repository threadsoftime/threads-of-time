# SPDX-License-Identifier: GPL-2.0-or-later
"""Unit tests for the brain decision loop's memory recall + write wiring
(Plan 2 Task 33).

These tests mock the LoopSupervisor's collaborators (triage, decider,
dispatcher, state_store, decision_log_writer) and assert that:

1. Before `decider.decide`, the loop calls `memory_client.recall(...)` and
   injects the recalled episodes into `hot_inputs["recalled_memories"]`.
2. After `dispatcher.dispatch`, the loop computes salience and calls
   `memory_client.write_episode(...)` IFF the score clears the threshold.

Scope: wiring only. Brain prompt-engineering for *using* recalls is out of
scope per Plan 2 header.
"""
from __future__ import annotations

from unittest.mock import AsyncMock, MagicMock

import pytest

from brain_sidecar.loop import LoopSupervisor
from brain_sidecar.memory_client import RecalledEpisode
from brain_sidecar.models import (
    Decision,
    DecisionKind,
    TickState,
    TriageResult,
)
from brain_sidecar.salience import SalienceScorer


def _make_loop(memory_client, salience_scorer):
    """Build a LoopSupervisor with all collaborators mocked."""
    triage = MagicMock()
    triage.evaluate = AsyncMock()

    decider = MagicMock()
    decider.decide = AsyncMock()

    dispatcher = MagicMock()
    dispatcher.dispatch = AsyncMock()

    state_store = MagicMock()
    state_store.append_decision = MagicMock()

    decision_log_writer = MagicMock()
    decision_log_writer.write = MagicMock()

    sup = LoopSupervisor(
        triage=triage,
        decider=decider,
        dispatcher=dispatcher,
        state_store=state_store,
        tick_interval_s=1.0,
        decision_log_writer=decision_log_writer,
        memory_client=memory_client,
        salience_scorer=salience_scorer,
    )
    return sup, triage, decider, dispatcher


@pytest.mark.asyncio
async def test_one_tick_calls_recall_before_decide_and_injects_into_hot_inputs():
    memory_client = MagicMock()
    memory_client.recall = AsyncMock(
        return_value=[
            RecalledEpisode(
                episode_id=42,
                content_text="Alice asked me to heal in BFD.",
                timestamp="2026-05-27T12:00:00Z",
                salience_score=0.6,
                score=0.84,
            ),
        ]
    )
    memory_client.write_episode = AsyncMock(return_value=999)

    salience_scorer = SalienceScorer()

    sup, triage, decider, dispatcher = _make_loop(memory_client, salience_scorer)

    triage.evaluate.return_value = TriageResult(
        should_decide=True, reason="fresh_chat", hot_inputs={"fresh_chat": []},
    )
    decider.decide.return_value = (
        Decision(kind=DecisionKind.NO_OP, tool=None, args=None,
                 confidence=0.5, reasoning="x", wakeup_in_ms=180_000),
        12.3,
        False,
    )
    dispatcher.dispatch.return_value = MagicMock(disposition="no_op", error=None)

    last_state = TickState(bot_guid=123, last_tick_ms=0)
    await sup._one_tick(123, last_state)

    # 1. recall was called with the bot's guid (as str) before decide
    assert memory_client.recall.await_count == 1
    recall_kwargs = memory_client.recall.await_args.kwargs or {}
    recall_args = memory_client.recall.await_args.args
    # accept either kw or positional invocation, but inspect what we got
    assert recall_kwargs.get("bot_guid") == "123" or (recall_args and recall_args[0] == "123")

    # 2. decide saw the recalled episodes in hot_inputs
    assert decider.decide.await_count == 1
    decide_kwargs = decider.decide.await_args.kwargs
    hot_inputs = decide_kwargs["hot_inputs"]
    assert "recalled_memories" in hot_inputs
    recalled = hot_inputs["recalled_memories"]
    assert isinstance(recalled, list)
    assert len(recalled) == 1
    # The injected payload is the dataclass converted to a dict (or a dict
    # with the same key set) so the existing prompt template can serialize it.
    assert recalled[0]["episode_id"] == 42
    assert recalled[0]["content_text"] == "Alice asked me to heal in BFD."


@pytest.mark.asyncio
async def test_one_tick_writes_episode_when_salience_clears_threshold():
    memory_client = MagicMock()
    memory_client.recall = AsyncMock(return_value=[])
    memory_client.write_episode = AsyncMock(return_value=1001)

    scorer = SalienceScorer()

    sup, triage, decider, dispatcher = _make_loop(memory_client, scorer)

    # Triage returns a combat hot input; the loop should treat the resulting
    # episode as a combat episode for salience purposes.
    triage.evaluate.return_value = TriageResult(
        should_decide=True, reason="combat",
        hot_inputs={"episode_type": "combat"},
    )
    decider.decide.return_value = (
        Decision(kind=DecisionKind.ACTION, tool="bot.attack", args={"bot_guid": 7},
                 confidence=0.8, reasoning="kill", wakeup_in_ms=120_000),
        14.5,
        False,
    )
    # action_result reports a kill — salience promotion to IMPORTANT (0.8) >> 0.3
    dispatcher.dispatch.return_value = MagicMock(
        disposition="ok", error=None, outcome="kill",
    )

    last_state = TickState(bot_guid=7, last_tick_ms=0)
    await sup._one_tick(7, last_state)

    # write_episode called once with a combat episode and salience score
    # equal to whatever the scorer returned for these inputs.
    assert memory_client.write_episode.await_count == 1
    kwargs = memory_client.write_episode.await_args.kwargs
    assert kwargs["bot_guid"] == "7"
    assert kwargs["episode_type"] == "combat"
    assert kwargs["salience_score"] >= scorer.threshold


@pytest.mark.asyncio
async def test_one_tick_skips_write_when_salience_below_threshold():
    memory_client = MagicMock()
    memory_client.recall = AsyncMock(return_value=[])
    memory_client.write_episode = AsyncMock()

    scorer = SalienceScorer()

    sup, triage, decider, dispatcher = _make_loop(memory_client, scorer)

    # observation = FILLER (0.2) < threshold (0.3) → no write
    triage.evaluate.return_value = TriageResult(
        should_decide=True, reason="organic_wakeup",
        hot_inputs={"episode_type": "observation"},
    )
    decider.decide.return_value = (
        Decision(kind=DecisionKind.NO_OP, tool=None, args=None,
                 confidence=0.0, reasoning="x", wakeup_in_ms=180_000),
        5.0,
        False,
    )
    dispatcher.dispatch.return_value = MagicMock(disposition="no_op", error=None)

    last_state = TickState(bot_guid=99, last_tick_ms=0)
    await sup._one_tick(99, last_state)

    assert memory_client.write_episode.await_count == 0


@pytest.mark.asyncio
async def test_one_tick_skips_recall_and_write_when_triage_says_no_decide():
    """No LLM call → no recall and no write (we don't perceive a meaningful event)."""
    memory_client = MagicMock()
    memory_client.recall = AsyncMock(return_value=[])
    memory_client.write_episode = AsyncMock()

    sup, triage, decider, dispatcher = _make_loop(memory_client, SalienceScorer())

    triage.evaluate.return_value = TriageResult(
        should_decide=False, reason="no_change", hot_inputs={},
    )

    await sup._one_tick(5, TickState(bot_guid=5, last_tick_ms=0))

    assert memory_client.recall.await_count == 0
    assert memory_client.write_episode.await_count == 0
    assert decider.decide.await_count == 0


@pytest.mark.asyncio
async def test_one_tick_swallows_recall_errors_and_continues():
    """A failing recall (network glitch) must not block decision-making."""
    memory_client = MagicMock()
    memory_client.recall = AsyncMock(side_effect=RuntimeError("harness down"))
    memory_client.write_episode = AsyncMock(return_value=1)

    sup, triage, decider, dispatcher = _make_loop(memory_client, SalienceScorer())

    triage.evaluate.return_value = TriageResult(
        should_decide=True, reason="fresh_chat", hot_inputs={"fresh_chat": []},
    )
    decider.decide.return_value = (
        Decision(kind=DecisionKind.NO_OP, tool=None, args=None,
                 confidence=0.1, reasoning="ok", wakeup_in_ms=180_000),
        9.0, False,
    )
    dispatcher.dispatch.return_value = MagicMock(disposition="no_op", error=None)

    # Should not raise
    await sup._one_tick(1, TickState(bot_guid=1, last_tick_ms=0))

    # decider still called
    assert decider.decide.await_count == 1
    # hot_inputs gets an empty recalled_memories list (graceful degradation)
    hot_inputs = decider.decide.await_args.kwargs["hot_inputs"]
    assert hot_inputs.get("recalled_memories", []) == []


@pytest.mark.asyncio
async def test_one_tick_swallows_write_errors_and_continues():
    """A failing write_episode must not break the loop."""
    memory_client = MagicMock()
    memory_client.recall = AsyncMock(return_value=[])
    memory_client.write_episode = AsyncMock(side_effect=RuntimeError("harness down"))

    sup, triage, decider, dispatcher = _make_loop(memory_client, SalienceScorer())

    triage.evaluate.return_value = TriageResult(
        should_decide=True, reason="combat", hot_inputs={"episode_type": "combat"},
    )
    decider.decide.return_value = (
        Decision(kind=DecisionKind.ACTION, tool="bot.attack",
                 args={"bot_guid": 1}, confidence=0.9, reasoning="kill",
                 wakeup_in_ms=120_000),
        9.0, False,
    )
    dispatcher.dispatch.return_value = MagicMock(
        disposition="ok", error=None, outcome="kill",
    )

    # Should not raise
    await sup._one_tick(1, TickState(bot_guid=1, last_tick_ms=0))

    # write_episode was attempted exactly once
    assert memory_client.write_episode.await_count == 1


@pytest.mark.asyncio
async def test_loop_supervisor_works_without_memory_client():
    """Back-compat: existing tests construct LoopSupervisor without memory
    wiring; that path must still work (recall/write simply skipped)."""
    sup_no_mem, triage, decider, dispatcher = _make_loop(
        memory_client=None, salience_scorer=None
    )

    triage.evaluate.return_value = TriageResult(
        should_decide=True, reason="fresh_chat", hot_inputs={"fresh_chat": []},
    )
    decider.decide.return_value = (
        Decision(kind=DecisionKind.NO_OP, tool=None, args=None,
                 confidence=0.0, reasoning="x", wakeup_in_ms=180_000),
        4.0, False,
    )
    dispatcher.dispatch.return_value = MagicMock(disposition="no_op", error=None)

    # Should not raise
    await sup_no_mem._one_tick(1, TickState(bot_guid=1, last_tick_ms=0))
