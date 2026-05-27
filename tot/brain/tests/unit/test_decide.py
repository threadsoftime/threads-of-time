"""Tests for the decider (C4)."""
from __future__ import annotations

from unittest.mock import AsyncMock, MagicMock

import pytest

from brain_sidecar.decide import Decider, KNOWN_TOOLS, _MAX_CONTENT_CHARS
from brain_sidecar.models import Decision, DecisionKind, PersonalityCard


def _make_card() -> PersonalityCard:
    return PersonalityCard(
        name="Caedon", race="Dwarf", **{"class": "Hunter"},
        backstory="A gruff old veteran.",
        talkativeness=0.4, courage=0.8, greed=0.3, attitude_to_master=0.6,
    )


# Minimal template using the 6 game-state placeholders only (no {personality_json}).
_TEMPLATE = (
    "tools={tools_summary} state={state_json} goals={goals_json} "
    "memories={memories_json} recent={recent_decisions_json} hot={hot_inputs_json}"
)


@pytest.fixture
def llm_client():
    return AsyncMock()


@pytest.fixture
def personality_cache():
    cache = AsyncMock()
    cache.get.return_value = _make_card()
    return cache


@pytest.fixture
def memory_mcp():
    m = AsyncMock()
    m.call.return_value = {"items": []}
    return m


@pytest.fixture
def state_store():
    s = MagicMock()
    s.decisions_recent.return_value = []
    return s


@pytest.mark.asyncio
async def test_decider_returns_action_on_valid_llm_response(
    llm_client, personality_cache, memory_mcp, state_store,
):
    llm_client.chat_completion_json.return_value = (
        {
            "kind": "action",
            "tool": "bot.set_strategy",
            "args": {"bot_guid": 1, "add": ["grind"]},
            "confidence": 0.85,
            "reasoning": "Player wants to grind",
        },
        '<raw>',
        2400.0,
    )
    d = Decider(
        llm_client=llm_client, personality_cache=personality_cache,
        memory_mcp=memory_mcp, state_store=state_store,
        prompt_template=_TEMPLATE,
    )
    decision, _latency, _ = await d.decide(bot_guid=1, hot_inputs={"fresh_chat": []})
    assert decision.kind is DecisionKind.ACTION
    assert decision.tool == "bot.set_strategy"
    assert decision.confidence == pytest.approx(0.85)


@pytest.mark.asyncio
async def test_decider_unparseable_returns_no_op_after_retry(
    llm_client, personality_cache, memory_mcp, state_store,
):
    llm_client.chat_completion_json.return_value = (None, "not json", 100.0)
    d = Decider(
        llm_client=llm_client, personality_cache=personality_cache,
        memory_mcp=memory_mcp, state_store=state_store,
        prompt_template=_TEMPLATE,
    )
    decision, _latency, _ = await d.decide(bot_guid=1, hot_inputs={})
    assert decision.kind is DecisionKind.NO_OP
    assert "llm_unparseable" in decision.reasoning
    # one initial + one retry = 2 calls
    assert llm_client.chat_completion_json.await_count == 2


@pytest.mark.asyncio
async def test_decider_passes_unknown_tool_through_to_dispatcher(
    llm_client, personality_cache, memory_mcp, state_store,
):
    """C4 must NOT demote unknown tools — that belongs in C5 (spec §5.1).

    C4 should return the ACTION decision with the unknown tool name intact
    so C5 can write an observability event and drop it there.
    """
    llm_client.chat_completion_json.return_value = (
        {
            "kind": "action",
            "tool": "bot.do_a_backflip",  # not in KNOWN_TOOLS
            "args": {"bot_guid": 1},
            "confidence": 0.9,
            "reasoning": "looked fun",
        },
        '<raw>',
        100.0,
    )
    d = Decider(
        llm_client=llm_client, personality_cache=personality_cache,
        memory_mcp=memory_mcp, state_store=state_store,
        prompt_template=_TEMPLATE,
    )
    decision, _latency, _ = await d.decide(bot_guid=1, hot_inputs={})
    # C4 passes it through: kind is ACTION, tool is the unknown value
    assert decision.kind is DecisionKind.ACTION
    assert decision.tool == "bot.do_a_backflip"


