"""Integration tests: an MCP tool call returns the same body shape as
the equivalent /v1/* HTTP call."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any
from unittest.mock import AsyncMock

import httpx
import pytest

from harness_daemon.app import build_app
from harness_daemon.config import DaemonConfig, TokenRecord


@dataclass
class FakeACResponse:
    status: int
    body: dict
    ac_latency_ms: int = 5


@pytest.fixture
def app_factory(tmp_path):
    def _make():
        cfg = DaemonConfig(
            max_augmented_bots=1,
            ac_bridge_url="http://127.0.0.1:8091",
            audit_path=str(tmp_path / "audit.jsonl"),
            listen_address="0.0.0.0:8090",
            tokens=[
                TokenRecord(token="ok-tok", identity="ok.user",
                            scope=["gm.*", "obs.*", "bot.*"]),
            ],
        )
        mock_ac = AsyncMock()
        mock_ac.dispatch = AsyncMock(return_value=FakeACResponse(
            status=200,
            body={"ok": True, "result": {"pong": True}},
        ))
        mock_ac.health = AsyncMock(return_value=True)
        mock_ac.close = AsyncMock()
        return build_app(cfg, ac_client=mock_ac), mock_ac
    return _make


@pytest.mark.asyncio
async def test_obs_ping_via_mcp_returns_same_result_as_http(app_factory):
    app, _ = app_factory()
    transport = httpx.ASGITransport(app=app)
    # FastMCP's session manager only runs inside the app's lifespan. Drive
    # the lifespan manually since httpx.ASGITransport doesn't start it.
    async with app.router.lifespan_context(app):
        # Use 127.0.0.1 (not "http://test") because FastMCP enables DNS-rebinding
        # host validation and rejects unknown hosts with 421 Misdirected Request.
        async with httpx.AsyncClient(transport=transport, base_url="http://127.0.0.1:8090") as client:
            # HTTP surface
            r_http = await client.post(
                "/v1/tools/obs.ping",
                json={},
                headers={"Authorization": "Bearer ok-tok"},
            )
            assert r_http.status_code == 200
            http_body = r_http.json()
            assert http_body["ok"] is True
            assert http_body["result"]["pong"] is True
            assert "request_id" in http_body
            assert http_body["identity"] == "ok.user"
            assert isinstance(http_body["latency_ms"], int)
            assert http_body["ac_latency_ms"] == 5

            # MCP surface — minimal hand-rolled streamable HTTP request.
            # FastMCP v1.12 streamable transport accepts JSON-RPC over POST.
            # We initialize a session, list tools, then call obs.ping.
            rpc_init = {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {"name": "parity-test", "version": "0"},
                },
            }
            r_init = await client.post(
                "/mcp/mcp",
                json=rpc_init,
                headers={
                    "Authorization": "Bearer ok-tok",
                    "Accept": "application/json, text/event-stream",
                },
            )
            assert r_init.status_code in (200, 202), r_init.text
            session_id = r_init.headers.get("mcp-session-id")
            assert session_id, "MCP did not return a session id"

            # FastMCP 1.27.1 wraps the pydantic-arg model under an outer `args`
            # property in the generated inputSchema (because the handler's
            # signature is `_handler(ctx, args: ModelCls)`). MCP clients must
            # therefore send `{"args": {...}}` rather than flat fields.
            rpc_call = {
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {"name": "obs.ping", "arguments": {"args": {}}},
            }
            r_call = await client.post(
                "/mcp/mcp",
                json=rpc_call,
                headers={
                    "Authorization": "Bearer ok-tok",
                    "Accept": "application/json, text/event-stream",
                    "mcp-session-id": session_id,
                },
            )
            assert r_call.status_code == 200, r_call.text
            # FastMCP returns the handler's dict in `result.structuredContent`
            # (per MCP 2024-11-05 spec). Body shape match is the assertion.
            # In mcp 1.27.1, the wrapped body actually lives in
            # `result.content[0].text` as JSON-encoded text — the fallback
            # branch below covers that path.
            rpc_body = _parse_jsonrpc_response(r_call)
            mcp_result = rpc_body["result"]
            assert mcp_result.get("isError") is False, rpc_body
            # The handler returned outcome.body which is `{"ok": True, "result": {...}}`.
            # FastMCP wraps it; the inner dict is in structuredContent OR content[0].text.
            sc = mcp_result.get("structuredContent")
            if sc is not None:
                assert sc["ok"] is True
                assert sc["result"]["pong"] is True
            else:
                # Fallback: parse the first text content as JSON
                import json as _json
                txt = mcp_result["content"][0]["text"]
                parsed = _json.loads(txt)
                assert parsed["ok"] is True
                assert parsed["result"]["pong"] is True


def _parse_jsonrpc_response(resp: httpx.Response) -> dict:
    """Parse a JSON-RPC response that may come back as JSON or SSE.

    FastMCP's streamable HTTP transport (mcp 1.27.1) advertises
    `text/event-stream` and may emit the JSON-RPC reply as a single
    `data: {...}` SSE frame even for non-streaming calls.
    """
    ctype = resp.headers.get("content-type", "")
    if ctype.startswith("application/json"):
        return resp.json()
    # SSE: find the first `data: ` line and parse it as JSON.
    import json as _json
    for line in resp.text.splitlines():
        if line.startswith("data:"):
            return _json.loads(line[len("data:"):].strip())
    raise AssertionError(f"could not parse response: ctype={ctype!r} body={resp.text!r}")


@pytest.mark.asyncio
async def test_mcp_unauthorized_without_bearer(app_factory):
    app, _ = app_factory()
    transport = httpx.ASGITransport(app=app)
    async with app.router.lifespan_context(app):
        async with httpx.AsyncClient(transport=transport, base_url="http://127.0.0.1:8090") as client:
            r = await client.post(
                "/mcp/mcp",
                json={"jsonrpc": "2.0", "id": 1, "method": "initialize",
                      "params": {"protocolVersion": "2024-11-05",
                                 "capabilities": {},
                                 "clientInfo": {"name": "x", "version": "0"}}},
                headers={"Accept": "application/json, text/event-stream"},
            )
            assert r.status_code == 401, r.text
