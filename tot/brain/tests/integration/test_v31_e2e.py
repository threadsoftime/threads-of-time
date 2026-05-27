"""V3.1 e2e: schema_builder → Decider → grammar-conformant LLM → dispatch."""
from __future__ import annotations

from unittest.mock import AsyncMock, MagicMock

import pytest

from brain_sidecar.decide import Decider
from brain_sidecar.models import PersonalityCard
from brain_sidecar.schema_builder import compose_oneof, fetch_schemas, render_prompt_summary
from .mocks import FakeLlm, FakeMcp


def _mock_tool(name, description, args_schema):
    t = MagicMock()
    t.name = name
    t.description = description
    t.inputSchema = {"type": "object",
                     "properties": {"args": args_schema},
                     "required": ["args"]}
    return t


@pytest.mark.asyncio
async def test_v31_end_to_end_bot_follow_dispatch_executes():
    # 1. Stub MCP list_tools to expose bot.follow with the V1.4 schema.
    follow_schema = {"type": "object",
                     "properties": {"bot_guid": {"type": "integer"},
                                    "leader_guid": {"type": "integer"}},
                     "required": ["bot_guid", "leader_guid"]}
    harness_lt = AsyncMock(return_value=[_mock_tool("bot.follow", "follow", follow_schema)])
    memory_lt = AsyncMock(return_value=[])
    harness_for_lt = MagicMock(list_tools=harness_lt)
    memory_for_lt = MagicMock(list_tools=memory_lt)

    per_tool = await fetch_schemas(harness_for_lt, memory_for_lt)
    schema = compose_oneof(per_tool)
    summary = render_prompt_summary(per_tool)

    # 2. Wire a Decider with the new schema + summary.
    template = "{tools_summary}{state_json}{goals_json}{memories_json}{recent_decisions_json}{hot_inputs_json}"
    llm = FakeLlm()
    llm.response_json = {"kind": "action", "tool": "bot.follow",
                         "args": {"bot_guid": 2562, "leader_guid": 1},
                         "confidence": 0.8, "reasoning": "follow request"}
    decider = Decider(
        llm_client=llm,
        personality_cache=_StubPersonality(),
        memory_mcp=FakeMcp(),
        state_store=_StubStateStore(),
        prompt_template=template,
        decision_schema=schema,
        tools_summary=summary,
    )

    decision, _, _2 = await decider.decide(bot_guid=2562, hot_inputs={"fresh_chat": []})

    # 3. Verify decision shape + schema was forwarded.
    assert decision.kind.value == "action"
    assert decision.tool == "bot.follow"
    assert decision.args == {"bot_guid": 2562, "leader_guid": 1}
    assert llm.last_json_schema is schema


class _StubPersonality:
    async def get(self, bot_guid):
        return PersonalityCard(
            name="Casmina", race="Human", **{"class": "Paladin"},
            backstory="bs", talkativeness=0.5, courage=0.5, greed=0.3, attitude_to_master=0.5,
        )


class _StubStateStore:
    def decisions_recent(self, *, bot_guid, k):
        return []
