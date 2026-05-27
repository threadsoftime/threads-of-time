"""Tests for triage.evaluate with sse_inputs (B5).

When sse_inputs={'fresh_chat': [...]} is non-empty:
  - triage skips the memory.search chat poll
  - returns should_decide=True, reason='fresh_chat'
  - hot_inputs['fresh_chat'] is the SSE-delivered list

When sse_inputs is None/empty, behaviour is unchanged from V3.1 (polls memory.search).
"""
from __future__ import annotations

import pytest
from unittest.mock import AsyncMock, MagicMock, call

from brain_sidecar.triage import TriageGate
from brain_sidecar.models import TickState, TriageResult


def _make_state(last_tick_ms: int = 1_716_321_540_000) -> TickState:
    return TickState(bot_guid=1003, last_tick_ms=last_tick_ms)


def _make_harness_mcp():
    harness = AsyncMock()

    async def harness_side_effect(tool, args):
        responses = {
            "obs.get_state": {"ok": True, "result": {"state": "FOLLOW"}},
            "obs.get_combat_log": {"ok": True, "result": {"events": []}},
        }
        return responses[tool]

    harness.call = AsyncMock(side_effect=harness_side_effect)
    return harness


def _make_memory_mcp(chat_items=None):
    memory = AsyncMock()
    memory.call = AsyncMock(return_value={
        "ok": True,
        "result": {"items": chat_items or []},
    })
    return memory


@pytest.mark.asyncio
async def test_evaluate_with_sse_inputs_skips_chat_poll_and_fires():
    """When sse_inputs.fresh_chat is non-empty, triage uses it directly and skips memory.search for chat."""
    harness = _make_harness_mcp()
    memory = _make_memory_mcp()

    gate = TriageGate(harness_mcp=harness, memory_mcp=memory)

    sse_items = [
        {"row_id": 42, "memory_id": "uuid-42", "bot_id": "1003",
         "text": "received whisper from Tebrack: come here",
         "created_ts": 1716321547},
    ]

    result = await gate.evaluate(
        bot_guid=1003,
        last_state=_make_state(),
        now_ms=1_716_321_547_000,
        sse_inputs={"fresh_chat": sse_items},
    )

    assert result.should_decide is True
    assert result.reason == "fresh_chat"
    assert result.hot_inputs["fresh_chat"] == sse_items

    # Verify memory.search (chat poll) was NOT called
    for c in memory.call.call_args_list:
        tool = c.args[0] if c.args else None
        assert tool != "memory.search", (
            f"memory.search should not be called when sse_inputs has fresh_chat; got call: {c}"
        )


@pytest.mark.asyncio
async def test_evaluate_without_sse_inputs_polls_memory_search():
    """Backward-compat: when sse_inputs is None, behaviour is unchanged from V3.1."""
    harness = _make_harness_mcp()
    memory = _make_memory_mcp()

    gate = TriageGate(harness_mcp=harness, memory_mcp=memory)
    await gate.evaluate(
        bot_guid=1003,
        last_state=_make_state(),
        now_ms=1_716_321_547_000,
    )

    # memory.search was called (for chat poll)
    search_calls = [
        c for c in memory.call.call_args_list
        if (c.args and c.args[0] == "memory.search")
    ]
    assert len(search_calls) >= 1, "memory.search must be called when sse_inputs is None"


@pytest.mark.asyncio
async def test_evaluate_with_empty_sse_inputs_polls_memory_search():
    """sse_inputs={"fresh_chat": []} is the same as no SSE input — polls normally."""
    harness = _make_harness_mcp()
    memory = _make_memory_mcp()

    gate = TriageGate(harness_mcp=harness, memory_mcp=memory)
    await gate.evaluate(
        bot_guid=1003,
        last_state=_make_state(),
        now_ms=1_716_321_547_000,
        sse_inputs={"fresh_chat": []},
    )

    search_calls = [
        c for c in memory.call.call_args_list
        if (c.args and c.args[0] == "memory.search")
    ]
    assert len(search_calls) >= 1


@pytest.mark.asyncio
async def test_evaluate_first_tick_always_fires_regardless_of_sse():
    """First tick (last_tick_ms=0) always returns should_decide=True."""
    harness = _make_harness_mcp()
    memory = _make_memory_mcp()

    gate = TriageGate(harness_mcp=harness, memory_mcp=memory)
    result = await gate.evaluate(
        bot_guid=1003,
        last_state=TickState(bot_guid=1003, last_tick_ms=0),
        now_ms=1_716_321_547_000,
        sse_inputs=None,
    )
    assert result.should_decide is True
    assert result.reason == "first_tick"
