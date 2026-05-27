"""Contract smoke: schema_builder.fetch_schemas works against live MCPs.

Each test opens its OWN fresh MCP connections — does NOT share the
session-scoped harness_client/memory_client fixtures. Reason: those
fixtures' background SSE reader gets stuck after the first list_tools()
call (anyio + pytest-asyncio session-loop interaction), so a second
list_tools() inside the same session hangs indefinitely. Fresh
connections per test avoid the deadlock entirely.
"""
from __future__ import annotations

import pytest

from brain_sidecar.mcp_clients import open_mcp
from brain_sidecar.schema_builder import compose_oneof, fetch_schemas, render_prompt_summary


@pytest.mark.contract
async def test_fetch_schemas_against_live_mcps(
    harness_url, harness_bearer, memory_url, memory_bearer
):
    async with open_mcp(harness_url, harness_bearer) as harness, \
               open_mcp(memory_url, memory_bearer) as memory:
        per_tool = await fetch_schemas(harness, memory)
    # Both MCPs are expected to expose tools.
    assert any(t.source_mcp == "harness" for t in per_tool.values()), \
        "no harness tools discovered — is harness MCP serving tools/list?"
    assert any(t.source_mcp == "memory" for t in per_tool.values()), \
        "no memory tools discovered — is memory MCP serving tools/list?"


@pytest.mark.contract
async def test_compose_oneof_produces_valid_union(
    harness_url, harness_bearer, memory_url, memory_bearer
):
    async with open_mcp(harness_url, harness_bearer) as harness, \
               open_mcp(memory_url, memory_bearer) as memory:
        per_tool = await fetch_schemas(harness, memory)
    union = compose_oneof(per_tool)
    assert "oneOf" in union
    titles = [b["title"] for b in union["oneOf"]]
    assert "no_op" in titles
    # At least one action branch for a known V1.4 harness tool.
    assert "action:bot.follow" in titles


@pytest.mark.contract
async def test_render_prompt_summary_against_live_tools(
    harness_url, harness_bearer, memory_url, memory_bearer
):
    async with open_mcp(harness_url, harness_bearer) as harness, \
               open_mcp(memory_url, memory_bearer) as memory:
        per_tool = await fetch_schemas(harness, memory)
    summary = render_prompt_summary(per_tool)
    # Sanity: at least one known signature appears in the rendered text.
    assert "bot.follow(" in summary
    assert "leader_guid: int" in summary
