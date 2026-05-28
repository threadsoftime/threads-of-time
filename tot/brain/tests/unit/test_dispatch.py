"""Tests for the tool dispatcher (C5) — risk gate + cross-bot guard + memory write."""
from __future__ import annotations

import time
from unittest.mock import AsyncMock

import pytest

from brain_sidecar.dispatch import (
    Dispatcher,
    RISK_TABLE,
    _BOT_OWN_KEYS,
    _display_name,
)
from brain_sidecar.models import Decision, DecisionKind
from brain_sidecar.tool_policy import ToolPolicyEnforcer


@pytest.fixture
def harness_mcp():
    m = AsyncMock()
    m.call.return_value = {"ok": True}
    return m


@pytest.fixture
def memory_mcp():
    m = AsyncMock()
    m.call.return_value = {"ok": True}
    return m


@pytest.mark.asyncio
async def test_no_op_writes_no_op_memory(harness_mcp, memory_mcp):
    d = Dispatcher(harness_mcp=harness_mcp, memory_mcp=memory_mcp)
    decision = Decision(kind=DecisionKind.NO_OP, tool=None, args=None,
                         confidence=0.0, reasoning="nothing")
    result = await d.dispatch(bot_guid=1, decision=decision)
    assert result.disposition == "no_op"
    # exactly one memory.write for the no_op
    write_calls = [c for c in memory_mcp.call.await_args_list
                   if c.args[0] in ("memory_write", "memory.write")]
    assert len(write_calls) == 1


@pytest.mark.asyncio
async def test_low_risk_high_confidence_executes(harness_mcp, memory_mcp):
    d = Dispatcher(harness_mcp=harness_mcp, memory_mcp=memory_mcp)
    decision = Decision(
        kind=DecisionKind.ACTION,
        tool="bot.set_strategy",
        args={"bot_guid": 1, "add": ["grind"]},
        confidence=0.85, reasoning="grind it",
    )
    result = await d.dispatch(bot_guid=1, decision=decision)
    assert result.disposition == "executed"
    harness_mcp.call.assert_any_await("bot.set_strategy", {"bot_guid": 1, "add": ["grind"]})


@pytest.mark.asyncio
async def test_low_risk_low_confidence_confirms(harness_mcp, memory_mcp):
    d = Dispatcher(harness_mcp=harness_mcp, memory_mcp=memory_mcp)
    decision = Decision(
        kind=DecisionKind.ACTION,
        tool="bot.set_strategy",
        args={"bot_guid": 1, "add": ["grind"]},
        confidence=0.5, reasoning="maybe?",
    )
    result = await d.dispatch(bot_guid=1, decision=decision)
    assert result.disposition == "confirmation_emitted"
    # send_chat is the confirmation channel
    chat_calls = [c for c in harness_mcp.call.await_args_list
                  if c.args[0] in ("bot.send_chat", "bot_send_chat")]
    assert len(chat_calls) == 1
    # Regression guard: dispatch must send "message" (not "text") — bug fixed in v0.1.7
    chat_args = chat_calls[0].args[1]
    assert "message" in chat_args, \
        "dispatch must send 'message' to bot.send_chat (not 'text') — v0.1.7 fix"
    assert "text" not in chat_args, \
        "dispatch must NOT send 'text' field to bot.send_chat — use 'message' instead"
    # pending_confirmation memory must include the 60s TTL deadline (Important 4)
    write_calls = [c for c in memory_mcp.call.await_args_list
                   if c.args[0] in ("memory_write", "memory.write")]
    assert len(write_calls) == 1
    metadata = write_calls[0].args[1].get("metadata", {})
    assert "confirms_by_ts_ms" in metadata
    # The deadline should be approximately now + 60 000 ms (allow ±2 s clock skew)
    now_ms = int(time.time() * 1000)
    assert abs(metadata["confirms_by_ts_ms"] - (now_ms + 60_000)) < 2_000


@pytest.mark.asyncio
async def test_high_risk_always_confirms_below_0_95(harness_mcp, memory_mcp):
    d = Dispatcher(harness_mcp=harness_mcp, memory_mcp=memory_mcp)
    decision = Decision(
        kind=DecisionKind.ACTION,
        tool="memory.delete",
        args={"bot_id": 1, "memory_id": "abc"},
        confidence=0.94,
        reasoning="prune stale",
    )
    result = await d.dispatch(bot_guid=1, decision=decision)
    assert result.disposition == "confirmation_emitted"


