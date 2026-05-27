"""Contract assertions: brain's hardcoded memory MCP call sites match live schemas.

Tests are async because they consume the async `memory_tools` fixture (which
opens an MCP session in the background).
"""
from __future__ import annotations

import pytest

from brain_sidecar.schema_builder import unwrap_fastmcp_args


def _required_and_props(tool):
    schema = unwrap_fastmcp_args(tool.inputSchema)
    return set(schema.get("required", [])), schema.get("properties", {})


@pytest.mark.contract
async def test_memory_search_call_site(memory_tools):
    # triage.py:92 — args = {bot_id, query, top_k, since_ts}
    tool = memory_tools["memory.search"]
    required, props = _required_and_props(tool)
    assert "bot_id" in required, \
        "memory.search lost bot_id (triage.py:92 will fail)"
    assert props["bot_id"]["type"] == "string"
    assert "query" in props
    assert "top_k" in props
    # since_ts is OPTIONAL and in Unix SECONDS (per kb_87a7eade working-style memory).
    assert "since_ts" in props


@pytest.mark.contract
async def test_memory_recall_about_call_site(memory_tools):
    # decide.py:191 — args = {bot_id, entity, top_k}
    tool = memory_tools["memory.recall_about"]
    required, props = _required_and_props(tool)
    assert "bot_id" in required, \
        "memory.recall_about lost bot_id (decide.py:_gather_memories will fail)"
    assert props["bot_id"]["type"] == "string"
    assert "entity" in required, \
        "memory.recall_about expects entity:str (single, not entities:list)"
    assert props["entity"]["type"] == "string"
    assert "top_k" in props


@pytest.mark.contract
async def test_memory_recall_call_site(memory_tools):
    # decide.py:203 — args = {bot_id, query, top_k}
    tool = memory_tools["memory.recall"]
    required, props = _required_and_props(tool)
    assert "bot_id" in required, \
        "memory.recall lost bot_id (decide.py:_gather_memories will fail)"
    assert "query" in props
    assert "top_k" in props


@pytest.mark.contract
async def test_memory_write_call_site(memory_tools):
    # dispatch.py:284 — args = {bot_id, text, salience, entities, relations}
    tool = memory_tools["memory.write"]
    required, props = _required_and_props(tool)
    assert "bot_id" in required, \
        "memory.write lost bot_id (dispatch.py:_write_outcome will fail)"
    assert "text" in required, \
        "memory.write text field renamed — restore 'text' or update dispatch.py"
    assert props["text"]["type"] == "string"
    assert "salience" in props
    assert "entities" in props
    assert "relations" in props


@pytest.mark.contract
async def test_memory_personality_get_call_site(memory_tools):
    # personality.py:51 — args = {bot_id}
    tool = memory_tools["memory.personality_get"]
    required, props = _required_and_props(tool)
    assert "bot_id" in required, \
        "memory.personality_get lost bot_id (personality.py:51 will fail)"
    assert props["bot_id"]["type"] == "string"


@pytest.mark.contract
async def test_memory_personality_set_call_site(memory_tools):
    # personality.py:66 — args = {bot_id, persona}
    tool = memory_tools["memory.personality_set"]
    required, props = _required_and_props(tool)
    assert "bot_id" in required, \
        "memory.personality_set lost bot_id (personality.py:66 will fail)"
    assert "persona" in required, \
        "memory.personality_set expects persona (JSON-encoded string), not card"
    # persona is a JSON-encoded string per kb_87a7eade working-style memory.
    assert props["persona"]["type"] == "string"


@pytest.mark.contract
async def test_goals_list_call_site(memory_tools):
    # decide.py:93 + triage.py:97 — args = {bot_id, status}
    tool = memory_tools["goals.list"]
    required, props = _required_and_props(tool)
    assert "bot_id" in required, \
        "goals.list lost bot_id (decide.py:93 and triage.py:97 will fail)"
    # status is optional in some impls; check it at least exists in props.
    assert "status" in props
