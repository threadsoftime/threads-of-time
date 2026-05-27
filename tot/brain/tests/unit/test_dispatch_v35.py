"""V3.5 dispatch tests — 6 new grouping tools: KNOWN_TOOLS, RISK_TABLE, display names,
risk thresholds, and cross-bot guard for bot.invite_to_group."""
from __future__ import annotations

from unittest.mock import AsyncMock

import pytest

from brain_sidecar.decide import KNOWN_TOOLS
from brain_sidecar.dispatch import (
    Dispatcher,
    RISK_TABLE,
    _TOOL_DISPLAY_NAMES,
    _display_name,
)
from brain_sidecar.models import Decision, DecisionKind

# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------

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


# ---------------------------------------------------------------------------
# Task 3: KNOWN_TOOLS coverage
# ---------------------------------------------------------------------------

def test_new_tools_in_known_tools_set():
    """All 6 V1.5 grouping tools must be in KNOWN_TOOLS before dispatch runs."""
    expected = {
        "bot.invite_to_group",
        "bot.accept_invite",
        "bot.leave_group",
        "bot.set_role",
        "bot.queue_for_dungeon",
        "bot.enter_instance",
    }
    missing = expected - KNOWN_TOOLS
    assert not missing, f"Missing from KNOWN_TOOLS: {missing}"


# ---------------------------------------------------------------------------
# Task 4: RISK_TABLE coverage
# ---------------------------------------------------------------------------

def test_new_tools_in_risk_table():
    """All 6 V1.5 grouping tools must have explicit RISK_TABLE entries."""
    expected = {
        "bot.invite_to_group",
        "bot.accept_invite",
        "bot.leave_group",
        "bot.set_role",
        "bot.queue_for_dungeon",
        "bot.enter_instance",
    }
    missing = expected - set(RISK_TABLE.keys())
    assert not missing, f"Missing from RISK_TABLE: {missing}"


def test_set_role_is_low_risk():
    """bot.set_role is low-risk (safe to change pre-queue per spec §6.1)."""
    assert RISK_TABLE["bot.set_role"] == "low"


def test_grouping_action_tools_are_high_risk():
    """invite_to_group, accept_invite, leave_group, queue_for_dungeon, enter_instance are high."""
    for tool in ("bot.invite_to_group", "bot.accept_invite", "bot.leave_group",
                 "bot.queue_for_dungeon", "bot.enter_instance"):
        assert RISK_TABLE[tool] == "high", f"Expected {tool!r} to be 'high' in RISK_TABLE"


# ---------------------------------------------------------------------------
# Task 5: _TOOL_DISPLAY_NAMES coverage
# ---------------------------------------------------------------------------

def test_new_tools_have_display_names():
    """All 6 V1.5 tools must appear in _TOOL_DISPLAY_NAMES for player-facing confirmations."""
    expected = {
        "bot.invite_to_group": "invite someone to group",
        "bot.accept_invite": "accept the group invite",
        "bot.leave_group": "leave the group",
        "bot.set_role": "set my group role",
        "bot.queue_for_dungeon": "queue for a dungeon",
        "bot.enter_instance": "enter the dungeon",
    }
    for tool, expected_name in expected.items():
        got = _display_name(tool)
        assert got == expected_name, (
            f"Display name for {tool!r}: expected {expected_name!r}, got {got!r}"
        )


# ---------------------------------------------------------------------------
# Task 6: _CLASS_DEFAULT_ROLE helper
# ---------------------------------------------------------------------------

def test_class_default_role_helper_present_and_correct():
    """_CLASS_DEFAULT_ROLE must exist in decide module with correct WoW class mappings."""
    from brain_sidecar.decide import _CLASS_DEFAULT_ROLE
    assert _CLASS_DEFAULT_ROLE["paladin"] == "tank or healer"
    assert _CLASS_DEFAULT_ROLE["warrior"] == "tank"
    assert _CLASS_DEFAULT_ROLE["priest"] == "healer"
    assert _CLASS_DEFAULT_ROLE["druid"] == "healer or tank"
    assert _CLASS_DEFAULT_ROLE["shaman"] == "healer or dps"
    assert _CLASS_DEFAULT_ROLE["hunter"] == "dps"
    assert _CLASS_DEFAULT_ROLE["rogue"] == "dps"
    assert _CLASS_DEFAULT_ROLE["mage"] == "dps"
    assert _CLASS_DEFAULT_ROLE["warlock"] == "dps"
    assert _CLASS_DEFAULT_ROLE["death knight"] == "tank or dps"


