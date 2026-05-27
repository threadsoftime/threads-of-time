# SPDX-License-Identifier: GPL-2.0-or-later
"""Tests for POST /v1/memory/{bot_guid}/recall (memory.recall).

Per design subspec §10.3 (recall request/response) and §6 (hybrid retrieval).
The route:

  1. Embeds the query text via the BYOLLM endpoint.
  2. Optionally resolves entity_ids → episode-id hard filter set.
  3. Calls retrieval.rerank.recall() to score candidates.
  4. Hydrates the returned episode_ids with full row data (timestamp, content,
     type, salience, component breakdown).
  5. Updates last_recalled_at + recall_count for each returned episode (side
     effect that distinguishes recall from search — see §10.4).
  6. Returns the response envelope with telemetry counters.
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


def _seed_episode(
    conn,
    ts_ms: int,
    text: str,
    etype: str,
    salience: float,
    vec: list[float] | None = None,
) -> int:
    cur = conn.execute(
        "INSERT INTO episodes (timestamp, content_text, episode_type, salience_score) "
        "VALUES (?, ?, ?, ?)",
        (ts_ms, text, etype, salience),
    )
    eid = int(cur.lastrowid)
    if vec is not None:
        conn.execute(
            "INSERT INTO embeddings_vec (rowid, embedding) VALUES (?, ?)",
            (eid, struct.pack(f"{len(vec)}f", *vec)),
        )
        conn.execute(
            "UPDATE episodes SET content_embedding_id = ? WHERE episode_id = ?",
            (eid, eid),
        )
    return eid


def _add_entity(conn, episode_id: int, kind: str, key: str, name: str) -> int:
    cur = conn.execute(
        "INSERT INTO entities (entity_kind, entity_key, display_name) "
        "VALUES (?, ?, ?)",
        (kind, key, name),
    )
    entity_id = int(cur.lastrowid)
    conn.execute(
        "INSERT INTO episode_entities (episode_id, entity_id, role) "
        "VALUES (?, ?, ?)",
        (episode_id, entity_id, "subject"),
    )
    return entity_id


def test_recall_returns_ranked_episodes(client: TestClient, respx_mock):
    """Happy path: seed two episodes; recall returns them ranked by hybrid score
    with the component breakdown surfaced per §10.3."""
    parallel = [1.0] + [0.0] * (EMBEDDING_DIM - 1)
    orthogonal = [0.0, 1.0] + [0.0] * (EMBEDDING_DIM - 2)

    data_dir = Path(client.app.state.settings.data_dir)
    conn = open_bot_db(data_dir, "test-bot")
    try:
        run_migrations(conn)
        winner = _seed_episode(
            conn, 1748395200000, "Alice taught me how to AoE", "social", 0.9, parallel
        )
        _seed_episode(
            conn, 1748395100000, "I died alone in the woods", "combat", 0.1, orthogonal
        )
        conn.commit()
    finally:
        conn.close()

    # The query embedding aligns with `winner`'s parallel vector → it wins on dense.
    respx_mock.post("http://stub.local/v1/embeddings").mock(
        return_value=httpx.Response(
            200,
            json={"data": [{"embedding": parallel, "index": 0}]},
        )
    )

    response = client.post(
        "/v1/memory/test-bot/recall",
        json={"query_text": "Alice AoE", "top_k": 5},
    )
    assert response.status_code == 200, response.text
    body = response.json()
    assert "results" in body
    assert len(body["results"]) >= 1
    top = body["results"][0]
    assert top["episode_id"] == winner
    assert top["content_text"] == "Alice taught me how to AoE"
    assert top["episode_type"] == "social"
    assert top["timestamp"] == 1748395200000
    assert top["salience_score"] == pytest.approx(0.9)
    assert "score" in top
    components = top["components"]
    for key in ("bm25_norm", "dense_norm", "decay", "salience", "entity_match"):
        assert key in components


def test_recall_filters_by_entity_names(client: TestClient, respx_mock):
    """entity_names hard-filters: episodes without any of the named entities
    are excluded before scoring (§6.1 stage 2)."""
    vec = [1.0] + [0.0] * (EMBEDDING_DIM - 1)

    data_dir = Path(client.app.state.settings.data_dir)
    conn = open_bot_db(data_dir, "test-bot")
    try:
        run_migrations(conn)
        alice_ep = _seed_episode(
            conn, 1748395200000, "we ran a dungeon", "social", 0.7, vec
        )
        _add_entity(conn, alice_ep, "player", "111", "Alice")

        bob_ep = _seed_episode(
            conn, 1748395200000, "we ran a dungeon", "social", 0.7, vec
        )
        _add_entity(conn, bob_ep, "player", "222", "Bob")
        conn.commit()
    finally:
        conn.close()

    respx_mock.post("http://stub.local/v1/embeddings").mock(
        return_value=httpx.Response(
            200, json={"data": [{"embedding": vec, "index": 0}]}
        )
    )

    response = client.post(
        "/v1/memory/test-bot/recall",
        json={"query_text": "dungeon", "top_k": 5, "entity_names": ["Alice"]},
    )
    assert response.status_code == 200, response.text
    ids = {r["episode_id"] for r in response.json()["results"]}
    assert alice_ep in ids
    assert bob_ep not in ids


def test_recall_updates_last_recalled_and_count(client: TestClient, respx_mock):
    """Returned episodes have their last_recalled_at and recall_count bumped —
    the side effect that distinguishes recall from search (§10.4)."""
    vec = [1.0] + [0.0] * (EMBEDDING_DIM - 1)

    data_dir = Path(client.app.state.settings.data_dir)
    conn = open_bot_db(data_dir, "test-bot")
    try:
        run_migrations(conn)
        eid = _seed_episode(
            conn, 1748395200000, "Alice tanked", "social", 0.7, vec
        )
        conn.commit()
    finally:
        conn.close()

    respx_mock.post("http://stub.local/v1/embeddings").mock(
        return_value=httpx.Response(
            200, json={"data": [{"embedding": vec, "index": 0}]}
        )
    )

    response = client.post(
        "/v1/memory/test-bot/recall",
        json={"query_text": "Alice", "top_k": 5},
    )
    assert response.status_code == 200, response.text
    assert any(r["episode_id"] == eid for r in response.json()["results"])

    # Verify the bump landed in the DB.
    conn = open_bot_db(data_dir, "test-bot")
    try:
        row = conn.execute(
            "SELECT last_recalled_at, recall_count FROM episodes WHERE episode_id = ?",
            (eid,),
        ).fetchone()
        assert row["recall_count"] == 1
        assert row["last_recalled_at"] is not None
    finally:
        conn.close()
