"""Unit test: McpClient.list_tools uses a per-call session (v0.2.6 reconnecting design)."""
from __future__ import annotations

from contextlib import asynccontextmanager
from unittest.mock import AsyncMock, MagicMock, patch

import pytest

from brain_sidecar.mcp_clients import McpClient


@pytest.mark.asyncio
async def test_list_tools_returns_tools_from_fresh_session():
    """McpClient.list_tools opens a fresh session and returns tool list."""
    tool_a = MagicMock()
    tool_a.name = "bot.follow"

    list_result = MagicMock()
    list_result.tools = [tool_a]

    # Mock the session that ClientSession() returns
    fake_session = MagicMock()
    fake_session.initialize = AsyncMock(return_value=None)
    fake_session.list_tools = AsyncMock(return_value=list_result)
    fake_session.__aenter__ = AsyncMock(return_value=fake_session)
    fake_session.__aexit__ = AsyncMock(return_value=False)

    # Mock streamablehttp_client to yield (read, write, None)
    fake_read, fake_write = MagicMock(), MagicMock()

    @asynccontextmanager
    async def _fake_transport(url, headers=None):
        yield fake_read, fake_write, None

    with patch("brain_sidecar.mcp_clients.streamablehttp_client", _fake_transport), \
         patch("brain_sidecar.mcp_clients.ClientSession", return_value=fake_session):
        client = McpClient(url="http://x/mcp", bearer="b")
        tools = await client.list_tools()

    assert len(tools) == 1
    assert tools[0].name == "bot.follow"
    fake_session.list_tools.assert_awaited_once()
