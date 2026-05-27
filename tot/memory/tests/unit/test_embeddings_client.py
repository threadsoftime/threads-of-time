# SPDX-License-Identifier: GPL-2.0-or-later
"""Tests for the BYOLLM embeddings client.

Per design subspec §4 (BYOLLM model) and §5 (sync embedding pipeline).
The client POSTs to ``{base_url}/embeddings`` with ``{"model": ..., "input": ...}``
and returns a list[float] of length EMBEDDING_DIM. Authorization bearer header is
sent only when ``api_key`` is non-empty. A dimension mismatch raises ValueError —
this catches operator misconfiguration where ``BRAIN_EMBEDDINGS_URL`` points at
an endpoint serving a model with a different dimension than the sqlite-vec table
was built for.
"""
from __future__ import annotations

import httpx
import pytest

from tot_memory.db.schema import EMBEDDING_DIM
from tot_memory.embeddings.client import EmbeddingsClient


@pytest.mark.asyncio
async def test_embed_calls_byollm_endpoint(respx_mock):
    expected_vec = [0.1] * EMBEDDING_DIM
    route = respx_mock.post("http://stub.local/v1/embeddings").mock(
        return_value=httpx.Response(
            200,
            json={
                "data": [{"embedding": expected_vec, "index": 0}],
                "model": "nomic-embed-text",
            },
        )
    )
    client = EmbeddingsClient(
        base_url="http://stub.local/v1",
        model="nomic-embed-text",
        api_key="",
    )
    try:
        result = await client.embed("hello world")
    finally:
        await client.aclose()

    assert route.called
    assert result == expected_vec
    # Verify request body matches the documented contract (§4.4)
    sent = route.calls.last.request
    import json as _json
    body = _json.loads(sent.content)
    assert body == {"model": "nomic-embed-text", "input": "hello world"}
    # No bearer header when api_key is empty
    assert "authorization" not in {k.lower() for k in sent.headers.keys()}


@pytest.mark.asyncio
async def test_embed_includes_bearer_when_api_key_set(respx_mock):
    route = respx_mock.post("http://stub.local/v1/embeddings").mock(
        return_value=httpx.Response(
            200,
            json={"data": [{"embedding": [0.0] * EMBEDDING_DIM, "index": 0}]},
        )
    )
    client = EmbeddingsClient(
        base_url="http://stub.local/v1",
        model="nomic-embed-text",
        api_key="sk-fake",
    )
    try:
        await client.embed("hello")
    finally:
        await client.aclose()

    assert route.called
    assert route.calls.last.request.headers["authorization"] == "Bearer sk-fake"


@pytest.mark.asyncio
async def test_embed_raises_on_dim_mismatch(respx_mock):
    """Operator misconfiguration: endpoint returns a vector of unexpected size.

    Per design subspec §4.4 — must raise ValueError so the operator notices the
    mismatch instead of silently writing garbage to sqlite-vec.
    """
    bad_dim = EMBEDDING_DIM + 16
    respx_mock.post("http://stub.local/v1/embeddings").mock(
        return_value=httpx.Response(
            200,
            json={"data": [{"embedding": [0.0] * bad_dim, "index": 0}]},
        )
    )
    client = EmbeddingsClient(
        base_url="http://stub.local/v1",
        model="some-other-model",
        api_key="",
    )
    try:
        with pytest.raises(ValueError, match="dimension"):
            await client.embed("hello")
    finally:
        await client.aclose()
