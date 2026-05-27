"""V3.1 unit tests for Decider: tools_summary, decision_schema passthrough."""
from __future__ import annotations

import pytest

from brain_sidecar.decide import Decider
from brain_sidecar.models import PersonalityCard


class _StubPersonality:
    async def get(self, bot_guid):
        return PersonalityCard(
            name="Casmina", race="Human", **{"class": "Paladin"},
            backstory="Test backstory.",
            talkativeness=0.6, courage=0.7, greed=0.3, attitude_to_master=0.5,
        )


class _StubMcp:
    async def call(self, tool, args):
        return {"result": {"items": []}}


class _StubStateStore:
    def decisions_recent(self, *, bot_guid, k):
        return []


@pytest.mark.asyncio
async def test_assemble_prompt_substitutes_tools_summary():
    template = "TOOLS:\n{tools_summary}\nSTATE: {state_json}\nGOALS: {goals_json}\nMEM: {memories_json}\nRECENT: {recent_decisions_json}\nHOT: {hot_inputs_json}\n"
    decider = Decider(
        llm_client=None,
        personality_cache=_StubPersonality(),
        memory_mcp=_StubMcp(),
        state_store=_StubStateStore(),
        prompt_template=template,
        decision_schema={"oneOf": []},
        tools_summary="  bot.follow(bot_guid: int, leader_guid: int)\n    — follow another player",
    )
    prompt = decider._assemble_prompt(
        personality=await _StubPersonality().get(1),
        state={"hp": 100},
        goals=[],
        memories=[],
        recent_decisions=[],
        hot_inputs={"fresh_chat": []},
        bot_guid=1,
    )
    assert "bot.follow(bot_guid: int, leader_guid: int)" in prompt["user"]
    assert "— follow another player" in prompt["user"]


class _RecordingLlm:
    def __init__(self):
        self.last_json_schema = None
        self.response = {"kind": "no_op", "tool": None, "args": None,
                         "confidence": 0.0, "reasoning": "n/a"}

    async def chat_completion_json(self, *, system, user, max_tokens=400, temperature=0.5,
                                   json_schema=None):
        import json as _json
        self.last_json_schema = json_schema
        return self.response, _json.dumps(self.response), 50.0


@pytest.mark.asyncio
async def test_decide_passes_decision_schema_to_llm_client():
    template = "{tools_summary}{state_json}{goals_json}{memories_json}{recent_decisions_json}{hot_inputs_json}"
    llm = _RecordingLlm()
    schema = {"oneOf": [{"title": "no_op", "type": "object",
                         "properties": {"kind": {"const": "no_op"}}, "required": ["kind"]}]}
    decider = Decider(
        llm_client=llm,
        personality_cache=_StubPersonality(),
        memory_mcp=_StubMcp(),
        state_store=_StubStateStore(),
        prompt_template=template,
        decision_schema=schema,
        tools_summary="  bot.follow(bot_guid: int)",
    )
    decision, latency_ms, _ = await decider.decide(bot_guid=1003, hot_inputs={"fresh_chat": []})
    assert llm.last_json_schema is schema
    assert decision.kind.value == "no_op"


@pytest.mark.asyncio
async def test_assemble_prompt_includes_bot_guid_in_system():
    """System prompt must tell the LLM its own bot_guid (V3.1 follow-up)."""
    template = "{tools_summary}{state_json}{goals_json}{memories_json}{recent_decisions_json}{hot_inputs_json}"
    decider = Decider(
        llm_client=None,
        personality_cache=_StubPersonality(),
        memory_mcp=_StubMcp(),
        state_store=_StubStateStore(),
        prompt_template=template,
        decision_schema={"oneOf": []},
        tools_summary="",
    )
    prompt = decider._assemble_prompt(
        personality=await _StubPersonality().get(2562),
        state={},
        goals=[],
        memories=[],
        recent_decisions=[],
        hot_inputs={"fresh_chat": []},
        bot_guid=2562,
    )
    assert "bot_guid is 2562" in prompt["system"], \
        f"system prompt missing bot_guid: {prompt['system'][:300]}"
    assert "2562" in prompt["system"]
