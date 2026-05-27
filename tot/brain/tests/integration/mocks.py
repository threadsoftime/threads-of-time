"""Lightweight in-memory mock MCPs + LLM for integration tests."""
from __future__ import annotations

import asyncio
import json
from collections.abc import Callable
from typing import Any


class FakeMcp:
    """In-memory MCP. Default behavior: return {} for everything; override per-tool via .responder."""

    def __init__(self) -> None:
        self.calls: list[tuple[str, dict[str, Any]]] = []
        self.responder: Callable[[str, dict[str, Any]], dict[str, Any]] | None = None
        self.delay_s: float = 0.0
        self.fail_with: Exception | None = None

    async def call(self, tool: str, args: dict[str, Any]) -> dict[str, Any]:
        self.calls.append((tool, args))
        if self.delay_s > 0:
            await asyncio.sleep(self.delay_s)
        if self.fail_with is not None:
            raise self.fail_with
        if self.responder is not None:
            return self.responder(tool, args)
        return {}


class FakeLlm:
    """In-memory LLM. Returns a pre-set decision JSON."""

    def __init__(self) -> None:
        self.response_json: dict[str, Any] | None = None
        self.delay_s: float = 0.0
        self.call_count: int = 0
        self.last_json_schema: dict[str, Any] | None = None

    async def chat_completion_json(
        self, *, system: str, user: str, max_tokens: int = 400, temperature: float = 0.5,
        json_schema: dict[str, Any] | None = None,
    ) -> tuple[dict[str, Any] | None, str, float]:
        self.call_count += 1
        self.last_json_schema = json_schema
        if self.delay_s > 0:
            await asyncio.sleep(self.delay_s)
        return self.response_json, json.dumps(self.response_json) if self.response_json else "", 100.0
