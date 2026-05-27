"""Session-scoped fixtures for contract tests.

Bearers are loaded from env (HARNESS_BEARER, MEMORY_BEARER). On laptop, export
before running `pytest -m contract`. On Heimdal, source from /etc/containers/
systemd/brain-sidecar.env before invoking build.sh.

Missing bearers → pytest.skip (no false reds on fresh checkout).
"""
from __future__ import annotations

import os
from collections.abc import AsyncIterator

import pytest
import pytest_asyncio

from brain_sidecar.mcp_clients import McpClient, open_mcp


@pytest.fixture(scope="session")
def harness_url() -> str:
    return os.environ.get("HARNESS_MCP_URL", "http://192.168.1.3:8099/mcp/mcp")


@pytest.fixture(scope="session")
def memory_url() -> str:
    return os.environ.get("MEMORY_MCP_URL", "http://192.168.1.3:8090/mcp/mcp")


@pytest.fixture(scope="session")
def harness_bearer() -> str:
    val = os.environ.get("HARNESS_BEARER")
    if not val:
        pytest.skip("HARNESS_BEARER not set; contract tests require live MCPs")
    return val


@pytest.fixture(scope="session")
def memory_bearer() -> str:
    val = os.environ.get("MEMORY_BEARER")
    if not val:
        pytest.skip("MEMORY_BEARER not set; contract tests require live MCPs")
    return val


@pytest_asyncio.fixture(scope="session", loop_scope="session")
async def harness_client(harness_url, harness_bearer) -> AsyncIterator[McpClient]:
    """Open one MCP session per pytest run; close at session end.

    Session scope avoids the anyio cancel-scope task-mismatch error that
    pytest-asyncio raises on per-function teardown of MCP context managers.
    """
    async with open_mcp(harness_url, harness_bearer) as client:
        yield client


@pytest_asyncio.fixture(scope="session", loop_scope="session")
async def memory_client(memory_url, memory_bearer) -> AsyncIterator[McpClient]:
    async with open_mcp(memory_url, memory_bearer) as client:
        yield client


@pytest_asyncio.fixture(scope="session", loop_scope="session")
async def harness_tools(harness_client) -> dict:
    """Return {tool_name: tool_obj} from live harness MCP. Cached for session."""
    tools = await harness_client.list_tools()
    return {t.name: t for t in tools}


@pytest_asyncio.fixture(scope="session", loop_scope="session")
async def memory_tools(memory_client) -> dict:
    """Return {tool_name: tool_obj} from live memory MCP. Cached for session."""
    tools = await memory_client.list_tools()
    return {t.name: t for t in tools}