@pytest.mark.asyncio
async def test_high_risk_above_0_95_executes(harness_mcp, memory_mcp):
    d = Dispatcher(harness_mcp=harness_mcp, memory_mcp=memory_mcp)
    decision = Decision(
        kind=DecisionKind.ACTION,
        tool="memory.delete",
        args={"bot_id": 1, "memory_id": "abc"},
        confidence=0.97, reasoning="duplicate",
    )
    result = await d.dispatch(bot_guid=1, decision=decision)
    assert result.disposition == "executed"


@pytest.mark.asyncio
async def test_cross_bot_guard_drops(harness_mcp, memory_mcp):
    d = Dispatcher(harness_mcp=harness_mcp, memory_mcp=memory_mcp)
    decision = Decision(
        kind=DecisionKind.ACTION,
        tool="bot.set_strategy",
        args={"bot_guid": 999, "add": ["grind"]},  # different bot
        confidence=0.9, reasoning="hostile",
    )
    result = await d.dispatch(bot_guid=1, decision=decision)
    assert result.disposition == "blocked_cross_bot"


@pytest.mark.asyncio
async def test_mcp_failure_logged_as_tool_call_failed(harness_mcp, memory_mcp):
    harness_mcp.call.side_effect = RuntimeError("bot offline")
    d = Dispatcher(harness_mcp=harness_mcp, memory_mcp=memory_mcp)
    decision = Decision(
        kind=DecisionKind.ACTION,
        tool="bot.set_strategy",
        args={"bot_guid": 1, "add": ["grind"]},
        confidence=0.9, reasoning="grind",
    )
    result = await d.dispatch(bot_guid=1, decision=decision)
    assert result.disposition == "tool_call_failed"
    assert "bot offline" in result.error


@pytest.mark.asyncio
async def test_unknown_tool_writes_invalid_memory_and_drops(harness_mcp, memory_mcp):
    """Per spec §5.1, C5 owns the unknown-tool check (moved from C4 in Task 7 fixes)."""
    d = Dispatcher(harness_mcp=harness_mcp, memory_mcp=memory_mcp)
    decision = Decision(
        kind=DecisionKind.ACTION,
        tool="bot.do_a_backflip",  # not in RISK_TABLE; also not in KNOWN_TOOLS conceptually
        args={"bot_guid": 1},
        confidence=0.99, reasoning="looked fun",
    )
    result = await d.dispatch(bot_guid=1, decision=decision)
    assert result.disposition == "invalid_tool"
    write_calls = [c for c in memory_mcp.call.await_args_list
                   if c.args[0] in ("memory_write", "memory.write")]
    # Exactly one memory write recording the invalid tool
    assert len(write_calls) == 1
    # The memory_type should mark this as decision_invalid_tool
    args_of_write = write_calls[0].args[1]
    assert args_of_write.get("memory_type") == "decision_invalid_tool"
    # No harness call attempted
    harness_mcp.call.assert_not_called()


# ---------------------------------------------------------------------------
# New tests for Task 8 domain review fixes
# ---------------------------------------------------------------------------

@pytest.mark.asyncio
async def test_cross_bot_guard_explicit_bot_own_keys_only(harness_mcp, memory_mcp):
    """Cross-bot guard uses an explicit _BOT_OWN_KEYS frozenset, not suffix matching.
    A tool arg like target_guid with a foreign value must NOT trigger the guard —
    it refers to an entity the bot is targeting, not the bot itself.
    """
    d = Dispatcher(harness_mcp=harness_mcp, memory_mcp=memory_mcp)
    decision = Decision(
        kind=DecisionKind.ACTION,
        tool="bot.set_strategy",
        args={"bot_guid": 1, "target_guid": 999, "add": ["grind"]},
        confidence=0.85, reasoning="different target is fine",
    )
    result = await d.dispatch(bot_guid=1, decision=decision)
    # target_guid=999 should NOT trigger the cross-bot guard
    assert result.disposition == "executed"
    # Also verify _BOT_OWN_KEYS is a frozenset (immutability guarantee)
    assert isinstance(_BOT_OWN_KEYS, frozenset)


