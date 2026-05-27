# SPDX-License-Identifier: GPL-2.0-or-later
"""Integration tests for retrieval/rerank.py — full §6 hybrid pipeline."""

import struct
from datetime import datetime, timezone
from pathlib import Path

from tot_memory.db.connection import open_bot_db
from tot_memory.db.runner import run_migrations
from tot_memory.db.schema import EMBEDDING_DIM
from tot_memory.retrieval.rerank import RecallResult, recall


def _ts_ms(year, month, day, hour=0) -> int:
    """Helper: build a unix epoch ms timestamp (matches episodes.timestamp INTEGER)."""
    return int(datetime(year, month, day, hour, tzinfo=timezone.utc).timestamp() * 1000)


def _seed_episode(conn, ts_ms: int, text: str, etype: str, salience: float, vec: list[float]) -> int:
    cur = conn.execute(
        "INSERT INTO episodes (timestamp, content_text, episode_type, salience_score) "
        "VALUES (?, ?, ?, ?)",
        (ts_ms, text, etype, salience),
    )
    eid = int(cur.lastrowid)
    conn.execute(
        "INSERT INTO embeddings_vec (rowid, embedding) VALUES (?, ?)",
        (eid, struct.pack(f"{len(vec)}f", *vec)),
    )
    conn.execute(
        "UPDATE episodes SET content_embedding_id = ? WHERE episode_id = ?", (eid, eid)
    )
    return eid


def _add_entity(conn, episode_id: int, display_name: str) -> int:
    cur = conn.execute(
        "INSERT INTO entities (entity_kind, entity_key, display_name) VALUES (?, ?, ?)",
        ("player", display_name.lower(), display_name),
    )
    entity_id = int(cur.lastrowid)
    conn.execute(
        "INSERT INTO episode_entities (episode_id, entity_id, role) VALUES (?, ?, ?)",
        (episode_id, entity_id, "subject"),
    )
    return entity_id


def test_recall_ranks_fresh_salient_keyword_match_first(tmp_path: Path):
    """Integration: a recent, salient, keyword-matching episode outranks a
    stale, low-salience, unmatched one — hybrid scoring end-to-end."""
    conn = open_bot_db(tmp_path, "test-bot")
    try:
        run_migrations(conn)
        now = datetime(2026, 5, 27, 13, tzinfo=timezone.utc)
        parallel = [1.0] + [0.0] * (EMBEDDING_DIM - 1)
        orthogonal = [0.0, 1.0] + [0.0] * (EMBEDDING_DIM - 2)

        # Fresh + salient + keyword-matching → should win
        winner_id = _seed_episode(
            conn, _ts_ms(2026, 5, 27, 12), "Alice and I cleared BFD",
            "social", 0.9, parallel,
        )
        # Old + low salience + no keyword match → should lose
        loser_id = _seed_episode(
            conn, _ts_ms(2026, 4, 1, 12), "I died alone",
            "combat", 0.1, orthogonal,
        )
        conn.commit()

        results = recall(
            conn, query_text="Alice", query_vec=parallel, now=now, top_k=2
        )
        assert len(results) >= 1
        assert results[0].episode_id == winner_id
        # Loser, if returned, has a lower score
        if len(results) > 1 and results[1].episode_id == loser_id:
            assert results[0].score > results[1].score
    finally:
        conn.close()


def test_recall_respects_top_k(tmp_path: Path):
    """top_k=1 returns at most one result even with many candidates."""
    conn = open_bot_db(tmp_path, "test-bot")
    try:
        run_migrations(conn)
        now = datetime(2026, 5, 27, 13, tzinfo=timezone.utc)
        vec = [1.0] + [0.0] * (EMBEDDING_DIM - 1)
        for i in range(5):
            _seed_episode(
                conn, _ts_ms(2026, 5, 27, 10 + i),
                f"Alice helped me episode {i}", "social", 0.5, vec,
            )
        conn.commit()
        results = recall(
            conn, query_text="Alice", query_vec=vec, now=now, top_k=1
        )
        assert len(results) == 1
    finally:
        conn.close()


def test_recall_returns_empty_when_no_candidates_match(tmp_path: Path):
    """Query with no BM25 hits and an empty embeddings_vec returns []."""
    conn = open_bot_db(tmp_path, "test-bot")
    try:
        run_migrations(conn)
        now = datetime(2026, 5, 27, 13, tzinfo=timezone.utc)
        results = recall(
            conn, query_text="nothingmatcheshere",
            query_vec=[0.0] * EMBEDDING_DIM, now=now, top_k=5,
        )
        assert results == []
    finally:
        conn.close()


def test_recall_with_entity_filter_drops_unmatched_episodes(tmp_path: Path):
    """entity_filter_ids hard-filters: episodes not in the set never appear."""
    conn = open_bot_db(tmp_path, "test-bot")
    try:
        run_migrations(conn)
        now = datetime(2026, 5, 27, 13, tzinfo=timezone.utc)
        vec = [1.0] + [0.0] * (EMBEDDING_DIM - 1)
        alice_ep = _seed_episode(
            conn, _ts_ms(2026, 5, 27, 12), "Alice helped me",
            "social", 0.7, vec,
        )
        _add_entity(conn, alice_ep, "Alice")

        bob_ep = _seed_episode(
            conn, _ts_ms(2026, 5, 27, 12), "Bob helped me",
            "social", 0.7, vec,
        )
        _add_entity(conn, bob_ep, "Bob")
        conn.commit()

        # Filter to only Alice's episode
        results = recall(
            conn, query_text="helped", query_vec=vec, now=now, top_k=5,
            entity_filter_ids={alice_ep},
        )
        returned_ids = {r.episode_id for r in results}
        assert alice_ep in returned_ids
        assert bob_ep not in returned_ids
    finally:
        conn.close()


def test_recall_result_exposes_score_breakdown(tmp_path: Path):
    """RecallResult records the component values for debugging / transparency."""
    conn = open_bot_db(tmp_path, "test-bot")
    try:
        run_migrations(conn)
        now = datetime(2026, 5, 27, 13, tzinfo=timezone.utc)
        vec = [1.0] + [0.0] * (EMBEDDING_DIM - 1)
        _seed_episode(
            conn, _ts_ms(2026, 5, 27, 12), "Alice tanked", "social", 0.7, vec,
        )
        conn.commit()

        results = recall(
            conn, query_text="Alice", query_vec=vec, now=now, top_k=1
        )
        assert len(results) == 1
        r = results[0]
        assert isinstance(r, RecallResult)
        assert 0.0 <= r.bm25_norm <= 1.0
        assert 0.0 <= r.dense_norm <= 1.0
        assert 0.0 < r.decay <= 1.0
        assert 0.0 <= r.salience <= 1.0
    finally:
        conn.close()
