# SPDX-License-Identifier: GPL-2.0-or-later
"""DELETE /v1/memory/{bot_guid}/episodes/{episode_id} — memory.delete route.

Per design subspec §10.7. Hard-delete:

  1. The ``episode_entities`` FK has ``ON DELETE CASCADE``, so the join rows
     vanish automatically.
  2. The ``ee_ad`` trigger (§2.3) decrements ``entities.total_episodes`` for
     each link removed.
  3. ``embeddings_vec`` must be deleted explicitly — sqlite-vec virtual
     tables do not honour FK cascades (§10.7 implementation note).
  4. Finally delete the ``episodes`` row.

We delete in vec → episodes order so the cascading INSERTS into FTS via the
``episodes_ad`` trigger see the row exactly once.

Response: 204 No Content (plan abbreviation). The design subspec §10.7
documents 200 with ``{episode_id, deleted: true}``; we follow the
abbreviation here because 204 is the idiomatic REST DELETE response and the
caller already knows the ID from the URL. The MCP wrapper layer can re-shape
to ``{deleted: true}`` if needed for tool-call ergonomics.

Errors:
- 404 ``episode_not_found`` — episode_id absent in this bot's DB.
"""
from __future__ import annotations

from fastapi import APIRouter, HTTPException, Request, Response

from tot_memory.db.connection import open_bot_db
from tot_memory.db.runner import run_migrations


router = APIRouter()


@router.delete(
    "/v1/memory/{bot_guid}/episodes/{episode_id}",
    status_code=204,
)
def delete_episode(
    bot_guid: str, episode_id: int, request: Request
) -> Response:
    settings = request.app.state.settings
    conn = open_bot_db(settings.data_dir, bot_guid)
    try:
        run_migrations(conn)

        existing = conn.execute(
            "SELECT episode_id FROM episodes WHERE episode_id = ?",
            (episode_id,),
        ).fetchone()
        if existing is None:
            raise HTTPException(status_code=404, detail="episode_not_found")

        # Step 1: drop the embeddings_vec row explicitly (no FK cascade for vec0).
        conn.execute(
            "DELETE FROM embeddings_vec WHERE rowid = ?", (episode_id,)
        )
        # Step 2: delete the episode — cascades to episode_entities; the ee_ad
        # trigger decrements entities.total_episodes for each link removed;
        # the episodes_ad trigger keeps the FTS index in sync.
        conn.execute(
            "DELETE FROM episodes WHERE episode_id = ?", (episode_id,)
        )
        conn.commit()

        return Response(status_code=204)
    finally:
        conn.close()
