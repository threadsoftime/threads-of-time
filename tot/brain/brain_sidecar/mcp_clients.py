"""Thin async wrappers around the heimdal-harness + heimdal-memory MCP clients.

Design note (v0.2.6): McpClient is now a reconnecting client — it opens a fresh
MCP session per call() invocation rather than holding one long-lived session.

Background: mcp 1.27.1's streamablehttp_client runs an internal anyio TaskGroup
that maintains a persistent GET SSE stream alongside POST requests. When that GET
stream disconnects (e.g. after an HTTP idle timeout, ~27 min in production), the
httpx.ReadError propagates through the TaskGroup as an ExceptionGroup that tears
down the entire streamablehttp_client context manager. Because the session was
held at lifespan scope, all subsequent harness/memory calls silently returned None
from _safe_call, causing 100% triage_timeout.

Fix: per-call sessions. Each call() opens, uses, and closes a fresh session.
Overhead: one HTTP round-trip per call (~5–10 ms at LAN speeds). At 5 s tick
intervals with 3 bots × 4 calls per tick, this adds ~60–120 ms/tick of extra
latency — acceptable vs the 5 s tick budget and much cheaper than the broken state.

list_tools() is called only at startup (schema_builder.fetch_schemas), so the
per-call cost there is immaterial.

open_mcp() is retained as a thin one-shot helper used only for startup probes
(SSE validation, schema fetch). It still opens a single-session context; callers
that need the reconnecting client should use McpClient(url, bearer) directly.
"""
from __future__ import annotations

import contextlib
import json as _json
from collections.abc import AsyncIterator
from dataclasses import dataclass
from typing import Any

from mcp.client.session import ClientSession
from mcp.client.streamable_http import streamablehttp_client


@dataclass
class McpClient:
    """Reconnecting MCP client — opens a fresh session per call().

    Holds only the URL and bearer; does NOT maintain a long-lived session.
    This makes the client resilient to HTTP idle-timeout disconnections that
    kill the underlying streamablehttp_client TaskGroup in mcp 1.27.1.
    """
    url: str
    bearer: str

    def _headers(self) -> dict[str, str]:
        return {"Authorization": f"Bearer {self.bearer}"} if self.bearer else {}

    async def call(self, tool_name: str, args: dict[str, Any]) -> dict[str, Any]:
        """Open a fresh MCP session, call tool_name with args, close the session.

        Both harness-daemon and memory-sidecar use FastMCP with a handler
        signature `_handler(ctx, args: SchemaModel)`. FastMCP registers the
        Python parameter name "args" as the sole top-level input property, so
        the MCP call arguments must be wrapped: {"args": <actual-params>}.
        """
        async with streamablehttp_client(self.url, headers=self._headers()) as (read, write, _):
            async with ClientSession(read, write) as session:
                await session.initialize()
                result = await session.call_tool(tool_name, arguments={"args": args})
        if result.isError:
            text = "".join(getattr(c, "text", "") for c in result.content)
            raise RuntimeError(f"MCP tool {tool_name} error: {text}")
        # Convention: harness/memory tools return a single JSON content block.
        for block in result.content:
            text = getattr(block, "text", None)
            if text is not None:
                return _json.loads(text)
        return {}

    async def list_tools(self) -> list[Any]:
        """Open a fresh session, list tools, close the session.

        Called only at startup by schema_builder.fetch_schemas(). Per-call
        overhead is immaterial at startup time.

        Each Tool has .name (str), .description (str | None), .inputSchema (dict).
        """
        async with streamablehttp_client(self.url, headers=self._headers()) as (read, write, _):
            async with ClientSession(read, write) as session:
                await session.initialize()
                result = await session.list_tools()
        return list(result.tools)


@contextlib.asynccontextmanager
async def open_mcp(url: str, bearer: str) -> AsyncIterator[McpClient]:
    """Yield a McpClient for the given URL/bearer.

    In v0.2.6 this is a thin wrapper that just constructs McpClient(url, bearer)
    — the client itself is now stateless (reconnects per call). Retained for API
    compatibility with callers that use it as a context manager (app.py lifespan,
    SSE validation probe). The context manager no longer holds any open connection;
    cleanup is a no-op.
    """
    yield McpClient(url=url, bearer=bearer)
