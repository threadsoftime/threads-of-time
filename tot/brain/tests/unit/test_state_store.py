# SPDX-License-Identifier: GPL-2.0-or-later
"""Tests for StateStore subset-gating additions (Tasks 8-11)."""
from __future__ import annotations

import pytest

from brain_sidecar.models import PersonalityCard
from brain_sidecar.state import StateStore


def _make_personality() -> PersonalityCard:
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


# ---------------------------------------------------------------------------
# Task 8 — migration 0004 idempotency
# ---------------------------------------------------------------------------

def test_migration_0004_idempotent(tmp_path):
    db = tmp_path / "test.sqlite"
    store = StateStore(str(db))
    store.migrate()
    store.migrate()  # second run must not raise or duplicate columns
    cols = [r[1] for r in store._conn.execute("PRAGMA table_info(living_bots)").fetchall()]
    for c in ("tier", "out_of_range_ticks", "in_range_ticks", "last_recompute_at", "pinned"):
        assert c in cols, f"column {c} missing after migration"
    row = store._conn.execute("SELECT MAX(version) FROM schema_version").fetchone()
    assert row[0] >= 4
    store.close()


# ---------------------------------------------------------------------------
# Task 9 — set_tier / get_tier
# ---------------------------------------------------------------------------

def test_set_tier_and_get_tier_round_trip(tmp_path):
    db = tmp_path / "test.sqlite"
    store = StateStore(str(db))
    store.migrate()
    store.enroll(bot_guid=1, enrolled_at_ms=0,
                 personality_seed=_make_personality())
    assert store.get_tier(1) == "full"
    store.set_tier(1, "reduced")
    assert store.get_tier(1) == "reduced"
    store.set_tier(1, "full")
    assert store.get_tier(1) == "full"
    store.close()


def test_get_tier_unknown_bot_returns_full_default(tmp_path):
    db = tmp_path / "test.sqlite"
    store = StateStore(str(db))
    store.migrate()
    assert store.get_tier(9999) == "full"
    store.close()


def test_set_tier_rejects_invalid_value(tmp_path):
    db = tmp_path / "test.sqlite"
    store = StateStore(str(db))
    store.migrate()
    store.enroll(bot_guid=1, enrolled_at_ms=0,
                 personality_seed=_make_personality())
    with pytest.raises(ValueError, match="invalid tier"):
        store.set_tier(1, "background")
    store.close()


# ---------------------------------------------------------------------------
# Task 10 — bump_hysteresis / get_hysteresis
# ---------------------------------------------------------------------------

def test_bump_hysteresis_in_range_increments_in_and_resets_out(tmp_path):
    db = tmp_path / "test.sqlite"
    store = StateStore(str(db))
    store.migrate()
    store.enroll(bot_guid=1, enrolled_at_ms=0,
                 personality_seed=_make_personality())
    assert store.get_hysteresis(1) == (0, 0)
    assert store.bump_hysteresis(1, in_range=True) == (1, 0)
    assert store.bump_hysteresis(1, in_range=True) == (2, 0)
    assert store.bump_hysteresis(1, in_range=False) == (0, 1)
    assert store.bump_hysteresis(1, in_range=False) == (0, 2)
    assert store.bump_hysteresis(1, in_range=True) == (1, 0)
    store.close()


def test_bump_hysteresis_unknown_bot_no_op(tmp_path):
    db = tmp_path / "test.sqlite"
    store = StateStore(str(db))
    store.migrate()
    assert store.bump_hysteresis(9999, in_range=True) == (0, 0)
    store.close()


# ---------------------------------------------------------------------------
# Task 11 — set_pin / list_pinned
# ---------------------------------------------------------------------------

def test_set_pin_and_list_pinned(tmp_path):
    db = tmp_path / "test.sqlite"
    store = StateStore(str(db))
    store.migrate()
    for g in (1, 2, 3):
        store.enroll(bot_guid=g, enrolled_at_ms=0,
                     personality_seed=_make_personality())
    assert store.list_pinned() == []
    store.set_pin(1, True)
    store.set_pin(3, True)
    assert sorted(store.list_pinned()) == [1, 3]
    store.set_pin(1, False)
    assert store.list_pinned() == [3]
    store.close()


# ---------------------------------------------------------------------------
# B4 — reactivate (flip status='active' + reset hysteresis atomically)
# ---------------------------------------------------------------------------

def test_reactivate_released_bot_resets_status_and_hysteresis(tmp_path):
    db = tmp_path / "test.sqlite"
    store = StateStore(str(db))
    store.migrate()
    store.enroll(bot_guid=1, enrolled_at_ms=0, personality_seed=_make_personality())
    store.set_status(1, "released")
    store.bump_hysteresis(1, in_range=False)
    store.bump_hysteresis(1, in_range=False)
    assert store.get_hysteresis(1) == (0, 2)
    store.reactivate(1)
    bot = store.get_bot(1)
    assert bot is not None and bot.status == "active"
    assert store.get_hysteresis(1) == (0, 0)
    store.close()


def test_reactivate_already_active_is_idempotent(tmp_path):
    db = tmp_path / "test.sqlite"
    store = StateStore(str(db))
    store.migrate()
    store.enroll(bot_guid=1, enrolled_at_ms=0, personality_seed=_make_personality())
    store.bump_hysteresis(1, in_range=True)
    store.reactivate(1)
    bot = store.get_bot(1)
    assert bot is not None and bot.status == "active"
    assert store.get_hysteresis(1) == (0, 0)
    store.close()


def test_reactivate_unknown_bot_no_op(tmp_path):
    db = tmp_path / "test.sqlite"
    store = StateStore(str(db))
    store.migrate()
    store.reactivate(9999)
    assert store.get_bot(9999) is None
    store.close()
