# SPDX-License-Identifier: GPL-2.0-or-later
import sqlite3
from importlib.resources import files


_META_TABLE_SQL = """
CREATE TABLE IF NOT EXISTS _meta_migrations (
    version INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    filename TEXT NOT NULL
)
"""


def run_migrations(conn: sqlite3.Connection) -> None:
    """Apply all unapplied migrations from db/migrations/*.sql in version order.

    Idempotent: tracks applied versions in `_meta_migrations` and skips any
    migration whose version is already recorded.
    """
    conn.execute(_META_TABLE_SQL)
    conn.commit()

    applied = {
        row[0]
        for row in conn.execute("SELECT version FROM _meta_migrations").fetchall()
    }

    migrations_dir = files("tot_memory.db.migrations")
    entries = sorted(migrations_dir.iterdir(), key=lambda e: e.name)
    for entry in entries:
        if not entry.name.endswith(".sql"):
            continue
        try:
            version = int(entry.name.split("_", 1)[0])
        except ValueError:
            continue
        if version in applied:
            continue
        sql = entry.read_text()
        conn.executescript(sql)
        conn.execute(
            "INSERT INTO _meta_migrations (version, filename) VALUES (?, ?)",
            (version, entry.name),
        )
        conn.commit()


def current_version(conn: sqlite3.Connection) -> int:
    """Return the highest applied migration version, or 0 if none."""
    row = conn.execute("SELECT MAX(version) FROM _meta_migrations").fetchone()
    return row[0] or 0
