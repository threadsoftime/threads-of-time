"""JSONL decision-log writer + stdlib logging config."""
from __future__ import annotations

import json
import logging
import sys
from pathlib import Path
from threading import Lock
from typing import Any


class JsonlDecisionLogWriter:
    """Append one JSON object per line. Thread-safe via a single lock.

    Uses threading.Lock (not asyncio.Lock) so the writer is also safe from sync
    contexts such as signal handlers and run_in_executor callbacks. The lock is
    over-protective for the async path — that is intentional and acceptable for
    MVP (per Task 10 domain-knowledge note #1).
    """

    def __init__(self, path: str) -> None:
        Path(path).parent.mkdir(parents=True, exist_ok=True)
        self._path = path
        self._lock = Lock()

    def write(self, record: dict[str, Any]) -> None:
        line = json.dumps(record, default=str)
        with self._lock, open(self._path, "a", encoding="utf-8") as f:
            f.write(line + "\n")


def setup_stdlib_logging(level: str = "INFO") -> None:
    logging.basicConfig(
        stream=sys.stdout,
        level=getattr(logging, level.upper(), logging.INFO),
        format="%(asctime)s %(levelname)s %(name)s :: %(message)s",
    )
