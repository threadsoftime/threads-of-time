# SPDX-License-Identifier: GPL-2.0-or-later
"""BYOLLM embeddings client.

Per design subspec §4 (BYOLLM model) and §4.4 (API contract). POSTs to an
OpenAI-compatible ``/v1/embeddings`` endpoint and returns the raw vector.

Assumes the model L2-normalizes its output (per §5.3 — ``nomic-embed-text``
returns L2-normalized vectors by default). If an operator substitutes a model
without L2 normalization, normalize before storage.
"""
from __future__ import annotations

import httpx

from tot_memory.db.schema import EMBEDDING_DIM


class EmbeddingsClient:
    """OpenAI-compatible ``/v1/embeddings`` client (BYOLLM).

    Args:
        base_url: Base URL of the endpoint, e.g. ``http://localhost:11434/v1``.
            The client appends ``/embeddings`` to this base.
        model: Model string sent in the request body (e.g. ``"nomic-embed-text"``).
        api_key: Bearer token. If empty string, no ``Authorization`` header is sent.
        timeout: Per-request timeout in seconds (default 30s — sync write path).

    The client is async because the write route is async; calls happen inline on
    the request path (§5.1 sync decision). Latency budget: ~50-80ms on local Ollama.
    """

    def __init__(
        self,
        base_url: str,
        model: str,
        api_key: str,
        timeout: float = 30.0,
    ) -> None:
        self._base_url = base_url.rstrip("/")
        self._model = model
        headers: dict[str, str] = {}
        if api_key:
            headers["Authorization"] = f"Bearer {api_key}"
        self._http = httpx.AsyncClient(timeout=timeout, headers=headers)

    async def embed(self, text: str) -> list[float]:
        """Embed ``text`` and return a list[float] of length EMBEDDING_DIM.

        Raises:
            httpx.HTTPError subclasses on network / non-2xx errors. The write
                route (§5.1) catches these and falls back to writing the episode
                with ``content_embedding_id = NULL``.
            ValueError: when the returned embedding length != EMBEDDING_DIM.
                This catches operator misconfiguration where the model behind
                ``BRAIN_EMBEDDINGS_URL`` returns a different dimension than the
                sqlite-vec table was sized for.
            KeyError / IndexError: when the response payload is malformed
                (missing ``data[0].embedding``).
        """
        response = await self._http.post(
            f"{self._base_url}/embeddings",
            json={"model": self._model, "input": text},
        )
        response.raise_for_status()
        payload = response.json()
        vec = payload["data"][0]["embedding"]
        if len(vec) != EMBEDDING_DIM:
            raise ValueError(
                f"Embedding dimension mismatch: endpoint returned {len(vec)}, "
                f"expected {EMBEDDING_DIM}. Check that BRAIN_EMBEDDINGS_MODEL "
                f"matches the dimension baked into the sqlite-vec table "
                f"(design subspec §3 + §4)."
            )
        return vec

    async def aclose(self) -> None:
        await self._http.aclose()
