"""SSE consumer for brain-sidecar — one per enrolled bot.

Key design facts (Phase-2 corrections):
  - Dedup is keyed by memory_id (string UUID) — the stable PK of a memory row.
  - row_id (int) is used as the cursor for the SSE Last-Event-ID header on reconnect.
  - Heartbeat frames carry no id: line → event.id is None → cursor is not advanced.
  - Coalescing: events accumulate in a 200ms window after the first event; the whole
    batch is handed to on_events at once (one decide cycle per burst).

v0.2.2: _DedupSet removed; SseConsumer now accepts an injected SeenMemoryIds
  (from brain_sidecar.dedup) so the supervisor can share the same set between
  the SSE path and the polling path, closing the SSE/poll dedup gap.
"""
from __future__ import annotations

import asyncio
import json
import logging
from dataclasses import dataclass, field
from typing import Awaitable, Callable, Optional

import httpx

from brain_sidecar.dedup import SeenMemoryIds
from brain_sidecar.sse_parser import parse_sse_chunks, SseEvent

logger = logging.getLogger(__name__)

_BACKOFF_SEQ: tuple[float, ...] = (0.5, 1.0, 2.0, 5.0, 5.0)


class _CoalesceBuffer:
    """Buffers events; drains via on_drain callback after window_ms of quiet.

    After the first event is added, a debounce timer starts.  Every
    subsequent add() cancels the previous timer and starts a fresh one.
    The buffer is drained (on_drain called) only once the quiet window
    has elapsed with no new events.
    """

    def __init__(self, window_ms: int, on_drain: Callable[[list[dict]], Awaitable[None]]) -> None:
        self._window_s = window_ms / 1000.0
        self._on_drain = on_drain
        self._buf: list[dict] = []
        self._task: Optional[asyncio.Task] = None
        self._lock = asyncio.Lock()

    async def add(self, item: dict) -> None:
        async with self._lock:
            self._buf.append(item)
            if self._task is not None and not self._task.done():
                self._task.cancel()
            self._task = asyncio.create_task(self._wait_and_drain())

    async def _wait_and_drain(self) -> None:
        try:
            await asyncio.sleep(self._window_s)
        except asyncio.CancelledError:
            return
        async with self._lock:
            if not self._buf:
                return
            items = self._buf[:]
            self._buf.clear()
        await self._on_drain(items)


@dataclass
class SseConsumer:
    """Long-lived SSE subscriber for one enrolled bot.

    Opened by the LoopSupervisor alongside the 5s polling tick.  Reconnects
    automatically with exponential backoff; persists Last-Event-ID cursor in
    state_store so replay catches up on reconnect without loss.

    Public interface:
      - run_until_complete(): main coroutine; cancel to stop.
      - cancel(): signal a clean stop (run_until_complete returns on next iteration).
      - _handle_event(): exposed for unit testing only.
    """

    bot_guid: int
    memory_url: str
    bearer: str
    state_store: object   # StateStore (not imported to avoid circular deps)
    on_events: Callable[[list[dict]], Awaitable[None]]
    prefixes: tuple[str, ...] = ()
    coalesce_window_ms: int = 200
    dedup_capacity: int = 100
    # v0.2.2: injected from LoopSupervisor so SSE and polling share the same set.
    # If None, a private SeenMemoryIds(capacity=dedup_capacity) is created.
    dedup_set: Optional[SeenMemoryIds] = None

    _dedup: SeenMemoryIds = field(init=False)
    _coalesce: _CoalesceBuffer = field(init=False)
    _cancel: bool = field(init=False, default=False)

    def __post_init__(self) -> None:
        self._dedup = (
            self.dedup_set
            if self.dedup_set is not None
            else SeenMemoryIds(self.dedup_capacity)
        )
        self._coalesce = _CoalesceBuffer(self.coalesce_window_ms, self._fire_events)
        self._cancel = False

    async def _fire_events(self, items: list[dict]) -> None:
        try:
            await self.on_events(items)
        except Exception:
            logger.exception("sse_on_events_failed bot_guid=%s", self.bot_guid)

    async def _handle_event(self, event: SseEvent) -> None:
        """Process one parsed SSE frame.

        Only 'memory' events are processed.  Heartbeats and errors are ignored
        or logged.  For memory events:
          - row_id (int) → cursor for Last-Event-ID via state_store
          - memory_id (str) → dedup key (stable UUID of the memory row)
        """
        if event.type != "memory":
            return

        try:
            payload = json.loads(event.data)
        except json.JSONDecodeError:
            logger.warning(
                "sse_bad_payload bot_guid=%s data=%r", self.bot_guid, event.data[:200]
            )
            return

        row_id = int(payload["row_id"])          # cursor / Last-Event-ID
        memory_id = str(payload["memory_id"])    # dedup key (stable UUID)

        if self._dedup.seen(memory_id):
            return

        self._dedup.mark(memory_id)

        # Persist the row_id cursor monotonically (never regress).
        try:
            self.state_store.write_last_event_id(self.bot_guid, row_id)
        except Exception:
            logger.exception(
                "sse_state_write_failed bot_guid=%s row_id=%s", self.bot_guid, row_id
            )

        await self._coalesce.add(payload)

    async def _stream_once(self) -> None:
        """Open one SSE stream and consume until closed or cancelled."""
        last_id = self.state_store.read_last_event_id(self.bot_guid)
        params = {
            "bot_id": str(self.bot_guid),
            "prefixes": ",".join(self.prefixes),
        }
        headers = {
            "Authorization": f"Bearer {self.bearer}",
            "Accept": "text/event-stream",
        }
        if last_id:
            headers["Last-Event-ID"] = str(last_id)

        timeout = httpx.Timeout(connect=5.0, read=None, write=5.0, pool=5.0)
        async with httpx.AsyncClient(timeout=timeout) as client:
            async with client.stream(
                "GET",
                f"{self.memory_url}/v1/events/stream",
                params=params,
                headers=headers,
            ) as response:
                response.raise_for_status()
                logger.info(
                    "sse_subscribed bot_guid=%s last_event_id=%s", self.bot_guid, last_id
                )
                async for ev in parse_sse_chunks(self._iter_text(response)):
                    if self._cancel:
                        return
                    await self._handle_event(ev)

    @staticmethod
    async def _iter_text(response: httpx.Response):
        """Yield text chunks from the httpx streaming response."""
        async for chunk in response.aiter_text():
            yield chunk

    async def run_until_complete(self, *, max_attempts: Optional[int] = None) -> None:
        """Run the consumer loop until cancelled or max_attempts exhausted.

        On each successful stream completion, backoff resets to zero.
        On failure, backoff follows _BACKOFF_SEQ (0.5s → 1.0 → 2.0 → 5.0 → 5.0 …).

        ``max_attempts`` is for unit-test use only; production calls without it.
        """
        attempts = 0
        backoff_idx = 0
        while not self._cancel:
            try:
                await self._stream_once()
                backoff_idx = 0   # successful run resets backoff
                if max_attempts is not None:
                    return
            except asyncio.CancelledError:
                raise
            except Exception as e:
                logger.warning("sse_disconnect bot_guid=%s err=%s", self.bot_guid, e)
                attempts += 1
                if max_attempts is not None and attempts >= max_attempts:
                    return
                delay = _BACKOFF_SEQ[min(backoff_idx, len(_BACKOFF_SEQ) - 1)]
                backoff_idx += 1
                await asyncio.sleep(delay)

    def cancel(self) -> None:
        """Signal the consumer to stop on the next iteration."""
        self._cancel = True
