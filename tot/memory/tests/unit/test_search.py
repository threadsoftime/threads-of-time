# SPDX-License-Identifier: GPL-2.0-or-later
"""Tests for POST /v1/memory/{bot_guid}/search (memory.search).

Per design subspec §10.4 + the plan abbreviation's pure-semantic shape:
``{query_text? | query_vec?, top_k: int}`` → ``{results: [{episode_id,
content_text, cosine_similarity}]}``. No BM25, no decay, no salience boost,
no telemetry side effects — strictly nearest-neighbour over embeddings_vec.
"""
from __future__ import annotations

import struct
from pathlib import Path

import httpx
import pytest
from fastapi.testclient import TestClient

from tot_memory.db.connection import open_bot_db
from tot_memory.db.runner import run_migrations
from tot_memory.db.schema import EMBEDDING_DIM


def _seed(conn, ts_ms: int, text: str, vec: list[float]) -> int:
    cur = conn.execute(
        "INSERT INTO episodes (timestamp, content_text, episode_type, salience_score) "
        "VALUES (?, ?, ?, ?)",
        (ts_ms, text, "chat", 0.5),
    )
    eid = int(cur.lastrowid)
    conn.execute(
        "INSERT INTO embeddings_vec (rowid, embedding) VALUES (?, ?)",
        (eid, struct.pack(f"{len(vec)}f", *vec)),
    )
    conn.execute(
        "UPDATE episodes SET content_embedding_id = ? WHERE episode_id = ?",
        (eid, eid),
    )
    return eid


def test_search_with_query_text_embeds_and_returns_nearest(
    client: TestClient, respx_mock
):
    """Happy path: query_text is embedded; returns nearest-neighbour episodes
    sorted by cosine similarity. Telemetry NOT touched (the side-effect
    distinction from recall)."""
    parallel = [1.0] + [0.0] * (EMBEDDING_DIM - 1)
    orthogonal = [0.0, 1.0] + [0.0] * (EMBEDDING_DIM - 2)

    data_dir = Path(client.app.state.settings.data_dir)
    conn = open_bot_db(data_dir, "test-bot")
    try:
        run_migrations(conn)
        near = _seed(conn, 1000, "near episode", parallel)
        far = _seed(conn, 2000, "far episode", orthogonal)
        conn.commit()
    finally:
        conn.close()

    respx_mock.post("http://stub.local/v1/embeddings").mock(
        return_value=httpx.Response(
            200, json={"data": [{"embedding": parallel, "index": 0}]}
        )
    )

    response = client.post(
        "/v1/memory/test-bot/search",
        json={"query_text": "anything", "top_k": 5},
    )
    assert response.status_code == 200, response.text
    body = response.json()
    assert len(body["results"]) == 2
    # `near` should outrank `far` (its embedding matches the query exactly).
    assert body["results"][0]["episode_id"] == near
    assert body["results"][1]["episode_id"] == far
    assert body["results"][0]["cosine_similarity"] > body["results"][1]["cosine_similarity"]
    assert body["results"][0]["content_text"] == "near episode"

    # Verify recall_count NOT bumped (memory.search has no telemetry side effect).
    conn = open_bot_db(data_dir, "test-bot")
    try:
        rows = conn.execute(
            "SELECT recall_count, last_recalled_at FROM episodes"
        ).fetchall()
        for row in rows:
            assert row["recall_count"] == 0
            assert row["last_recalled_at"] is None
    finally:
        conn.close()


def test_search_with_query_vec_skips_embedding_call(
    client: TestClient, respx_mock
):
    """When the caller supplies query_vec directly, the embedding endpoint is
    not called. Useful for re-running the same search without re-embedding."""
    vec = [1.0] + [0.0] * (EMBEDDING_DIM - 1)

    data_dir = Path(client.app.state.settings.data_dir)
    conn = open_bot_db(data_dir, "test-bot")
    try:
        run_migrations(conn)
        eid = _seed(conn, 1000, "hello", vec)
        conn.commit()
    finally:
        conn.close()

    # Deliberately do NOT mock the embeddings endpoint — if the route tries to
    # call it, respx will raise. The presence of query_vec must short-circuit.
    response = client.post(
        "/v1/memory/test-bot/search",
        json={"query_vec": vec, "top_k": 5},
    )
    assert response.status_code == 200, response.text
    body = response.json()
    assert len(body["results"]) == 1
    assert body["results"][0]["episode_id"] == eid


def test_search_rejects_missing_query(client: TestClient):
    """At least one of query_text / query_vec is required (400)."""
    response = client.post(
        "/v1/memory/test-bot/search",
        json={"top_k": 5},
    )
    assert response.status_code == 400
    assert "query_text" in response.json()["detail"] or "query_vec" in response.json()["detail"]


def test_search_rejects_query_vec_with_wrong_dim(client: TestClient):
    """A query_vec that doesn't match EMBEDDING_DIM is 400 — catches operator
    misconfiguration before it reaches sqlite-vec (which would silently
    misbehave on a wrong-sized buffer)."""
    response = client.post(
        "/v1/memory/test-bot/search",
        json={"query_vec": [0.1] * (EMBEDDING_DIM - 1), "top_k": 5},
    )
    assert response.status_code == 400
    assert "dimension" in response.json()["detail"].lower()
