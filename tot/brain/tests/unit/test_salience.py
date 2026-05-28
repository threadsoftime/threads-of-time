# SPDX-License-Identifier: GPL-2.0-or-later
"""Unit tests for brain-side salience scorer (Plan 2 Task 34).

This scorer is a HINT passed to memory.write as `salience_hint` (per design
subspec §8.1). The server-side scorer in tot/memory is authoritative for
retrieval-time scoring; this just decides whether the brain bothers writing
the episode at all (`score >= threshold`).

The rules below follow design subspec §8 tiers + adjustments — verified
against the subspec, not invented.
"""
from __future__ import annotations

import pytest

from brain_sidecar.salience import (
    SalienceScorer,
    SALIENCE_FILLER,
    SALIENCE_IMPORTANT,
    SALIENCE_NORMAL,
    SALIENCE_NOTABLE,
    SALIENCE_PIVOTAL,
)


# ----------------------------------------------------------------------
# Class shape + defaults
# ----------------------------------------------------------------------


def test_threshold_default_is_zero_point_three():
    """Spec template (Plan 2 Task 34): default threshold 0.3."""
    scorer = SalienceScorer()
    assert scorer.threshold == 0.3


def test_threshold_overridable():
    scorer = SalienceScorer(threshold=0.5)
    assert scorer.threshold == 0.5


def test_score_returns_float_between_zero_and_one():
    scorer = SalienceScorer()
    s = scorer.score(
        perception={"episode_type": "chat"},
        decision={"kind": "no_op"},
        action_result={"outcome": "ok"},
    )
    assert isinstance(s, float)
    assert 0.0 <= s <= 1.0


# ----------------------------------------------------------------------
# Per-type defaults (subspec §8.2)
# ----------------------------------------------------------------------


@pytest.mark.parametrize(
    "episode_type,expected_default",
    [
        ("chat",        SALIENCE_NORMAL),
        ("combat",      SALIENCE_NORMAL),
        ("social",      SALIENCE_NOTABLE),
        ("quest",       SALIENCE_IMPORTANT),
        ("discovery",   SALIENCE_NOTABLE),
        ("goal",        SALIENCE_IMPORTANT),
        ("reflection",  SALIENCE_IMPORTANT),
        ("observation", SALIENCE_FILLER),
    ],
)
def test_per_episode_type_default_salience(episode_type, expected_default):
    scorer = SalienceScorer()
    s = scorer.score(
        perception={"episode_type": episode_type},
        decision={"kind": "no_op"},
        action_result={},  # no special outcome → use the type default
    )
    assert s == pytest.approx(expected_default)


def test_unknown_episode_type_falls_back_to_normal():
    scorer = SalienceScorer()
    s = scorer.score(
        perception={"episode_type": "nonsense_type"},
        decision={"kind": "no_op"},
        action_result={},
    )
    assert s == pytest.approx(SALIENCE_NORMAL)


def test_missing_episode_type_falls_back_to_normal():
    scorer = SalienceScorer()
    s = scorer.score(
        perception={},
        decision={"kind": "no_op"},
        action_result={},
    )
    assert s == pytest.approx(SALIENCE_NORMAL)


# ----------------------------------------------------------------------
# Combat adjustments (subspec §8.3)
# ----------------------------------------------------------------------


def test_combat_kill_promoted_to_important():
    scorer = SalienceScorer()
    s = scorer.score(
        perception={"episode_type": "combat"},
        decision={"kind": "action", "tool": "bot.attack"},
        action_result={"outcome": "kill"},
    )
    assert s == pytest.approx(SALIENCE_IMPORTANT)


def test_combat_near_death_promoted_to_notable():
    scorer = SalienceScorer()
    s = scorer.score(
        perception={"episode_type": "combat"},
        decision={"kind": "no_op"},
        action_result={"outcome": "near_death"},
    )
    assert s == pytest.approx(SALIENCE_NOTABLE)


def test_combat_wipe_set_to_important():
    scorer = SalienceScorer()
    s = scorer.score(
        perception={"episode_type": "combat"},
        decision={"kind": "no_op"},
        action_result={"outcome": "wipe"},
    )
    assert s == pytest.approx(SALIENCE_IMPORTANT)


# ----------------------------------------------------------------------
# Quest adjustments (subspec §8.3)
# ----------------------------------------------------------------------


def test_quest_complete_promoted_to_important():
    scorer = SalienceScorer()
    s = scorer.score(
        perception={"episode_type": "quest"},
        decision={"kind": "no_op"},
        action_result={"event": "complete"},
    )
    assert s == pytest.approx(SALIENCE_IMPORTANT)


