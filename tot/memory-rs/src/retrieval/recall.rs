//! `score_memory` — weighted recall scoring primitive.
//!
//! Ports `recall.py::score_memory`:
//! ```python
//! def score_memory(m, query_emb, now, w, recency_basis="created"):
//!     if m.embedding is None:
//!         relevance = 0.0
//!     else:
//!         relevance = cosine(m.embedding, query_emb)
//!     ts = m.created_ts  # v0.2 default (recency_basis="created")
//!     age = max(0, now - ts)
//!     recency = math.exp(-age / w.tau_seconds)
//!     importance = max(0.0, min(1.0, m.salience))
//!     return w.w_rel * relevance + w.w_rec * recency + w.w_imp * importance
//! ```
//!
//! Formula: `w_rel·cosine(emb, q) + w_rec·exp(−age/τ) + w_imp·clip(salience, 0, 1)`
//!
//! NULL embedding → relevance = 0.0 (not an error).
//! Negative age is clamped to 0 (same as Python's `max(0, now - ts)`).

use crate::config::ScoringWeights;
use crate::retrieval::cosine::cosine;

/// Score a single memory episode against `q` using the weighted formula.
///
/// Arguments:
/// * `emb`        — the memory's embedding (`None` if not yet embedded).
/// * `q`          — the query embedding.
/// * `salience`   — the memory's salience (will be clamped to `[0.0, 1.0]`).
/// * `created_ts` — Unix timestamp of memory creation (seconds).
/// * `now`        — current Unix timestamp (seconds).
/// * `w`          — scoring weights + tau_seconds.
///
/// Returns a non-negative f64 (higher = more relevant).
pub fn score_memory(
    emb: Option<&[f32]>,
    q: &[f32],
    salience: f64,
    created_ts: i64,
    now: i64,
    w: &ScoringWeights,
) -> f64 {
    let relevance = match emb {
        Some(v) => cosine(v, q),
        None => 0.0,
    };
    let age = (now - created_ts).max(0) as f64;
    let recency = (-age / w.tau_seconds as f64).exp();
    let importance = salience.max(0.0).min(1.0);
    w.w_rel * relevance + w.w_rec * recency + w.w_imp * importance
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ScoringWeights;

    fn weights() -> ScoringWeights {
        ScoringWeights {
            w_rel: 0.5,
            w_rec: 0.2,
            w_imp: 0.3,
            tau_seconds: 604800,
        }
    }

    /// age=0, cos=1, sal=1 → 0.5*1 + 0.2*1 + 0.3*1 = 1.0 exactly.
    #[test]
    fn all_max_returns_one() {
        let w = weights();
        let now = 1_000_000i64;
        let q = vec![1.0f32, 0.0, 0.0];
        let emb = vec![1.0f32, 0.0, 0.0];
        let score = score_memory(Some(&emb), &q, 1.0, now, now, &w);
        assert!((score - 1.0).abs() < 1e-12, "expected 1.0, got {score}");
    }

    /// age=τ → recency = exp(-1).
    /// score = 0.5*1 + 0.2*exp(-1) + 0.3*1
    #[test]
    fn age_one_tau_recency_is_exp_minus_one() {
        let w = weights();
        let tau = w.tau_seconds as i64;
        let now = 1_000_000i64;
        let created_ts = now - tau;
        let q = vec![1.0f32, 0.0, 0.0];
        let emb = vec![1.0f32, 0.0, 0.0];
        let score = score_memory(Some(&emb), &q, 1.0, created_ts, now, &w);
        let expected = 0.5 * 1.0 + 0.2 * (-1.0f64).exp() + 0.3 * 1.0;
        assert!((score - expected).abs() < 1e-12, "expected {expected}, got {score}");
    }

    /// Isolated recency term: age=τ, cos=0 (orthogonal), sal=0.
    /// → recency term alone = 0.2 * exp(-1)
    #[test]
    fn isolated_recency_term_at_tau() {
        let w = weights();
        let tau = w.tau_seconds as i64;
        let now = 0i64;
        let created_ts = now - tau;
        let q = vec![1.0f32, 0.0];
        let emb = vec![0.0f32, 1.0]; // orthogonal → cos=0
        let score = score_memory(Some(&emb), &q, 0.0, created_ts, now, &w);
        let expected = 0.2 * (-1.0f64).exp();
        assert!((score - expected).abs() < 1e-12, "expected {expected}, got {score}");
    }

    /// NULL embedding → relevance = 0.0, not an error.
    #[test]
    fn null_embedding_zero_relevance() {
        let w = weights();
        let now = 1_000_000i64;
        let q = vec![1.0f32, 0.0, 0.0];
        // age=0, sal=1, no emb → 0.5*0 + 0.2*1 + 0.3*1 = 0.5
        let score = score_memory(None, &q, 1.0, now, now, &w);
        let expected = 0.0 * w.w_rel + 1.0 * w.w_rec + 1.0 * w.w_imp;
        assert!((score - expected).abs() < 1e-12, "expected {expected}, got {score}");
    }

    /// age < 0 (future created_ts) is clamped to 0.
    #[test]
    fn future_ts_clamps_age_to_zero() {
        let w = weights();
        let now = 1_000_000i64;
        let future_ts = now + 9999;
        let q = vec![1.0f32, 0.0, 0.0];
        let emb = vec![1.0f32, 0.0, 0.0];
        let score = score_memory(Some(&emb), &q, 1.0, future_ts, now, &w);
        assert!((score - 1.0).abs() < 1e-12, "future ts: expected 1.0, got {score}");
    }

    /// salience > 1.0 is clamped to 1.0.
    #[test]
    fn salience_above_one_clamped() {
        let w = weights();
        let now = 1_000_000i64;
        let q = vec![1.0f32, 0.0];
        let emb = vec![1.0f32, 0.0];
        let score_high = score_memory(Some(&emb), &q, 2.5, now, now, &w);
        let score_one = score_memory(Some(&emb), &q, 1.0, now, now, &w);
        assert!((score_high - score_one).abs() < 1e-12,
            "salience 2.5 should clamp to 1.0; high={score_high} one={score_one}");
    }

    /// salience < 0.0 is clamped to 0.0.
    #[test]
    fn salience_below_zero_clamped() {
        let w = weights();
        let now = 1_000_000i64;
        let q = vec![1.0f32, 0.0];
        let emb = vec![1.0f32, 0.0];
        let score_neg = score_memory(Some(&emb), &q, -0.5, now, now, &w);
        let score_zero = score_memory(Some(&emb), &q, 0.0, now, now, &w);
        assert!((score_neg - score_zero).abs() < 1e-12,
            "salience -0.5 should clamp to 0; neg={score_neg} zero={score_zero}");
    }
}
