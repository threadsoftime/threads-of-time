"""Boot-wiring test: app.create_app must fetch_schemas at startup, fail-loud on error."""
from __future__ import annotations

from unittest.mock import AsyncMock, MagicMock, patch

import pytest


@pytest.mark.asyncio
async def test_lifespan_calls_fetch_schemas_and_passes_to_decider(monkeypatch, tmp_path):
    # Stub MCPs to expose list_tools.
    fake_harness_tool = MagicMock()
    fake_harness_tool.name = "bot.follow"
    fake_harness_tool.description = "follow"
    fake_harness_tool.inputSchema = {"type": "object",
                                     "properties": {"args": {"type": "object",
                                                              "properties": {"bot_guid": {"type": "integer"},
                                                                             "leader_guid": {"type": "integer"}},
                                                              "required": ["bot_guid", "leader_guid"]}},
                                     "required": ["args"]}

    fake_harness = MagicMock()
    fake_harness.list_tools = AsyncMock(return_value=[fake_harness_tool])
    fake_harness.call = AsyncMock(return_value={})
    fake_memory = MagicMock()
    fake_memory.list_tools = AsyncMock(return_value=[])
    fake_memory.call = AsyncMock(return_value={})

    class _StubCtx:
        def __init__(self, client):
            self._client = client
        async def __aenter__(self): return self._client
        async def __aexit__(self, *a): return False

    def stub_open_mcp(url, bearer):
        return _StubCtx(fake_harness) if "8099" in url else _StubCtx(fake_memory)

    monkeypatch.setenv("BRAIN_STATE_DB", str(tmp_path / "state.sqlite"))
    monkeypatch.setenv("BRAIN_DECISIONS_LOG", str(tmp_path / "decisions.jsonl"))
    monkeypatch.setenv("HARNESS_MCP_URL", "http://x:8099/mcp/mcp")
    monkeypatch.setenv("MEMORY_MCP_URL", "http://x:8090/mcp/mcp")
    monkeypatch.setenv("HARNESS_BEARER", "h")
    monkeypatch.setenv("MEMORY_BEARER", "m")

    async def _ok_validate(**kwargs):
        return None

    with patch("brain_sidecar.app.open_mcp", stub_open_mcp), \
         patch("brain_sidecar.app.validate_sse_endpoint_or_raise", _ok_validate):
        from brain_sidecar.app import create_app
        app = create_app()
        # Drive the lifespan manually to verify the decider gets the schema.
        async with app.router.lifespan_context(app):
            decider = app.state.supervisor.decider
            assert decider.decision_schema is not None
            titles = [b["title"] for b in decider.decision_schema["oneOf"]]
            assert "action:bot.follow" in titles
            assert decider.tools_summary is not None
            assert "bot.follow" in decider.tools_summary


@pytest.mark.asyncio
async def test_lifespan_exits_on_schema_fetch_failure(monkeypatch, tmp_path):
    fake_harness = MagicMock()
    fake_harness.list_tools = AsyncMock(side_effect=RuntimeError("MCP unreachable"))
    fake_memory = MagicMock()
    fake_memory.list_tools = AsyncMock(return_value=[])

    class _StubCtx:
        def __init__(self, client):
            self._client = client
        async def __aenter__(self): return self._client
        async def __aexit__(self, *a): return False

    def stub_open_mcp(url, bearer):
        return _StubCtx(fake_harness) if "8099" in url else _StubCtx(fake_memory)

    monkeypatch.setenv("BRAIN_STATE_DB", str(tmp_path / "state.sqlite"))
    monkeypatch.setenv("BRAIN_DECISIONS_LOG", str(tmp_path / "decisions.jsonl"))
    monkeypatch.setenv("HARNESS_MCP_URL", "http://x:8099/mcp/mcp")
    monkeypatch.setenv("MEMORY_MCP_URL", "http://x:8090/mcp/mcp")

    async def _ok_validate(**kwargs):
        return None

    with patch("brain_sidecar.app.open_mcp", stub_open_mcp), \
         patch("brain_sidecar.app.validate_sse_endpoint_or_raise", _ok_validate):
        from brain_sidecar.app import create_app
        app = create_app()
        with pytest.raises(SystemExit) as exc:
            async with app.router.lifespan_context(app):
                pass
        assert exc.value.code == 1
