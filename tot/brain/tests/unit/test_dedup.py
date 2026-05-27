"""Unit tests for SeenMemoryIds in brain_sidecar.dedup (v0.2.2).

SeenMemoryIds is the supervisor-owned, per-bot dedup set shared between
the SSE path and the polling path.  Tests follow TDD: written before the
implementation exists so they fail first.
"""
from __future__ import annotations

from brain_sidecar.dedup import SeenMemoryIds


def test_seen_initially_false():
    """A freshly constructed SeenMemoryIds has no seen ids."""
    s = SeenMemoryIds(capacity=10)
    assert not s.seen("uuid-a")


def test_mark_then_seen():
    """mark() registers an id; seen() returns True afterwards."""
    s = SeenMemoryIds(capacity=10)
    s.mark("uuid-a")
    assert s.seen("uuid-a")
    assert not s.seen("uuid-b")


def test_lru_eviction_at_capacity():
    """When capacity is exceeded the oldest id is evicted (LRU)."""
    s = SeenMemoryIds(capacity=3)
    s.mark("uuid-1")
    s.mark("uuid-2")
    s.mark("uuid-3")
    s.mark("uuid-4")   # evicts "uuid-1"
    assert not s.seen("uuid-1"), "uuid-1 should have been evicted"
    assert s.seen("uuid-2")
    assert s.seen("uuid-3")
    assert s.seen("uuid-4")


def test_idempotent_mark_does_not_evict_itself():
    """Marking the same id twice must not cause a spurious eviction."""
    s = SeenMemoryIds(capacity=2)
    s.mark("uuid-a")
    s.mark("uuid-a")  # second mark — should NOT evict anything
    assert s.seen("uuid-a")
    # No second id was ever added, so we're still within capacity.
    assert len(s._od) == 1
