# SPDX-License-Identifier: GPL-2.0-or-later
"""Unit tests for the brain-side memory client (Plan 2 Task 32).

Mocks the harness's HTTP surface (memory.recall + memory.write tool endpoints)
via pytest_httpx and asserts request shape + response parsing.
"""
from __future__ import annotations

import json
import os
from unittest import mock

import pytest

from brain_sidecar.memory_client import MemoryClient, RecalledEpisode


# ----------------------------------------------------------------------
# recall()
# ----------------------------------------------------------------------


@pytest.mark.asyncio
async def test_recall_posts_to_memory_recall_with_bearer(httpx_mock):
    """recall() POSTs to {base}/v1/tools/memory.recall with Bearer header + JSON body."""
    httpx_mock.add_response(
        url="http://harness/v1/tools/memory.recall",
        json={
            "ok": True,
            "result": {
                "results": [
                    {
                        "episode_id": 42,
                        "content_text": "Alice asked me to heal in BFD.",
                        "timestamp": "2026-05-27T12:00:00Z",
                        "salience_score": 0.6,
                        "score": 0.842,
                    },
                    {
                        "episode_id": 17,
                        "content_text": "Cleared Westfall with Bob.",
                        "timestamp": "2026-05-26T18:30:00Z",
                        "salience_score": 0.5,
                        "score": 0.452,
                    },
                ]
            },
        },
    )

    client = MemoryClient(base_url="http://harness", bearer_token="t0k3n")
    try:
        out = await client.recall(bot_guid="1234567", query_text="what did Alice say", top_k=5)
    finally:
        await client.aclose()

    # Response parsing
    assert isinstance(out, list)
    assert len(out) == 2
    assert isinstance(out[0], RecalledEpisode)
    assert out[0].episode_id == 42
    assert out[0].content_text == "Alice asked me to heal in BFD."
    assert out[0].timestamp == "2026-05-27T12:00:00Z"
    assert out[0].salience_score == 0.6
    assert out[0].score == 0.842

    # Request shape
    request = httpx_mock.get_request()
    assert request is not None
    assert request.headers.get("Authorization") == "Bearer t0k3n"
    body = json.loads(request.content)
    assert body == {
        "bot_guid": "1234567",
        "query_text": "what did Alice say",
        "top_k": 5,
    }


@pytest.mark.asyncio
async def test_recall_default_top_k_is_five(httpx_mock):
    httpx_mock.add_response(
        url="http://harness/v1/tools/memory.recall",
        json={"ok": True, "result": {"results": []}},
    )
    client = MemoryClient(base_url="http://harness", bearer_token="t")
    try:
        out = await client.recall(bot_guid="1", query_text="anything")
    finally:
        await client.aclose()
    assert out == []
    body = json.loads(httpx_mock.get_request().content)
    assert body["top_k"] == 5


@pytest.mark.asyncio
async def test_recall_strips_trailing_slash_from_base_url(httpx_mock):
    httpx_mock.add_response(
        url="http://harness/v1/tools/memory.recall",
        json={"ok": True, "result": {"results": []}},
    )
    client = MemoryClient(base_url="http://harness/", bearer_token="t")
    try:
        await client.recall(bot_guid="1", query_text="x")
    finally:
        await client.aclose()
    # The httpx_mock matched the URL without a doubled slash — that's the assertion.
    assert httpx_mock.get_request() is not None


@pytest.mark.asyncio
async def test_recall_raises_on_http_error(httpx_mock):
    import httpx as _httpx
    httpx_mock.add_response(
        url="http://harness/v1/tools/memory.recall",
        status_code=503, json={"error": "embedding_service_unavailable"},
    )
    client = MemoryClient(base_url="http://harness", bearer_token="t")
    try:
        with pytest.raises(_httpx.HTTPStatusError):
            await client.recall(bot_guid="1", query_text="x")
    finally:
        await client.aclose()


# ----------------------------------------------------------------------
# write_episode()
# ----------------------------------------------------------------------


@pytest.mark.asyncio
async def test_write_episode_posts_to_memory_write_and_returns_id(httpx_mock):
    httpx_mock.add_response(
        url="http://harness/v1/tools/memory.write",
        json={
            "ok": True,
            "result": {
                "episode_id": 1011,
                "embedding_generated": True,
                "salience_score": 0.6,
            },
        },
    )

    client = MemoryClient(base_url="http://harness", bearer_token="t0k3n")
    try:
        episode_id = await client.write_episode(
            bot_guid="1234567",
            content_text="Killed Hogger in the wetlands.",
            episode_type="combat",
            timestamp_iso="2026-05-27T12:00:00Z",
            salience_score=0.8,
        )
    finally:
        await client.aclose()

    assert episode_id == 1011

    request = httpx_mock.get_request()
    assert request is not None
    assert request.headers.get("Authorization") == "Bearer t0k3n"
    body = json.loads(request.content)
    assert body == {
        "bot_guid": "1234567",
        "content_text": "Killed Hogger in the wetlands.",
        "episode_type": "combat",
        "timestamp": "2026-05-27T12:00:00Z",
        "salience_score": 0.8,
        "entities": [],
    }


@pytest.mark.asyncio
async def test_write_episode_raises_on_http_error(httpx_mock):
    import httpx as _httpx
    httpx_mock.add_response(
        url="http://harness/v1/tools/memory.write",
        status_code=400, json={"error": "content_too_long"},
    )
    client = MemoryClient(base_url="http://harness", bearer_token="t")
    try:
        with pytest.raises(_httpx.HTTPStatusError):
            await client.write_episode(
                bot_guid="1",
                content_text="x" * 5,
                episode_type="chat",
                timestamp_iso="2026-05-27T00:00:00Z",
                salience_score=0.5,
            )
    finally:
        await client.aclose()


# ----------------------------------------------------------------------
# from_env() factory
# ----------------------------------------------------------------------


def test_from_env_reads_harness_base_url_and_bearer_token():
    with mock.patch.dict(
        os.environ,
        {
            "HARNESS_BASE_URL": "http://harness:8099",
            "HARNESS_BEARER_TOKEN": "secret",
        },
        clear=False,
    ):
        client = MemoryClient.from_env()
    # Internal state inspection is acceptable in unit tests of small helper.
    assert client._base_url == "http://harness:8099"
    # Bearer token applied as default header on the underlying httpx client.
    assert client._http.headers.get("Authorization") == "Bearer secret"


def test_from_env_raises_when_harness_base_url_missing():
    env = {k: v for k, v in os.environ.items()
           if k not in ("HARNESS_BASE_URL",)}
    with mock.patch.dict(os.environ, env, clear=True):
        with pytest.raises(RuntimeError, match="HARNESS_BASE_URL"):
            MemoryClient.from_env()
