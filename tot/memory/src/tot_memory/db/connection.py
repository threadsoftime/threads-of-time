# SPDX-License-Identifier: GPL-2.0-or-later
import sqlite3
from pathlib import Path

import sqlite_vec


def open_bot_db(data_dir: Path, bot_guid: str) -> sqlite3.Connection:
    """Open the per-bot SQLite database with sqlite-vec loaded.

    Per design subspec §9: one DB per bot_guid at
    data_dir/<bot_guid>/memory.sqlite.
    """
    bot_dir = Path(data_dir) / bot_guid
    bot_dir.mkdir(parents=True, exist_ok=True)
    db_path = bot_dir / "memory.sqlite"

    conn = sqlite3.connect(str(db_path))
    conn.row_factory = sqlite3.Row
    conn.enable_load_extension(True)
    sqlite_vec.load(conn)
    conn.enable_load_extension(False)
    conn.execute("PRAGMA journal_mode = WAL")
    conn.execute("PRAGMA foreign_keys = ON")
    return conn
