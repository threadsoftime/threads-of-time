"""Unit tests for the inline SSE frame parser (B3)."""
from __future__ import annotations

import pytest

from brain_sidecar.sse_parser import parse_sse_chunks, SseEvent


@pytest.mark.asyncio
async def test_parse_single_memory_event():
    async def chunks():
        yield 'event: memory\nid: 42\ndata: {"row_id":42,"text":"hi"}\n\n'

    events = [e async for e in parse_sse_chunks(chunks())]
    assert len(events) == 1
    assert events[0] == SseEvent(type="memory", id="42", data='{"row_id":42,"text":"hi"}')


@pytest.mark.asyncio
async def test_parse_multiple_events_split_across_chunks():
    async def chunks():
        yield "event: memory\n"
        yield 'id: 1\ndata: {"a":1}\n\nevent: heart'
        yield "beat\ndata: {}\n\n"

    events = [e async for e in parse_sse_chunks(chunks())]
    assert len(events) == 2
    assert events[0].type == "memory" and events[0].id == "1"
    assert events[1].type == "heartbeat"


@pytest.mark.asyncio
async def test_parse_ignores_comment_lines():
    async def chunks():
        yield ":keepalive\n\nevent: memory\nid: 1\ndata: x\n\n"

    events = [e async for e in parse_sse_chunks(chunks())]
    assert len(events) == 1
    assert events[0].type == "memory"


@pytest.mark.asyncio
async def test_parse_default_event_type_is_message():
    async def chunks():
        yield "data: hello\n\n"

    events = [e async for e in parse_sse_chunks(chunks())]
    assert events[0].type == "message"
    assert events[0].data == "hello"
    assert events[0].id is None


@pytest.mark.asyncio
async def test_parse_multiline_data():
    async def chunks():
        yield "event: memory\ndata: line1\ndata: line2\n\n"

    events = [e async for e in parse_sse_chunks(chunks())]
    assert events[0].data == "line1\nline2"


@pytest.mark.asyncio
async def test_parse_heartbeat_no_id():
    """Heartbeat frames have no id: line — event.id must be None."""
    async def chunks():
        yield 'event: heartbeat\ndata: {"ts":1716321552}\n\n'

    events = [e async for e in parse_sse_chunks(chunks())]
    assert len(events) == 1
    assert events[0].type == "heartbeat"
    assert events[0].id is None


@pytest.mark.asyncio
async def test_parse_empty_stream_yields_nothing():
    async def chunks():
        return
        yield  # make it an async generator

    events = [e async for e in parse_sse_chunks(chunks())]
    assert events == []


@pytest.mark.asyncio
async def test_parse_crlf_line_endings():
    """SSE spec allows \r\n; strip the \r from lines."""
    async def chunks():
        yield "event: memory\r\nid: 7\r\ndata: test\r\n\r\n"

    events = [e async for e in parse_sse_chunks(chunks())]
    assert len(events) == 1
    assert events[0].id == "7"
    assert events[0].data == "test"
