"""Tests for the SQLite state store (C6)."""
from __future__ import annotations

import json

import pytest

from brain_sidecar.models import Decision, DecisionKind, PersonalityCard
from brain_sidecar.state import StateStore


@pytest.fixture
def store(temp_db_path):
    s = StateStore(temp_db_path)
    s.migrate()
    yield s
    s.close()


def test_migrate_idempotent(store):
    store.migrate()
    store.migrate()  # second run must not raise


def test_enroll_round_trips(store, now_ms):
    card = PersonalityCard(
        name="C", race="Dwarf", **{"class": "Hunter"}, backstory="x",
        talkativeness=0.5, courage=0.5, greed=0.5, attitude_to_master=0.5,
    )
    store.enroll(bot_guid=42, enrolled_at_ms=now_ms, personality_seed=card)
    row = store.get_bot(42)
    assert row.status == "active"
    assert row.enrolled_at == now_ms
    assert row.personality_seed.name == "C"


def test_double_enroll_raises(store, now_ms):
    card = PersonalityCard(
        name="C", race="Dwarf", **{"class": "Hunter"}, backstory="x",
        talkativeness=0.5, courage=0.5, greed=0.5, attitude_to_master=0.5,
    )
    store.enroll(bot_guid=42, enrolled_at_ms=now_ms, personality_seed=card)
    with pytest.raises(ValueError, match="already enrolled"):
        store.enroll(bot_guid=42, enrolled_at_ms=now_ms, personality_seed=card)


def test_list_active_excludes_released(store, now_ms):
    card = PersonalityCard(
        name="C", race="Dwarf", **{"class": "Hunter"}, backstory="x",
        talkativeness=0.5, courage=0.5, greed=0.5, attitude_to_master=0.5,
    )
    store.enroll(bot_guid=1, enrolled_at_ms=now_ms, personality_seed=card)
    store.enroll(bot_guid=2, enrolled_at_ms=now_ms, personality_seed=card)
    store.set_status(2, "released")
    actives = list(store.list_active())
    guids = [b.bot_guid for b in actives]
    assert 1 in guids and 2 not in guids


def test_decisions_recent_ring_buffer_evicts_oldest(store, now_ms):
    card = PersonalityCard(
        name="C", race="Dwarf", **{"class": "Hunter"}, backstory="x",
        talkativeness=0.5, courage=0.5, greed=0.5, attitude_to_master=0.5,
    )
    store.enroll(bot_guid=7, enrolled_at_ms=now_ms, personality_seed=card)
    # write 25 decisions, expect only last 20 retained
    for i in range(25):
        d = Decision(
            kind=DecisionKind.NO_OP, tool=None, args=None,
            confidence=0.0, reasoning=f"r{i}",
        )
        store.append_decision(bot_guid=7, ts_ms=now_ms + i, decision=d)
    recent = store.decisions_recent(bot_guid=7, k=20)
    assert len(recent) == 20
    # most recent first
    assert recent[0].reasoning == "r24"
    assert recent[-1].reasoning == "r5"


def test_decisions_recent_k_limits(store, now_ms):
    card = PersonalityCard(
        name="C", race="Dwarf", **{"class": "Hunter"}, backstory="x",
        talkativeness=0.5, courage=0.5, greed=0.5, attitude_to_master=0.5,
    )
    store.enroll(bot_guid=8, enrolled_at_ms=now_ms, personality_seed=card)
    for i in range(5):
        d = Decision(
            kind=DecisionKind.NO_OP, tool=None, args=None,
            confidence=0.0, reasoning=f"r{i}",
        )
        store.append_decision(bot_guid=8, ts_ms=now_ms + i, decision=d)
    last3 = store.decisions_recent(bot_guid=8, k=3)
    assert [d.reasoning for d in last3] == ["r4", "r3", "r2"]
