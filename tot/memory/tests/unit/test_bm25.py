# SPDX-License-Identifier: GPL-2.0-or-later
"""Unit tests for retrieval/bm25.py — per design subspec §6 BM25 component."""

from pathlib import Path

from tot_memory.db.connection import open_bot_db
from tot_memory.db.runner import run_migrations
from tot_memory.retrieval.bm25 import Bm25Result, bm25_search


def _seed(conn, episodes):
    """Insert episodes; episodes is iterable of (timestamp_ms, text, etype, salience)."""
    for ts, text, etype, salience in episodes:
        conn.execute(
            "INSERT INTO episodes (timestamp, content_text, episode_type, salience_score) "
            "VALUES (?, ?, ?, ?)",
            (ts, text, etype, salience),
        )
    conn.commit()


def test_bm25_returns_matching_episodes(tmp_path: Path):
    """BM25 search returns episodes whose content matches the query token."""
    conn = open_bot_db(tmp_path, "test-bot")
    try:
        run_migrations(conn)
        _seed(
            conn,
            [
                (1748395200000, "Alice taught me how to AoE pull", "social", 0.7),
                (1748395500000, "I died to a boss", "combat", 0.5),
                (1748395800000, "Alice and I cleared a dungeon", "social", 0.8),
            ],
        )
        results = bm25_search(conn, "Alice", top_k=10)
        ids = {r.episode_id for r in results}
        assert 1 in ids
        assert 3 in ids
        assert 2 not in ids
    finally:
        conn.close()


def test_bm25_score_is_higher_for_better_match(tmp_path: Path):
    """Negated bm25() means higher score = more relevant (matches §6 hybrid convention)."""
    conn = open_bot_db(tmp_path, "test-bot")
    try:
        run_migrations(conn)
        _seed(
            conn,
            [
                (1748395200000, "Alice Alice Alice tanks BFD", "social", 0.7),
                (1748395500000, "Alice mentioned tanks once", "social", 0.5),
            ],
        )
        results = bm25_search(conn, "Alice tanks", top_k=10)
        assert len(results) == 2
        # All scores should be positive (negated rank); higher = better match
        assert all(r.bm25_score > 0 for r in results)
        # Episode 1 (Alice repeated) should outscore episode 2
        ranked = sorted(results, key=lambda r: r.bm25_score, reverse=True)
        assert ranked[0].episode_id == 1


    finally:
        conn.close()


def test_bm25_empty_db_returns_empty_list(tmp_path: Path):
    """No episodes → BM25 search returns empty list, no error."""
    conn = open_bot_db(tmp_path, "test-bot")
    try:
        run_migrations(conn)
        results = bm25_search(conn, "anything", top_k=5)
        assert results == []
    finally:
        conn.close()


def test_bm25_result_is_dataclass_with_fields():
    """Bm25Result has episode_id (int) and bm25_score (float)."""
    r = Bm25Result(episode_id=42, bm25_score=1.23)
    assert r.episode_id == 42
    assert r.bm25_score == 1.23
