"""Integration-test fixtures (in-memory FakeMcp wires)."""
from __future__ import annotations

import pytest

from .mocks import FakeMcp, FakeLlm
from brain_sidecar.models import PersonalityCard
from brain_sidecar.state import StateStore


@pytest.fixture
def fake_harness():
    return FakeMcp()


@pytest.fixture
def fake_memory():
    return FakeMcp()


@pytest.fixture
def fake_llm():
    return FakeLlm()


# ---------------------------------------------------------------------------
# SubsetGate helpers (Plan 3 T22)
# ---------------------------------------------------------------------------

def _make_personality() -> PersonalityCard:
    """Minimal PersonalityCard for StateStore.enroll() in integration tests."""
    return PersonalityCard(
        name="TestBot",
        race="Human",
        **{"class": "Warrior"},
        backstory="integration test bot",
        talkativeness=0.5,
        courage=0.5,
        greed=0.5,
        attitude_to_master=0.5,
    )


def _fake_state_store(db_path: str | None = None) -> StateStore:
    """Return a migrated in-memory (or file-backed) StateStore.

    When db_path is None, uses ':memory:' — suitable for tests that don't
    need persistence across StateStore instances.  Pass tmp_path / "test.sqlite"
    for tests that exercise list_active() from a separate code path.
    """
    import tempfile, os as _os
    if db_path is None:
        # Use an in-process SQLite file in a temp dir so migrate() can find it.
        fd, db_path = tempfile.mkstemp(suffix=".sqlite")
        _os.close(fd)
    store = StateStore(db_path)
    store.migrate()
    return store
