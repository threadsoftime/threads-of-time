"""Shared pytest fixtures."""
from __future__ import annotations

import os
import tempfile
from collections.abc import Iterator
from pathlib import Path

import pytest


@pytest.fixture
def temp_db_path() -> Iterator[str]:
    """Yield a path to a fresh temp sqlite file; clean up after."""
    fd, path = tempfile.mkstemp(suffix=".sqlite")
    os.close(fd)
    yield path
    Path(path).unlink(missing_ok=True)


@pytest.fixture
def now_ms() -> int:
    """Fixed 'current time' for deterministic tests."""
    return 1_700_000_000_000  # 2023-11-14T22:13:20Z
