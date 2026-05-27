"""C7: in-process LRU personality cache backed by memory MCP.

Memory-sidecar contract (memory.personality_set / memory.personality_get):
  - Both tools expect bot_id as a STRING (not int).
  - memory.personality_set: {"bot_id": str, "persona": str} — persona is a
    JSON-encoded string of the PersonalityCard dict (≤4000 chars).
  - memory.personality_get: {"bot_id": str} → {"persona": str} — caller
    must JSON-decode the persona string back into a PersonalityCard.

V3.6: takes an LlmClient and runs morph_personality on cache-miss when
the persisted card has any None v2 field (lazy migration of V1 cards).
Per-bot asyncio.Lock serializes the migrate-and-persist path.
"""
from __future__ import annotations

import asyncio
import json
import time
from collections import OrderedDict
from collections.abc import Callable
from dataclasses import dataclass
from typing import Any

from brain_sidecar.llm_client import LlmClient
from brain_sidecar.mcp_clients import McpClient
from brain_sidecar.models import PersonalityCard


@dataclass
class _Entry:
    card: PersonalityCard
    fetched_at: float


class PersonalityCache:
    def __init__(
        self,
        *,
        memory_mcp: McpClient | Any,
        ttl_s: float,
        capacity: int,
        llm_client: LlmClient | Any | None = None,
        now_fn: Callable[[], float] | None = None,
    ) -> None:
        self._mcp = memory_mcp
        self._ttl_s = ttl_s
        self._capacity = capacity
        self._llm_client = llm_client
        self._now = now_fn or time.monotonic
        self._cache: OrderedDict[int, _Entry] = OrderedDict()
        # V3.6: per-bot locks for the migration path
        self._locks: dict[int, asyncio.Lock] = {}

    def _lock_for(self, bot_guid: int) -> asyncio.Lock:
        """Return the lock for this bot, creating it lazily."""
        lock = self._locks.get(bot_guid)
        if lock is None:
            lock = asyncio.Lock()
            self._locks[bot_guid] = lock
        return lock

    async def get(self, bot_guid: int) -> PersonalityCard:
        # Fast path: cache hit without lock
        now = self._now()
        entry = self._cache.get(bot_guid)
        if entry is not None and (now - entry.fetched_at) < self._ttl_s:
            self._cache.move_to_end(bot_guid)
            return entry.card

        # Cache miss or expired — acquire per-bot lock to serialize migration
        async with self._lock_for(bot_guid):
            # Double-check: another caller may have populated cache while we waited
            entry = self._cache.get(bot_guid)
            if entry is not None and (self._now() - entry.fetched_at) < self._ttl_s:
                self._cache.move_to_end(bot_guid)
                return entry.card

            # Fetch from memory MCP (existing flow)
            raw = await self._mcp.call(
                "memory.personality_get", {"bot_id": str(bot_guid)}
            )
            payload = raw.get("result", raw) if isinstance(raw, dict) else {}
            persona_json = payload.get("persona", "{}")
            card = PersonalityCard.model_validate(json.loads(persona_json))

            # V3.6 migration: if any v2 field is None, morph + persist back.
            # Lazy import avoids a circular-import risk between morph.py and
            # personality.py (morph.py imports PersonalityCard from models).
            from brain_sidecar.morph import _needs_morph, morph_personality
            if _needs_morph(card):
                if self._llm_client is None:
                    # Constructor allowed llm_client=None for back-compat; in
                    # production, app.py wires it. If somehow None here, log and
                    # skip morph (returning a card with None v2 fields would
                    # leak into _assemble_prompt — accept that as a soft fail).
                    import logging
                    logging.getLogger(__name__).error(
                        "PersonalityCache.get bot_guid=%d: llm_client is None; "
                        "cannot migrate v1 card. v2 fields remain None.",
                        bot_guid,
                    )
                else:
                    card = await morph_personality(card, self._llm_client)
                    try:
                        await self._mcp.call(
                            "memory.personality_set",
                            {
                                "bot_id": str(bot_guid),
                                "persona": json.dumps(card.model_dump(by_alias=True)),
                            },
                        )
                    except Exception as e:
                        import logging
                        logging.getLogger(__name__).error(
                            "PersonalityCache.get bot_guid=%d: migration persist "
                            "failed: %s. Cache populated; will retry on eviction.",
                            bot_guid, e,
                        )

            # Cache + return (existing flow)
            self._cache[bot_guid] = _Entry(card=card, fetched_at=self._now())
            self._cache.move_to_end(bot_guid)
            while len(self._cache) > self._capacity:
                self._cache.popitem(last=False)
            return card

    async def seed(self, bot_guid: int, card: PersonalityCard) -> None:
        # persona must be a JSON string per memory-sidecar schema; bot_id must be str.
        persona_str = json.dumps(card.model_dump(by_alias=True))
        await self._mcp.call(
            "memory.personality_set",
            {"bot_id": str(bot_guid), "persona": persona_str},
        )
        self._cache[bot_guid] = _Entry(card=card, fetched_at=self._now())
        self._cache.move_to_end(bot_guid)
        while len(self._cache) > self._capacity:
            self._cache.popitem(last=False)