def test_known_tools_includes_v14_set():
    expected = {
        "bot.set_strategy", "bot.get_strategies", "bot.send_chat",
        "bot.follow", "bot.stop", "bot.set_goal",
        "memory.write", "memory_write",
        "memory.goals.create", "goals_create",
        "memory.recall_about", "memory_recall_about",
        "memory.personality.get", "memory.personality_get",
        "goals.list", "goals.read", "goals.update",
    }
    for t in expected:
        assert t in KNOWN_TOOLS, f"missing known tool: {t}"


# ---------------------------------------------------------------------------
# New tests for review issues (Critical 1, Critical 2, Important 5)
# ---------------------------------------------------------------------------

def test_assemble_prompt_persona_in_system_not_user(personality_cache, memory_mcp, state_store):
    """Critical 1 — persona fields must appear in 'system', NOT in 'user'."""
    card = _make_card()
    d = Decider(
        llm_client=AsyncMock(), personality_cache=personality_cache,
        memory_mcp=memory_mcp, state_store=state_store,
        prompt_template=_TEMPLATE,
    )
    prompt = d._assemble_prompt(
        personality=card,
        state={},
        goals=[],
        memories=[],
        recent_decisions=[],
        hot_inputs={},
        bot_guid=1,
    )
    # Persona identifiers must be in system
    assert "Caedon" in prompt["system"]
    assert "Dwarf" in prompt["system"]
    assert "Hunter" in prompt["system"]
    assert "A gruff old veteran" in prompt["system"]
    # JSON-only constraint must be in system
    assert "ONLY" in prompt["system"]

    # Persona fields must NOT appear in user
    assert "Caedon" not in prompt["user"]
    assert "Dwarf" not in prompt["user"]
    assert "A gruff old veteran" not in prompt["user"]


def test_assemble_prompt_truncates_long_memory_content(personality_cache, memory_mcp, state_store):
    """Critical 2 — memory items with content > 300 chars must be clipped with the ellipsis marker.

    json.dumps with default ensure_ascii=True encodes '…' (U+2026) as '\\u2026', so
    we assert for that escaped form in the serialised user string.
    Back-compat: 'content' field (old fixtures) still clipped.
    """
    long_content = "x" * 1000
    memories = [{"id": "m1", "content": long_content, "relevance": 0.9}]
    card = _make_card()
    d = Decider(
        llm_client=AsyncMock(), personality_cache=personality_cache,
        memory_mcp=memory_mcp, state_store=state_store,
        prompt_template=_TEMPLATE,
    )
    prompt = d._assemble_prompt(
        personality=card,
        state={},
        goals=[],
        memories=memories,
        recent_decisions=[],
        hot_inputs={},
        bot_guid=1,
    )
    # The full 1000-char string must not appear verbatim
    assert long_content not in prompt["user"]
    # json.dumps with ensure_ascii=True encodes '…' (U+2026) as the literal
    # 6-character sequence … in the output string.
    assert "\\u2026" in prompt["user"]
    # The clipped portion (300 chars of 'x') must be there, but not 301+
    assert "x" * _MAX_CONTENT_CHARS in prompt["user"]
    assert "x" * (_MAX_CONTENT_CHARS + 1) not in prompt["user"]


def test_assemble_prompt_truncates_long_memory_text_field(personality_cache, memory_mcp, state_store):
    """Regression — live memory MCP uses 'text' not 'content'. _truncate_memory_items must clip both."""
    long_text = "y" * 1000
    memories = [{"id": "m2", "text": long_text, "salience": 0.8}]
    card = _make_card()
    d = Decider(
        llm_client=AsyncMock(), personality_cache=personality_cache,
        memory_mcp=memory_mcp, state_store=state_store,
        prompt_template=_TEMPLATE,
    )
    prompt = d._assemble_prompt(
        personality=card,
        state={},
        goals=[],
        memories=memories,
        recent_decisions=[],
        hot_inputs={},
        bot_guid=1,
    )
    assert long_text not in prompt["user"]
    assert "\\u2026" in prompt["user"]
    assert "y" * _MAX_CONTENT_CHARS in prompt["user"]
    assert "y" * (_MAX_CONTENT_CHARS + 1) not in prompt["user"]


