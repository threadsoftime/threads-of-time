# SPDX-License-Identifier: GPL-2.0-or-later
"""POST /v1/memory/{bot_guid}/recall — memory.recall route.

Per design subspec §10.3 (memory.recall) and §6 (hybrid retrieval). The route:

  1. Embeds the query text via the BYOLLM endpoint (synchronously — same
     latency budget as the write path, §5.1).
  2. If ``entity_names`` is non-empty, resolves it to the set of episode_ids
     referencing those entities via :func:`retrieval.entity.entity_filter`.
  3. Calls :func:`retrieval.rerank.recall` to score candidates via the §6.2
     hybrid formula (BM25 + dense + decay + salience + entity_match).
  4. Hydrates the returned episode IDs with row-level fields (timestamp,
     content, type, salience).
  5. Bumps ``last_recalled_at`` + ``recall_count`` on each returned episode.
     This side effect is the load-bearing distinction from ``memory.search``
     (§10.4).

Divergences from subspec §10.3 in this implementation (caller-friendly,
matches the abbreviated plan template + existing ``entity_filter`` helper):

- Request uses ``entity_names: list[str]`` (matches the helper signature) instead
  of ``entity_ids: list[int]``. Resolution from names → episode_ids happens in
  the route layer via ``retrieval.entity.entity_filter``.
- Time filter takes the shape ``{"after": ms, "before": ms}`` instead of
  ``since_ms`` / ``until_ms`` top-level fields. Semantically equivalent.
- Per-call weight overrides ``alpha``/``beta``/``gamma``/``delta`` are accepted
  in addition to the §10.3 ``mmr_lambda``. ``mmr_lambda`` is accepted for
  API forward-compatibility but is currently unused (Phase 5's recall does not
  yet apply the §6.3 MMR rerank — that lands in a follow-up).
"""
from __future__ import annotations

import logging
from datetime import datetime, timezone
from typing import Any, Optional

import httpx
from fastapi import APIRouter, HTTPException, Request
from pydantic import BaseModel, Field

from tot_memory.db.connection import open_bot_db
from tot_memory.db.runner import run_migrations
from tot_memory.embeddings.client import EmbeddingsClient
from tot_memory.retrieval.entity import entity_filter
from tot_memory.retrieval.hybrid import (
    DEFAULT_ALPHA,
    DEFAULT_BETA,
    DEFAULT_DELTA,
    DEFAULT_GAMMA,
)
from tot_memory.retrieval.rerank import recall as recall_orchestrator


router = APIRouter()
logger = logging.getLogger(__name__)


class TimeFilter(BaseModel):
    after: Optional[int] = Field(None, description="Episodes with timestamp >= after (ms)")
    before: Optional[int] = Field(None, description="Episodes with timestamp <= before (ms)")


class RecallRequest(BaseModel):
    query_text: str = Field(..., min_length=1, max_length=500)
    top_k: int = Field(5, ge=1, le=20)
    entity_names: Optional[list[str]] = Field(
        None,
        description="OR-logic entity hard filter by display_name; "
        "empty/null = no filter",
    )
    episode_types: Optional[list[str]] = Field(
        None,
        description="Restrict to these episode_types; empty/null = all types",
    )
    time_filter: Optional[TimeFilter] = Field(
        None,
        description="Optional timestamp window (epoch ms): {after, before}",
    )
    alpha: Optional[float] = Field(None, ge=0.0, description="Override BM25 weight (§6.2)")
    beta: Optional[float] = Field(None, ge=0.0, description="Override dense weight")
    gamma: Optional[float] = Field(None, ge=0.0, description="Override salience-boost multiplier")
    delta: Optional[float] = Field(None, ge=0.0, description="Override entity-match bonus")
    mmr_lambda: Optional[float] = Field(
        None,
        ge=0.0,
        le=1.0,
        description="MMR diversity weight (§6.3); accepted for forward-compat",
    )


class RecallComponentBreakdown(BaseModel):
    bm25_norm: float
    dense_norm: float
    decay: float
    salience: float
    entity_match: float


class RecallHit(BaseModel):
    episode_id: int
    content_text: str
    timestamp: int
    episode_type: str
    salience_score: float
    score: float
    components: RecallComponentBreakdown


class RecallResponse(BaseModel):
    results: list[RecallHit]


