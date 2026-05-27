# SPDX-License-Identifier: GPL-2.0-or-later
"""Tests for PATCH /v1/memory/{bot_guid}/episodes/{episode_id} (memory.update).

Per design subspec §10.6. Mutable fields: content_text, salience_score,
metadata. Updating content_text triggers a re-embedding (sync, same budget
as the initial write). Updating salience_score or metadata is fast (no
re-embedding). 404 when the episode is missing. 400 when no fields are
supplied (no_fields_to_update).
"""
from __future__ import annotations

import json
import struct
from pathlib import Path

import httpx
import pytest
from fastapi.testclient import TestClient

from tot_memory.db.connection import open_bot_db
from tot_memory.db.runner import run_migrations
from tot_memory.db.schema import EMBEDDING_DIM


def _seed(
    conn,
    text: str = "original text",
    salience: float = 0.5,
    metadata: dict | None = None,
) -> int:
    cur = conn.execute(
        "INSERT INTO episodes "
        "(timestamp, content_text, episode_type, salience_score, metadata) "
        "VALUES (?, ?, ?, ?, ?)",
        (
            1000,
            text,
            "chat",
            salience,
            json.dumps(metadata) if metadata is not None else None,
        ),
    )
    return int(cur.lastrowid)


def test_update_content_text_reembeds(client: TestClient, respx_mock):
    """Updating content_text triggers a re-embedding; the new vector replaces
    the old one in embeddings_vec; the response reports reembedded=true."""
    new_vec = [0.5] * EMBEDDING_DIM

    data_dir = Path(client.app.state.settings.data_dir)
    conn = open_bot_db(data_dir, "test-bot")
    try:
        run_migrations(conn)
        eid = _seed(conn, text="original text")
        # Pre-existing embedding (zeros) so we can verify the row was replaced.
        conn.execute(
            "INSERT INTO embeddings_vec (rowid, embedding) VALUES (?, ?)",
            (eid, struct.pack(f"{EMBEDDING_DIM}f", *([0.0] * EMBEDDING_DIM))),
        )
        conn.execute(
            "UPDATE episodes SET content_embedding_id = ? WHERE episode_id = ?",
            (eid, eid),
        )
        conn.commit()
    finally:
        conn.close()

    embed_route = respx_mock.post("http://stub.local/v1/embeddings").mock(
        return_value=httpx.Response(
            200, json={"data": [{"embedding": new_vec, "index": 0}]}
        )
    )

    response = client.patch(
        f"/v1/memory/test-bot/episodes/{eid}",
        json={"content_text": "rewritten text"},
    )
    assert response.status_code == 200, response.text
    body = response.json()
    assert body["episode_id"] == eid
    assert "content_text" in body["updated_fields"]
    assert body["reembedded"] is True
    assert embed_route.called

    # Verify DB state: new text, vec row replaced.
    conn = open_bot_db(data_dir, "test-bot")
    try:
        row = conn.execute(
            "SELECT content_text, content_embedding_id FROM episodes WHERE episode_id = ?",
            (eid,),
        ).fetchone()
        assert row["content_text"] == "rewritten text"
        assert row["content_embedding_id"] == eid

        stored_vec = conn.execute(
            "SELECT embedding FROM embeddings_vec WHERE rowid = ?", (eid,)
        ).fetchone()
        # Unpack and verify it's the new vector (0.5 floats).
        unpacked = struct.unpack(
            f"{EMBEDDING_DIM}f", stored_vec["embedding"]
        )
        assert unpacked[0] == pytest.approx(0.5)
    finally:
        conn.close()


def test_update_salience_only_skips_embedding(client: TestClient, respx_mock):
    """Updating salience_score (without content_text) does NOT call the
    embedding endpoint — reembedded=false in the response."""
    data_dir = Path(client.app.state.settings.data_dir)
    conn = open_bot_db(data_dir, "test-bot")
    try:
        run_migrations(conn)
        eid = _seed(conn, salience=0.3)
        conn.commit()
    finally:
        conn.close()

    # Do not mock embeddings — if the route calls it, respx raises.
    response = client.patch(
        f"/v1/memory/test-bot/episodes/{eid}",
        json={"salience_score": 0.9},
    )
    assert response.status_code == 200, response.text
    body = response.json()
    assert body["reembedded"] is False
    assert body["updated_fields"] == ["salience_score"]

    conn = open_bot_db(data_dir, "test-bot")
    try:
        row = conn.execute(
            "SELECT salience_score FROM episodes WHERE episode_id = ?", (eid,)
        ).fetchone()
        assert row["salience_score"] == pytest.approx(0.9)
    finally:
        conn.close()


def test_update_404_when_episode_missing(client: TestClient):
    data_dir = Path(client.app.state.settings.data_dir)
    conn = open_bot_db(data_dir, "test-bot")
    try:
        run_migrations(conn)
    finally:
        conn.close()

    response = client.patch(
        "/v1/memory/test-bot/episodes/99999",
        json={"salience_score": 0.5},
    )
    assert response.status_code == 404
    assert response.json()["detail"] == "episode_not_found"


def test_update_400_when_no_fields_supplied(client: TestClient):
    data_dir = Path(client.app.state.settings.data_dir)
    conn = open_bot_db(data_dir, "test-bot")
    try:
        run_migrations(conn)
        eid = _seed(conn)
        conn.commit()
    finally:
        conn.close()

    response = client.patch(
        f"/v1/memory/test-bot/episodes/{eid}",
        json={},
    )
    assert response.status_code == 400
    assert response.json()["detail"] == "no_fields_to_update"
