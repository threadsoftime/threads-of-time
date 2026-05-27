"""Unit tests for SseConsumer (B4).

Key design facts:
- Dedup is keyed by memory_id (string UUID), NOT row_id (int)
- row_id is used as the cursor for Last-Event-ID (state_store)
- Heartbeat frames have no id: line → event.id is None → no cursor advance
- v0.2.2: dedup lives in brain_sidecar.dedup.SeenMemoryIds (was _DedupSet
  internal to sse_consumer). Tests for SeenMemoryIds semantics are in
  tests/unit/test_dedup.py; kept here: SseConsumer integration with dedup.
"""
from __future__ import annotations

import asyncio
import json
import pytest
from unittest.mock import AsyncMock, MagicMock, call

from brain_sidecar.sse_consumer import SseConsumer, _CoalesceBuffer
from brain_sidecar.dedup import SeenMemoryIds


# --- SeenMemoryIds smoke tests via SseConsumer (kept for regression) ---

def test_dedup_marks_and_recognizes():
    d = SeenMemoryIds(capacity=3)
    d.mark("uuid-a")
    d.mark("uuid-b")
    d.mark("uuid-c")
    assert d.seen("uuid-a") and d.seen("uuid-b") and d.seen("uuid-c")
    assert not d.seen("uuid-x")


def test_dedup_evicts_oldest_at_capacity():
    d = SeenMemoryIds(capacity=3)
    d.mark("uuid-1")
    d.mark("uuid-2")
    d.mark("uuid-3")
    d.mark("uuid-4")   # evicts "uuid-1"
    assert not d.seen("uuid-1")
    assert d.seen("uuid-2") and d.seen("uuid-3") and d.seen("uuid-4")


def test_dedup_idempotent_mark():
    """Marking same id twice doesn't grow beyond capacity."""
    d = SeenMemoryIds(capacity=2)
    d.mark("uuid-a")
    d.mark("uuid-a")  # second mark should not evict itself
    assert d.seen("uuid-a")


# --- _CoalesceBuffer ---

@pytest.mark.asyncio
async def test_coalesce_emits_after_window():
    fired = []

    async def on_drain(items):
        fired.append(list(items))

    buf = _CoalesceBuffer(window_ms=50, on_drain=on_drain)
    await buf.add({"row_id": 1})
    await buf.add({"row_id": 2})
    await asyncio.sleep(0.12)
    assert fired == [[{"row_id": 1}, {"row_id": 2}]]


@pytest.mark.asyncio
async def test_coalesce_resets_timer_on_new_event():
    fired = []

    async def on_drain(items):
        fired.append(list(items))

    buf = _CoalesceBuffer(window_ms=80, on_drain=on_drain)
    await buf.add({"row_id": 1})
    await asyncio.sleep(0.05)   # 50ms — within window, not yet fired
    await buf.add({"row_id": 2})   # resets timer
    await asyncio.sleep(0.05)      # 50ms more — 100ms total from first, only 50ms from second
    assert fired == []             # not yet fired (still within the 80ms window)
    await asyncio.sleep(0.06)      # now 110ms from second: fired
    assert len(fired) == 1
    assert fired[0] == [{"row_id": 1}, {"row_id": 2}]


# --- _handle_event ---

class _FakeEvent:
    """Mimics SseEvent for _handle_event testing."""
    def __init__(self, type_: str, id_: str | None, data: str):
        self.type = type_
        self.id = id_
        self.data = data


def _make_consumer(on_events=None, state_store=None):
    if state_store is None:
        state_store = MagicMock()
        state_store.read_last_event_id.return_value = 0
        state_store.write_last_event_id = MagicMock()
    if on_events is None:
        on_events = AsyncMock()
    return SseConsumer(
        bot_guid=1003,
        memory_url="http://test",
        bearer="x",
        state_store=state_store,
        on_events=on_events,
        prefixes=("received whisper",),
        coalesce_window_ms=50,
        dedup_capacity=100,
    )


@pytest.mark.asyncio
async def test_handle_event_memory_deduplicates_by_memory_id():
    """Duplicate memory_id should be dropped; second delivery is silently ignored."""
    state_store = MagicMock()
    state_store.read_last_event_id.return_value = 0
    state_store.write_last_event_id = MagicMock()

    consumer = _make_consumer(state_store=state_store)

    payload = {"row_id": 5, "memory_id": "uuid-abc", "bot_id": "1003",
               "text": "received whisper from Tebrack: hi", "created_ts": 1716321547}
    ev = _FakeEvent("memory", "5", json.dumps(payload))

    await consumer._handle_event(ev)
    await consumer._handle_event(ev)  # duplicate

    # write_last_event_id called only once (first delivery)
    assert state_store.write_last_event_id.call_count == 1
    assert state_store.write_last_event_id.call_args == call(1003, 5)


@pytest.mark.asyncio
async def test_handle_event_heartbeat_ignored():
    """Heartbeat events (type != 'memory') must be silently ignored.
    No cursor advance, no dedup entry."""
    state_store = MagicMock()
    state_store.read_last_event_id.return_value = 0
    state_store.write_last_event_id = MagicMock()

    consumer = _make_consumer(state_store=state_store)
    hb = _FakeEvent("heartbeat", None, '{"ts":1716321552}')
    await consumer._handle_event(hb)

    state_store.write_last_event_id.assert_not_called()


@pytest.mark.asyncio
async def test_handle_event_persists_row_id_as_cursor():
    """Cursor (Last-Event-ID) must use row_id (int), not memory_id."""
    state_store = MagicMock()
    state_store.read_last_event_id.return_value = 0
    state_store.write_last_event_id = MagicMock()

    consumer = _make_consumer(state_store=state_store)

    for row_id, memory_id in [(10, "uuid-10"), (20, "uuid-20")]:
        payload = {"row_id": row_id, "memory_id": memory_id, "bot_id": "1003",
                   "text": "received whisper x", "created_ts": 1716321547}
        await consumer._handle_event(_FakeEvent("memory", str(row_id), json.dumps(payload)))

    calls = state_store.write_last_event_id.call_args_list
    assert calls[0] == call(1003, 10)
    assert calls[1] == call(1003, 20)


@pytest.mark.asyncio
async def test_handle_event_bad_json_logged_not_raised(caplog):
    """Malformed data is logged and dropped, not raised."""
    import logging
    caplog.set_level(logging.WARNING)
    consumer = _make_consumer()
    ev = _FakeEvent("memory", "1", "not-json")
    await consumer._handle_event(ev)  # must not raise
    assert "sse_bad_payload" in caplog.text


# --- Backoff sequence ---

@pytest.mark.asyncio
async def test_consumer_backoff_sequence(monkeypatch):
    """On repeated failure, delays follow 0.5, 1.0, 2.0, 5.0, 5.0 ..."""
    delays = []

    async def fake_sleep(d):
        delays.append(d)

    import brain_sidecar.sse_consumer as mod
    monkeypatch.setattr(mod.asyncio, "sleep", fake_sleep)

    consumer = _make_consumer()

    attempt_count = 0

    async def fail_stream():
        nonlocal attempt_count
        attempt_count += 1
        raise ConnectionError("network")

    monkeypatch.setattr(consumer, "_stream_once", fail_stream)
    # max_attempts=5 → 5 failures → 4 sleeps (no sleep after the last failure before return)
    task = asyncio.create_task(consumer.run_until_complete(max_attempts=5))
    await task

    assert delays == [0.5, 1.0, 2.0, 5.0]
