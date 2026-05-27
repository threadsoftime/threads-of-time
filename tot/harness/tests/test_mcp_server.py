"""Unit tests for build_mcp_server and per-tool registration."""

from __future__ import annotations

from unittest.mock import AsyncMock

import pytest

from harness_daemon.auth import TokenStore
from harness_daemon.config import TokenRecord
from harness_daemon.mcp_server import build_mcp_server
from harness_daemon.registry import build_v1_registry
from harness_daemon.tool_schemas import TOOL_SCHEMAS


@pytest.mark.asyncio
async def test_build_mcp_server_registers_every_v1_tool() -> None:
    store = TokenStore([TokenRecord(token="t", identity="i", scope=["*"])])
    registry = build_v1_registry()

    async def fake_dispatch(*, name, args, auth, request_id, **_kw):
        from harness_daemon.app import DispatchOutcome
        return DispatchOutcome(
            status=200,
            body={"ok": True, "result": {"echoed": {"name": name, "args": args}}},
            audit_outcome="ok",
        )

    mcp = build_mcp_server(
        token_store=store,
        registry=registry,
        dispatch_fn=fake_dispatch,
        audit=None,  # T7 covers the audit path; here we just check shape
    )

    # FastMCP exposes registered tools via `list_tools()` async API.
    tools = await mcp.list_tools()
    names = {t.name for t in tools}
    assert names == set(TOOL_SCHEMAS.keys()), (
        f"missing={set(TOOL_SCHEMAS) - names}, "
        f"extra={names - set(TOOL_SCHEMAS)}"
    )
    # Every tool must have a non-empty description.
    for t in tools:
        assert t.description, f"{t.name} missing description"


def test_tool_schemas_covers_full_registry() -> None:
    """TOOL_SCHEMAS must enumerate every name in the V1 registry."""
    registry_names = set(build_v1_registry().names())
    schema_names = set(TOOL_SCHEMAS.keys())
    assert registry_names == schema_names, (
        f"registry-only={registry_names - schema_names}, "
        f"schema-only={schema_names - registry_names}"
    )
