"""Tests for last_event_id helpers on StateStore (migration 0002)."""
from __future__ import annotations

import pytest

from brain_sidecar.models import PersonalityCard
from brain_sidecar.state import StateStore


def _card() -> PersonalityCard:
    return PersonalityCard(
        name="Casmina",
        race="Human",
        **{"class": "Paladin"},
        backstory="A tested knight.",
        talkativeness=0.5,
        courage=0.7,
        greed=0.2,
        attitude_to_master=0.8,
    )


@pytest.fixture
def store(temp_db_path):
    s = StateStore(temp_db_path)
    s.migrate()
    yield s
    s.close()


def test_last_event_id_defaults_to_zero(store):
    store.enroll(bot_guid=1003, enrolled_at_ms=1_700_000_000_000, personality_seed=_card())
    assert store.read_last_event_id(1003) == 0


def test_write_then_read_last_event_id(store):
    store.enroll(bot_guid=1003, enrolled_at_ms=1_700_000_000_000, personality_seed=_card())
    store.write_last_event_id(1003, 12345)
    assert store.read_last_event_id(1003) == 12345


def test_write_last_event_id_monotonic(store):
    """Don't regress to a lower value if a stale write tries."""
    store.enroll(bot_guid=1003, enrolled_at_ms=1_700_000_000_000, personality_seed=_card())
    store.write_last_event_id(1003, 100)
    store.write_last_event_id(1003, 50)  # should NOT regress
    assert store.read_last_event_id(1003) == 100


def test_read_last_event_id_unknown_bot_returns_zero(store):
    """Reading an unenrolled bot returns 0, not an error."""
    assert store.read_last_event_id(9999) == 0
