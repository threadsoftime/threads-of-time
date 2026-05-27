# SPDX-License-Identifier: GPL-2.0-or-later
from pathlib import Path
import pytest
from tot_memory.db.connection import open_bot_db


def test_open_bot_db_creates_per_bot_file(tmp_path: Path):
    bot_guid = "abcdef12-3456-7890-abcd-ef1234567890"
    conn = open_bot_db(tmp_path, bot_guid)
    try:
        path = tmp_path / bot_guid / "memory.sqlite"
        assert path.exists()
    finally:
        conn.close()


def test_open_bot_db_loads_sqlite_vec(tmp_path: Path):
    conn = open_bot_db(tmp_path, "test-bot")
    try:
        cursor = conn.execute("SELECT vec_version()")
        version = cursor.fetchone()[0]
        assert version  # sqlite-vec extension is loaded
    finally:
        conn.close()
