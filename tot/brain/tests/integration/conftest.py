"""Integration-test fixtures (in-memory FakeMcp wires)."""
from __future__ import annotations

import pytest

from .mocks import FakeMcp, FakeLlm


@pytest.fixture
def fake_harness():
    return FakeMcp()


@pytest.fixture
def fake_memory():
    return FakeMcp()


@pytest.fixture
def fake_llm():
    return FakeLlm()
