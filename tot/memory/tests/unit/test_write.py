# SPDX-License-Identifier: GPL-2.0-or-later
"""Tests for POST /v1/memory/{bot_guid}/episodes (memory.write).

Per design subspec §5 (sync embedding pipeline with graceful degradation) and
§10.1 (memory.write request/response). The write path:

  1. Tries to embed the content synchronously.
  2. Inserts the episode row.
  3. If embedding succeeded, inserts the vector + sets content_embedding_id.
     If embedding failed (network / 5xx), the episode is still written with
     content_embedding_id = NULL and the response says embedding_generated=False.
  4. Upserts entities + episode_entities links.

The graceful-degradation behavior is the load-bearing invariant — the episode
must remain BM25-searchable + readable even when the embeddings endpoint is down.
A backfill CLI (§5.4) handles eventual consistency.
"""
from __future__ import annotations

import httpx
import pytest
from fastapi.testclient import TestClient

from tot_memory.db.schema import EMBEDDING_DIM


def test_write_episode_creates_row_and_embedding(client: TestClient, respx_mock):
    """Happy path: embeddings endpoint returns a valid vector; episode written
    with embedding linked; entities upserted."""
    respx_mock.post("http://stub.local/v1/embeddings").mock(
        return_value=httpx.Response(
            200,
            json={"data": [{"embedding": [0.1] * EMBEDDING_DIM, "index": 0}]},
        )
    )
    response = client.post(
        "/v1/memory/test-bot/episodes",
        json={
            "content_text": "Alice taught me how to AoE pull",
            "episode_type": "social",
            "timestamp": 1748395200000,
            "salience_hint": 0.7,
            "entities": [
                {
                    "entity_kind": "player",
                    "entity_key": "1234567",
                    "display_name": "Alice",
                    "role": "subject",
                }
            ],
            "metadata": {"speaker_name": "Alice"},
            "source": "chat",
        },
    )
    assert response.status_code == 201, response.text
    body = response.json()
    assert isinstance(body["episode_id"], int)
    assert body["embedding_generated"] is True

    # Verify the row + embedding actually landed in the bot's DB
    from pathlib import Path
    from tot_memory.db.connection import open_bot_db

    data_dir = Path(client.app.state.settings.data_dir)
    conn = open_bot_db(data_dir, "test-bot")
    try:
        row = conn.execute(
            "SELECT episode_id, content_text, episode_type, content_embedding_id, "
            "salience_score, source FROM episodes"
        ).fetchone()
        assert row is not None
        assert row["episode_id"] == body["episode_id"]
        assert row["content_text"] == "Alice taught me how to AoE pull"
        assert row["episode_type"] == "social"
        assert row["content_embedding_id"] == body["episode_id"]
        assert row["salience_score"] == pytest.approx(0.7)
        assert row["source"] == "chat"

        # Embedding row exists in vec table
        vec_count = conn.execute(
            "SELECT COUNT(*) FROM embeddings_vec WHERE rowid = ?",
            (body["episode_id"],),
        ).fetchone()[0]
        assert vec_count == 1

        # Entity + join row written
        entities = conn.execute(
            "SELECT entity_kind, entity_key, display_name FROM entities"
        ).fetchall()
        assert len(entities) == 1
        assert entities[0]["entity_kind"] == "player"
        assert entities[0]["entity_key"] == "1234567"
        assert entities[0]["display_name"] == "Alice"

        join_rows = conn.execute(
            "SELECT role FROM episode_entities WHERE episode_id = ?",
            (body["episode_id"],),
        ).fetchall()
        assert len(join_rows) == 1
        assert join_rows[0]["role"] == "subject"
    finally:
        conn.close()


def test_write_episode_succeeds_with_null_embedding_when_endpoint_down(
    client: TestClient, respx_mock
):
    """Graceful degradation per design subspec §5: the embeddings endpoint is
    down (timeout / 503), but the episode is still written with
    content_embedding_id = NULL. The response is 201 (not 503 / 500) with
    embedding_generated=False. A backfill CLI handles eventual consistency.

    Status 503 is reserved for service-level failures the caller must retry;
    a missing embedding is a documented degraded mode, not a failed write.
    """
    respx_mock.post("http://stub.local/v1/embeddings").mock(
        return_value=httpx.Response(503, json={"error": "model loading"})
    )
    response = client.post(
        "/v1/memory/test-bot/episodes",
        json={
            "content_text": "Bot saw a player log in nearby",
            "episode_type": "observation",
            "timestamp": 1748395200000,
            "entities": [],
        },
    )
    assert response.status_code == 201, response.text
    body = response.json()
    assert isinstance(body["episode_id"], int)
    assert body["embedding_generated"] is False, (
        "graceful degradation: episode must be written even when embed fails"
    )

    # The episode row IS persisted; content_embedding_id IS NULL.
    from pathlib import Path
    from tot_memory.db.connection import open_bot_db

    data_dir = Path(client.app.state.settings.data_dir)
    conn = open_bot_db(data_dir, "test-bot")
    try:
        row = conn.execute(
            "SELECT episode_id, content_text, content_embedding_id "
            "FROM episodes WHERE episode_id = ?",
            (body["episode_id"],),
        ).fetchone()
        assert row is not None
        assert row["content_text"] == "Bot saw a player log in nearby"
        assert row["content_embedding_id"] is None

        # No vec row was inserted
        vec_count = conn.execute(
            "SELECT COUNT(*) FROM embeddings_vec WHERE rowid = ?",
            (body["episode_id"],),
        ).fetchone()[0]
        assert vec_count == 0
    finally:
        conn.close()
