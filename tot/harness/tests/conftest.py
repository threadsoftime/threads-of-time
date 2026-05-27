"""Shared fixtures for harness-daemon unit tests."""

from __future__ import annotations

import pytest


@pytest.fixture
def example_token_record() -> dict:
    """Minimal valid TokenRecord-shaped dict for tests."""
    return {
        "token": "tok-test-123",
        "identity": "test.runner",
        "scope": ["gm.*", "obs.*"],
    }


@pytest.fixture
def example_config_dict() -> dict:
    """Minimal valid config dict (round-trip-equivalent to YAML)."""
    return {
        "max_augmented_bots": 1,
        "ac_bridge_url": "http://127.0.0.1:8091",
        "audit_path": "/tmp/harness-daemon-test-audit.jsonl",
        "listen_address": "0.0.0.0:8090",
        "tokens": [
            {
                "token": "tok-test-123",
                "identity": "test.runner",
                "scope": ["gm.*", "obs.*"],
            },
        ],
    }
