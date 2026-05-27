# SPDX-License-Identifier: GPL-2.0-or-later
"""Tests for DELETE /v1/memory/{bot_guid}/episodes/{episode_id} (memory.delete).

Per design subspec §10.7. Hard-delete:
- episode_entities rows cascade via the FK ON DELETE CASCADE.
- entities.total_episodes is decremented by the ee_ad trigger (§2.3).
- embeddings_vec row must be deleted explicitly — sqlite-vec virtual tables
  don't honor FK cascades.

Returns 204 No Content (plan abbreviation) on success. 404 when the episode
is missing.
"""
from __future__ import annotations

import struct
from pathlib import Path

from fastapi.testclient import TestClient

from tot_memory.db.connection import open_bot_db
from tot_memory.db.runner import run_migrations
from tot_memory.db.schema import EMBEDDING_DIM


def _seed_with_entity_and_vec(conn) -> tuple[int, int]:
    """Insert one episode + one entity + one episode_entities link + one
    embeddings_vec row. Returns (episode_id, entity_id)."""
    cur = conn.execute(
        "INSERT INTO episodes (timestamp, content_text, episode_type, salience_score) "
        "VALUES (?, ?, ?, ?)",
        (1000, "Alice tanked", "social", 0.7),
    )
    eid = int(cur.lastrowid)
    conn.execute(
        "INSERT INTO embeddings_vec (rowid, embedding) VALUES (?, ?)",
        (eid, struct.pack(f"{EMBEDDING_DIM}f", *([0.1] * EMBEDDING_DIM))),
    )
    conn.execute(
        "UPDATE episodes SET content_embedding_id = ? WHERE episode_id = ?",
        (eid, eid),
    )
    ent_cur = conn.execute(
        "INSERT INTO entities (entity_kind, entity_key, display_name) "
        "VALUES (?, ?, ?)",
        ("player", "111", "Alice"),
    )
    ent_id = int(ent_cur.lastrowid)
    conn.execute(
        "INSERT INTO episode_entities (episode_id, entity_id, role) "
        "VALUES (?, ?, ?)",
        (eid, ent_id, "subject"),
    )
    return eid, ent_id


def test_delete_removes_episode_entity_link_and_vec_row(client: TestClient):
    """Happy path: episode, episode_entities link, AND embeddings_vec row all
    gone after the DELETE. The entity row itself remains (it may be linked
    to other episodes). entities.total_episodes is decremented by the trigger."""
    data_dir = Path(client.app.state.settings.data_dir)
    conn = open_bot_db(data_dir, "test-bot")
    try:
        run_migrations(conn)
        eid, ent_id = _seed_with_entity_and_vec(conn)
        conn.commit()
        # Sanity: total_episodes was incremented by ee_ai trigger.
        pre = conn.execute(
            "SELECT total_episodes FROM entities WHERE entity_id = ?", (ent_id,)
        ).fetchone()[0]
        assert pre == 1
    finally:
        conn.close()

    response = client.delete(f"/v1/memory/test-bot/episodes/{eid}")
    assert response.status_code == 204, response.text
    assert response.content == b""

    conn = open_bot_db(data_dir, "test-bot")
    try:
        # Episode gone.
        assert (
            conn.execute(
                "SELECT episode_id FROM episodes WHERE episode_id = ?", (eid,)
            ).fetchone()
            is None
        )
        # episode_entities link cascade-deleted.
        assert (
            conn.execute(
                "SELECT episode_id FROM episode_entities WHERE episode_id = ?",
                (eid,),
            ).fetchone()
            is None
        )
        # embeddings_vec row explicitly deleted (no FK cascade for vec0).
        assert (
            conn.execute(
                "SELECT rowid FROM embeddings_vec WHERE rowid = ?", (eid,)
            ).fetchone()
            is None
        )
        # Entity row still exists (it may have other episodes). total_episodes
        # is decremented to 0 by the ee_ad trigger.
        post = conn.execute(
            "SELECT total_episodes FROM entities WHERE entity_id = ?", (ent_id,)
        ).fetchone()[0]
        assert post == 0
    finally:
        conn.close()


def test_delete_404_when_missing(client: TestClient):
    """No matching episode_id → 404 with detail episode_not_found."""
    data_dir = Path(client.app.state.settings.data_dir)
    conn = open_bot_db(data_dir, "test-bot")
    try:
        run_migrations(conn)
    finally:
        conn.close()

    response = client.delete("/v1/memory/test-bot/episodes/99999")
    assert response.status_code == 404
    assert response.json()["detail"] == "episode_not_found"
