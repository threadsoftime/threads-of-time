"""Tests for the triage gate (C3)."""
from __future__ import annotations

from unittest.mock import AsyncMock

import pytest

from brain_sidecar.models import TickState
from brain_sidecar.triage import TriageGate, _is_player_chat


@pytest.fixture
def harness_mcp():
    m = AsyncMock()
    m.call.return_value = {}
    return m


@pytest.fixture
def memory_mcp():
    m = AsyncMock()
    m.call.return_value = {"items": []}
    return m


@pytest.mark.asyncio
async def test_first_tick_after_enroll_fires(harness_mcp, memory_mcp):
    gate = TriageGate(harness_mcp=harness_mcp, memory_mcp=memory_mcp)
    last_state = TickState(bot_guid=1, last_tick_ms=0)
    # last_tick_ms=0 means "first tick" by convention
    result = await gate.evaluate(bot_guid=1, last_state=last_state, now_ms=1_000_000)
    assert result.should_decide is True
    assert result.reason == "first_tick"


@pytest.mark.asyncio
async def test_no_change_returns_no_fire(harness_mcp, memory_mcp):
    harness_mcp.call.side_effect = lambda name, args: {
        "obs.get_state": {"in_combat": False, "level": 10, "xp_current": 100},
        "obs.get_combat_log": {"events": []},
    }.get(name, {})
    memory_mcp.call.side_effect = lambda name, args: {
        "memory.search": {"items": []},
        "goals.list": {"items": []},
    }.get(name, {})

    gate = TriageGate(harness_mcp=harness_mcp, memory_mcp=memory_mcp)
    last_state = TickState(bot_guid=1, last_tick_ms=1_000_000)
    result = await gate.evaluate(bot_guid=1, last_state=last_state, now_ms=1_005_000)
    assert result.should_decide is False
    assert result.reason == "no_change"


@pytest.mark.asyncio
async def test_fresh_chat_fires(harness_mcp, memory_mcp):
    harness_mcp.call.side_effect = lambda name, args: {
        "obs.get_state": {"in_combat": False, "level": 10, "xp_current": 100},
        "obs.get_combat_log": {"events": []},
    }.get(name, {})
    # Real memory-sidecar schema: "text" field (not "content"); "ts" in Unix seconds (not ts_ms).
    # The item's "ts" (1_004) must be > since_ts_s (last_tick_ms=1_000_000 // 1000 = 1000).
    # The text must contain a player-chat signal so _is_player_chat() returns True.
    memory_mcp.call.side_effect = lambda name, args: {
        "memory.search": {"items": [
            {"id": "m1", "text": "received whisper from player1: let's grind to 15", "ts": 1_004}
        ]},
        "goals.list": {"items": []},
    }.get(name, {})

    gate = TriageGate(harness_mcp=harness_mcp, memory_mcp=memory_mcp)
    last_state = TickState(bot_guid=1, last_tick_ms=1_000_000)
    result = await gate.evaluate(bot_guid=1, last_state=last_state, now_ms=1_005_000)
    assert result.should_decide is True
    assert result.reason == "fresh_chat"
    assert len(result.hot_inputs["fresh_chat"]) == 1


@pytest.mark.asyncio
async def test_brain_outcome_memory_does_not_trigger_fresh_chat(harness_mcp, memory_mcp):
    """Regression: brain_no_op outcome rows must NOT be treated as fresh player chat.

    The dispatcher writes brain_no_op memories on every no_op tick. The dense retrieval
    can match them on the 'chat OR whisper' query. Without the _is_player_chat filter
    the brain would loop on fresh_chat forever even with no real player interaction.
    """
    harness_mcp.call.side_effect = lambda name, args: {
        "obs.get_state": {"in_combat": False, "level": 10, "xp_current": 100},
        "obs.get_combat_log": {"events": []},
    }.get(name, {})
    # Simulates a brain_no_op row returned by dense search
    memory_mcp.call.side_effect = lambda name, args: {
        "memory.search": {"items": [
            {"id": "m2",
             "text": "brain_no_op: No relevant information or goals to act upon.",
             "ts": 1_004}
        ]},
        "goals.list": {"items": []},
    }.get(name, {})

    gate = TriageGate(harness_mcp=harness_mcp, memory_mcp=memory_mcp)
    last_state = TickState(bot_guid=1, last_tick_ms=1_000_000)
    result = await gate.evaluate(bot_guid=1, last_state=last_state, now_ms=1_005_000)
    # brain_no_op rows must NOT trigger fresh_chat; no other events → no_change
    assert result.should_decide is False
    assert result.reason == "no_change"


