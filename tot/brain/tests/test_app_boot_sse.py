"""Boot-time SSE endpoint validation tests (B7).

validate_sse_endpoint_or_raise() probes the memory-sidecar SSE endpoint
at brain startup when BRAIN_SSE_ENABLED=1. If the probe fails (non-200,
wrong Content-Type, or connection error), it raises RuntimeError.

Spec criterion #4: the BRAIN ITSELF refuses to start when validation
raises — the RuntimeError must propagate out of lifespan so uvicorn
exits non-zero. The operator's recovery path is BRAIN_SSE_ENABLED=0
(spec §8 rollback playbook), which skips validation entirely.
"""
from __future__ import annotations

from unittest.mock import AsyncMock, MagicMock, patch

import pytest

from brain_sidecar.app import validate_sse_endpoint_or_raise


@pytest.mark.asyncio
async def test_validate_sse_endpoint_fails_on_404(httpx_mock):
    httpx_mock.add_response(
        method="GET",
        url="http://test-mem:8090/v1/events/stream?bot_id=0&prefixes=received+whisper",
        status_code=404,
    )
    with pytest.raises(RuntimeError, match="SSE endpoint validation"):
        await validate_sse_endpoint_or_raise(
            memory_url="http://test-mem:8090",
            bearer="tok",
            probe_bot_id="0",
        )


@pytest.mark.asyncio
async def test_validate_sse_endpoint_fails_on_wrong_content_type(httpx_mock):
    httpx_mock.add_response(
        method="GET",
        url="http://test-mem:8090/v1/events/stream?bot_id=0&prefixes=received+whisper",
        status_code=200,
        headers={"Content-Type": "application/json"},
        text='{"error":"nope"}',
    )
    with pytest.raises(RuntimeError, match="SSE endpoint validation"):
        await validate_sse_endpoint_or_raise(
            memory_url="http://test-mem:8090",
            bearer="tok",
            probe_bot_id="0",
        )


@pytest.mark.asyncio
async def test_validate_sse_endpoint_ok_on_200_event_stream(httpx_mock):
    httpx_mock.add_response(
        method="GET",
        url="http://test-mem:8090/v1/events/stream?bot_id=0&prefixes=received+whisper",
        status_code=200,
        headers={"Content-Type": "text/event-stream"},
        text=":keepalive\n\n",
    )
    # Should not raise
    await validate_sse_endpoint_or_raise(
        memory_url="http://test-mem:8090",
        bearer="tok",
        probe_bot_id="0",
    )


@pytest.mark.asyncio
async def test_validate_sse_endpoint_fails_on_connection_error(httpx_mock):
    import httpx as httpx_lib
    httpx_mock.add_exception(
        httpx_lib.ConnectError("connection refused"),
        method="GET",
    )
    with pytest.raises(RuntimeError, match="SSE endpoint validation"):
        await validate_sse_endpoint_or_raise(
            memory_url="http://test-mem:8090",
            bearer="tok",
            probe_bot_id="0",
        )


# ---- App-level: criterion #4 ("Brain refuses to start") ----------------------

class _StubMcpCtx:
    def __init__(self, client):
        self._client = client
    async def __aenter__(self):
        return self._client
    async def __aexit__(self, *a):
        return False


@pytest.mark.asyncio
async def test_app_lifespan_refuses_to_start_when_sse_validation_raises(
    monkeypatch, tmp_path,
):
    """Spec §7 criterion #4: brain refuses to start (RuntimeError propagates
    out of lifespan) when validate_sse_endpoint_or_raise fails.
    """
    fake_harness = MagicMock()
    fake_harness.list_tools = AsyncMock(return_value=[])
    fake_memory = MagicMock()
    fake_memory.list_tools = AsyncMock(return_value=[])

    def stub_open_mcp(url, bearer):
        return _StubMcpCtx(fake_harness) if "8099" in url else _StubMcpCtx(fake_memory)

    monkeypatch.setenv("BRAIN_STATE_DB", str(tmp_path / "state.sqlite"))
    monkeypatch.setenv("BRAIN_DECISIONS_LOG", str(tmp_path / "decisions.jsonl"))
    monkeypatch.setenv("HARNESS_MCP_URL", "http://x:8099/mcp/mcp")
    monkeypatch.setenv("MEMORY_MCP_URL", "http://x:8090/mcp/mcp")
    monkeypatch.setenv("HARNESS_BEARER", "h")
    monkeypatch.setenv("MEMORY_BEARER", "m")
    monkeypatch.setenv("BRAIN_SSE_ENABLED", "1")

    async def fail_validate(**kwargs):
        raise RuntimeError("SSE endpoint validation failed: status=404")

    with patch("brain_sidecar.app.open_mcp", stub_open_mcp), \
         patch("brain_sidecar.app.validate_sse_endpoint_or_raise", fail_validate):
        from brain_sidecar.app import create_app
        app = create_app()
        with pytest.raises(RuntimeError, match="SSE endpoint validation"):
            async with app.router.lifespan_context(app):
                pass


@pytest.mark.asyncio
async def test_app_lifespan_skips_sse_validation_when_disabled(
    monkeypatch, tmp_path,
):
    """BRAIN_SSE_ENABLED=0 — operator's degraded-mode recovery path. The
    validation is not called at all, so a broken SSE endpoint does not
    prevent startup. Brain runs in polling-only mode.
    """
    fake_harness = MagicMock()
    fake_harness.list_tools = AsyncMock(return_value=[])
    fake_memory = MagicMock()
    fake_memory.list_tools = AsyncMock(return_value=[])

    def stub_open_mcp(url, bearer):
        return _StubMcpCtx(fake_harness) if "8099" in url else _StubMcpCtx(fake_memory)

    monkeypatch.setenv("BRAIN_STATE_DB", str(tmp_path / "state.sqlite"))
    monkeypatch.setenv("BRAIN_DECISIONS_LOG", str(tmp_path / "decisions.jsonl"))
    monkeypatch.setenv("HARNESS_MCP_URL", "http://x:8099/mcp/mcp")
    monkeypatch.setenv("MEMORY_MCP_URL", "http://x:8090/mcp/mcp")
    monkeypatch.setenv("HARNESS_BEARER", "h")
    monkeypatch.setenv("MEMORY_BEARER", "m")
    monkeypatch.setenv("BRAIN_SSE_ENABLED", "0")

    async def fail_validate(**kwargs):
        raise RuntimeError("SSE endpoint validation failed: should not be called")

    with patch("brain_sidecar.app.open_mcp", stub_open_mcp), \
         patch("brain_sidecar.app.validate_sse_endpoint_or_raise", fail_validate):
        from brain_sidecar.app import create_app
        app = create_app()
        async with app.router.lifespan_context(app):
            # Lifespan completed without raising — validation was skipped.
            assert app.state.supervisor is not None
