"""Tests for the JSONL audit logger."""

from __future__ import annotations

import json

from harness_daemon.audit import AuditLogger, AuditEvent


def test_audit_writes_one_line_per_event(tmp_path) -> None:
    path = tmp_path / "audit.jsonl"
    logger = AuditLogger(path)
    logger.write(AuditEvent(
        ts=1779000000.123,
        request_id="req_1",
        identity="alice",
        tool="obs.ping",
        args_body={"x": 1},
        outcome="ok",
        status=200,
        latency_ms=12,
        ac_latency_ms=5,
    ))
    lines = path.read_text().strip().split("\n")
    assert len(lines) == 1
    rec = json.loads(lines[0])
    assert rec["request_id"] == "req_1"
    assert rec["identity"] == "alice"
    assert rec["tool"] == "obs.ping"
    assert rec["outcome"] == "ok"
    assert rec["status"] == 200


def test_audit_hashes_args_not_logs_body(tmp_path) -> None:
    path = tmp_path / "audit.jsonl"
    logger = AuditLogger(path)
    secret_payload = {"password": "do-not-log-me"}
    logger.write(AuditEvent(
        ts=1779000000.123,
        request_id="req_2",
        identity="alice",
        tool="gm.run_console",
        args_body=secret_payload,
        outcome="ok",
        status=200,
        latency_ms=10,
        ac_latency_ms=2,
    ))
    raw = path.read_text()
    assert "do-not-log-me" not in raw
    rec = json.loads(raw.strip())
    assert "args_sha256" in rec
    # Hash is deterministic — same input → same digest.
    assert len(rec["args_sha256"]) == 64