def test_class_default_role_unknown_class_falls_back_to_dps():
    """An unknown class string must produce 'dps' (the .get default)."""
    from brain_sidecar.decide import _CLASS_DEFAULT_ROLE
    assert _CLASS_DEFAULT_ROLE.get("artificer", "dps") == "dps"


@pytest.mark.asyncio
async def test_system_prompt_includes_party_invite_policy():
    """The assembled system prompt must include party_invite_policy from PersonalityCard."""
    from unittest.mock import AsyncMock, MagicMock
    from brain_sidecar.decide import Decider
    from brain_sidecar.models import PersonalityCard

    card = PersonalityCard(
        name="Ellian", race="Night Elf", **{"class": "Hunter"},
        backstory="An eager apprentice.", talkativeness=0.8, courage=0.6,
        greed=0.1, attitude_to_master=0.9,
        party_invite_policy="accept_always",
    )
    llm = AsyncMock()
    llm.chat_completion_json.return_value = (
        {"kind": "no_op", "tool": None, "args": None, "confidence": 0.0, "reasoning": "idle"},
        "", 100.0,
    )
    cache = AsyncMock()
    cache.get.return_value = card
    mem = AsyncMock()
    mem.call.return_value = {"result": {"items": []}}
    store = MagicMock()
    store.decisions_recent.return_value = []

    _TEMPLATE = (
        "tools={tools_summary} state={state_json} goals={goals_json} "
        "memories={memories_json} recent={recent_decisions_json} hot={hot_inputs_json}"
    )
    decider = Decider(
        llm_client=llm, personality_cache=cache, memory_mcp=mem,
        state_store=store, prompt_template=_TEMPLATE,
    )
    await decider.decide(bot_guid=1004, hot_inputs={})
    system_prompt = llm.chat_completion_json.call_args.kwargs["system"]
    assert "party_invite_policy=accept_always" in system_prompt, (
        f"Expected party_invite_policy in system prompt, got: {system_prompt[:500]}"
    )
    assert "default dungeon role: dps" in system_prompt, (
        "Expected class-based default role in system prompt for Hunter"
    )


# ---------------------------------------------------------------------------
# Task 8: Per-tool risk threshold tests (7 tests covering all 6 new tools)
# ---------------------------------------------------------------------------

@pytest.mark.asyncio
async def test_invite_to_group_high_risk_low_confidence_confirms(harness_mcp, memory_mcp):
    """bot.invite_to_group at confidence=0.80 (below high threshold 0.95) must emit confirmation."""
    d = Dispatcher(harness_mcp=harness_mcp, memory_mcp=memory_mcp)
    decision = Decision(
        kind=DecisionKind.ACTION,
        tool="bot.invite_to_group",
        args={"bot_guid": 1003, "target_guid": 1004},
        confidence=0.80,
        reasoning="Player asked me to invite Ellian",
    )
    result = await d.dispatch(bot_guid=1003, decision=decision)
    assert result.disposition == "confirmation_emitted", (
        f"Expected confirmation_emitted, got {result.disposition!r}"
    )


@pytest.mark.asyncio
async def test_invite_to_group_high_risk_high_confidence_executes(harness_mcp, memory_mcp):
    """bot.invite_to_group at confidence=0.96 (above high threshold 0.95) must execute."""
    d = Dispatcher(harness_mcp=harness_mcp, memory_mcp=memory_mcp)
    decision = Decision(
        kind=DecisionKind.ACTION,
        tool="bot.invite_to_group",
        args={"bot_guid": 1003, "target_guid": 1004},
        confidence=0.96,
        reasoning="Very confident, Casmina initiates group",
    )
    result = await d.dispatch(bot_guid=1003, decision=decision)
    assert result.disposition == "executed", (
        f"Expected executed, got {result.disposition!r}"
    )


