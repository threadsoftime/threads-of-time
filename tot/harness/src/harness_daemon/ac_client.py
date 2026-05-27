"""Async HTTP client to the mod-harness-bridge AC module.

Forwards parsed tool calls to POST /dispatch. Propagates the daemon's
request_id and identity via X-Daemon-* headers (AC bridge uses these
in its own JSONL audit).
"""

from __future__ import annotations

import time
from dataclasses import dataclass
from typing import Any

import httpx


class ACClientError(Exception):
    """AC bridge unreachable or returned a transport error."""


@dataclass
class ACResponse:
    status:        int
    body:          dict
    ac_latency_ms: int


class ACClient:
    """Thin async wrapper around httpx.AsyncClient for the AC bridge."""

    def __init__(self, base_url: str, timeout_s: float = 3.0) -> None:
        self._base_url = base_url.rstrip("/")
        self._timeout = timeout_s
        self._client = httpx.AsyncClient(
            base_url=self._base_url,
            timeout=httpx.Timeout(self._timeout),
        )

    async def dispatch(
        self,
        tool: str,
        args: dict[str, Any],
        request_id: str,
        identity: str,
    ) -> ACResponse:
        """POST /dispatch with the parsed tool call."""
        # V1.2: the gm.run_console C++ adapter REQUIRES args.request_id so it
        # can use the daemon-generated id as the console-capture key (see
        # docs/superpowers/specs/2026-05-18-agentic-harness-v1.2-design.md
        # §4.2). Mirror the daemon's request_id into args for this tool only,
        # using a defensive copy so we never mutate the caller's dict.
        forwarded_args = args
        if tool == "gm.run_console":
            forwarded_args = dict(args)
            forwarded_args["request_id"] = request_id

        t0 = time.perf_counter()
        try:
            resp = await self._client.post(
                "/dispatch",
                json={"tool": tool, "args": forwarded_args},
                headers={
                    "X-Daemon-Request-Id": request_id,
                    "X-Daemon-Identity":   identity,
                    "Content-Type":        "application/json",
                },
            )
        except httpx.RequestError as e:
            raise ACClientError(f"AC bridge unreachable: {e!r}")

        latency_ms = int((time.perf_counter() - t0) * 1000)
        try:
            body = resp.json()
        except ValueError:
            body = {"ok": False, "error": "non_json_response", "raw": resp.text[:200]}
        return ACResponse(status=resp.status_code, body=body, ac_latency_ms=latency_ms)

    async def health(self) -> bool:
        try:
            resp = await self._client.get("/health")
            return resp.status_code == 200
        except httpx.RequestError:
            return False

    async def close(self) -> None:
        await self._client.aclose()
