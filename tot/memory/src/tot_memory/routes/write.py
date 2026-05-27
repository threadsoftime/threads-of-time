# SPDX-License-Identifier: GPL-2.0-or-later
"""POST /v1/memory/{bot_guid}/episodes — memory.write route.

Per design subspec §5 (sync embedding pipeline with graceful degradation) and
§10.1 (request/response schema). The route:

  1. Opens the per-bot SQLite + runs migrations (idempotent).
  2. Attempts to embed the content synchronously via the BYOLLM endpoint.
     On httpx error (timeout, connection refused, non-2xx, dim mismatch), the
     episode is still written with content_embedding_id = NULL. The episode
     stays BM25-searchable and readable; a backfill CLI (§5.4) reconciles
     eventually. The response carries embedding_generated=False so the caller
     can log / decide.
  3. Inserts the episode row.
  4. If embed succeeded, inserts the vector row and sets content_embedding_id.
  5. Upserts entities + episode_entities (per design subspec §2.2).

salience_hint is taken as-is when provided (0.0-1.0); otherwise a default of
0.5 is used. The full per-type rule-based scorer (subspec §8) lands in a later
task — this route honors the brain's hint and uses 0.5 as a safe default.
"""
from __future__ import annotations

import logging
import struct
from typing import Any, Optional

import httpx
from fastapi import APIRouter, Request
from pydantic import BaseModel, Field

from tot_memory.db.connection import open_bot_db
from tot_memory.db.runner import run_migrations
from tot_memory.embeddings.client import EmbeddingsClient


router = APIRouter()
logger = logging.getLogger(__name__)


# Roles allowed by the episode_entities CHECK constraint (subspec §2.1).
_ALLOWED_ROLES = {"subject", "participant", "target", "witness"}


class EntityRef(BaseModel):
    """Per design subspec §10.1 EntityRef shape."""

    entity_kind: str = Field(..., description="player|npc|item|location|faction")
    entity_key: str = Field(..., description="Canonical key for this kind")
    display_name: str = Field(..., description="Human-readable label")
    role: Optional[str] = Field(
        "participant", description="subject|participant|target|witness"
    )


class WriteEpisodeRequest(BaseModel):
    """Per design subspec §10.1 MemoryWriteArgs shape (bot_guid lives in path)."""

    content_text: str = Field(..., min_length=1, max_length=4000)
    episode_type: str = Field(..., description="One of the §1.2 vocabulary values")
    timestamp: int = Field(..., gt=0, description="Unix epoch ms of the event")
    salience_hint: Optional[float] = Field(
        None, ge=0.0, le=1.0, description="Optional brain-supplied salience override"
    )
    entities: list[EntityRef] = Field(default_factory=list)
    metadata: Optional[dict[str, Any]] = Field(None)
    source: Optional[str] = Field("self", description="self|chat|observed|system")


class WriteEpisodeResponse(BaseModel):
    episode_id: int
    embedding_generated: bool
    salience_score: float


@router.post(
    "/v1/memory/{bot_guid}/episodes",
    status_code=201,
    response_model=WriteEpisodeResponse,
)
async def write_episode(
    bot_guid: str, body: WriteEpisodeRequest, request: Request
) -> WriteEpisodeResponse:
    settings = request.app.state.settings

    # --- Step 1: try to embed sync; graceful degradation per §5.1 ---
    vec: Optional[list[float]] = None
    embed_client = EmbeddingsClient(
        base_url=settings.embeddings_url,
        model=settings.embeddings_model,
        api_key=settings.embeddings_api_key,
    )
    try:
        try:
            vec = await embed_client.embed(body.content_text)
        except (httpx.HTTPError, ValueError) as exc:
            # httpx.HTTPError covers timeouts, connection errors, non-2xx.
            # ValueError catches dim-mismatch from the client. Both are recoverable
            # at write time per §5 — write the episode without an embedding and
            # let backfill catch up.
            logger.warning(
                "embedding call failed for bot %s (%s: %s); writing episode "
                "with content_embedding_id=NULL",
                bot_guid,
                type(exc).__name__,
                exc,
            )
    finally:
        await embed_client.aclose()

    # --- Step 2-5: write episode + (optional) vec + entities atomically ---
    conn = open_bot_db(settings.data_dir, bot_guid)
    try:
        run_migrations(conn)

        salience_score = (
            body.salience_hint if body.salience_hint is not None else 0.5
        )

        cursor = conn.execute(
            "INSERT INTO episodes "
            "(timestamp, content_text, episode_type, salience_score, source, metadata) "
            "VALUES (?, ?, ?, ?, ?, ?)",
            (
                body.timestamp,
                body.content_text,
                body.episode_type,
                salience_score,
                body.source or "self",
                _serialize_metadata(body.metadata),
            ),
        )
        episode_id = int(cursor.lastrowid)

        if vec is not None:
            vec_bytes = struct.pack(f"{len(vec)}f", *vec)
            conn.execute(
                "INSERT INTO embeddings_vec (rowid, embedding) VALUES (?, ?)",
                (episode_id, vec_bytes),
            )
            conn.execute(
                "UPDATE episodes SET content_embedding_id = ? WHERE episode_id = ?",
                (episode_id, episode_id),
            )

        _upsert_entities(conn, episode_id, body.entities, body.timestamp)

        conn.commit()
    finally:
        conn.close()

    return WriteEpisodeResponse(
        episode_id=episode_id,
        embedding_generated=vec is not None,
        salience_score=salience_score,
    )


def _serialize_metadata(metadata: Optional[dict[str, Any]]) -> Optional[str]:
    if metadata is None:
        return None
    import json

    return json.dumps(metadata, separators=(",", ":"))


def _upsert_entities(
    conn: Any, episode_id: int, entities: list[EntityRef], event_ts_ms: int
) -> None:
    """Per design subspec §2.2: upsert on (entity_kind, entity_key); link via
    episode_entities; trigger keeps total_episodes in sync."""
    for ent in entities:
        role = ent.role or "participant"
        if role not in _ALLOWED_ROLES:
            role = "participant"

        # Upsert entity: insert if missing, else update last_seen_at + display_name.
        existing = conn.execute(
            "SELECT entity_id FROM entities WHERE entity_kind = ? AND entity_key = ?",
            (ent.entity_kind, ent.entity_key),
        ).fetchone()
        if existing is None:
            cur = conn.execute(
                "INSERT INTO entities "
                "(entity_kind, entity_key, display_name, last_seen_at) "
                "VALUES (?, ?, ?, ?)",
                (ent.entity_kind, ent.entity_key, ent.display_name, event_ts_ms),
            )
            entity_id = int(cur.lastrowid)
        else:
            entity_id = int(existing["entity_id"])
            conn.execute(
                "UPDATE entities SET last_seen_at = ?, display_name = ? "
                "WHERE entity_id = ?",
                (event_ts_ms, ent.display_name, entity_id),
            )

        # Link episode -> entity (PK is (episode_id, entity_id, role))
        conn.execute(
            "INSERT OR IGNORE INTO episode_entities (episode_id, entity_id, role) "
            "VALUES (?, ?, ?)",
            (episode_id, entity_id, role),
        )