@pytest.mark.asyncio
async def test_combat_event_fires(harness_mcp, memory_mcp):
    harness_mcp.call.side_effect = lambda name, args: {
        "obs.get_state": {"in_combat": True, "level": 10, "xp_current": 100},
        "obs.get_combat_log": {"events": [{"kind": "enter_combat", "ts_ms": 1_004_500}]},
    }.get(name, {})
    memory_mcp.call.side_effect = lambda name, args: {
        "memory.search": {"items": []},
        "goals.list": {"items": []},
    }.get(name, {})

    gate = TriageGate(harness_mcp=harness_mcp, memory_mcp=memory_mcp)
    last_state = TickState(bot_guid=1, last_tick_ms=1_000_000)
    result = await gate.evaluate(bot_guid=1, last_state=last_state, now_ms=1_005_000)
    assert result.should_decide is True
    assert result.reason == "combat_event"


@pytest.mark.asyncio
async def test_memory_search_uses_since_ts_and_top_k(harness_mcp, memory_mcp):
    """Regression: triage must send since_ts (not since_ms) and top_k (not k) to memory.search."""
    harness_mcp.call.side_effect = lambda name, args: {
        "obs.get_state": {"in_combat": False, "level": 10, "xp_current": 100},
        "obs.get_combat_log": {"events": []},
    }.get(name, {})

    captured_args: dict = {}

    def _capture(name, args):
        if name == "memory.search":
            captured_args.update(args)
        return {"items": []}

    memory_mcp.call.side_effect = _capture

    gate = TriageGate(harness_mcp=harness_mcp, memory_mcp=memory_mcp)
    last_state = TickState(bot_guid=1, last_tick_ms=1_000_000)
    await gate.evaluate(bot_guid=1, last_state=last_state, now_ms=1_005_000)

    assert "since_ts" in captured_args, "memory.search must use since_ts not since_ms"
    assert "top_k" in captured_args, "memory.search must use top_k not k"
    assert "since_ms" not in captured_args, "since_ms must not be sent to memory.search"
    assert "k" not in captured_args, "'k' must not be sent to memory.search (use top_k)"
    # memory-sidecar stores created_ts in Unix seconds; last_tick_ms=1_000_000ms → 1000s
    assert captured_args["since_ts"] == 1000, (
        f"since_ts must be seconds (last_tick_ms // 1000), got {captured_args['since_ts']}"
    )


@pytest.mark.asyncio
async def test_triage_mcp_timeout_returns_no_fire(harness_mcp, memory_mcp):
    import asyncio
    async def hang(name, args):
        await asyncio.sleep(10)
    harness_mcp.call.side_effect = hang
    memory_mcp.call.return_value = {"items": []}

    gate = TriageGate(harness_mcp=harness_mcp, memory_mcp=memory_mcp, per_call_timeout_s=0.05)
    last_state = TickState(bot_guid=1, last_tick_ms=1_000_000)
    result = await gate.evaluate(bot_guid=1, last_state=last_state, now_ms=1_005_000)
    assert result.should_decide is False
    assert result.reason == "triage_timeout"


# ---------------------------------------------------------------------------
# _is_player_chat unit tests
# ---------------------------------------------------------------------------

def test_is_player_chat_accepts_whisper_text_field():
    """Live schema: 'text' field with 'received whisper from ...'."""
    assert _is_player_chat({"text": "received whisper from Bob: hey"}) is True


def test_is_player_chat_accepts_whisper_content_back_compat():
    """Back-compat: 'content' field with 'received whisper from ...'."""
    assert _is_player_chat({"content": "received whisper from Alice: hello"}) is True


def test_is_player_chat_rejects_brain_no_op():
    """brain_no_op rows must NOT be treated as player chat."""
    assert _is_player_chat({"text": "brain_no_op: No relevant information."}) is False


def test_is_player_chat_rejects_action_taken():
    """action_taken rows are brain outcomes, not player chat."""
    assert _is_player_chat({"text": "action_taken: bot.set_strategy — Player wants to grind"}) is False


def test_is_player_chat_rejects_empty():
    """Empty item returns False."""
    assert _is_player_chat({}) is False


def test_is_player_chat_accepts_chat_received_signal():
    """chat_received substring (from old memory_type convention) is accepted."""
    assert _is_player_chat({"text": "chat_received: hey come grind"}) is True


# V3.6.1 hotfix: state_summary must be in hot_inputs for at_cap derivation
@pytest.mark.asyncio
async def test_v36_state_summary_populated_in_fresh_chat_hot_inputs(harness_mcp, memory_mcp):
    """V3.6.1: triage must populate hot_inputs['state_summary'] so decide.py can
    derive at_cap from self.level. Without this fix, at_cap is always False in
    production even for L25 bots (V3.6 ship bug 2026-05-22)."""
    obs_state_payload = {"self": {"level": 25, "name": "Casmina"}, "location": {}}
    harness_mcp.call.side_effect = lambda name, args: {
        "obs.get_state": obs_state_payload,
        "obs.get_combat_log": {"events": []},
    }.get(name, {})
    memory_mcp.call.side_effect = lambda name, args: {
        "memory.search": {"items": [
            {"id": "m1", "text": "received whisper from player: hello", "ts": 1_004}
        ]},
        "goals.list": {"items": []},
    }.get(name, {})

    gate = TriageGate(harness_mcp=harness_mcp, memory_mcp=memory_mcp)
    last_state = TickState(bot_guid=1003, last_tick_ms=1_000_000)
    result = await gate.evaluate(bot_guid=1003, last_state=last_state, now_ms=1_005_000)
    assert result.should_decide is True
    assert result.reason == "fresh_chat"
    # The critical V3.6.1 fix: state_summary in hot_inputs
    assert "state_summary" in result.hot_inputs
    assert result.hot_inputs["state_summary"]["self"]["level"] == 25


