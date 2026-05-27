# SPDX-License-Identifier: GPL-2.0-or-later
"""GET /v1/memory/{bot_guid}/episodes — memory.list route.

Per design subspec §10.5 (memory.list). Paginated list of episodes in
reverse-chronological order. Filters narrow the candidate set; pagination
applies after filtering.

Query params:
  episode_type? -- exact match against episodes.episode_type
  entity_name?  -- OR-logic against entities.display_name (the plan
                   abbreviation calls this entity_name; §10.5 calls it
                   entity_id. We use entity_name to match the rest of the
                   memory.* surface (recall also uses display names).
  after?        -- timestamp >= after (epoch ms)
  before?       -- timestamp <= before (epoch ms)
  limit         -- default 50, max 200 (plan abbreviation; §10.5 says 20/100
                   but the abbreviation's wider caps are more useful for the
                   inspector tool that drives this endpoint)
  offset        -- default 0

Response shape: ``{results, total, limit, offset, has_more}`` per §10.5,
using ``results`` (the cross-route convention used by recall + search) in
place of §10.5's ``episodes`` for naming consistency.
"""
from __future__ import annotations

import json
from typing import Any, Optional

from fastapi import APIRouter, HTTPException, Query, Request
from pydantic import BaseModel

from tot_memory.db.connection import open_bot_db
from tot_memory.db.runner import run_migrations


router = APIRouter()


class ListEpisodeRow(BaseModel):
    episode_id: int
    timestamp: int
    content_text: str
    episode_type: str
    salience_score: float
    source: str
    embedding_generated: bool
    metadata: Optional[dict[str, Any]]


class ListEpisodesResponse(BaseModel):
    results: list[ListEpisodeRow]
    total: int
    limit: int
    offset: int
    has_more: bool


@router.get(
    "/v1/memory/{bot_guid}/episodes",
    status_code=200,
    response_model=ListEpisodesResponse,
)
def list_episodes(
    bot_guid: str,
    request: Request,
    episode_type: Optional[str] = Query(None),
    entity_name: Optional[str] = Query(None),
    after: Optional[int] = Query(None, description="timestamp >= after (ms)"),
    before: Optional[int] = Query(None, description="timestamp <= before (ms)"),
    limit: int = Query(50, ge=1, le=200),
    offset: int = Query(0, ge=0),
) -> ListEpisodesResponse:
    settings = request.app.state.settings
    conn = open_bot_db(settings.data_dir, bot_guid)
    try:
        run_migrations(conn)

        clauses: list[str] = []
        params: list[Any] = []
        joins = ""

        if entity_name:
            joins = (
                " JOIN episode_entities AS ee ON ee.episode_id = episodes.episode_id"
                " JOIN entities AS e ON e.entity_id = ee.entity_id"
            )
            clauses.append("e.display_name = ?")
            params.append(entity_name)
        if episode_type:
            clauses.append("episodes.episode_type = ?")
            params.append(episode_type)
        if after is not None:
            clauses.append("episodes.timestamp >= ?")
            params.append(after)
        if before is not None:
            clauses.append("episodes.timestamp <= ?")
            params.append(before)

        where = ("WHERE " + " AND ".join(clauses)) if clauses else ""

        # COUNT(DISTINCT ...) so an episode joined through multiple entity rows
        # (it cannot be — episode_entities PK is (episode, entity, role) — but
        # belt and braces) isn't counted twice.
        total = int(
            conn.execute(
                f"SELECT COUNT(DISTINCT episodes.episode_id) "
                f"FROM episodes{joins} {where}",
                tuple(params),
            ).fetchone()[0]
        )

        rows = conn.execute(
            f"SELECT DISTINCT episodes.episode_id, episodes.timestamp, "
            f"episodes.content_text, episodes.episode_type, "
            f"episodes.salience_score, episodes.source, "
            f"episodes.content_embedding_id, episodes.metadata "
            f"FROM episodes{joins} {where} "
            f"ORDER BY episodes.timestamp DESC, episodes.episode_id DESC "
            f"LIMIT ? OFFSET ?",
            (*params, limit, offset),
        ).fetchall()

        results = []
        for row in rows:
            metadata = None
            if row["metadata"]:
                try:
                    metadata = json.loads(row["metadata"])
                except (TypeError, ValueError):
                    metadata = None
            results.append(
                ListEpisodeRow(
                    episode_id=int(row["episode_id"]),
                    timestamp=int(row["timestamp"]),
                    content_text=str(row["content_text"]),
                    episode_type=str(row["episode_type"]),
                    salience_score=float(row["salience_score"]),
                    source=str(row["source"]),
                    embedding_generated=row["content_embedding_id"] is not None,
                    metadata=metadata,
                )
            )

        return ListEpisodesResponse(
            results=results,
            total=total,
            limit=limit,
            offset=offset,
            has_more=(offset + len(results)) < total,
        )
    finally:
        conn.close()
