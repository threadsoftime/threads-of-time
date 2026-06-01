# SPDX-License-Identifier: GPL-2.0-or-later
"""Hybrid recall orchestrator — per design subspec §6.1 two-stage pipeline.

Stage 1 (candidate generation): pull ``candidate_multiplier * top_k`` hits
from each of BM25 (``bm25_search``) and dense KNN (``dense_search``),
union the candidate ``episode_id`` sets, and apply any caller-supplied
entity hard filter (subset of episode_ids that referenced a named entity).

Stage 2 (hybrid scoring): for each surviving candidate, hydrate
``timestamp`` + ``salience_score`` + ``episode_type`` from the ``episodes``
table, compute per-episode time decay (with the §7.2 per-type half-life),
mark the entity-match bonus, and combine via :class:`HybridScorer`.
Sort descending by hybrid score and return the top-K.

This is the entry point the ``POST /v1/memory/{bot_guid}/recall`` route
(Phase 6 Task 18) calls, and the ``memory.recall`` MCP tool ultimately
delegates to.
"""

import sqlite3
from dataclasses import dataclass
from datetime import datetime, timezone

from .bm25 import bm25_search
from .decay import decay_weight, half_life_for
from .dense import dense_search
from .hybrid import (
    DEFAULT_ALPHA,
    DEFAULT_BETA,
    DEFAULT_DELTA,
    DEFAULT_GAMMA,
    ComponentScores,
    HybridScorer,
)


@dataclass(frozen=True)
class RecallResult:
    """A single hybrid-scored episode with its component breakdown.

    The component fields are kept for transparency / debugging — Phase 6
    surfaces them in the recall HTTP response and the JSONL telemetry log.
    """

    episode_id: int
    score: float
    bm25_norm: float
    dense_norm: float
    decay: float
    salience: float
    entity_match: float


def _max_normalize(scores: dict[int, float]) -> dict[int, float]:
    """Max-normalise the score map to ``[0, 1]``.

    Empty input → empty dict. All-zero or negative max → zero dict.
    Negative inputs are clipped to 0 (the BM25 surface already negates the
    SQLite rank, so all post-negation values should be non-negative).
    """
    if not scores:
        return {}
    max_s = max(scores.values())
    if max_s <= 0:
        return {k: 0.0 for k in scores}
    return {k: max(0.0, v) / max_s for k, v in scores.items()}


def recall(
    conn: sqlite3.Connection,
    query_text: str,
    query_vec: list[float],
    now: datetime,
    top_k: int,
    entity_filter_ids: set[int] | None = None,
    alpha: float = DEFAULT_ALPHA,
    beta: float = DEFAULT_BETA,
    gamma: float = DEFAULT_GAMMA,
    delta: float = DEFAULT_DELTA,
    half_life_hours: float | None = None,
    candidate_multiplier: int = 5,
) -> list[RecallResult]:
    """Hybrid recall: BM25 ∪ dense → entity filter → score → top-K.

    Args:
        conn:                 Bot-scoped sqlite3 connection.
        query_text:           Natural-language query for BM25; falsy → BM25 skipped.
        query_vec:            Embedding for dense KNN; empty list → dense skipped.
        now:                  Wall-clock for decay calculations (TZ-aware datetime).
        top_k:                Number of results to return after final ranking.
        entity_filter_ids:    Hard filter — when not ``None``, only episodes in
            this set are scored. The caller (Phase 6 route) builds this from
            :func:`tot_memory.retrieval.entity.entity_filter` when entity names
            were supplied in the recall request.
        alpha, beta, gamma, delta:
            Hybrid scorer weights, defaulting to the §6.2 values.
        half_life_hours:      Override half-life. When ``None`` (default), each
            episode uses its episode-type's half-life from
            :data:`tot_memory.retrieval.decay.HALF_LIVES_HOURS` (§7.2).
        candidate_multiplier: Stage-1 over-fetch factor (subspec §6.1 calls for
            ``top_k * 10``; ``5`` is the plan-template default and gives a
            reasonable balance between recall and per-query cost).

    Returns:
        Up to ``top_k`` :class:`RecallResult` instances, sorted descending by
        hybrid score.
    """
    candidate_k = max(top_k * candidate_multiplier, 1)

    bm25_hits = bm25_search(conn, query_text, candidate_k) if query_text else []
    dense_hits = (
        dense_search(conn, query_vec, candidate_k) if query_vec else []
    )

    bm25_scores = _max_normalize({h.episode_id: h.bm25_score for h in bm25_hits})
    dense_scores = _max_normalize(
        {h.episode_id: h.cosine_similarity for h in dense_hits}
    )

    candidate_ids = set(bm25_scores) | set(dense_scores)
    if entity_filter_ids is not None:
        candidate_ids &= entity_filter_ids
    if not candidate_ids:
        return []

    placeholders = ",".join("?" * len(candidate_ids))
    rows = conn.execute(
        f"SELECT episode_id, timestamp, salience_score, episode_type "
        f"FROM episodes WHERE episode_id IN ({placeholders})",
        tuple(candidate_ids),
    ).fetchall()

    scorer = HybridScorer(alpha=alpha, beta=beta, gamma=gamma, delta=delta)
    results: list[RecallResult] = []
    for row in rows:
        eid = int(row["episode_id"])
        # episodes.timestamp is INTEGER unix epoch ms per migration 001.
        episode_time = datetime.fromtimestamp(
            int(row["timestamp"]) / 1000.0, tz=timezone.utc
        )
        h = (
            half_life_hours
            if half_life_hours is not None
            else half_life_for(str(row["episode_type"]))
        )
        decay = decay_weight(episode_time, now, h)
        salience = float(row["salience_score"])
        # Entity-match bonus: 1.0 when the episode is part of the entity-filter
        # set the caller passed in; 0.0 otherwise. (§6.2 allows stacking up to
        # 3·δ for multi-entity hits; that fine-tuning is deferred to Phase 6's
        # route layer, which can pre-compute per-entity match counts.)
        entity_match = (
            1.0
            if entity_filter_ids is not None and eid in entity_filter_ids
            else 0.0
        )
        components = ComponentScores(
            bm25_norm=bm25_scores.get(eid, 0.0),
            dense_norm=dense_scores.get(eid, 0.0),
            decay=decay,
            salience=salience,
            entity_match=entity_match,
        )
        results.append(
            RecallResult(
                episode_id=eid,
                score=scorer.score(components),
                bm25_norm=components.bm25_norm,
                dense_norm=components.dense_norm,
                decay=components.decay,
                salience=components.salience,
                entity_match=components.entity_match,
            )
        )

    # Total order: descending score, ascending episode_id as the tiebreak.
    # The episode_id secondary key disambiguates exact float ties so the
    # ordering is deterministic and matches the Rust recall sort
    # (`b.score.partial_cmp(&a.score).then(a.episode_id.cmp(&b.episode_id))`).
    # Behaviour-identical on distinct scores — this only resolves equal-score
    # ties — and is required for the cross-implementation parity gate.
    results.sort(key=lambda r: (-r.score, r.episode_id))
    return results[:top_k]