@pytest.mark.asyncio
async def test_confirmation_text_uses_display_name(harness_mcp, memory_mcp):
    """Confirmation chat must use friendly display names, not raw tool names.
    A bot.set_strategy confirmation should say "change my strategy", not "bot.set_strategy".
    """
    d = Dispatcher(harness_mcp=harness_mcp, memory_mcp=memory_mcp)
    decision = Decision(
        kind=DecisionKind.ACTION,
        tool="bot.set_strategy",
        args={"bot_guid": 1, "add": ["grind"]},
        confidence=0.5, reasoning="grinding seems good",
    )
    result = await d.dispatch(bot_guid=1, decision=decision)
    assert result.disposition == "confirmation_emitted"

    chat_calls = [c for c in harness_mcp.call.await_args_list
                  if c.args[0] in ("bot.send_chat", "bot_send_chat")]
    assert len(chat_calls) == 1
    text_sent = chat_calls[0].args[1]["message"]
    assert "change my strategy" in text_sent
    assert "bot.set_strategy" not in text_sent


@pytest.mark.parametrize("tool", [
    "obs.get_state",
    "obs.get_combat_log",
    "memory.search",
    "memory.recall_about",
    "memory.personality.get",
])
@pytest.mark.asyncio
async def test_known_read_tools_default_to_low_and_dont_block_at_confidence_0_85(
    tool, harness_mcp, memory_mcp
):
    """obs.* reads and memory read tools must be classified as low-risk.
    At confidence=0.85 (above LOW_RISK_THRESHOLD=0.7) they must execute, not confirm.
    Prior to this fix these tools were absent from RISK_TABLE and defaulted to 'high',
    which would have required confidence >= 0.95 to execute without confirmation.
    """
    assert RISK_TABLE.get(tool) == "low", (
        f"Expected {tool!r} to be 'low' in RISK_TABLE but got {RISK_TABLE.get(tool)!r}"
    )
    d = Dispatcher(harness_mcp=harness_mcp, memory_mcp=memory_mcp)
    # obs.* tools route to harness; memory.* route to memory — figure out args accordingly
    if tool.startswith("obs.") or tool.startswith("bot."):
        args: dict = {"bot_guid": 1}
        expected_mcp = harness_mcp
    else:
        args = {"bot_id": 1}
        expected_mcp = memory_mcp

    decision = Decision(
        kind=DecisionKind.ACTION,
        tool=tool,
        args=args,
        confidence=0.85,
        reasoning="read-only check",
    )
    result = await d.dispatch(bot_guid=1, decision=decision)
    assert result.disposition == "executed", (
        f"Tool {tool!r} at confidence=0.85 should execute but got {result.disposition!r}"
    )


@pytest.mark.asyncio
async def test_outcome_memory_write_uses_new_schema(harness_mcp, memory_mcp):
    """Regression for v0.1.3: memory.write must use {text, salience, entities, relations}.

    The old shape {memory_type, content, metadata} was silently rejected by the live
    memory-sidecar (wrong field names) so outcome writes failed on every tick.
    """
    d = Dispatcher(harness_mcp=harness_mcp, memory_mcp=memory_mcp)
    decision = Decision(
        kind=DecisionKind.ACTION,
        tool="bot.set_strategy",
        args={"bot_guid": 1, "add": ["grind"]},
        confidence=0.85, reasoning="grind it",
    )
    await d.dispatch(bot_guid=1, decision=decision)

    write_calls = [c for c in memory_mcp.call.await_args_list
                   if c.args[0] == "memory.write"]
    assert len(write_calls) >= 1
    payload = write_calls[-1].args[1]  # last write = action_taken outcome
    assert "text" in payload, "memory.write must include 'text' field"
    assert "salience" in payload, "memory.write must include 'salience' field"
    assert "entities" in payload, "memory.write must include 'entities' field"
    assert "relations" in payload, "memory.write must include 'relations' field"
    assert isinstance(payload["text"], str) and len(payload["text"]) > 0
    assert isinstance(payload["salience"], float)
    assert isinstance(payload["entities"], list)
    assert isinstance(payload["relations"], list)
    # bot_guid should be in entities
    assert "1" in [str(e) for e in payload["entities"]]


def test_display_name_falls_back_to_raw_tool_name():
    """Unknown tools should fall back to their raw name rather than raising."""
    assert _display_name("some.unknown_tool") == "some.unknown_tool"


