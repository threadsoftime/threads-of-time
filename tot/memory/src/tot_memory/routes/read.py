# SPDX-License-Identifier: GPL-2.0-or-later
"""GET /v1/memory/{bot_guid}/episodes/{episode_id} — memory.read route.

Per design subspec §10.2. Returns the full episode row plus all linked entities
via the ``episode_entities`` JOIN. 404 when the episode does not exist.

The route does NOT bump ``last_recalled_at`` — that side effect is reserved for
``memory.recall`` (§10.4). ``memory.read`` is a no-side-effect lookup intended
for introspection / debugging tools.
"""
from __future__ import annotations

import json
from typing import Any, Optional

from fastapi import APIRouter, HTTPException, Request
from pydantic import BaseModel

from tot_memory.db.connection import open_bot_db
from tot_memory.db.runner import run_migrations


router = APIRouter()


class EntityOut(BaseModel):
    entity_id: int
    entity_kind: str
    entity_key: str
    display_name: str
    role: str


class EpisodeReadResponse(BaseModel):
    episode_id: int
    timestamp: int
    created_at: int
    content_text: str
    episode_type: str
    salience_score: float
    last_recalled_at: Optional[int]
    recall_count: int
    source: str
    embedding_generated: bool
    metadata: Optional[dict[str, Any]]
    entities: list[EntityOut]


@router.get(
    "/v1/memory/{bot_guid}/episodes/{episode_id}",
    status_code=200,
    response_model=EpisodeReadResponse,
)
def read_episode(
    bot_guid: str, episode_id: int, request: Request
) -> EpisodeReadResponse:
    settings = request.app.state.settings
    conn = open_bot_db(settings.data_dir, bot_guid)
    try:
        run_migrations(conn)
        row = conn.execute(
            "SELECT episode_id, timestamp, created_at, content_text, episode_type, "
            "salience_score, last_recalled_at, recall_count, source, "
            "content_embedding_id, metadata "
            "FROM episodes WHERE episode_id = ?",
            (episode_id,),
        ).fetchone()
        if row is None:
            raise HTTPException(status_code=404, detail="episode_not_found")

        entity_rows = conn.execute(
            "SELECT e.entity_id, e.entity_kind, e.entity_key, e.display_name, "
            "ee.role "
            "FROM episode_entities AS ee "
            "JOIN entities AS e ON e.entity_id = ee.entity_id "
            "WHERE ee.episode_id = ?",
            (episode_id,),
        ).fetchall()

        metadata = None
        if row["metadata"]:
            try:
                metadata = json.loads(row["metadata"])
            except (TypeError, ValueError):
                metadata = None

        return EpisodeReadResponse(
            episode_id=int(row["episode_id"]),
            timestamp=int(row["timestamp"]),
            created_at=int(row["created_at"]),
            content_text=str(row["content_text"]),
            episode_type=str(row["episode_type"]),
            salience_score=float(row["salience_score"]),
            last_recalled_at=(
                int(row["last_recalled_at"])
                if row["last_recalled_at"] is not None
                else None
            ),
            recall_count=int(row["recall_count"]),
            source=str(row["source"]),
            embedding_generated=row["content_embedding_id"] is not None,
            metadata=metadata,
            entities=[
                EntityOut(
                    entity_id=int(r["entity_id"]),
                    entity_kind=str(r["entity_kind"]),
                    entity_key=str(r["entity_key"]),
                    display_name=str(r["display_name"]),
                    role=str(r["role"]),
                )
                for r in entity_rows
            ],
        )
    finally:
        conn.close()
