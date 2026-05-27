# SPDX-License-Identifier: GPL-2.0-or-later
from pathlib import Path

import pytest

from tot_memory.db.connection import open_bot_db
from tot_memory.db.runner import run_migrations


def test_entities_table_exists(tmp_path: Path):
    conn = open_bot_db(tmp_path, "test-bot")
    try:
        run_migrations(conn)
        row = conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='entities'"
        ).fetchone()
        assert row is not None
    finally:
        conn.close()


def test_episode_entities_join_table_exists(tmp_path: Path):
    """Per design subspec §2: M:N relationship between episodes and entities
    via episode_entities join table with role enum."""
    conn = open_bot_db(tmp_path, "test-bot")
    try:
        run_migrations(conn)
        row = conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='episode_entities'"
        ).fetchone()
        assert row is not None
    finally:
        conn.close()


def test_entities_kind_check_constraint(tmp_path: Path):
    """Per design subspec §2.1: entity_kind must be one of
    player|npc|item|location|faction."""
    conn = open_bot_db(tmp_path, "test-bot")
    try:
        run_migrations(conn)
        with pytest.raises(Exception):
            conn.execute(
                "INSERT INTO entities (entity_kind, entity_key, display_name) "
                "VALUES (?, ?, ?)",
                ("alien", "abc", "Marvin"),
            )
    finally:
        conn.close()


def test_episode_entities_role_check_constraint(tmp_path: Path):
    """Per design subspec §2.1: role must be one of
    subject|participant|target|witness."""
    conn = open_bot_db(tmp_path, "test-bot")
    try:
        run_migrations(conn)
        # Create supporting rows
        ep_cur = conn.execute(
            "INSERT INTO episodes (timestamp, content_text, episode_type) VALUES (?, ?, ?)",
            (1748395200000, "Hello Alice", "chat"),
        )
        ep_id = ep_cur.lastrowid
        ent_cur = conn.execute(
            "INSERT INTO entities (entity_kind, entity_key, display_name) VALUES (?, ?, ?)",
            ("player", "12345", "Alice"),
        )
        ent_id = ent_cur.lastrowid

        with pytest.raises(Exception):
            conn.execute(
                "INSERT INTO episode_entities (episode_id, entity_id, role) VALUES (?, ?, ?)",
                (ep_id, ent_id, "bystander"),
            )
    finally:
        conn.close()


def test_total_episodes_trigger_increments(tmp_path: Path):
    """Per design subspec §2.3: AFTER INSERT trigger on episode_entities
    increments entities.total_episodes."""
    conn = open_bot_db(tmp_path, "test-bot")
    try:
        run_migrations(conn)
        ep_cur = conn.execute(
            "INSERT INTO episodes (timestamp, content_text, episode_type) VALUES (?, ?, ?)",
            (1748395200000, "Hello Alice", "chat"),
        )
        ep_id = ep_cur.lastrowid
        ent_cur = conn.execute(
            "INSERT INTO entities (entity_kind, entity_key, display_name) VALUES (?, ?, ?)",
            ("player", "12345", "Alice"),
        )
        ent_id = ent_cur.lastrowid

        # Initial total_episodes = 0
        before = conn.execute(
            "SELECT total_episodes FROM entities WHERE entity_id = ?", (ent_id,)
        ).fetchone()[0]
        assert before == 0

        conn.execute(
            "INSERT INTO episode_entities (episode_id, entity_id, role) VALUES (?, ?, ?)",
            (ep_id, ent_id, "subject"),
        )
        after = conn.execute(
            "SELECT total_episodes FROM entities WHERE entity_id = ?", (ent_id,)
        ).fetchone()[0]
        assert after == 1
    finally:
        conn.close()


def test_total_episodes_trigger_decrements(tmp_path: Path):
    """Per design subspec §2.3: AFTER DELETE trigger on episode_entities
    decrements entities.total_episodes (clamped at 0)."""
    conn = open_bot_db(tmp_path, "test-bot")
    try:
        run_migrations(conn)
        ep_cur = conn.execute(
            "INSERT INTO episodes (timestamp, content_text, episode_type) VALUES (?, ?, ?)",
            (1748395200000, "Hello Alice", "chat"),
        )
        ep_id = ep_cur.lastrowid
        ent_cur = conn.execute(
            "INSERT INTO entities (entity_kind, entity_key, display_name) VALUES (?, ?, ?)",
            ("player", "12345", "Alice"),
        )
        ent_id = ent_cur.lastrowid

        conn.execute(
            "INSERT INTO episode_entities (episode_id, entity_id, role) VALUES (?, ?, ?)",
            (ep_id, ent_id, "subject"),
        )
        conn.execute(
            "DELETE FROM episode_entities WHERE episode_id = ? AND entity_id = ? AND role = ?",
            (ep_id, ent_id, "subject"),
        )
        after = conn.execute(
            "SELECT total_episodes FROM entities WHERE entity_id = ?", (ent_id,)
        ).fetchone()[0]
        assert after == 0
    finally:
        conn.close()
