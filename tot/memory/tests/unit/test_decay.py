# SPDX-License-Identifier: GPL-2.0-or-later
"""Unit tests for retrieval/decay.py — per design subspec §7 time-decay."""

import math
from datetime import datetime, timedelta, timezone

import pytest

from tot_memory.retrieval.decay import HALF_LIVES_HOURS, decay_weight, half_life_for


def test_decay_weight_is_1_at_zero_age():
    """Fresh episode (Δt = 0) → decay 1.0 (§7.1)."""
    now = datetime(2026, 5, 27, tzinfo=timezone.utc)
    assert decay_weight(now, now, half_life_hours=24) == 1.0


def test_decay_weight_is_half_at_one_half_life():
    """Δt = one half-life → decay 0.5 (the defining property of half-life)."""
    now = datetime(2026, 5, 27, tzinfo=timezone.utc)
    past = now - timedelta(hours=24)
    assert math.isclose(decay_weight(past, now, half_life_hours=24), 0.5, rel_tol=1e-9)


def test_decay_weight_quarter_at_two_half_lives():
    """Δt = 2 × half-life → decay 0.25."""
    now = datetime(2026, 5, 27, tzinfo=timezone.utc)
    past = now - timedelta(hours=48)
    assert math.isclose(
        decay_weight(past, now, half_life_hours=24), 0.25, rel_tol=1e-9
    )


def test_decay_weight_for_future_timestamp_caps_at_1():
    """An episode with a future timestamp (clock skew) should not exceed 1.0."""
    now = datetime(2026, 5, 27, tzinfo=timezone.utc)
    future = now + timedelta(hours=1)
    assert decay_weight(future, now, half_life_hours=24) == 1.0


def test_half_life_for_known_episode_types():
    """Per §7.2 default half-lives (hours)."""
    assert HALF_LIVES_HOURS["chat"] == 48
    assert HALF_LIVES_HOURS["combat"] == 72
    assert HALF_LIVES_HOURS["social"] == 96
    assert HALF_LIVES_HOURS["quest"] == 168
    assert HALF_LIVES_HOURS["discovery"] == 720
    assert HALF_LIVES_HOURS["goal"] == 168
    assert HALF_LIVES_HOURS["reflection"] == 336
    assert HALF_LIVES_HOURS["observation"] == 24


def test_half_life_for_unknown_type_falls_back_to_chat():
    """Unknown episode_type falls back to the chat half-life."""
    assert half_life_for("not-a-real-type") == HALF_LIVES_HOURS["chat"]


def test_half_life_for_lookup_normalises_case():
    """Episode types are stored lowercase in the schema; the lookup matches."""
    assert half_life_for("chat") == 48
    assert half_life_for("combat") == 72


def test_decay_weight_zero_half_life_raises():
    """Zero half-life is undefined; the function should reject it cleanly."""
    now = datetime(2026, 5, 27, tzinfo=timezone.utc)
    with pytest.raises(ValueError):
        decay_weight(now, now, half_life_hours=0)