# V3.7: organic_wakeup branch
@pytest.mark.asyncio
async def test_v37_organic_wakeup_fires_when_timer_expired(harness_mcp, memory_mcp):
    """When now_ms >= next_wakeup_at_ms, triage fires with reason=organic_wakeup."""
    obs_state_payload = {"self": {"level": 25, "name": "Casmina"}, "location": {}}
    harness_mcp.call.side_effect = lambda name, args: {
        "obs.get_state": obs_state_payload,
        "obs.get_combat_log": {"events": []},
    }.get(name, {})
    memory_mcp.call.side_effect = lambda name, args: {
        "memory.search": {"items": []},
        "goals.list": {"items": []},
    }.get(name, {})

    gate = TriageGate(harness_mcp=harness_mcp, memory_mcp=memory_mcp)
    last_state = TickState(bot_guid=1003, last_tick_ms=1_000_000)
    # next_wakeup_at_ms is in the past — fire
    result = await gate.evaluate(
        bot_guid=1003, last_state=last_state, now_ms=1_005_000,
        next_wakeup_at_ms=1_004_000,
    )
    assert result.should_decide is True
    assert result.reason == "organic_wakeup"
    assert result.hot_inputs["state_summary"]["self"]["level"] == 25


@pytest.mark.asyncio
async def test_v37_organic_wakeup_silent_when_timer_not_expired(harness_mcp, memory_mcp):
    """When now_ms < next_wakeup_at_ms, triage returns no_change."""
    harness_mcp.call.side_effect = lambda name, args: {
        "obs.get_state": {"self": {"level": 10}},
        "obs.get_combat_log": {"events": []},
    }.get(name, {})
    memory_mcp.call.side_effect = lambda name, args: {
        "memory.search": {"items": []},
        "goals.list": {"items": []},
    }.get(name, {})

    gate = TriageGate(harness_mcp=harness_mcp, memory_mcp=memory_mcp)
    last_state = TickState(bot_guid=1003, last_tick_ms=1_000_000)
    # next_wakeup_at_ms is in the future — silent
    result = await gate.evaluate(
        bot_guid=1003, last_state=last_state, now_ms=1_005_000,
        next_wakeup_at_ms=1_010_000,
    )
    assert result.should_decide is False
    assert result.reason == "no_change"


@pytest.mark.asyncio
async def test_v37_organic_wakeup_silent_when_param_is_none(harness_mcp, memory_mcp):
    """When next_wakeup_at_ms is None (V3.7 not yet wired or first tick already handled),
    triage falls through to no_change rather than firing organic_wakeup."""
    harness_mcp.call.side_effect = lambda name, args: {
        "obs.get_state": {"self": {"level": 10}},
        "obs.get_combat_log": {"events": []},
    }.get(name, {})
    memory_mcp.call.side_effect = lambda name, args: {
        "memory.search": {"items": []},
        "goals.list": {"items": []},
    }.get(name, {})

    gate = TriageGate(harness_mcp=harness_mcp, memory_mcp=memory_mcp)
    last_state = TickState(bot_guid=1003, last_tick_ms=1_000_000)
    result = await gate.evaluate(
        bot_guid=1003, last_state=last_state, now_ms=1_005_000,
        # next_wakeup_at_ms omitted (None)
    )
    assert result.should_decide is False
    assert result.reason == "no_change"


@pytest.mark.asyncio
async def test_v37_organic_wakeup_loses_to_reactive_fresh_chat(harness_mcp, memory_mcp):
    """When BOTH a fresh_chat row AND an expired organic timer are present,
    triage fires fresh_chat (organic is lowest priority above no_change)."""
    harness_mcp.call.side_effect = lambda name, args: {
        "obs.get_state": {"self": {"level": 10}},
        "obs.get_combat_log": {"events": []},
    }.get(name, {})
    memory_mcp.call.side_effect = lambda name, args: {
        "memory.search": {"items": [
            {"id": "m1", "text": "received whisper from player1: hello", "ts": 1_004}
        ]},
        "goals.list": {"items": []},
    }.get(name, {})

    gate = TriageGate(harness_mcp=harness_mcp, memory_mcp=memory_mcp)
    last_state = TickState(bot_guid=1003, last_tick_ms=1_000_000)
    result = await gate.evaluate(
        bot_guid=1003, last_state=last_state, now_ms=1_005_000,
        next_wakeup_at_ms=1_004_000,  # expired
    )
    # Fresh chat wins
    assert result.should_decide is True
    assert result.reason == "fresh_chat"
