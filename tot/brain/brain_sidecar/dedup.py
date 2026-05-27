"""Shared dedup set for brain-sidecar (v0.2.2).

SeenMemoryIds is a supervisor-owned, per-bot LRU set of memory_ids.  It is
shared between SseConsumer (which marks ids as they arrive via SSE) and the
_one_tick post-triage filter in LoopSupervisor (which filters out ids that
the SSE path already decided on before the polling path can re-decide them).

Design note: keyed by memory_id (stable UUID string) not row_id (int) —
row_ids are only cursor-advancing labels and may be reassigned; memory_ids
are the stable primary key of the memory store.
"""
from __future__ import annotations

from collections import OrderedDict


class SeenMemoryIds:
    """Bounded LRU set of seen memory_ids (string UUIDs).

    Thread-safety: NOT thread-safe.  All access must happen from the same
    asyncio event loop (which is the case for brain-sidecar — single-loop,
    cooperative multitasking only).

    Args:
        capacity: Maximum number of ids to retain.  When exceeded the
            oldest entry is evicted (LRU discipline).
    """

    def __init__(self, capacity: int) -> None:
        self._capacity = capacity
        self._od: OrderedDict[str, None] = OrderedDict()

    def mark(self, memory_id: str) -> None:
        """Register memory_id as seen.  No-op if already present (idempotent)."""
        if memory_id in self._od:
            self._od.move_to_end(memory_id)
        else:
            self._od[memory_id] = None
            if len(self._od) > self._capacity:
                self._od.popitem(last=False)

    def seen(self, memory_id: str) -> bool:
        """Return True if memory_id was previously marked."""
        return memory_id in self._od