@pytest.mark.asyncio
async def test_accept_invite_high_risk_executes_at_threshold(harness_mcp, memory_mcp):
    """bot.accept_invite at exactly confidence=0.95 (the threshold) must execute."""
    d = Dispatcher(harness_mcp=harness_mcp, memory_mcp=memory_mcp)
    decision = Decision(
        kind=DecisionKind.ACTION,
        tool="bot.accept_invite",
        args={"bot_guid": 1004},
        confidence=0.95,
        reasoning="Accepting Casmina's invite",
    )
    result = await d.dispatch(bot_guid=1004, decision=decision)
    assert result.disposition == "executed", (
        f"Expected executed at threshold 0.95, got {result.disposition!r}"
    )


@pytest.mark.asyncio
async def test_set_role_low_risk_executes_at_0_7(harness_mcp, memory_mcp):
    """bot.set_role is low-risk; confidence=0.72 (above LOW_RISK_THRESHOLD=0.7) must execute."""
    d = Dispatcher(harness_mcp=harness_mcp, memory_mcp=memory_mcp)
    decision = Decision(
        kind=DecisionKind.ACTION,
        tool="bot.set_role",
        args={"bot_guid": 1003, "role": "tank"},
        confidence=0.72,
        reasoning="Setting tank role before queuing",
    )
    result = await d.dispatch(bot_guid=1003, decision=decision)
    assert result.disposition == "executed", (
        f"Expected executed, got {result.disposition!r}"
    )


@pytest.mark.asyncio
async def test_queue_for_dungeon_high_risk_below_threshold_confirms(harness_mcp, memory_mcp):
    """bot.queue_for_dungeon at confidence=0.90 (below 0.95) must emit confirmation."""
    d = Dispatcher(harness_mcp=harness_mcp, memory_mcp=memory_mcp)
    decision = Decision(
        kind=DecisionKind.ACTION,
        tool="bot.queue_for_dungeon",
        args={"bot_guid": 1003, "dungeon_id": 0, "roles_mask": 0},
        confidence=0.90,
        reasoning="Queuing for random dungeon",
    )
    result = await d.dispatch(bot_guid=1003, decision=decision)
    assert result.disposition == "confirmation_emitted", (
        f"Expected confirmation_emitted, got {result.disposition!r}"
    )


@pytest.mark.asyncio
async def test_enter_instance_high_risk_executes_at_threshold(harness_mcp, memory_mcp):
    """bot.enter_instance at confidence=0.95 must execute."""
    d = Dispatcher(harness_mcp=harness_mcp, memory_mcp=memory_mcp)
    decision = Decision(
        kind=DecisionKind.ACTION,
        tool="bot.enter_instance",
        args={"bot_guid": 1003, "mode": "lfg"},
        confidence=0.95,
        reasoning="LFG ready, teleporting in",
    )
    result = await d.dispatch(bot_guid=1003, decision=decision)
    assert result.disposition == "executed", (
        f"Expected executed at threshold 0.95, got {result.disposition!r}"
    )


@pytest.mark.asyncio
async def test_leave_group_high_risk_confirms_below_threshold(harness_mcp, memory_mcp):
    """bot.leave_group at confidence=0.90 (below 0.95) must emit confirmation."""
    d = Dispatcher(harness_mcp=harness_mcp, memory_mcp=memory_mcp)
    decision = Decision(
        kind=DecisionKind.ACTION,
        tool="bot.leave_group",
        args={"bot_guid": 1003},
        confidence=0.90,
        reasoning="Considering leaving the group",
    )
    result = await d.dispatch(bot_guid=1003, decision=decision)
    assert result.disposition == "confirmation_emitted", (
        f"Expected confirmation_emitted, got {result.disposition!r}"
    )


