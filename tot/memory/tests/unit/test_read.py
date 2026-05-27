# SPDX-License-Identifier: GPL-2.0-or-later
"""Tests for GET /v1/memory/{bot_guid}/episodes/{episode_id} (memory.read).

Per design subspec §10.2. The route returns the full episode row plus its
linked entity rows (via the episode_entities JOIN). 404 when not found.
"""
from __future__ import annotations

import json
import struct
from pathlib import Path

from fastapi.testclient import TestClient

from tot_memory.db.connection import open_bot_db
from tot_memory.db.runner import run_migrations
from tot_memory.db.schema import EMBEDDING_DIM


def test_read_returns_episode_with_entities(client: TestClient):
    """Happy path: episode + entity link returned per §10.2 response shape."""
    vec = [0.1] * EMBEDDING_DIM
    data_dir = Path(client.app.state.settings.data_dir)
    conn = open_bot_db(data_dir, "test-bot")
    try:
        run_migrations(conn)
        cur = conn.execute(
            "INSERT INTO episodes "
            "(timestamp, content_text, episode_type, salience_score, source, metadata) "
            "VALUES (?, ?, ?, ?, ?, ?)",
            (
                1748395200000,
                "Alice told me she usually tanks in BFD.",
                "chat",
                0.6,
                "chat",
                json.dumps({"speaker_name": "Alice", "channel": "party"}),
            ),
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
        ent_cur = conn.execute(
            "INSERT INTO entities (entity_kind, entity_key, display_name) "
            "VALUES (?, ?, ?)",
            ("player", "1234567", "Alice"),
        )
        ent_id = int(ent_cur.lastrowid)
        conn.execute(
            "INSERT INTO episode_entities (episode_id, entity_id, role) "
            "VALUES (?, ?, ?)",
            (eid, ent_id, "subject"),
        )
        conn.commit()
    finally:
        conn.close()

    response = client.get(f"/v1/memory/test-bot/episodes/{eid}")
    assert response.status_code == 200, response.text
    body = response.json()
    assert body["episode_id"] == eid
    assert body["timestamp"] == 1748395200000
    assert body["content_text"] == "Alice told me she usually tanks in BFD."
    assert body["episode_type"] == "chat"
    assert body["salience_score"] == 0.6
    assert body["source"] == "chat"
    assert body["embedding_generated"] is True
    assert body["metadata"] == {"speaker_name": "Alice", "channel": "party"}
    assert len(body["entities"]) == 1
    ent = body["entities"][0]
    assert ent["entity_kind"] == "player"
    assert ent["entity_key"] == "1234567"
    assert ent["display_name"] == "Alice"
    assert ent["role"] == "subject"


def test_read_returns_404_for_missing_episode(client: TestClient):
    """404 when no episode with the given ID exists in the bot's DB."""
    # Open the bot DB so the migrations exist; otherwise the lookup table
    # itself wouldn't exist.
    data_dir = Path(client.app.state.settings.data_dir)
    conn = open_bot_db(data_dir, "test-bot")
    try:
        run_migrations(conn)
    finally:
        conn.close()

    response = client.get("/v1/memory/test-bot/episodes/9999")
    assert response.status_code == 404
    assert response.json()["detail"] == "episode_not_found"
