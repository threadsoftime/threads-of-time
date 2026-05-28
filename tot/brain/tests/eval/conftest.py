# SPDX-License-Identifier: GPL-2.0-or-later
"""Shared helpers for eval tests.

These are the same helpers as tests/integration/conftest.py, inlined here so
the eval package has no cross-package import dependency.
"""
from __future__ import annotations

from brain_sidecar.models import PersonalityCard
from brain_sidecar.state import StateStore


def _make_personality() -> PersonalityCard:
    """Minimal PersonalityCard for StateStore.enroll() in eval tests."""
    return PersonalityCard(
        name="EvalBot",
        race="Human",
        **{"class": "Warrior"},
        backstory="eval test bot",
        talkativeness=0.5,
        courage=0.5,
        greed=0.5,
        attitude_to_master=0.5,
    )


def _fake_state_store(db_path: str | None = None) -> StateStore:
    """Return a migrated in-memory (or file-backed) StateStore."""
    import os
    import tempfile

    if db_path is None:
        fd, db_path = tempfile.mkstemp(suffix=".sqlite")
        os.close(fd)
    store = StateStore(db_path)
    store.migrate()
    return store
