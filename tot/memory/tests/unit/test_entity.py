# SPDX-License-Identifier: GPL-2.0-or-later
"""Unit tests for retrieval/entity.py — per design subspec §6 entity-filter."""

from pathlib import Path

from tot_memory.db.connection import open_bot_db
from tot_memory.db.runner import run_migrations
from tot_memory.retrieval.entity import entity_filter


def _insert_episode(conn, text: str, etype: str = "social") -> int:
    cur = conn.execute(
        "INSERT INTO episodes (timestamp, content_text, episode_type, salience_score) "
        "VALUES (?, ?, ?, ?)",
        (1748395200000, text, etype, 0.5),
    )
    return int(cur.lastrowid)


def _insert_entity(conn, kind: str, key: str, display: str) -> int:
    cur = conn.execute(
        "INSERT INTO entities (entity_kind, entity_key, display_name) VALUES (?, ?, ?)",
        (kind, key, display),
    )
    return int(cur.lastrowid)


def _link(conn, episode_id: int, entity_id: int, role: str = "participant") -> None:
    conn.execute(
        "INSERT INTO episode_entities (episode_id, entity_id, role) VALUES (?, ?, ?)",
        (episode_id, entity_id, role),
    )


def test_entity_filter_returns_episodes_with_any_named_entity(tmp_path: Path):
    """entity_filter returns episode_ids referencing any of the given names (OR)."""
    conn = open_bot_db(tmp_path, "test-bot")
    try:
        run_migrations(conn)
        e1 = _insert_episode(conn, "Alice and Bob raided")
        e2 = _insert_episode(conn, "Bob alone")
        e3 = _insert_episode(conn, "no entities")
        alice = _insert_entity(conn, "player", "100", "Alice")
        bob = _insert_entity(conn, "player", "200", "Bob")
        _link(conn, e1, alice)
        _link(conn, e1, bob)
        _link(conn, e2, bob)
        conn.commit()

        ids = entity_filter(conn, ["Alice"])
        assert ids == {e1}

        ids = entity_filter(conn, ["Bob"])
        assert ids == {e1, e2}

        ids = entity_filter(conn, ["Alice", "Bob"])
        assert ids == {e1, e2}

        # e3 has no entities — never returned
        assert e3 not in ids
    finally:
        conn.close()


def test_entity_filter_unknown_name_returns_empty(tmp_path: Path):
    """Names not in entities table yield an empty set, no error."""
    conn = open_bot_db(tmp_path, "test-bot")
    try:
        run_migrations(conn)
        assert entity_filter(conn, ["DoesNotExist"]) == set()
    finally:
        conn.close()


def test_entity_filter_empty_list_returns_empty(tmp_path: Path):
    """An empty entity_names list returns the empty set (caller's signal to skip filter)."""
    conn = open_bot_db(tmp_path, "test-bot")
    try:
        run_migrations(conn)
        assert entity_filter(conn, []) == set()
    finally:
        conn.close()


def test_entity_filter_dedupes_when_episode_has_multiple_roles(tmp_path: Path):
    """A single episode that links to one entity in multiple roles appears once."""
    conn = open_bot_db(tmp_path, "test-bot")
    try:
        run_migrations(conn)
        e = _insert_episode(conn, "Carol fights Carol")
        carol = _insert_entity(conn, "player", "300", "Carol")
        _link(conn, e, carol, role="subject")
        _link(conn, e, carol, role="target")
        conn.commit()
        ids = entity_filter(conn, ["Carol"])
        assert ids == {e}
    finally:
        conn.close()
