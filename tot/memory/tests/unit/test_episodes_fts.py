# SPDX-License-Identifier: GPL-2.0-or-later
from pathlib import Path

from tot_memory.db.connection import open_bot_db
from tot_memory.db.runner import run_migrations


def test_episodes_fts_table_exists(tmp_path: Path):
    """Per design subspec §6 (BM25 component): episodes_fts FTS5 vtable."""
    conn = open_bot_db(tmp_path, "test-bot")
    try:
        run_migrations(conn)
        row = conn.execute(
            "SELECT name FROM sqlite_master "
            "WHERE type IN ('table', 'virtual') AND name = 'episodes_fts'"
        ).fetchone()
        assert row is not None
    finally:
        conn.close()


def test_episodes_fts_insert_trigger_indexes_content(tmp_path: Path):
    """AFTER INSERT trigger should push new episodes into the FTS index so
    MATCH queries return them (per design subspec §6 BM25 component)."""
    conn = open_bot_db(tmp_path, "test-bot")
    try:
        run_migrations(conn)
        conn.execute(
            "INSERT INTO episodes (timestamp, content_text, episode_type, salience_score) "
            "VALUES (?, ?, ?, ?)",
            (1748395200000, "Alice taught me how to AoE pull", "social", 0.7),
        )
        conn.commit()
        rows = conn.execute(
            "SELECT rowid FROM episodes_fts WHERE episodes_fts MATCH ?",
            ("AoE",),
        ).fetchall()
        assert len(rows) == 1
    finally:
        conn.close()


def test_episodes_fts_delete_trigger_removes_from_index(tmp_path: Path):
    """AFTER DELETE trigger should remove deleted episodes from FTS index."""
    conn = open_bot_db(tmp_path, "test-bot")
    try:
        run_migrations(conn)
        cur = conn.execute(
            "INSERT INTO episodes (timestamp, content_text, episode_type, salience_score) "
            "VALUES (?, ?, ?, ?)",
            (1748395200000, "Bob raided Molten Core last night", "combat", 0.6),
        )
        ep_id = cur.lastrowid
        conn.commit()

        assert len(
            conn.execute(
                "SELECT rowid FROM episodes_fts WHERE episodes_fts MATCH ?",
                ("Molten",),
            ).fetchall()
        ) == 1

        conn.execute("DELETE FROM episodes WHERE episode_id = ?", (ep_id,))
        conn.commit()

        assert len(
            conn.execute(
                "SELECT rowid FROM episodes_fts WHERE episodes_fts MATCH ?",
                ("Molten",),
            ).fetchall()
        ) == 0
    finally:
        conn.close()


def test_episodes_fts_update_trigger_refreshes_index(tmp_path: Path):
    """AFTER UPDATE OF content_text trigger should re-index on edits."""
    conn = open_bot_db(tmp_path, "test-bot")
    try:
        run_migrations(conn)
        cur = conn.execute(
            "INSERT INTO episodes (timestamp, content_text, episode_type, salience_score) "
            "VALUES (?, ?, ?, ?)",
            (1748395200000, "Old text about gnomes", "observation", 0.4),
        )
        ep_id = cur.lastrowid
        conn.commit()

        # Old term findable; new term not yet
        assert len(
            conn.execute(
                "SELECT rowid FROM episodes_fts WHERE episodes_fts MATCH ?",
                ("gnomes",),
            ).fetchall()
        ) == 1
        assert len(
            conn.execute(
                "SELECT rowid FROM episodes_fts WHERE episodes_fts MATCH ?",
                ("dragons",),
            ).fetchall()
        ) == 0

        conn.execute(
            "UPDATE episodes SET content_text = ? WHERE episode_id = ?",
            ("New text about dragons", ep_id),
        )
        conn.commit()

        # After update: old term gone, new term findable
        assert len(
            conn.execute(
                "SELECT rowid FROM episodes_fts WHERE episodes_fts MATCH ?",
                ("gnomes",),
            ).fetchall()
        ) == 0
        assert len(
            conn.execute(
                "SELECT rowid FROM episodes_fts WHERE episodes_fts MATCH ?",
                ("dragons",),
            ).fetchall()
        ) == 1
    finally:
        conn.close()
