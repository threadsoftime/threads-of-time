"""Minimal SSE frame parser. No extra dependencies.

Handles the subset that memory-sidecar v0.3.0 emits:
  - event: <type>
  - id: <str>      (absent on heartbeat frames)
  - data: <str>    (multi-line joined with newline)
  - : comment      (ignored)

Frames are terminated by a blank line per the SSE spec.
"""
from __future__ import annotations

from dataclasses import dataclass
from typing import AsyncIterator, Optional


@dataclass(frozen=True)
class SseEvent:
    type: str
    id: Optional[str]
    data: str


async def parse_sse_chunks(chunks: AsyncIterator[str]) -> AsyncIterator[SseEvent]:
    """Parse SSE text chunks into SseEvent objects.

    Frames terminate on blank lines per the SSE spec.  Comment lines
    (starting with ``:``) are ignored.  Multiple ``data:`` lines are
    joined with ``\\n`` per spec.
    """
    buf = ""
    event_type = "message"
    event_id: Optional[str] = None
    data_lines: list[str] = []

    async for chunk in chunks:
        buf += chunk
        while "\n" in buf:
            line, buf = buf.split("\n", 1)
            line = line.rstrip("\r")  # handle CRLF

            if line == "":
                # Blank line → dispatch event (if there is data or a non-default type/id)
                if data_lines or event_id is not None or event_type != "message":
                    yield SseEvent(
                        type=event_type,
                        id=event_id,
                        data="\n".join(data_lines),
                    )
                # Reset to defaults for next event
                event_type = "message"
                event_id = None
                data_lines = []
            elif line.startswith(":"):
                # Comment — ignore
                continue
            elif line.startswith("event:"):
                event_type = line[6:].strip()
            elif line.startswith("id:"):
                event_id = line[3:].strip()
            elif line.startswith("data:"):
                data_lines.append(line[5:].lstrip(" "))
