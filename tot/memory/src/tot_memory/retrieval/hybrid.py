# SPDX-License-Identifier: GPL-2.0-or-later
"""Hybrid scoring formula — per design subspec §6.2.

The hybrid score blends keyword + dense relevance, attenuates by time-decay,
boosts by the bot's salience judgment for the episode, and adds a flat bonus
when the episode references a query entity:

    score = (α · bm25_norm + β · dense_norm)
            · decay(t)
            · (1 + γ · salience)
          + δ · entity_match

Defaults (§6.2):

    α = 0.45    BM25 weight
    β = 0.45    dense weight
    γ = 0.20    salience-boost multiplier
    δ = 0.15    entity-match bonus

All four weights are runtime-configurable — see the env-var reference in
Appendix B of the design subspec (``MEMORY_SCORE_ALPHA`` etc.).
"""

from dataclasses import dataclass


# Subspec §6.2 / Appendix B defaults — keep in sync with config.py and the
# env-var reference in the design subspec when tuning.
DEFAULT_ALPHA: float = 0.45
DEFAULT_BETA: float = 0.45
DEFAULT_GAMMA: float = 0.20
DEFAULT_DELTA: float = 0.15


@dataclass(frozen=True)
class ComponentScores:
    """Per-episode normalised inputs to the hybrid scorer.

    Attributes:
        bm25_norm:    Max-normalised BM25 score, ``[0, 1]``.
        dense_norm:   Max-normalised dense-cosine score, ``[0, 1]``.
        decay:        Time-decay multiplier from :mod:`tot_memory.retrieval.decay`.
        salience:     ``episodes.salience_score`` in ``[0, 1]``.
        entity_match: Bonus signal: ``1.0`` if the episode references any query
            entity, ``0.0`` otherwise (subspec §6.2 also allows multiple-entity
            stacking up to ``3·δ``; the orchestrator passes a clamped value).
    """

    bm25_norm: float
    dense_norm: float
    decay: float
    salience: float
    entity_match: float


class HybridScorer:
    """Compute the §6.2 hybrid score for a single episode.

    Construct once with the desired weights and reuse across a recall batch;
    the instance is read-only after init.
    """

    def __init__(
        self,
        alpha: float = DEFAULT_ALPHA,
        beta: float = DEFAULT_BETA,
        gamma: float = DEFAULT_GAMMA,
        delta: float = DEFAULT_DELTA,
    ) -> None:
        self.alpha = alpha
        self.beta = beta
        self.gamma = gamma
        self.delta = delta

    def score(self, c: ComponentScores) -> float:
        """Return the hybrid score for the given component breakdown.

        Formula (subspec §6.2):
            (α·bm25 + β·dense) · decay · (1 + γ·salience) + δ·entity_match
        """
        keyword_dense = self.alpha * c.bm25_norm + self.beta * c.dense_norm
        return (
            keyword_dense * c.decay * (1.0 + self.gamma * c.salience)
            + self.delta * c.entity_match
        )
