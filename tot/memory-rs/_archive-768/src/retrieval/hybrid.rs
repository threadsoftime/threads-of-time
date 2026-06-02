// SPDX-License-Identifier: GPL-2.0-or-later
//! Hybrid scoring formula — design subspec §6.2 / §6.6.
//!
//! score = (α·bm25_norm + β·dense_norm) · decay · (1 + γ·salience) + δ·entity_match

/// Default BM25 weight (subspec §6.2).
pub const DEFAULT_ALPHA: f64 = 0.45;
/// Default dense weight (subspec §6.2).
pub const DEFAULT_BETA: f64 = 0.45;
/// Default salience-boost multiplier (subspec §6.2).
pub const DEFAULT_GAMMA: f64 = 0.20;
/// Default entity-match additive bonus (subspec §6.2).
pub const DEFAULT_DELTA: f64 = 0.15;

/// Per-episode normalised inputs to the hybrid scorer.
///
/// All fields are in `[0, 1]` except `decay` which is in `(0, 1]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComponentScores {
    /// Max-normalised BM25 score.
    pub bm25_norm: f64,
    /// Max-normalised dense cosine score.
    pub dense_norm: f64,
    /// Time-decay multiplier from `decay_weight`.
    pub decay: f64,
    /// `episodes.salience_score` in `[0, 1]`.
    pub salience: f64,
    /// `1.0` if episode references a query entity, `0.0` otherwise.
    pub entity_match: f64,
}

/// Computes the §6.2 hybrid score for a single episode.
///
/// Construct once with the desired weights and reuse across a recall batch.
#[derive(Debug, Clone, Copy)]
pub struct HybridScorer {
    pub alpha: f64,
    pub beta: f64,
    pub gamma: f64,
    pub delta: f64,
}

impl HybridScorer {
    pub fn new(alpha: f64, beta: f64, gamma: f64, delta: f64) -> Self {
        Self { alpha, beta, gamma, delta }
    }

    /// `(α·bm25 + β·dense) · decay · (1 + γ·salience) + δ·entity_match`
    pub fn score(&self, c: &ComponentScores) -> f64 {
        let keyword_dense = self.alpha * c.bm25_norm + self.beta * c.dense_norm;
        keyword_dense * c.decay * (1.0 + self.gamma * c.salience)
            + self.delta * c.entity_match
    }
}

impl Default for HybridScorer {
    fn default() -> Self {
        Self::new(DEFAULT_ALPHA, DEFAULT_BETA, DEFAULT_GAMMA, DEFAULT_DELTA)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_constants_match_subspec_6_2() {
        assert_eq!(DEFAULT_ALPHA, 0.45);
        assert_eq!(DEFAULT_BETA,  0.45);
        assert_eq!(DEFAULT_GAMMA, 0.20);
        assert_eq!(DEFAULT_DELTA, 0.15);
    }

    #[test]
    fn default_constructor_uses_subspec_defaults() {
        let s = HybridScorer::default();
        assert_eq!(s.alpha, DEFAULT_ALPHA);
        assert_eq!(s.beta,  DEFAULT_BETA);
        assert_eq!(s.gamma, DEFAULT_GAMMA);
        assert_eq!(s.delta, DEFAULT_DELTA);
    }

    /// Worked formula verification:
    /// (0.45·0.8 + 0.45·0.6) · 0.7 · (1 + 0.20·0.5) + 0.15·1.0
    #[test]
    fn hybrid_formula_matches_subspec_6_2() {
        let alpha = 0.45_f64;
        let beta  = 0.45_f64;
        let gamma = 0.20_f64;
        let delta = 0.15_f64;
        let scorer = HybridScorer::new(alpha, beta, gamma, delta);
        let c = ComponentScores {
            bm25_norm:    0.8,
            dense_norm:   0.6,
            decay:        0.7,
            salience:     0.5,
            entity_match: 1.0,
        };
        let expected = (alpha * 0.8 + beta * 0.6) * 0.7 * (1.0 + gamma * 0.5) + delta * 1.0;
        let got = scorer.score(&c);
        assert!((got - expected).abs() < 1e-9, "expected {expected}, got {got}");
    }

    /// §6.4 worked example Episode 42: ≈ 0.842
    /// bm25=0.595, dense=0.81, decay=0.943, salience=0.8, entity_match=1.0
    #[test]
    fn hybrid_worked_example_episode_42() {
        let scorer = HybridScorer::default();
        let c = ComponentScores {
            bm25_norm:    0.595,
            dense_norm:   0.81,
            decay:        0.943,
            salience:     0.8,
            entity_match: 1.0,
        };
        let score = scorer.score(&c);
        assert!((score - 0.842).abs() < 0.005, "expected ~0.842, got {score}");
    }

    /// §6.4 worked example Episode 8: entity_match=0 → no δ bonus, ≈ 0.625
    /// bm25=0.581, dense=0.73, decay=0.999, salience=0.3, entity_match=0
    #[test]
    fn hybrid_worked_example_episode_8_no_entity_match() {
        let scorer = HybridScorer::default();
        let c = ComponentScores {
            bm25_norm:    0.581,
            dense_norm:   0.73,
            decay:        0.999,
            salience:     0.3,
            entity_match: 0.0,
        };
        let score = scorer.score(&c);
        assert!((score - 0.625).abs() < 0.005, "expected ~0.625, got {score}");
    }

    /// decay=0 → only the additive entity-match bonus survives (δ·entity_match).
    #[test]
    fn hybrid_decay_zero_leaves_only_entity_bonus() {
        let scorer = HybridScorer::default();
        let c = ComponentScores {
            bm25_norm:    1.0,
            dense_norm:   1.0,
            decay:        0.0,
            salience:     1.0,
            entity_match: 1.0,
        };
        let score = scorer.score(&c);
        assert!((score - DEFAULT_DELTA).abs() < 1e-9,
            "expected {}, got {score}", DEFAULT_DELTA);
    }

    /// All-zero inputs → score 0.0.
    #[test]
    fn hybrid_all_zero_components_scores_zero() {
        let scorer = HybridScorer::default();
        let c = ComponentScores {
            bm25_norm: 0.0, dense_norm: 0.0,
            decay: 0.0, salience: 0.0, entity_match: 0.0,
        };
        assert_eq!(scorer.score(&c), 0.0);
    }

    /// Higher salience strictly increases the score (holding other inputs equal).
    #[test]
    fn hybrid_salience_boost_increases_score() {
        let scorer = HybridScorer::default();
        let base = ComponentScores {
            bm25_norm: 0.5, dense_norm: 0.5, decay: 0.5, salience: 0.0, entity_match: 0.0,
        };
        let boosted = ComponentScores { salience: 1.0, ..base };
        assert!(scorer.score(&boosted) > scorer.score(&base));
    }

    /// entity_match=1.0 adds exactly δ over entity_match=0.0 (additive, not multiplicative).
    #[test]
    fn hybrid_entity_match_is_additive() {
        let scorer = HybridScorer::default();
        let no_entity = ComponentScores {
            bm25_norm: 0.5, dense_norm: 0.5, decay: 0.8, salience: 0.5, entity_match: 0.0,
        };
        let with_entity = ComponentScores { entity_match: 1.0, ..no_entity };
        let diff = scorer.score(&with_entity) - scorer.score(&no_entity);
        assert!((diff - DEFAULT_DELTA).abs() < 1e-9,
            "expected δ={}, got diff={diff}", DEFAULT_DELTA);
    }
}
