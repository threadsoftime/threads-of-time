# SPDX-License-Identifier: GPL-2.0-or-later
"""Brain-side thin async wrapper over the harness's memory.* HTTP tool surface.

Plan 2 Task 32. The harness daemon registers `memory.recall` and `memory.write`
(among others) as FastMCP tools and exposes them at POST
`{HARNESS_BASE_URL}/v1/tools/<tool_name>` (matching the same pattern used by
`bot.*` and `obs.*`). This client is what the brain's decision loop calls
before each "think" (recall) and after each notable event (write).

The harness wraps the tool response in `{"ok": true, "result": {...}}`; this
client unwraps `result` for the caller.
"""
from __future__ import annotations

import os
from dataclasses import dataclass

import httpx


@dataclass(frozen=True)
class RecalledEpisode:
    """One episode returned by `memory.recall`.

    Fields mirror the design subspec §10.3 response shape, projected down to
    what the brain needs to inject into the prompt.
    """

    episode_id: int
    content_text: str
    timestamp: str           # ISO-8601 string OR epoch ms as str — server-defined
    salience_score: float
    score: float             # hybrid recall_score (§6.2 of the subspec)


class MemoryClient:
    """Async client over the harness `memory.*` HTTP tools.

    Construct directly with `MemoryClient(base_url, bearer_token)` for tests,
    or use `MemoryClient.from_env()` in production wiring (reads
    `HARNESS_BASE_URL` and `HARNESS_BEARER_TOKEN`).

    Call `aclose()` when done — the client owns an `httpx.AsyncClient`.
    """

    def __init__(
        self,
        base_url: str,
        bearer_token: str,
        timeout: float = 10.0,
    ) -> None:
        self._base_url = base_url.rstrip("/")
        self._http = httpx.AsyncClient(
            timeout=timeout,
            headers={"Authorization": f"Bearer {bearer_token}"},
        )

    # ------------------------------------------------------------------
    # Factory
    # ------------------------------------------------------------------

    @classmethod
    def from_env(cls) -> "MemoryClient":
        """Build a client from `HARNESS_BASE_URL` + `HARNESS_BEARER_TOKEN`.

        `HARNESS_BASE_URL` is required; `HARNESS_BEARER_TOKEN` defaults to "" if
        unset (acceptable for local dev with auth disabled).
        """
        base_url = os.environ.get("HARNESS_BASE_URL")
        if not base_url:
            raise RuntimeError(
                "HARNESS_BASE_URL is not set; MemoryClient cannot reach the harness daemon."
            )
        bearer = os.environ.get("HARNESS_BEARER_TOKEN", "")
        return cls(base_url=base_url, bearer_token=bearer)

    # ------------------------------------------------------------------
    # Tool calls
    # ------------------------------------------------------------------

    async def recall(
        self,
        bot_guid: str,
        query_text: str,
        top_k: int = 5,
    ) -> list[RecalledEpisode]:
        """Call `memory.recall` and return the top-K hybrid-scored episodes."""
        response = await self._http.post(
            f"{self._base_url}/v1/tools/memory.recall",
            json={
                "bot_guid": bot_guid,
                "query_text": query_text,
                "top_k": top_k,
            },
        )
        response.raise_for_status()
        body = response.json()
        result = body.get("result", body) if isinstance(body, dict) else {}
        results = result.get("results", []) if isinstance(result, dict) else []
        return [
            RecalledEpisode(
                episode_id=item["episode_id"],
                content_text=item["content_text"],
                timestamp=item["timestamp"],
                salience_score=float(item["salience_score"]),
                score=float(item["score"]),
            )
            for item in results
        ]

    async def write_episode(
        self,
        bot_guid: str,
        content_text: str,
        episode_type: str,
        timestamp_iso: str,
        salience_score: float,
    ) -> int:
        """Call `memory.write` and return the newly-created `episode_id`.

        `entities` is left as an empty list at this layer; entity extraction is
        the brain's responsibility in a later task — Plan 2 Phase 7 scope is
        wire-up only.
        """
        response = await self._http.post(
            f"{self._base_url}/v1/tools/memory.write",
            json={
                "bot_guid": bot_guid,
                "content_text": content_text,
                "episode_type": episode_type,
                "timestamp": timestamp_iso,
                "salience_score": salience_score,
                "entities": [],
            },
        )
        response.raise_for_status()
        body = response.json()
        result = body.get("result", body) if isinstance(body, dict) else {}
        return int(result["episode_id"])

    async def aclose(self) -> None:
        await self._http.aclose()