def test_display_name_known_tools():
    """Spot-check that _TOOL_DISPLAY_NAMES covers the dual spellings."""
    assert _display_name("bot.set_strategy") == "change my strategy"
    assert _display_name("memory.write") == "remember that"
    assert _display_name("memory_write") == "remember that"
    assert _display_name("goals_create") == "set that as a goal"
    assert _display_name("memory.goals.create") == "set that as a goal"


# ---------------------------------------------------------------------------
# Task 27: ToolPolicyEnforcer integration in Dispatcher
# ---------------------------------------------------------------------------

@pytest.mark.asyncio
async def test_dispatch_denies_bot_send_chat_in_reduced_tier(harness_mcp, memory_mcp):
    """bot.send_chat must be denied in REDUCED tier when tool_policy is set."""
    enforcer = ToolPolicyEnforcer()
    dispatcher = Dispatcher(
        harness_mcp=harness_mcp,
        memory_mcp=memory_mcp,
        tool_policy=enforcer,
    )
    decision = Decision(
        kind=DecisionKind.ACTION,
        tool="bot.send_chat",
        args={"bot_guid": 1, "message": "hi", "channel": "say"},
        confidence=0.9,
        reasoning="wants to chat",
    )
    result = await dispatcher.dispatch(bot_guid=1, decision=decision, tier="reduced")
    assert result.disposition == "policy_denied"
    assert result.tool == "bot.send_chat"
    # Assert the MCP client was NOT called with bot.send_chat.
    for call in harness_mcp.call.await_args_list:
        assert call.args[0] != "bot.send_chat", (
            "harness MCP must NOT be called for a policy_denied tool"
        )


@pytest.mark.asyncio
async def test_dispatch_allows_bot_set_strategy_in_reduced_tier(harness_mcp, memory_mcp):
    """bot.set_strategy is in REDUCED whitelist; must reach the MCP client."""
    enforcer = ToolPolicyEnforcer()
    dispatcher = Dispatcher(
        harness_mcp=harness_mcp,
        memory_mcp=memory_mcp,
        tool_policy=enforcer,
    )
    decision = Decision(
        kind=DecisionKind.ACTION,
        tool="bot.set_strategy",
        args={"bot_guid": 1, "add": ["grind"]},
        confidence=0.9,
        reasoning="grinding time",
    )
    result = await dispatcher.dispatch(bot_guid=1, decision=decision, tier="reduced")
    # bot.set_strategy is allowed in REDUCED — must execute, not be denied.
    assert result.disposition == "executed"
    harness_mcp.call.assert_any_await("bot.set_strategy", {"bot_guid": 1, "add": ["grind"]})


@pytest.mark.asyncio
async def test_dispatch_no_policy_enforcer_allows_all(harness_mcp, memory_mcp):
    """When tool_policy is None (default), all tools pass the gate regardless of tier."""
    dispatcher = Dispatcher(
        harness_mcp=harness_mcp,
        memory_mcp=memory_mcp,
        tool_policy=None,
    )
    decision = Decision(
        kind=DecisionKind.ACTION,
        tool="bot.send_chat",
        args={"bot_guid": 1, "message": "hi", "channel": "say"},
        confidence=0.9,
        reasoning="no policy set",
    )
    result = await dispatcher.dispatch(bot_guid=1, decision=decision, tier="reduced")
    # No enforcer → policy gate is skipped → should proceed to risk gate & execute.
    assert result.disposition == "executed"


@pytest.mark.asyncio
async def test_dispatch_full_tier_allows_bot_send_chat(harness_mcp, memory_mcp):
    """FULL tier must allow bot.send_chat even with a ToolPolicyEnforcer installed."""
    enforcer = ToolPolicyEnforcer()
    dispatcher = Dispatcher(
        harness_mcp=harness_mcp,
        memory_mcp=memory_mcp,
        tool_policy=enforcer,
    )
    decision = Decision(
        kind=DecisionKind.ACTION,
        tool="bot.send_chat",
        args={"bot_guid": 1, "message": "hi", "channel": "say"},
        confidence=0.9,
        reasoning="chatting in full tier",
    )
    result = await dispatcher.dispatch(bot_guid=1, decision=decision, tier="full")
    assert result.disposition == "executed"