def test_quest_fail_promoted_to_notable_or_higher():
    """Quest fail >= NOTABLE; the type default (IMPORTANT) already exceeds NOTABLE."""
    scorer = SalienceScorer()
    s = scorer.score(
        perception={"episode_type": "quest"},
        decision={"kind": "no_op"},
        action_result={"event": "fail"},
    )
    assert s >= SALIENCE_NOTABLE


# ----------------------------------------------------------------------
# Goal adjustments (subspec §8.3)
# ----------------------------------------------------------------------


@pytest.mark.parametrize("event", ["completed", "achieved"])
def test_goal_completed_set_to_pivotal(event):
    scorer = SalienceScorer()
    s = scorer.score(
        perception={"episode_type": "goal"},
        decision={"kind": "no_op"},
        action_result={"event": event},
    )
    assert s == pytest.approx(SALIENCE_PIVOTAL)


# ----------------------------------------------------------------------
# Chat adjustments (subspec §8.3)
# ----------------------------------------------------------------------


def test_chat_from_player_promoted_to_notable():
    scorer = SalienceScorer()
    s = scorer.score(
        perception={"episode_type": "chat", "source": "player"},
        decision={"kind": "no_op"},
        action_result={},
    )
    assert s == pytest.approx(SALIENCE_NOTABLE)


def test_chat_from_npc_stays_at_normal():
    scorer = SalienceScorer()
    s = scorer.score(
        perception={"episode_type": "chat", "source": "npc"},
        decision={"kind": "no_op"},
        action_result={},
    )
    assert s == pytest.approx(SALIENCE_NORMAL)


# ----------------------------------------------------------------------
# Brain override hint (subspec §8.3 / §10.1)
# ----------------------------------------------------------------------


def test_salience_hint_overrides_rule_score_when_in_range():
    """salience_hint in [0,1] is returned verbatim (rounded to 3 decimals)."""
    scorer = SalienceScorer()
    s = scorer.score(
        perception={"episode_type": "chat", "salience_hint": 0.95},
        decision={"kind": "no_op"},
        action_result={},
    )
    assert s == pytest.approx(0.95)


def test_salience_hint_out_of_range_ignored():
    """A bogus hint (e.g., 1.5 or -0.1) is ignored; the rule score wins."""
    scorer = SalienceScorer()
    s_high = scorer.score(
        perception={"episode_type": "chat", "salience_hint": 1.5},
        decision={"kind": "no_op"},
        action_result={},
    )
    s_low = scorer.score(
        perception={"episode_type": "chat", "salience_hint": -0.1},
        decision={"kind": "no_op"},
        action_result={},
    )
    assert s_high == pytest.approx(SALIENCE_NORMAL)
    assert s_low == pytest.approx(SALIENCE_NORMAL)


# ----------------------------------------------------------------------
# Output clamping / rounding
# ----------------------------------------------------------------------


def test_score_clamped_to_one_max():
    scorer = SalienceScorer()
    # Goal "completed" sets PIVOTAL=1.0 — already at the ceiling, just verify
    # nothing pushes past 1.0.
    s = scorer.score(
        perception={"episode_type": "goal"},
        decision={"kind": "no_op"},
        action_result={"event": "completed"},
    )
    assert s <= 1.0


def test_score_clamped_to_zero_min():
    """A subzero salience_hint is treated as out-of-range (ignored), so this
    just sanity-checks the floor for any future negative paths."""
    scorer = SalienceScorer()
    s = scorer.score(
        perception={"episode_type": "observation"},
        decision={"kind": "no_op"},
        action_result={},
    )
    assert s >= 0.0


# ----------------------------------------------------------------------
# Threshold gate behavior (the actual use site in the decision loop)
# ----------------------------------------------------------------------


def test_observation_does_not_clear_default_threshold():
    """Default observation salience = FILLER (0.2) < default threshold 0.3 →
    not written. This is the gate behavior that prevents log spam."""
    scorer = SalienceScorer()
    s = scorer.score(
        perception={"episode_type": "observation"},
        decision={"kind": "no_op"},
        action_result={},
    )
    assert s < scorer.threshold


def test_combat_kill_clears_default_threshold():
    """combat outcome=kill → IMPORTANT (0.8) >> 0.3 → written."""
    scorer = SalienceScorer()
    s = scorer.score(
        perception={"episode_type": "combat"},
        decision={"kind": "action"},
        action_result={"outcome": "kill"},
    )
    assert s >= scorer.threshold
