# SPDX-License-Identifier: GPL-2.0-or-later
"""Quality gate for hybrid recall — asserts metrics from design subspec §12.2.

The gate loads the 200-episode synthetic corpus into a fresh per-bot SQLite,
runs every labeled query in ``queries/recall_set.jsonl`` through
:func:`tot_memory.retrieval.rerank.recall`, computes per-query
``recall@5`` / ``precision@5`` / wall-clock latency, aggregates to means + p95,
and asserts the §12.2 thresholds.

This test is marked ``@pytest.mark.eval`` so it's excluded from the default
``pytest`` run (a corpus seed + 50 recalls takes a couple of seconds and isn't
something we want every commit to pay for). Run it explicitly with:

    pytest -m eval tests/eval/test_quality_gate.py

Embeddings are stubbed: every text is hashed (SHA-256) to a deterministic
768-dim L2-normalized vector. With stub embeddings the dense component carries
no real semantic signal — recall is driven by BM25 (FTS5 keyword overlap) +
the entity hard-filter + time-decay + salience. The *real* numbers on the live
nomic-embed-text model are expected to be higher; this gate verifies the
combinatorial logic on the algorithm side. The §12.3 integration suite on
the live server exercises real embeddings + real network latency.

Metric notes
------------
``precision@5`` here is the **bounded** variant — ``hits / min(5, |expected|)``
— because some labeled queries have intentionally small expected sets (e.g.,
a single high-salience reflection). The classical ``hits / 5`` form would cap
those queries at 0.20 even with a perfect retrieval. The subspec threshold
(0.70) targets the bounded form. See ``_precision_at_k`` for the rationale.
"""
from __future__ import annotations

import hashlib
import json
import math
import re
import statistics
import struct
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

import pytest

from tot_memory.db.connection import open_bot_db
from tot_memory.db.runner import run_migrations
from tot_memory.db.schema import EMBEDDING_DIM
from tot_memory.retrieval.entity import entity_filter
from tot_memory.retrieval.rerank import recall as recall_orchestrator


# --- Subspec §12.2 thresholds ------------------------------------------------
RECALL_AT_5_THRESHOLD = 0.80
PRECISION_AT_5_THRESHOLD = 0.70
P95_LATENCY_MS_THRESHOLD = 100.0


# Fixed clock used to seed the corpus AND to run recall (so decay is reproducible).
# Matches the FIXED_NOW in tests/eval/_generate_fixtures.py.
EVAL_NOW = datetime(2026, 5, 27, 0, 0, 0, tzinfo=timezone.utc)


# --- FTS5 query sanitization --------------------------------------------------
# FTS5 treats characters like ", (, ), *, : as syntax. The brain pipeline
# (Phase 7 / Plan 1) is expected to do query cleanup before issuing recall;
# the eval harness mirrors that with a conservative tokenizer that keeps
# alphanumeric runs and joins them with OR semantics (FTS5 implicit AND would
# require every word to match, which is too strict for natural-language queries).
_TOKEN_RE = re.compile(r"[A-Za-z0-9]+")


def _sanitize_fts_query(text: str) -> str:
    """Lower-case alphanumeric token extraction joined by FTS5 OR.

    Drops apostrophes, punctuation, and quotes so FTS5 doesn't error on names
    like "Mor'Ladim" or phrases with hyphens. Returns empty string if the
    query has no usable tokens (the caller treats that as "BM25 skipped").
    """
    tokens = _TOKEN_RE.findall(text)
    if not tokens:
        return ""
    return " OR ".join(tokens)


# --- Stub embedding helper ----------------------------------------------------
def _stub_embed(text: str) -> list[float]:
    """Deterministic SHA-256 → 768-float L2-normalized vector.

    Repeats the digest as needed to fill EMBEDDING_DIM (768 / 32 = 24 reps),
    converts each byte to a centered float in roughly [-1, 1], then L2-normalizes.
    Stable across runs (no random seed) and the same text always maps to the
    same vector — required because the same text is embedded once at seed-time
    and once at query-time (we want them to match exactly).
    """
    h = hashlib.sha256(text.encode("utf-8")).digest()
    reps_needed = (EMBEDDING_DIM + len(h) - 1) // len(h)
    raw = (h * reps_needed)[:EMBEDDING_DIM]
    vec = [(b - 128) / 128.0 for b in raw]
    norm = math.sqrt(sum(v * v for v in vec))
    if norm == 0:
        return [0.0] * EMBEDDING_DIM
    return [v / norm for v in vec]


# --- Corpus + query loaders ---------------------------------------------------
def _load_corpus() -> list[dict[str, Any]]:
    path = Path(__file__).parent / "fixtures" / "synthetic_episodes.jsonl"
    eps: list[dict[str, Any]] = []
    with path.open() as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            eps.append(json.loads(line))
    return eps


