"""Append-only JSONL audit logger.

Args bodies are NEVER serialized — we log a SHA-256 digest instead
(spec §9.1). Each call appends one line. Daily rotation happens
externally via a Quadlet sidecar timer (Task 22).
"""

from __future__ import annotations

import hashlib
import json
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any


@dataclass
class AuditEvent:
    ts:              float
    request_id:      str
    identity:        str
    tool:            str
    args_body:       Any            # hashed, never serialized
    outcome:         str
    status:          int
    latency_ms:      int
    ac_latency_ms:   int = 0
    error_detail:    str = ""
    transport:       str = "http"


def _sha256_args(body: Any) -> str:
    canonical = json.dumps(body, sort_keys=True, separators=(",", ":"), default=str)
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()


class AuditLogger:
    """Append a single line per event. Thread-safe at OS append level."""

    def __init__(self, path: Path | str) -> None:
        self._path = Path(path)
        self._path.parent.mkdir(parents=True, exist_ok=True)

    def write(self, ev: AuditEvent) -> None:
        record = {
            "ts":             ev.ts,
            "request_id":     ev.request_id,
            "identity":       ev.identity,
            "tool":           ev.tool,
            "args_sha256":    _sha256_args(ev.args_body),
            "outcome":        ev.outcome,
            "status":         ev.status,
            "latency_ms":     ev.latency_ms,
            "ac_latency_ms":  ev.ac_latency_ms,
            "transport":      ev.transport,
        }
        if ev.error_detail:
            record["error_detail"] = ev.error_detail

        with self._path.open("a") as fh:
            fh.write(json.dumps(record, separators=(",", ":")))
            fh.write("\n")
