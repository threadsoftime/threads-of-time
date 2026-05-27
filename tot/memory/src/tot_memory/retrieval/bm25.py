# SPDX-License-Identifier: GPL-2.0-or-later
"""BM25 keyword retrieval over the ``episodes_fts`` FTS5 virtual table.

Per design subspec §6 (BM25 component): SQLite's built-in ``bm25()`` returns a
negative rank where *lower* values indicate a better match. The hybrid scorer
in :mod:`tot_memory.retrieval.hybrid` expects *higher = more relevant*, so this
module negates the rank before returning it.
"""

import sqlite3
from dataclasses import dataclass


@dataclass(frozen=True)
class Bm25Result:
    """A single BM25 hit.

    Attributes:
        episode_id: Primary-key id from the ``episodes`` table.
        bm25_score: Negated FTS5 ``bm25()`` value — *higher = better match*.
    """

    episode_id: int
    bm25_score: float


def bm25_search(conn: sqlite3.Connection, query: str, top_k: int) -> list[Bm25Result]:
    """Return up to ``top_k`` episodes whose ``content_text`` matches ``query``.

    The query is passed straight to FTS5's ``MATCH`` operator. Empty result set
    if the query does not match anything; the caller is responsible for
    validating non-empty queries.

    SQLite's ``bm25()`` returns lower-is-better; we negate so higher scores
    rank first, matching the convention used by the hybrid scorer (§6.2).
    """
    rows = conn.execute(
        "SELECT rowid, -bm25(episodes_fts) AS score "
        "FROM episodes_fts "
        "WHERE episodes_fts MATCH ? "
        "ORDER BY score DESC LIMIT ?",
        (query, top_k),
    ).fetchall()
    return [Bm25Result(episode_id=int(r[0]), bm25_score=float(r[1])) for r in rows]
