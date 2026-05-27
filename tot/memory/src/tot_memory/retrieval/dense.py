# SPDX-License-Identifier: GPL-2.0-or-later
"""Dense vector retrieval via sqlite-vec KNN over ``embeddings_vec``.

Per design subspec §6 (dense component). The ``embeddings_vec`` virtual table
stores L2-normalised 768-dim vectors (rowid == ``episodes.episode_id``;
see §3.1). sqlite-vec returns L2 distance — lower is closer; we convert to a
similarity proxy by ``1 - distance`` so that higher = more relevant, matching
the hybrid scorer's convention.

For strict cosine similarity of L2-normalised vectors the exact identity is
``cos_sim = 1 - d²/2`` (§6.2). The simpler ``1 - distance`` is used here
because the downstream :mod:`tot_memory.retrieval.rerank` orchestrator
max-normalises dense scores to ``[0, 1]`` before combining — the monotonic
relationship between distance and rank is what matters, not the exact value.
"""

import sqlite3
import struct
from dataclasses import dataclass


@dataclass(frozen=True)
class DenseResult:
    """A single dense-KNN hit.

    Attributes:
        episode_id: ``rowid`` of the matched ``embeddings_vec`` row, which by
            convention (§3.1) equals ``episodes.episode_id``.
        cosine_similarity: ``1 - L2_distance`` — higher = more relevant.
    """

    episode_id: int
    cosine_similarity: float


def dense_search(
    conn: sqlite3.Connection,
    query_vec: list[float],
    top_k: int,
) -> list[DenseResult]:
    """Return the top-``top_k`` episodes by L2 distance to ``query_vec``.

    Packs the query vector into the raw float32 bytes format that sqlite-vec
    expects for ``MATCH``. Empty ``embeddings_vec`` table → empty list.
    """
    if top_k <= 0:
        return []
    query_bytes = struct.pack(f"{len(query_vec)}f", *query_vec)
    rows = conn.execute(
        "SELECT rowid, distance FROM embeddings_vec "
        "WHERE embedding MATCH ? AND k = ? "
        "ORDER BY distance",
        (query_bytes, top_k),
    ).fetchall()
    return [
        DenseResult(episode_id=int(r[0]), cosine_similarity=1.0 - float(r[1]))
        for r in rows
    ]