def _load_queries() -> list[dict[str, Any]]:
    path = Path(__file__).parent / "queries" / "recall_set.jsonl"
    qs: list[dict[str, Any]] = []
    with path.open() as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            qs.append(json.loads(line))
    return qs


# --- DB seeding (uses raw INSERTs; bypasses the HTTP write path) -------------
def _seed_db(conn, eps: list[dict[str, Any]]) -> None:
    """Insert all 200 episodes + entities + embeddings into the per-bot DB.

    Episode IDs come out of SQLite AUTOINCREMENT and equal the 1-indexed line
    number from the JSONL — that's the invariant the query file relies on for
    its ``expected_episode_ids``. The test asserts the invariant after seeding.
    """
    run_migrations(conn)
    entity_cache: dict[tuple[str, str], int] = {}

    for ep in eps:
        # Insert episode row
        cur = conn.execute(
            "INSERT INTO episodes "
            "(timestamp, content_text, episode_type, salience_score, source, metadata) "
            "VALUES (?, ?, ?, ?, ?, ?)",
            (
                ep["timestamp"],
                ep["content_text"],
                ep["episode_type"],
                ep["salience_score"],
                ep.get("source", "self"),
                json.dumps(ep.get("metadata", {}), separators=(",", ":")),
            ),
        )
        episode_id = int(cur.lastrowid)

        # Stub embedding
        vec = _stub_embed(ep["content_text"])
        conn.execute(
            "INSERT INTO embeddings_vec (rowid, embedding) VALUES (?, ?)",
            (episode_id, struct.pack(f"{len(vec)}f", *vec)),
        )
        conn.execute(
            "UPDATE episodes SET content_embedding_id = ? WHERE episode_id = ?",
            (episode_id, episode_id),
        )

        # Entities
        for ent in ep.get("entities", []):
            key = (ent["entity_kind"], ent["entity_key"])
            entity_id = entity_cache.get(key)
            if entity_id is None:
                existing = conn.execute(
                    "SELECT entity_id FROM entities WHERE entity_kind = ? AND entity_key = ?",
                    key,
                ).fetchone()
                if existing is None:
                    ecur = conn.execute(
                        "INSERT INTO entities "
                        "(entity_kind, entity_key, display_name, last_seen_at) "
                        "VALUES (?, ?, ?, ?)",
                        (
                            ent["entity_kind"],
                            ent["entity_key"],
                            ent["display_name"],
                            ep["timestamp"],
                        ),
                    )
                    entity_id = int(ecur.lastrowid)
                else:
                    entity_id = int(existing["entity_id"])
                entity_cache[key] = entity_id
            conn.execute(
                "INSERT OR IGNORE INTO episode_entities "
                "(episode_id, entity_id, role) VALUES (?, ?, ?)",
                (episode_id, entity_id, ent.get("role", "participant")),
            )
    conn.commit()


# --- Metrics ------------------------------------------------------------------
def _recall_at_k(retrieved: list[int], expected: list[int], k: int) -> float:
    """Fraction of ``expected`` that appears in the top-``k`` of ``retrieved``.

    If ``expected`` has more items than ``k``, the metric is bounded by k/len(expected)
    — that's fine; queries with broad expected sets (e.g., "all Stormwind episodes")
    still get a meaningful score on a per-query basis.
    """
    if not expected:
        return 1.0
    top = set(retrieved[:k])
    hits = sum(1 for e in expected if e in top)
    return hits / min(len(expected), k) if len(expected) > k else hits / len(expected)


def _precision_at_k(retrieved: list[int], expected: list[int], k: int) -> float:
    """Bounded precision@k: hits / min(k, |expected|).

    We use the *bounded* form (sometimes called R-precision when k = |expected|)
    because the labeled-query schema deliberately keeps some ``expected_episode_ids``
    sets small (1-3 items for tightly-specified factual queries). With the
    classical ``hits / k`` form, a query with one expected episode caps at 0.20
    no matter how well the algorithm performs — that's not what we want to
    measure. The bounded form gives 1.0 when the algorithm puts every expected
    item in the top-K, which matches the intent of the §12.2 gate.
    """
    if k <= 0:
        return 0.0
    top = retrieved[:k]
    if not top or not expected:
        return 0.0
    expected_set = set(expected)
    hits = sum(1 for r in top if r in expected_set)
    denom = min(k, len(expected))
    return hits / denom