@pytest.mark.asyncio
async def test_gather_memories_extracts_sender_from_whisper_content(memory_mcp, state_store):
    """Important 5 — regex fallback must work when fresh_chat has no 'from'/'sender' field."""
    card = _make_card()
    personality_cache = AsyncMock()
    personality_cache.get.return_value = card

    # Simulate T3-style whisper memory: no structured sender, sender embedded in content
    hot_inputs = {
        "fresh_chat": [
            {"content": "received whisper from Alice: hey want to group?"}
        ]
    }

    captured_entities: list[str] = []

    async def fake_call(method, params):
        if method == "memory.recall_about":
            # Live schema: entity is a single string, not a list
            entity = params.get("entity", "")
            if entity:
                captured_entities.append(entity)
        return {"items": []}

    memory_mcp.call.side_effect = fake_call

    d = Decider(
        llm_client=AsyncMock(), personality_cache=personality_cache,
        memory_mcp=memory_mcp, state_store=state_store,
        prompt_template=_TEMPLATE,
    )
    await d._gather_memories(bot_guid=1, hot_inputs=hot_inputs)

    assert "Alice" in captured_entities, (
        "Expected 'Alice' to be extracted via regex from whisper content and sent as entity: str"
    )


@pytest.mark.asyncio
async def test_decide_extracts_message_from_text_field():
    """Regression for post-deploy compound bug (v0.1.6).

    Memory MCP items use 'text' field, not 'content'. The brain was reading
    'content' (always empty for live items) so the sender was never extracted,
    memory.recall_about was never called, and the LLM always saw empty context.

    This test verifies:
    - Sender is extracted via regex from items using the 'text' field.
    - _project_hot_inputs produces a non-empty 'message' for 'text'-field items.
    """
    card = _make_card()
    personality_cache = AsyncMock()
    personality_cache.get.return_value = card
    state_store = MagicMock()
    state_store.decisions_recent.return_value = []

    # Live schema: memory item uses "text", no "content" field
    hot_inputs = {
        "fresh_chat": [
            {"text": "received whisper from Bob: let's grind to 25 tonight"}
        ]
    }

    captured_entities: list[str] = []

    async def fake_call(method, params):
        if method == "memory.recall_about":
            entity = params.get("entity", "")
            if entity:
                captured_entities.append(entity)
        elif method == "goals.list":
            return {"items": []}
        return {"items": []}

    memory_mcp = AsyncMock()
    memory_mcp.call.side_effect = fake_call

    llm_response = {
        "kind": "no_op", "tool": None, "args": None,
        "confidence": 0.1, "reasoning": "test",
    }
    llm_client = AsyncMock()
    llm_client.chat_completion_json.return_value = (llm_response, "<raw>", 100.0)

    d = Decider(
        llm_client=llm_client, personality_cache=personality_cache,
        memory_mcp=memory_mcp, state_store=state_store,
        prompt_template=_TEMPLATE,
    )

    # Verify sender extraction via _gather_memories
    await d._gather_memories(bot_guid=1, hot_inputs=hot_inputs)
    assert "Bob" in captured_entities, (
        "Expected 'Bob' extracted via regex from 'text' field and sent as entity"
    )

    # Verify _project_hot_inputs populates message from 'text' field
    projected = d._project_hot_inputs(hot_inputs)
    chat_items = projected.get("fresh_chat", [])
    assert len(chat_items) == 1
    assert "grind" in chat_items[0]["message"], (
        "Expected message to be populated from 'text' field, not empty string"
    )
    assert chat_items[0]["sender"] == "Bob"


@pytest.mark.asyncio
async def test_gather_memories_back_compat_content_field():
    """Back-compat: 'content' field still works for pre-migration fixtures."""
    card = _make_card()
    personality_cache = AsyncMock()
    personality_cache.get.return_value = card
    state_store = MagicMock()
    state_store.decisions_recent.return_value = []

    # Old schema: memory item uses "content"
    hot_inputs = {
        "fresh_chat": [
            {"content": "received whisper from Alice: hey want to group?"}
        ]
    }

    captured_entities: list[str] = []

    async def fake_call(method, params):
        if method == "memory.recall_about":
            entity = params.get("entity", "")
            if entity:
                captured_entities.append(entity)
        return {"items": []}

    memory_mcp = AsyncMock()
    memory_mcp.call.side_effect = fake_call

    d = Decider(
        llm_client=AsyncMock(), personality_cache=personality_cache,
        memory_mcp=memory_mcp, state_store=state_store,
        prompt_template=_TEMPLATE,
    )
    await d._gather_memories(bot_guid=1, hot_inputs=hot_inputs)
    assert "Alice" in captured_entities, (
        "Back-compat: 'Alice' should still be extracted from legacy 'content' field"
    )