@router.post(
    "/v1/memory/{bot_guid}/recall",
    status_code=200,
    response_model=RecallResponse,
)
async def recall_route(
    bot_guid: str, body: RecallRequest, request: Request
) -> RecallResponse:
    settings = request.app.state.settings

    # --- Embed the query (sync; same budget as write path) ---
    embed_client = EmbeddingsClient(
        base_url=settings.embeddings_url,
        model=settings.embeddings_model,
        api_key=settings.embeddings_api_key,
    )
    try:
        try:
            query_vec = await embed_client.embed(body.query_text)
        except (httpx.HTTPError, ValueError) as exc:
            logger.warning(
                "embedding call failed during recall for bot %s (%s: %s); "
                "falling back to BM25-only",
                bot_guid,
                type(exc).__name__,
                exc,
            )
            query_vec = []
    finally:
        await embed_client.aclose()

    # --- Open DB + resolve entity filter ---
    conn = open_bot_db(settings.data_dir, bot_guid)
    try:
        run_migrations(conn)

        entity_filter_ids: Optional[set[int]] = None
        if body.entity_names:
            entity_filter_ids = entity_filter(conn, body.entity_names)
            # If the caller asked to filter but no matching episodes exist,
            # return early with an empty result — never silently skip the
            # filter (matches the §6.1 "hard filter" semantics).
            if not entity_filter_ids:
                return RecallResponse(results=[])

        # Apply additional episode_type / time_filter constraints by
        # narrowing the entity_filter_ids set (or building one from scratch).
        if body.episode_types or body.time_filter:
            extra = _ids_matching_constraints(
                conn,
                episode_types=body.episode_types,
                after_ms=body.time_filter.after if body.time_filter else None,
                before_ms=body.time_filter.before if body.time_filter else None,
            )
            entity_filter_ids = (
                extra if entity_filter_ids is None else entity_filter_ids & extra
            )
            if not entity_filter_ids:
                return RecallResponse(results=[])

        now = datetime.now(tz=timezone.utc)
        scored = recall_orchestrator(
            conn,
            query_text=body.query_text,
            query_vec=query_vec,
            now=now,
            top_k=body.top_k,
            entity_filter_ids=entity_filter_ids,
            alpha=body.alpha if body.alpha is not None else DEFAULT_ALPHA,
            beta=body.beta if body.beta is not None else DEFAULT_BETA,
            gamma=body.gamma if body.gamma is not None else DEFAULT_GAMMA,
            delta=body.delta if body.delta is not None else DEFAULT_DELTA,
        )

        if not scored:
            return RecallResponse(results=[])

        # Hydrate full row data for each returned episode_id.
        ids = [r.episode_id for r in scored]
        placeholders = ",".join("?" * len(ids))
        rows = {
            int(row["episode_id"]): row
            for row in conn.execute(
                f"SELECT episode_id, content_text, timestamp, episode_type, "
                f"salience_score FROM episodes WHERE episode_id IN ({placeholders})",
                tuple(ids),
            ).fetchall()
        }

        hits: list[RecallHit] = []
        for r in scored:
            row = rows.get(r.episode_id)
            if row is None:
                # Race — episode deleted between scoring and hydration; skip.
                continue
            hits.append(
                RecallHit(
                    episode_id=r.episode_id,
                    content_text=str(row["content_text"]),
                    timestamp=int(row["timestamp"]),
                    episode_type=str(row["episode_type"]),
                    salience_score=float(row["salience_score"]),
                    score=r.score,
                    components=RecallComponentBreakdown(
                        bm25_norm=r.bm25_norm,
                        dense_norm=r.dense_norm,
                        decay=r.decay,
                        salience=r.salience,
                        entity_match=r.entity_match,
                    ),
                )
            )

        # --- Side effect: bump last_recalled_at + recall_count (§10.3) ---
        if hits:
            now_ms = int(now.timestamp() * 1000)
            hit_ids = [h.episode_id for h in hits]
            ph = ",".join("?" * len(hit_ids))
            conn.execute(
                f"UPDATE episodes SET last_recalled_at = ?, "
                f"recall_count = recall_count + 1 "
                f"WHERE episode_id IN ({ph})",
                (now_ms, *hit_ids),
            )
            conn.commit()

        return RecallResponse(results=hits)
    finally:
        conn.close()


def _ids_matching_constraints(
    conn: Any,
    episode_types: Optional[list[str]],
    after_ms: Optional[int],
    before_ms: Optional[int],
) -> set[int]:
    """Return episode_ids satisfying the episode_type + time_filter constraints.

    Used to narrow the recall candidate set when the caller supplies
    ``episode_types`` and/or ``time_filter``. Both arguments are optional;
    when both are ``None``/empty, returns all episode_ids.
    """
    clauses: list[str] = []
    params: list[Any] = []
    if episode_types:
        ph = ",".join("?" * len(episode_types))
        clauses.append(f"episode_type IN ({ph})")
        params.extend(episode_types)
    if after_ms is not None:
        clauses.append("timestamp >= ?")
        params.append(after_ms)
    if before_ms is not None:
        clauses.append("timestamp <= ?")
        params.append(before_ms)

    where = ("WHERE " + " AND ".join(clauses)) if clauses else ""
    rows = conn.execute(
        f"SELECT episode_id FROM episodes {where}", tuple(params)
    ).fetchall()
    return {int(r[0]) for r in rows}
