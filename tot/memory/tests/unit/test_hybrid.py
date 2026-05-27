# SPDX-License-Identifier: GPL-2.0-or-later
"""Unit tests for retrieval/hybrid.py — per design subspec §6.2."""

import math

import pytest

from tot_memory.retrieval.hybrid import (
    DEFAULT_ALPHA,
    DEFAULT_BETA,
    DEFAULT_DELTA,
    DEFAULT_GAMMA,
    ComponentScores,
    HybridScorer,
)


def test_default_hyperparameters_match_subspec_6_2():
    """§6.2: α=0.45, β=0.45, γ=0.20, δ=0.15."""
    assert DEFAULT_ALPHA == 0.45
    assert DEFAULT_BETA == 0.45
    assert DEFAULT_GAMMA == 0.20
    assert DEFAULT_DELTA == 0.15


def test_default_constructor_uses_subspec_defaults():
    """HybridScorer() with no args uses the §6.2 defaults."""
    s = HybridScorer()
    assert s.alpha == DEFAULT_ALPHA
    assert s.beta == DEFAULT_BETA
    assert s.gamma == DEFAULT_GAMMA
    assert s.delta == DEFAULT_DELTA


def test_hybrid_formula_matches_subspec_6_2():
    """score = (α·bm25 + β·dense) · decay · (1 + γ·salience) + δ·entity_match."""
    alpha, beta, gamma, delta = 0.45, 0.45, 0.20, 0.15
    scorer = HybridScorer(alpha=alpha, beta=beta, gamma=gamma, delta=delta)
    components = ComponentScores(
        bm25_norm=0.8,
        dense_norm=0.6,
        decay=0.7,
        salience=0.5,
        entity_match=1.0,
    )
    expected = (
        (alpha * 0.8 + beta * 0.6) * 0.7 * (1.0 + gamma * 0.5) + delta * 1.0
    )
    assert math.isclose(scorer.score(components), expected, rel_tol=1e-9)


def test_hybrid_worked_example_episode_42_from_subspec_6_4():
    """§6.4 worked example: Episode 42 should score ≈ 0.842.

    bm25_norm=0.595, dense=0.81, decay=0.943, salience=0.8, entity_match=1.0
        (0.45·0.595 + 0.45·0.81) · 0.943 · (1 + 0.20·0.8) + 0.15·1
    """
    scorer = HybridScorer()  # use subspec defaults
    score = scorer.score(
        ComponentScores(
            bm25_norm=0.595,
            dense_norm=0.81,
            decay=0.943,
            salience=0.8,
            entity_match=1.0,
        )
    )
    # Subspec arithmetic rounds to 0.842; allow generous tolerance for the
    # three-decimal intermediate values in §6.4
    assert math.isclose(score, 0.842, abs_tol=0.005)


def test_hybrid_worked_example_episode_8_no_entity_match():
    """§6.4 Episode 8: entity_match=0 → no δ bonus.

    bm25_norm=0.581, dense=0.73, decay=0.999, salience=0.3, entity_match=0
        (0.45·0.581 + 0.45·0.73) · 0.999 · (1 + 0.20·0.3)
    Worked example: ≈ 0.625
    """
    scorer = HybridScorer()
    score = scorer.score(
        ComponentScores(
            bm25_norm=0.581,
            dense_norm=0.73,
            decay=0.999,
            salience=0.3,
            entity_match=0.0,
        )
    )
    assert math.isclose(score, 0.625, abs_tol=0.005)


def test_hybrid_decay_dominates_when_zero():
    """An episode with decay=0 (infinitely old, hypothetically) scores only
    the entity-match bonus δ·entity_match."""
    scorer = HybridScorer()
    score = scorer.score(
        ComponentScores(
            bm25_norm=1.0,
            dense_norm=1.0,
            decay=0.0,
            salience=1.0,
            entity_match=1.0,
        )
    )
    # Only the additive entity-match term survives
    assert math.isclose(score, DEFAULT_DELTA, rel_tol=1e-9)


def test_hybrid_zero_all_components_scores_zero():
    """All zero inputs → score 0."""
    scorer = HybridScorer()
    assert scorer.score(
        ComponentScores(0.0, 0.0, 0.0, 0.0, 0.0)
    ) == 0.0


def test_hybrid_salience_boost_increases_score():
    """Higher salience strictly increases the score (holding other inputs equal)."""
    scorer = HybridScorer()
    base = ComponentScores(0.5, 0.5, 0.5, 0.0, 0.0)
    boosted = ComponentScores(0.5, 0.5, 0.5, 1.0, 0.0)
    assert scorer.score(boosted) > scorer.score(base)


def test_component_scores_is_frozen_dataclass():
    """ComponentScores is immutable — accidental mutation should raise."""
    c = ComponentScores(0.1, 0.2, 0.3, 0.4, 0.5)
    with pytest.raises((AttributeError, Exception)):
        c.bm25_norm = 99.0  # type: ignore[misc]
