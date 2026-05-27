# SPDX-License-Identifier: GPL-2.0-or-later
"""POST /v1/memory/{bot_guid}/search — memory.search route.

Per design subspec §10.4 and the plan abbreviation's pure-semantic shape.
Pure nearest-neighbour over ``embeddings_vec`` — no BM25, no decay, no
salience boost, no entity filter, no recall_count / last_recalled_at bump.

Distinguished from ``memory.recall`` (§10.3) by the lack of side effects and
the lack of hybrid scoring. The brain uses this when it wants to look up
"what episodes are most similar to this text/vector?" without polluting the
recall telemetry.

Divergences from subspec §10.4 (which says "same schema as recall"):
- The plan-template abbreviation explicitly carves out a different shape for
  search — request takes ``query_text? | query_vec?`` (one required), and
  response returns ``{episode_id, content_text, cosine_similarity}``. This
  implementation follows the abbreviation. The §10.4 shape (full recall
  envelope) can be re-introduced later if a caller needs the full
  hybrid-style breakdown, but no current caller does.
"""
from __future__ import annotations

import logging
import struct
from typing import Optional

import httpx
from fastapi import APIRouter, HTTPException, Request
from pydantic import BaseModel, Field

from tot_memory.db.connection import open_bot_db
from tot_memory.db.runner import run_migrations
from tot_memory.db.schema import EMBEDDING_DIM
from tot_memory.embeddings.client import EmbeddingsClient
from tot_memory.retrieval.dense import dense_search


router = APIRouter()
logger = logging.getLogger(__name__)


class SearchRequest(BaseModel):
    query_text: Optional[str] = Field(
        None, min_length=1, max_length=500, description="Text to embed + search"
    )
    query_vec: Optional[list[float]] = Field(
        None, description=f"Pre-computed query embedding (length must equal {EMBEDDING_DIM})"
    )
    top_k: int = Field(10, ge=1, le=100)


class SearchHit(BaseModel):
    episode_id: int
    content_text: str
    cosine_similarity: float


class SearchResponse(BaseModel):
    results: list[SearchHit]


@router.post(
    "/v1/memory/{bot_guid}/search",
    status_code=200,
    response_model=SearchResponse,
)
async def search(
    bot_guid: str, body: SearchRequest, request: Request
) -> SearchResponse:
    settings = request.app.state.settings

    if body.query_text is None and body.query_vec is None:
        raise HTTPException(
            status_code=400,
            detail="one of query_text or query_vec is required",
        )

    if body.query_vec is not None:
        if len(body.query_vec) != EMBEDDING_DIM:
            raise HTTPException(
                status_code=400,
                detail=(
                    f"query_vec dimension {len(body.query_vec)} does not match "
                    f"EMBEDDING_DIM {EMBEDDING_DIM}"
                ),
            )
        vec: list[float] = body.query_vec
    else:
        # body.query_text is non-None here.
        embed_client = EmbeddingsClient(
            base_url=settings.embeddings_url,
            model=settings.embeddings_model,
            api_key=settings.embeddings_api_key,
        )
        try:
            try:
                vec = await embed_client.embed(body.query_text)
            except (httpx.HTTPError, ValueError) as exc:
                logger.warning(
                    "embedding call failed during search for bot %s (%s: %s)",
                    bot_guid,
                    type(exc).__name__,
                    exc,
                )
                raise HTTPException(
                    status_code=503,
                    detail="embedding_service_unavailable",
                ) from exc
        finally:
            await embed_client.aclose()

    conn = open_bot_db(settings.data_dir, bot_guid)
    try:
        run_migrations(conn)
        hits = dense_search(conn, vec, body.top_k)
        if not hits:
            return SearchResponse(results=[])

        ids = [h.episode_id for h in hits]
        placeholders = ",".join("?" * len(ids))
        rows = {
            int(row["episode_id"]): str(row["content_text"])
            for row in conn.execute(
                f"SELECT episode_id, content_text FROM episodes "
                f"WHERE episode_id IN ({placeholders})",
                tuple(ids),
            ).fetchall()
        }

        results = []
        for h in hits:
            content_text = rows.get(h.episode_id)
            if content_text is None:
                continue  # race: episode deleted between KNN and hydration
            results.append(
                SearchHit(
                    episode_id=h.episode_id,
                    content_text=content_text,
                    cosine_similarity=h.cosine_similarity,
                )
            )
        return SearchResponse(results=results)
    finally:
        conn.close()