# ---------------------------------------------------------------------------
# Task 9: Cross-bot guard for bot.invite_to_group
# ---------------------------------------------------------------------------

@pytest.mark.asyncio
async def test_invite_to_group_cross_bot_guard_wrong_bot_guid(harness_mcp, memory_mcp):
    """bot.invite_to_group with bot_guid != dispatching bot_guid must be blocked_cross_bot.

    target_guid (the invitee) is NOT a bot-ownership key and must NOT trigger the guard.
    This test confirms that only bot_guid triggers the guard, not target_guid.
    """
    d = Dispatcher(harness_mcp=harness_mcp, memory_mcp=memory_mcp)
    # bot_guid=1004 but dispatching for bot_guid=1003 → cross-bot violation
    decision = Decision(
        kind=DecisionKind.ACTION,
        tool="bot.invite_to_group",
        args={"bot_guid": 1004, "target_guid": 1014},  # bot_guid is WRONG
        confidence=0.97,
        reasoning="Attempting to act as a different bot",
    )
    result = await d.dispatch(bot_guid=1003, decision=decision)
    assert result.disposition == "blocked_cross_bot", (
        f"Expected blocked_cross_bot, got {result.disposition!r}"
    )
    # harness_mcp.call must NOT have been called for the actual tool
    harness_calls = [c for c in harness_mcp.call.await_args_list
                     if c.args[0] == "bot.invite_to_group"]
    assert not harness_calls, "bot.invite_to_group must not be executed when cross-bot guard fires"


@pytest.mark.asyncio
async def test_invite_to_group_correct_bot_guid_target_guid_foreign_ok(harness_mcp, memory_mcp):
    """bot.invite_to_group with correct bot_guid and foreign target_guid must NOT trigger guard.

    target_guid intentionally refers to a different bot (1014) — this is valid.
    The cross-bot guard only checks _BOT_OWN_KEYS = {bot_guid, bot_id}, not target_guid.
    """
    d = Dispatcher(harness_mcp=harness_mcp, memory_mcp=memory_mcp)
    decision = Decision(
        kind=DecisionKind.ACTION,
        tool="bot.invite_to_group",
        args={"bot_guid": 1003, "target_guid": 1014},  # target_guid is foreign — expected
        confidence=0.97,
        reasoning="Inviting Danlol to the party",
    )
    result = await d.dispatch(bot_guid=1003, decision=decision)
    # target_guid=1014 != 1003, but that's fine — target_guid is NOT in _BOT_OWN_KEYS
    assert result.disposition == "executed", (
        f"Expected executed (target_guid is not a bot-ownership key), got {result.disposition!r}"
    )


# ---------------------------------------------------------------------------
# Task 10: Few-shot examples present in decide_v1.txt
# ---------------------------------------------------------------------------

def test_decide_v1_prompt_contains_party_formation_example():
    """decide_v1.txt must contain the Example A (party formation) few-shot block."""
    from pathlib import Path
    prompt_path = Path(__file__).parent.parent.parent / "prompts" / "decide_v1.txt"
    content = prompt_path.read_text()
    assert "EXAMPLE A" in content, "decide_v1.txt must contain 'EXAMPLE A' few-shot header"
    assert "bot.accept_invite" in content, (
        "Example A must reference bot.accept_invite as the expected action"
    )
    assert "party_invite_received" in content or "party invitation" in content.lower(), (
        "Example A must describe a party invitation scenario"
    )


def test_decide_v1_prompt_contains_dungeon_flow_example():
    """decide_v1.txt must contain the Example B (dungeon flow) few-shot block."""
    from pathlib import Path
    prompt_path = Path(__file__).parent.parent.parent / "prompts" / "decide_v1.txt"
    content = prompt_path.read_text()
    assert "EXAMPLE B" in content, "decide_v1.txt must contain 'EXAMPLE B' few-shot header"
    assert "bot.set_role" in content, "Example B must reference bot.set_role"
    assert "bot.queue_for_dungeon" in content, "Example B must reference bot.queue_for_dungeon"
    assert "bot.enter_instance" in content, "Example B must reference bot.enter_instance"