@pytest.mark.asyncio
async def test_gather_memories_loop_nesting_all_entities_recalled():
    """Regression for Bug 2 — _gather_memories must recall for EVERY entity, not just the last.

    Before the fix, the for-item loop was outside the for-ent loop (indentation bug),
    so only the last entity's recall results were merged. This test places two distinct
    senders in fresh_chat and verifies both get passed to memory.recall_about.
    """
    card = _make_card()
    personality_cache = AsyncMock()
    personality_cache.get.return_value = card
    state_store = MagicMock()
    state_store.decisions_recent.return_value = []

    hot_inputs = {
        "fresh_chat": [
            {"text": "received whisper from Carol: help me"},
            {"text": "received whisper from Dave: let's party"},
        ]
    }

    recalled_entities: list[str] = []

    async def fake_call(method, params):
        if method == "memory.recall_about":
            entity = params.get("entity", "")
            recalled_entities.append(entity)
            # Each entity has one distinct item
            return {"items": [{"id": f"item-{entity}", "text": f"memory of {entity}"}]}
        return {"items": []}

    memory_mcp = AsyncMock()
    memory_mcp.call.side_effect = fake_call

    d = Decider(
        llm_client=AsyncMock(), personality_cache=personality_cache,
        memory_mcp=memory_mcp, state_store=state_store,
        prompt_template=_TEMPLATE,
    )
    merged = await d._gather_memories(bot_guid=1, hot_inputs=hot_inputs)

    assert "Carol" in recalled_entities, "Carol should be recalled (was lost before fix)"
    assert "Dave" in recalled_entities, "Dave should be recalled"
    # Both items should be in merged (before fix: only Dave's item would appear)
    item_ids = [m.get("id") for m in merged]
    assert "item-Carol" in item_ids, "Carol's memory item must appear in merged result"
    assert "item-Dave" in item_ids, "Dave's memory item must appear in merged result"


@pytest.mark.asyncio
async def test_decider_surfaces_llm_latency_ms(
    llm_client, personality_cache, memory_mcp, state_store,
):
    """I3 — Decider.decide() must propagate llm_latency_ms from LlmClient as the second tuple element."""
    llm_client.chat_completion_json.return_value = (
        {
            "kind": "no_op",
            "tool": None,
            "args": None,
            "confidence": 0.1,
            "reasoning": "nothing to do",
        },
        "<raw>",
        2418.0,
    )
    d = Decider(
        llm_client=llm_client, personality_cache=personality_cache,
        memory_mcp=memory_mcp, state_store=state_store,
        prompt_template=_TEMPLATE,
    )
    decision, latency, _ = await d.decide(bot_guid=1, hot_inputs={})
    assert decision.kind is DecisionKind.NO_OP
    assert latency == pytest.approx(2418.0), (
        f"Expected latency 2418.0 forwarded from LlmClient mock; got {latency}"
    )


# V3.6: at_cap is returned alongside Decision + latency
@pytest.mark.asyncio
async def test_decide_returns_at_cap_true_when_level_at_cap():
    """decide() returns a triple including at_cap=True when bot is at cap."""
    from dataclasses import dataclass as _dc
    from unittest.mock import AsyncMock as _AM

    @_dc
    class _StubSettings:
        max_player_level: int = 25

    llm = _AM()
    llm.chat_completion_json.return_value = (
        {"kind": "no_op", "tool": None, "args": None, "confidence": 0.5,
         "reasoning": "idle"},
        "raw", 10.0,
    )
    pc = _AM()
    pc.get.return_value = PersonalityCard.model_validate({
        "name": "X", "race": "Y", "class": "Z", "backstory": "x",
        "talkativeness": 0.5, "courage": 0.5, "greed": 0.5,
        "attitude_to_master": 0.0,
        "pvp_appetite": 0.5, "raid_appetite": 0.5,
        "completionist_streak": 0.5, "gold_motivation": 0.5,
        "profession_appetite": 0.5,
    })
    mem = _AM()
    mem.call.return_value = {"items": []}
    ss = _AM()
    ss.decisions_recent = lambda *, bot_guid, k: []

    d = Decider(
        llm_client=llm, personality_cache=pc, memory_mcp=mem, state_store=ss,
        prompt_template="{state_json}{goals_json}{memories_json}{recent_decisions_json}{hot_inputs_json}{tools_summary}",
        decision_schema={"type": "object"},
        tools_summary="[]",
    )
    d.settings = _StubSettings()

    result = await d.decide(
        bot_guid=1003,
        hot_inputs={"state_summary": {"self": {"level": 25}}},
    )
    # V3.6 contract: decide() returns triple (Decision, latency_ms, at_cap)
    assert isinstance(result, tuple)
    assert len(result) == 3
    decision, latency, at_cap = result
    assert isinstance(decision, Decision)
    assert at_cap is True


