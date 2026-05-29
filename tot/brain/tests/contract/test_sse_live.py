"""Contract: live SSE subscribe → memory write → receive within 1s.

Requires MEMORY_MCP_URL + MEMORY_BEARER env vars (set in build.sh gate).
Deselected by default via pytest marker; enabled in the contract gate.

The MEMORY_MCP_URL env var points to the MCP path (e.g.
http://127.0.0.1:8090/mcp/mcp). The SSE endpoint lives at the base
(http://127.0.0.1:8090/v1/events/stream), so strip the /mcp/mcp suffix.
"""
from __future__ import annotations

import asyncio
import json
import os
import time

import httpx
import pytest

pytestmark = pytest.mark.contract


def _memory_http_url() -> str:
    """Extract the HTTP base URL from MEMORY_MCP_URL."""
    raw = os.environ.get("MEMORY_MCP_URL", "http://127.0.0.1:8090/mcp/mcp")
    return raw.removesuffix("/mcp/mcp")


@pytest.mark.asyncio
async def test_sse_endpoint_delivers_live_chat_within_1s():
    """Subscribe to SSE; write a matching memory; receive the event within 1s.

    Cleans up the probe row via /memory/forget on exit (success or fail) so
    the contract gate doesn't accumulate `bot_id=9999` rows across runs.
    """
    base = _memory_http_url()
    bearer = os.environ["MEMORY_BEARER"]

    # Use a probe bot_id that doesn't collide with any live enrolled bot.
    bot_id = "9999"
    text = f"chat_received sse-contract-probe-{int(time.time() * 1000)}"
    received: list[tuple[dict, float]] = []
    written_memory_id: str | None = None

    try:
        async with httpx.AsyncClient(timeout=httpx.Timeout(connect=5.0, read=10.0, write=5.0, pool=5.0)) as c:
            async with c.stream(
                "GET",
                f"{base}/v1/events/stream",
                params={"bot_id": bot_id, "prefixes": "chat_received"},
                headers={
                    "Authorization": f"Bearer {bearer}",
                    "Accept": "text/event-stream",
                },
            ) as response:
                assert response.status_code == 200, (
                    f"Expected 200 from SSE endpoint; got {response.status_code}"
                )
                assert "text/event-stream" in response.headers.get("content-type", ""), (
                    f"Expected text/event-stream; got {response.headers.get('content-type')}"
                )

                async def writer():
                    nonlocal written_memory_id
                    await asyncio.sleep(0.15)  # let stream settle
                    async with httpx.AsyncClient(timeout=5.0) as wc:
                        r = await wc.post(
                            f"{base}/memory/remember",
                            json={
                                "bot_id": bot_id,
                                "text": text,
                                "salience": 0.5,
                                "entities": [],
                                "relations": [],
                            },
                            headers={"Authorization": f"Bearer {bearer}"},
                        )
                        assert r.status_code in (200, 201), (
                            f"memory write failed: {r.status_code} {r.text[:200]}"
                        )
                        try:
                            written_memory_id = r.json().get("memory_id")
                        except Exception:
                            written_memory_id = None

                writer_task = asyncio.create_task(writer())
                t0 = time.time()
                buf = ""

                try:
                    async with asyncio.timeout(3.0):
                        async for chunk in response.aiter_text():
                            buf += chunk
                            # Parse complete SSE events
                            for raw_event in buf.split("\n\n"):
                                for ln in raw_event.split("\n"):
                                    if ln.startswith("data:"):
                                        try:
                                            payload = json.loads(ln[5:].strip())
                                        except json.JSONDecodeError:
                                            continue
                                        if payload.get("text") == text:
                                            received.append((payload, time.time() - t0))
                            if received:
                                break
                finally:
                    await writer_task

        assert received, "No SSE event received within 3s"
        payload, latency = received[0]
        assert latency < 1.0, f"SSE delivery too slow: {latency:.3f}s (limit: 1.0s)"
        assert payload["bot_id"] == bot_id
    finally:
        # Clean up the probe row to avoid accumulating rows across contract runs.
        if written_memory_id:
            try:
                async with httpx.AsyncClient(timeout=5.0) as cc:
                    await cc.post(
                        f"{base}/memory/forget",
                        json={"bot_id": bot_id, "memory_id": written_memory_id},
                        headers={"Authorization": f"Bearer {bearer}"},
                    )
            except Exception:
                pass  # best-effort cleanup; don't fail the test on cleanup failure


@pytest.mark.asyncio
async def test_sse_endpoint_rejects_missing_bearer():
    """No auth → 401 or 403."""
    base = _memory_http_url()
    async with httpx.AsyncClient(timeout=5.0) as c:
        r = await c.get(
            f"{base}/v1/events/stream",
            params={"bot_id": "0", "prefixes": "chat_received"},
        )
        assert r.status_code in (401, 403), (
            f"Expected 401/403 for missing bearer; got {r.status_code}"
        )


@pytest.mark.asyncio
async def test_sse_endpoint_rejects_wrong_bearer():
    """Wrong bearer → 401 or 403."""
    base = _memory_http_url()
    async with httpx.AsyncClient(timeout=5.0) as c:
        r = await c.get(
            f"{base}/v1/events/stream",
            params={"bot_id": "0", "prefixes": "chat_received"},
            headers={"Authorization": "Bearer definitely-wrong-token"},
        )
        assert r.status_code in (401, 403), (
            f"Expected 401/403 for wrong bearer; got {r.status_code}"
        )


@pytest.mark.asyncio
async def test_sse_endpoint_rejects_too_many_prefixes():
    """More than 16 prefixes → 400."""
    base = _memory_http_url()
    bearer = os.environ["MEMORY_BEARER"]
    prefixes = ",".join(f"p{i}" for i in range(17))
    async with httpx.AsyncClient(timeout=5.0) as c:
        r = await c.get(
            f"{base}/v1/events/stream",
            params={"bot_id": "0", "prefixes": prefixes},
            headers={"Authorization": f"Bearer {bearer}"},
        )
        assert r.status_code == 400, (
            f"Expected 400 for too many prefixes; got {r.status_code}"
        )


@pytest.mark.asyncio
async def test_sse_endpoint_rejects_missing_bot_id():
    """Missing bot_id → 400."""
    base = _memory_http_url()
    bearer = os.environ["MEMORY_BEARER"]
    async with httpx.AsyncClient(timeout=5.0) as c:
        r = await c.get(
            f"{base}/v1/events/stream",
            params={"prefixes": "chat_received"},
            headers={"Authorization": f"Bearer {bearer}"},
        )
        assert r.status_code in (400, 422), (
            f"Expected 400/422 for missing bot_id; got {r.status_code}"
        )