# --- Pytest gate --------------------------------------------------------------
@pytest.mark.eval
def test_quality_gate(tmp_path):
    """Aggregate recall@5, precision@5, p95 latency across 50 labeled queries."""
    # --- Seed corpus -----------------------------------------------------------
    eps = _load_corpus()
    assert len(eps) == 200, f"corpus size drift: expected 200, got {len(eps)}"
    queries = _load_queries()
    assert len(queries) >= 50, f"query set too small: {len(queries)}"

    conn = open_bot_db(tmp_path, "eval-bot")
    try:
        _seed_db(conn, eps)

        # Invariant check: SQLite AUTOINCREMENT episode_id == 1-indexed line number.
        max_id = conn.execute("SELECT MAX(episode_id) FROM episodes").fetchone()[0]
        assert max_id == 200, f"episode_id != line number: max={max_id}"

        # --- Run every query ---------------------------------------------------
        per_query_recall: list[float] = []
        per_query_precision: list[float] = []
        per_query_latency_ms: list[float] = []

        for q in queries:
            query_text = q["query_text"]
            top_k = int(q.get("top_k", 5))
            expected = list(q["expected_episode_ids"])
            entity_names = q.get("entity_filter") or []

            query_vec = _stub_embed(query_text)

            entity_filter_ids: set[int] | None = None
            if entity_names:
                entity_filter_ids = entity_filter(conn, entity_names)
                # An entity filter that matched nothing is a corpus drift bug.
                assert entity_filter_ids, (
                    f"entity_filter {entity_names} returned empty set for query "
                    f"{query_text!r}; corpus drift?"
                )

            # Optional episode_types narrowing — mirrors the route layer's
            # _ids_matching_constraints helper. Intersect with entity_filter_ids
            # if both are present; build from scratch otherwise.
            wanted_types = q.get("episode_types") or []
            if wanted_types:
                ph = ",".join("?" * len(wanted_types))
                type_ids = {
                    int(r[0])
                    for r in conn.execute(
                        f"SELECT episode_id FROM episodes WHERE episode_type IN ({ph})",
                        tuple(wanted_types),
                    ).fetchall()
                }
                entity_filter_ids = (
                    type_ids if entity_filter_ids is None
                    else entity_filter_ids & type_ids
                )

            t0 = time.perf_counter()
            results = recall_orchestrator(
                conn,
                query_text=_sanitize_fts_query(query_text),
                query_vec=query_vec,
                now=EVAL_NOW,
                top_k=top_k,
                entity_filter_ids=entity_filter_ids,
            )
            t1 = time.perf_counter()

            retrieved_ids = [r.episode_id for r in results]
            per_query_recall.append(_recall_at_k(retrieved_ids, expected, 5))
            per_query_precision.append(_precision_at_k(retrieved_ids, expected, 5))
            per_query_latency_ms.append((t1 - t0) * 1000.0)
    finally:
        conn.close()

    # --- Aggregate -------------------------------------------------------------
    mean_recall = statistics.fmean(per_query_recall)
    mean_precision = statistics.fmean(per_query_precision)
    p95_latency = _percentile(per_query_latency_ms, 95.0)
    p50_latency = _percentile(per_query_latency_ms, 50.0)

    # Print a small summary so CI logs show actuals on a failure or success.
    print(
        f"\n[eval] recall@5={mean_recall:.3f} "
        f"precision@5={mean_precision:.3f} "
        f"p50_latency_ms={p50_latency:.2f} "
        f"p95_latency_ms={p95_latency:.2f}"
    )

    # --- Assert thresholds -----------------------------------------------------
    assert mean_recall >= RECALL_AT_5_THRESHOLD, (
        f"recall@5 {mean_recall:.3f} < threshold {RECALL_AT_5_THRESHOLD}; "
        f"worst queries: {_worst_queries(queries, per_query_recall, 5)}"
    )
    assert mean_precision >= PRECISION_AT_5_THRESHOLD, (
        f"precision@5 {mean_precision:.3f} < threshold {PRECISION_AT_5_THRESHOLD}; "
        f"worst queries: {_worst_queries(queries, per_query_precision, 5)}"
    )
    assert p95_latency <= P95_LATENCY_MS_THRESHOLD, (
        f"p95 latency {p95_latency:.2f}ms > threshold {P95_LATENCY_MS_THRESHOLD}ms"
    )


def _percentile(values: list[float], pct: float) -> float:
    if not values:
        return 0.0
    sv = sorted(values)
    k = (len(sv) - 1) * (pct / 100.0)
    f = int(k)
    c = min(f + 1, len(sv) - 1)
    if f == c:
        return sv[f]
    return sv[f] + (sv[c] - sv[f]) * (k - f)


def _worst_queries(
    queries: list[dict[str, Any]], scores: list[float], n: int
) -> list[tuple[str, float]]:
    """Return ``n`` lowest-scoring queries as ``(query_text, score)`` for debug."""
    paired = [(q["query_text"], s) for q, s in zip(queries, scores)]
    paired.sort(key=lambda x: x[1])
    return paired[:n]