@pytest.mark.asyncio
async def test_decide_returns_at_cap_false_when_level_below_cap():
    """decide() returns at_cap=False when bot below cap."""
    from dataclasses import dataclass as _dc
    from unittest.mock import AsyncMock as _AM

    @_dc
    class _StubSettings:
        max_player_level: int = 25

    llm = _AM()
    llm.chat_completion_json.return_value = (
        {"kind": "no_op", "tool": None, "args": None, "confidence": 0.5,
         "reasoning": "idle"},
        "raw", 10.0,
    )
    pc = _AM()
    pc.get.return_value = PersonalityCard.model_validate({
        "name": "X", "race": "Y", "class": "Z", "backstory": "x",
        "talkativeness": 0.5, "courage": 0.5, "greed": 0.5,
        "attitude_to_master": 0.0,
        "pvp_appetite": 0.5, "raid_appetite": 0.5,
        "completionist_streak": 0.5, "gold_motivation": 0.5,
        "profession_appetite": 0.5,
    })
    mem = _AM()
    mem.call.return_value = {"items": []}
    ss = _AM()
    ss.decisions_recent = lambda *, bot_guid, k: []

    d = Decider(
        llm_client=llm, personality_cache=pc, memory_mcp=mem, state_store=ss,
        prompt_template="{state_json}{goals_json}{memories_json}{recent_decisions_json}{hot_inputs_json}{tools_summary}",
        decision_schema={"type": "object"},
        tools_summary="[]",
    )
    d.settings = _StubSettings()

    _decision, _latency, at_cap = await d.decide(
        bot_guid=1003,
        hot_inputs={"state_summary": {"self": {"level": 15}}},
    )
    assert at_cap is False


@pytest.mark.asyncio
async def test_v371_temperature_bumped_for_organic_wakeup():
    """V3.7.1 Gap #3b: organic_wakeup ticks use temperature=0.7; all other
    triage_reasons (fresh_chat, combat_event, first_tick, etc.) use 0.5.

    Verifies the temperature kwarg is forwarded to chat_completion_json
    based on triage_reason — not the personality or the bot_guid.
    """
    card = _make_card()
    personality_cache = AsyncMock()
    personality_cache.get.return_value = card
    state_store = MagicMock()
    state_store.decisions_recent.return_value = []
    memory_mcp = AsyncMock()
    memory_mcp.call.return_value = {"items": []}

    llm_response = {
        "kind": "no_op", "tool": None, "args": None,
        "confidence": 0.1, "reasoning": "test",
    }
    llm_client = AsyncMock()
    llm_client.chat_completion_json.return_value = (llm_response, "<raw>", 100.0)

    d = Decider(
        llm_client=llm_client, personality_cache=personality_cache,
        memory_mcp=memory_mcp, state_store=state_store,
        prompt_template=_TEMPLATE,
    )

    # Organic-wakeup tick: temperature must be 0.7
    await d.decide(bot_guid=1, hot_inputs={}, triage_reason="organic_wakeup")
    organic_kwargs = llm_client.chat_completion_json.await_args.kwargs
    assert organic_kwargs.get("temperature") == 0.7, (
        f"organic_wakeup tick must use temperature=0.7; got {organic_kwargs.get('temperature')}"
    )

    # Reactive tick (fresh_chat): temperature must be 0.5
    llm_client.chat_completion_json.reset_mock()
    llm_client.chat_completion_json.return_value = (llm_response, "<raw>", 100.0)
    await d.decide(bot_guid=1, hot_inputs={}, triage_reason="fresh_chat")
    reactive_kwargs = llm_client.chat_completion_json.await_args.kwargs
    assert reactive_kwargs.get("temperature") == 0.5, (
        f"fresh_chat tick must use temperature=0.5; got {reactive_kwargs.get('temperature')}"
    )

    # No triage_reason (back-compat / first_tick path): default to 0.5
    llm_client.chat_completion_json.reset_mock()
    llm_client.chat_completion_json.return_value = (llm_response, "<raw>", 100.0)
    await d.decide(bot_guid=1, hot_inputs={})
    default_kwargs = llm_client.chat_completion_json.await_args.kwargs
    assert default_kwargs.get("temperature") == 0.5, (
        f"unspecified triage_reason must default to temperature=0.5; got {default_kwargs.get('temperature')}"
    )
