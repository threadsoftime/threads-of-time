# SPDX-License-Identifier: GPL-2.0-or-later
from pathlib import Path

import pytest

from tot_memory.db.connection import open_bot_db
from tot_memory.db.runner import run_migrations


def test_episodes_table_exists(tmp_path: Path):
    conn = open_bot_db(tmp_path, "test-bot")
    try:
        run_migrations(conn)
        row = conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='episodes'"
        ).fetchone()
        assert row is not None
    finally:
        conn.close()


def test_episodes_columns_match_design_subspec(tmp_path: Path):
    """Per design subspec §1: episode_id, timestamp, content_text,
    content_embedding_id, episode_type, salience_score, created_at,
    last_recalled_at, recall_count."""
    conn = open_bot_db(tmp_path, "test-bot")
    try:
        run_migrations(conn)
        cols = {row[1] for row in conn.execute("PRAGMA table_info(episodes)").fetchall()}
        expected = {
            "episode_id",
            "timestamp",
            "content_text",
            "content_embedding_id",
            "episode_type",
            "salience_score",
            "created_at",
            "last_recalled_at",
            "recall_count",
        }
        missing = expected - cols
        assert not missing, f"missing columns: {missing}"
    finally:
        conn.close()


def test_episodes_check_constraints_reject_invalid_values(tmp_path: Path):
    """Per design subspec §1.1: CHECK constraints on content_text length,
    episode_type enum, salience_score range, recall_count >= 0."""
    conn = open_bot_db(tmp_path, "test-bot")
    try:
        run_migrations(conn)

        # Invalid episode_type should be rejected
        with pytest.raises(Exception):
            conn.execute(
                "INSERT INTO episodes (timestamp, content_text, episode_type) "
                "VALUES (?, ?, ?)",
                (1748395200000, "hello", "not_a_real_type"),
            )

        # Empty content_text should be rejected
        with pytest.raises(Exception):
            conn.execute(
                "INSERT INTO episodes (timestamp, content_text, episode_type) "
                "VALUES (?, ?, ?)",
                (1748395200000, "", "chat"),
            )

        # salience_score out of range should be rejected
        with pytest.raises(Exception):
            conn.execute(
                "INSERT INTO episodes (timestamp, content_text, episode_type, salience_score) "
                "VALUES (?, ?, ?, ?)",
                (1748395200000, "hello", "chat", 1.5),
            )
    finally:
        conn.close()


def test_episodes_valid_insert_succeeds(tmp_path: Path):
    """Verify a valid episode insert works end-to-end."""
    conn = open_bot_db(tmp_path, "test-bot")
    try:
        run_migrations(conn)
        cursor = conn.execute(
            "INSERT INTO episodes (timestamp, content_text, episode_type, salience_score) "
            "VALUES (?, ?, ?, ?)",
            (1748395200000, "Alice said hi", "chat", 0.6),
        )
        assert cursor.lastrowid is not None
        conn.commit()
        row = conn.execute(
            "SELECT episode_type, salience_score, recall_count, source FROM episodes WHERE episode_id = ?",
            (cursor.lastrowid,),
        ).fetchone()
        assert row["episode_type"] == "chat"
        assert row["salience_score"] == 0.6
        assert row["recall_count"] == 0
        assert row["source"] == "self"  # default per design subspec §1.1
    finally:
        conn.close()
