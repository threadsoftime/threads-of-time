"""Tests for SSE-specific settings fields (B2)."""
from __future__ import annotations

import pytest

from brain_sidecar.settings import get_settings


def test_sse_defaults():
    s = get_settings()
    assert s.brain_sse_enabled is True
    assert s.brain_sse_coalesce_ms == 200
    assert s.brain_sse_dedup_capacity == 100


def test_sse_env_override(monkeypatch):
    monkeypatch.setenv("BRAIN_SSE_ENABLED", "0")
    monkeypatch.setenv("BRAIN_SSE_COALESCE_MS", "350")
    monkeypatch.setenv("BRAIN_SSE_DEDUP_CAPACITY", "200")
    s = get_settings()
    assert s.brain_sse_enabled is False
    assert s.brain_sse_coalesce_ms == 350
    assert s.brain_sse_dedup_capacity == 200


def test_sse_enabled_truthy_values(monkeypatch):
    for val in ("1", "true", "True", "TRUE", "yes"):
        monkeypatch.setenv("BRAIN_SSE_ENABLED", val)
        s = get_settings()
        assert s.brain_sse_enabled is True, f"expected True for {val!r}"
