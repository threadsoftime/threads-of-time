"""MCP-side strip→equip identity test (kb_642162c3 follow-up #6).

Mirror of test_strip_equip_identity.py that routes through the
FastMCP streamable-HTTP transport at /mcp/mcp instead of the HTTP
/v1/* surface. Closes the V0.3.1 MCP-transport parity gap.

Env vars (required to run; skipped otherwise):

    HEIMDAL_HARNESS_URL     e.g. http://192.168.1.3:8099  (default)
    HEIMDAL_HARNESS_TOKEN   bearer token
    HEIMDAL_TEST_BOT_GUID   online bot low-32 GUID
"""

from __future__ import annotations

import json
import os
import uuid
from typing import Any

import httpx
import pytest


URL_ENV = "HEIMDAL_HARNESS_URL"
TOKEN_ENV = "HEIMDAL_HARNESS_TOKEN"
BOT_GUID_ENV = "HEIMDAL_TEST_BOT_GUID"


def _require_env() -> tuple[str, str, int]:
    base = os.environ.get(URL_ENV, "http://192.168.1.3:8099")
    token = os.environ.get(TOKEN_ENV)
    guid = os.environ.get(BOT_GUID_ENV)
    if not token:
        pytest.skip(f"set {TOKEN_ENV} to run this test")
    if not guid:
        pytest.skip(f"set {BOT_GUID_ENV} to an online bot guid")
    try:
        return base, token, int(guid)
    except ValueError:
        pytest.skip(f"{BOT_GUID_ENV} must be int; got {guid!r}")


def _initialize_mcp(c: httpx.Client) -> str:
    """Initialize the MCP session and return the session id."""
    init_payload = {
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "v14-b4-regression", "version": "0.1"},
        },
    }
    resp = c.post("/mcp/mcp", json=init_payload, headers={
        "Accept": "application/json, text/event-stream",
    })
    if resp.status_code != 200:
        pytest.fail(f"initialize: HTTP {resp.status_code}: {resp.text[:300]}")
    session_id = resp.headers.get("mcp-session-id")
    if not session_id:
        pytest.fail(f"initialize: no mcp-session-id header; headers={dict(resp.headers)}")
    return session_id


def _call_tool(c: httpx.Client, session_id: str, name: str, args: dict) -> dict:
    """Call an MCP tool and return the parsed result dict."""
    payload = {
        "jsonrpc": "2.0", "id": str(uuid.uuid4()), "method": "tools/call",
        "params": {"name": name, "arguments": {"args": args}},
    }
    resp = c.post("/mcp/mcp", json=payload, headers={
        "mcp-session-id": session_id,
        "Accept": "application/json, text/event-stream",
    })
    if resp.status_code != 200:
        pytest.fail(f"{name}: HTTP {resp.status_code}: {resp.text[:300]}")

    # Response may be JSON or SSE — handle both.
    ctype = resp.headers.get("content-type", "")
    if "text/event-stream" in ctype:
        # Parse the first data: line.
        data_line = next(
            (l[len("data: "):] for l in resp.text.splitlines() if l.startswith("data: ")),
            None,
        )
        if data_line is None:
            pytest.fail(f"{name}: SSE response with no data line: {resp.text[:300]}")
        envelope = json.loads(data_line)
    else:
        envelope = resp.json()

    if "error" in envelope:
        pytest.fail(f"{name}: JSON-RPC error: {envelope['error']}")

    # MCP 1.27 wraps tool result text in result.content[0].text as JSON-encoded string.
    content = envelope["result"]["content"]
    text = content[0]["text"]
    return json.loads(text)


def _count_items(inventory_result: dict) -> int:
    """Count equipped + bags + nested_bags items (matches V0.3.1 scope)."""
    body = inventory_result["result"] if "result" in inventory_result else inventory_result
    count = len(body.get("equipped", []))
    count += len(body.get("bags", []))
    for nested in body.get("nested_bags", []):
        count += len(nested.get("contents", []))
    return count


def test_mcp_strip_equip_identity():
    base, token, bot_guid = _require_env()

    with httpx.Client(
        base_url=base,
        headers={"Authorization": f"Bearer {token}", "Content-Type": "application/json"},
        timeout=15.0,
    ) as c:
        session_id = _initialize_mcp(c)

        before = _call_tool(c, session_id, "obs.get_inventory", {"target_guid": bot_guid})
        if not before.get("ok"):
            pytest.fail(f"obs.get_inventory before: ok=false: {before!r}")
        before_count = _count_items(before)

        strip = _call_tool(c, session_id, "gm.strip_gear", {"target_guid": bot_guid})
        # strip may legitimately be ok OR validator_rejected (strict-mode path);
        # either is acceptable as long as inventory is conserved.

        equip = _call_tool(c, session_id, "gm.equip_all", {"target_guid": bot_guid})
        # equip may also legitimately return without restoring everything;
        # the contract is just inventory conservation.

        after = _call_tool(c, session_id, "obs.get_inventory", {"target_guid": bot_guid})
        if not after.get("ok"):
            pytest.fail(f"obs.get_inventory after: ok=false: {after!r}")
        after_count = _count_items(after)

        assert after_count == before_count, (
            f"strip→equip identity violated on MCP transport: "
            f"before={before_count} after={after_count} "
            f"(strip outcome: {strip.get('ok')!r}, "
            f"equip outcome: {equip.get('ok')!r})"
        )
