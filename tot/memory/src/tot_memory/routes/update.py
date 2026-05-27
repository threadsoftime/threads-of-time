# SPDX-License-Identifier: GPL-2.0-or-later
"""PATCH /v1/memory/{bot_guid}/episodes/{episode_id} — memory.update route.

Per design subspec §10.6 (memory.update). Mutable fields:

- ``content_text``  — triggers a synchronous re-embedding (replaces the
  embeddings_vec row). The FTS5 ``episodes_au`` trigger (migration 004)
  keeps the BM25 index in sync automatically.
- ``salience_score`` — overrides the server-computed salience. Fast (no
  re-embed).
- ``metadata`` — replaces the existing JSON blob entirely.

The plan abbreviation called the salience field ``salience_hint``; we use the
column's actual name ``salience_score`` here (matching the §10.6 schema).
The brain-supplied ``salience_hint`` from memory.write writes into the same
column, so the semantics are identical.

Errors:
- 404 ``episode_not_found`` — when the episode_id is absent.
- 400 ``no_fields_to_update`` — when all three optional fields are null.
- 400 ``content_too_long`` — caught by the pydantic constraint (max_length).
- 503 ``embedding_service_unavailable`` — when content_text was supplied but
  the BYOLLM endpoint is unreachable. Unlike the write path's graceful
  degradation (§5.1), update fails loudly because the caller explicitly
  asked for a re-embedding; silently leaving the stale embedding in place
  would be a worse outcome than a 503 retry.
"""
from __future__ import annotations

import json
import logging
import struct
from typing import Any, Optional

import httpx
from fastapi import APIRouter, HTTPException, Request
from pydantic import BaseModel, Field

from tot_memory.db.connection import open_bot_db
from tot_memory.db.runner import run_migrations
from tot_memory.embeddings.client import EmbeddingsClient


router = APIRouter()
logger = logging.getLogger(__name__)


class UpdateRequest(BaseModel):
    content_text: Optional[str] = Field(
        None, min_length=1, max_length=4000, description="Triggers re-embedding"
    )
    salience_score: Optional[float] = Field(None, ge=0.0, le=1.0)
    metadata: Optional[dict[str, Any]] = Field(
        None, description="Replaces existing metadata entirely"
    )


class UpdateResponse(BaseModel):
    episode_id: int
    updated_fields: list[str]
    reembedded: bool


@router.patch(
    "/v1/memory/{bot_guid}/episodes/{episode_id}",
    status_code=200,
    response_model=UpdateResponse,
)
async def update_episode(
    bot_guid: str,
    episode_id: int,
    body: UpdateRequest,
    request: Request,
) -> UpdateResponse:
    settings = request.app.state.settings

    if (
        body.content_text is None
        and body.salience_score is None
        and body.metadata is None
    ):
        raise HTTPException(status_code=400, detail="no_fields_to_update")

    # If a re-embed is needed, do the network call before opening the DB so we
    # don't hold the connection open across the embedding round-trip.
    new_vec: Optional[list[float]] = None
    reembedded = False
    if body.content_text is not None:
        embed_client = EmbeddingsClient(
            base_url=settings.embeddings_url,
            model=settings.embeddings_model,
            api_key=settings.embeddings_api_key,
        )
        try:
            try:
                new_vec = await embed_client.embed(body.content_text)
                reembedded = True
            except (httpx.HTTPError, ValueError) as exc:
                logger.warning(
                    "re-embed during update failed for bot %s episode %s (%s: %s)",
                    bot_guid,
                    episode_id,
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

        existing = conn.execute(
            "SELECT episode_id FROM episodes WHERE episode_id = ?",
            (episode_id,),
        ).fetchone()
        if existing is None:
            raise HTTPException(status_code=404, detail="episode_not_found")

        updated_fields: list[str] = []
        sets: list[str] = []
        params: list[Any] = []
        if body.content_text is not None:
            sets.append("content_text = ?")
            params.append(body.content_text)
            updated_fields.append("content_text")
        if body.salience_score is not None:
            sets.append("salience_score = ?")
            params.append(body.salience_score)
            updated_fields.append("salience_score")
        if body.metadata is not None:
            sets.append("metadata = ?")
            params.append(json.dumps(body.metadata, separators=(",", ":")))
            updated_fields.append("metadata")

        params.append(episode_id)
        conn.execute(
            f"UPDATE episodes SET {', '.join(sets)} WHERE episode_id = ?",
            tuple(params),
        )

        if new_vec is not None:
            # Replace the embeddings_vec row. sqlite-vec virtual tables don't
            # support UPSERT; delete + insert is the standard idiom.
            conn.execute(
                "DELETE FROM embeddings_vec WHERE rowid = ?", (episode_id,)
            )
            conn.execute(
                "INSERT INTO embeddings_vec (rowid, embedding) VALUES (?, ?)",
                (episode_id, struct.pack(f"{len(new_vec)}f", *new_vec)),
            )
            conn.execute(
                "UPDATE episodes SET content_embedding_id = ? WHERE episode_id = ?",
                (episode_id, episode_id),
            )

        conn.commit()
        return UpdateResponse(
            episode_id=episode_id,
            updated_fields=updated_fields,
            reembedded=reembedded,
        )
    finally:
        conn.close()
