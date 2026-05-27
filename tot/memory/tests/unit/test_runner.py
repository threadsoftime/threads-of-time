# SPDX-License-Identifier: GPL-2.0-or-later
from pathlib import Path
from tot_memory.db.connection import open_bot_db
from tot_memory.db.runner import run_migrations, current_version


def test_run_migrations_creates_meta_table(tmp_path: Path):
    conn = open_bot_db(tmp_path, "test-bot")
    try:
        run_migrations(conn)
        cursor = conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='_meta_migrations'"
        )
        assert cursor.fetchone() is not None
    finally:
        conn.close()


def test_run_migrations_is_idempotent(tmp_path: Path):
    conn = open_bot_db(tmp_path, "test-bot")
    try:
        run_migrations(conn)
        v1 = current_version(conn)
        run_migrations(conn)
        v2 = current_version(conn)
        assert v1 == v2
    finally:
        conn.close()
