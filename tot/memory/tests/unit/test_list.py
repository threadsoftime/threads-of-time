# SPDX-License-Identifier: GPL-2.0-or-later
"""Tests for GET /v1/memory/{bot_guid}/episodes (memory.list).

Per design subspec §10.5. Paginated list of episodes in reverse-chronological
order. Filters: episode_type, entity_name, before (ms), after (ms).
"""
from __future__ import annotations

from pathlib import Path

from fastapi.testclient import TestClient

from tot_memory.db.connection import open_bot_db
from tot_memory.db.runner import run_migrations


def _seed(conn, ts_ms: int, text: str, etype: str, salience: float = 0.5) -> int:
    cur = conn.execute(
        "INSERT INTO episodes (timestamp, content_text, episode_type, salience_score) "
        "VALUES (?, ?, ?, ?)",
        (ts_ms, text, etype, salience),
    )
    return int(cur.lastrowid)


def _link_entity(conn, eid: int, kind: str, key: str, name: str) -> int:
    cur = conn.execute(
        "INSERT INTO entities (entity_kind, entity_key, display_name) "
        "VALUES (?, ?, ?)",
        (kind, key, name),
    )
    ent_id = int(cur.lastrowid)
    conn.execute(
        "INSERT INTO episode_entities (episode_id, entity_id, role) "
        "VALUES (?, ?, ?)",
        (eid, ent_id, "subject"),
    )
    return ent_id


def test_list_returns_reverse_chronological_with_total(client: TestClient):
    """Happy path: 3 episodes, list returns them newest first with total."""
    data_dir = Path(client.app.state.settings.data_dir)
    conn = open_bot_db(data_dir, "test-bot")
    try:
        run_migrations(conn)
        e_old = _seed(conn, 1000, "old", "chat")
        e_mid = _seed(conn, 2000, "mid", "chat")
        e_new = _seed(conn, 3000, "new", "chat")
        conn.commit()
    finally:
        conn.close()

    response = client.get("/v1/memory/test-bot/episodes")
    assert response.status_code == 200, response.text
    body = response.json()
    assert body["total"] == 3
    assert body["limit"] == 50
    assert body["offset"] == 0
    assert body["has_more"] is False
    ids = [r["episode_id"] for r in body["results"]]
    assert ids == [e_new, e_mid, e_old]


def test_list_pagination_boundary(client: TestClient):
    """5 episodes + limit=2 + offset=2 returns the middle page; has_more=True
    when more rows remain after the current page."""
    data_dir = Path(client.app.state.settings.data_dir)
    conn = open_bot_db(data_dir, "test-bot")
    try:
        run_migrations(conn)
        seeded = [_seed(conn, 1000 + i, f"ep{i}", "chat") for i in range(5)]
        conn.commit()
    finally:
        conn.close()

    response = client.get("/v1/memory/test-bot/episodes?limit=2&offset=2")
    assert response.status_code == 200
    body = response.json()
    assert body["total"] == 5
    assert body["limit"] == 2
    assert body["offset"] == 2
    assert len(body["results"]) == 2
    # Reverse-chronological: newest (seeded[4]) at offset 0;
    # offset=2 lands on seeded[2] then seeded[1].
    ids = [r["episode_id"] for r in body["results"]]
    assert ids == [seeded[2], seeded[1]]
    assert body["has_more"] is True  # seeded[0] remains beyond this page


def test_list_filters_by_type_and_entity(client: TestClient):
    """episode_type + entity_name filters narrow the result set + total."""
    data_dir = Path(client.app.state.settings.data_dir)
    conn = open_bot_db(data_dir, "test-bot")
    try:
        run_migrations(conn)
        alice_chat = _seed(conn, 3000, "alice chat", "chat")
        _link_entity(conn, alice_chat, "player", "111", "Alice")

        alice_combat = _seed(conn, 2000, "alice combat", "combat")
        _link_entity(conn, alice_combat, "player", "222", "Alice")

        bob_chat = _seed(conn, 1000, "bob chat", "chat")
        _link_entity(conn, bob_chat, "player", "333", "Bob")

        conn.commit()
    finally:
        conn.close()

    # Filter by type=chat → 2 results (alice_chat + bob_chat); not the combat.
    response = client.get("/v1/memory/test-bot/episodes?episode_type=chat")
    assert response.status_code == 200
    body = response.json()
    assert body["total"] == 2
    ids = {r["episode_id"] for r in body["results"]}
    assert ids == {alice_chat, bob_chat}

    # Filter by entity_name=Alice → 2 results across both types.
    response = client.get("/v1/memory/test-bot/episodes?entity_name=Alice")
    assert response.status_code == 200
    body = response.json()
    assert body["total"] == 2
    ids = {r["episode_id"] for r in body["results"]}
    assert ids == {alice_chat, alice_combat}

    # Combined: type=chat AND entity=Alice → 1 result.
    response = client.get(
        "/v1/memory/test-bot/episodes?episode_type=chat&entity_name=Alice"
    )
    assert response.status_code == 200
    body = response.json()
    assert body["total"] == 1
    assert body["results"][0]["episode_id"] == alice_chat
