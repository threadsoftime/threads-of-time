# SPDX-License-Identifier: GPL-2.0-or-later
"""Unit tests for retrieval/dense.py — per design subspec §6 dense component."""

import struct
from pathlib import Path

from tot_memory.db.connection import open_bot_db
from tot_memory.db.runner import run_migrations
from tot_memory.db.schema import EMBEDDING_DIM
from tot_memory.retrieval.dense import DenseResult, dense_search


def _seed_vec(conn, rowid: int, vec: list[float]) -> None:
    conn.execute(
        "INSERT INTO embeddings_vec (rowid, embedding) VALUES (?, ?)",
        (rowid, struct.pack(f"{len(vec)}f", *vec)),
    )


def test_dense_ranks_parallel_vector_above_orthogonal(tmp_path: Path):
    """A query vector aligned with episode A's embedding should rank A above
    an orthogonal episode B."""
    conn = open_bot_db(tmp_path, "test-bot")
    try:
        run_migrations(conn)
        # Episode 1: parallel to query (cosine 1.0, L2 distance 0)
        parallel = [1.0] + [0.0] * (EMBEDDING_DIM - 1)
        # Episode 2: orthogonal (cosine 0.0, L2 distance sqrt(2))
        orthogonal = [0.0, 1.0] + [0.0] * (EMBEDDING_DIM - 2)
        _seed_vec(conn, 1, parallel)
        _seed_vec(conn, 2, orthogonal)
        conn.commit()

        results = dense_search(conn, parallel, top_k=2)
        assert len(results) == 2
        assert results[0].episode_id == 1
        assert results[0].cosine_similarity > results[1].cosine_similarity
    finally:
        conn.close()


def test_dense_distance_to_similarity_conversion(tmp_path: Path):
    """sqlite-vec returns distance (lower=better); we return similarity = 1-distance."""
    conn = open_bot_db(tmp_path, "test-bot")
    try:
        run_migrations(conn)
        vec = [1.0] + [0.0] * (EMBEDDING_DIM - 1)
        _seed_vec(conn, 1, vec)
        conn.commit()

        results = dense_search(conn, vec, top_k=1)
        assert len(results) == 1
        # Identical vectors → L2 distance 0 → similarity 1.0
        assert abs(results[0].cosine_similarity - 1.0) < 1e-5
    finally:
        conn.close()


def test_dense_empty_vec_table_returns_empty(tmp_path: Path):
    """KNN over an empty embeddings_vec returns no results."""
    conn = open_bot_db(tmp_path, "test-bot")
    try:
        run_migrations(conn)
        query = [0.5] * EMBEDDING_DIM
        results = dense_search(conn, query, top_k=5)
        assert results == []
    finally:
        conn.close()


def test_dense_result_is_dataclass_with_fields():
    r = DenseResult(episode_id=7, cosine_similarity=0.91)
    assert r.episode_id == 7
    assert r.cosine_similarity == 0.91
