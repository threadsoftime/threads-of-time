# SPDX-License-Identifier: GPL-2.0-or-later
import struct
from pathlib import Path

from tot_memory.db.connection import open_bot_db
from tot_memory.db.runner import run_migrations
from tot_memory.db.schema import EMBEDDING_DIM


def test_embeddings_vec_table_exists(tmp_path: Path):
    """Per design subspec §3: sqlite-vec virtual table embeddings_vec."""
    conn = open_bot_db(tmp_path, "test-bot")
    try:
        run_migrations(conn)
        # Virtual tables show up in sqlite_master as type='table'
        row = conn.execute(
            "SELECT name FROM sqlite_master "
            "WHERE type IN ('table', 'virtual') AND name = 'embeddings_vec'"
        ).fetchone()
        assert row is not None
    finally:
        conn.close()


def test_embedding_dim_constant_matches_design_subspec(tmp_path: Path):
    """Per design subspec §3 + §4: EMBEDDING_DIM = 768 (nomic-embed-text)."""
    assert EMBEDDING_DIM == 768


def test_embeddings_vec_accepts_dim_vector(tmp_path: Path):
    """Insert a vector of the design-declared dimension; verify it lands."""
    conn = open_bot_db(tmp_path, "test-bot")
    try:
        run_migrations(conn)
        vec_bytes = struct.pack(f"{EMBEDDING_DIM}f", *([0.0] * EMBEDDING_DIM))
        conn.execute(
            "INSERT INTO embeddings_vec (rowid, embedding) VALUES (1, ?)",
            (vec_bytes,),
        )
        conn.commit()
        count = conn.execute("SELECT COUNT(*) FROM embeddings_vec").fetchone()[0]
        assert count == 1
    finally:
        conn.close()
