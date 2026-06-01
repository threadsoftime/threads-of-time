// SPDX-License-Identifier: GPL-2.0-or-later
//! Hybrid recall orchestrator — design subspec §6.1 / §6.4.
//!
//! `max_normalize` is ported here in Task 2.3.
//! The full `recall` orchestrator is added in Task 2.7.

use std::collections::HashMap;

/// Max-normalise a score map to `[0, 1]`.
///
/// * Empty input → empty map.
/// * `max ≤ 0` → all 0.0 (guards against all-zero and negative-only maps).
/// * Negative individual values are clipped to 0 before dividing.
pub fn max_normalize(scores: &HashMap<i64, f64>) -> HashMap<i64, f64> {
    if scores.is_empty() {
        return HashMap::new();
    }
    let max = scores.values().cloned().fold(f64::NEG_INFINITY, f64::max);
    if max <= 0.0 {
        return scores.keys().map(|&k| (k, 0.0)).collect();
    }
    scores
        .iter()
        .map(|(&k, &v)| (k, (v.max(0.0)) / max))
        .collect()
}

/// A single hybrid-scored episode with its component breakdown.
///
/// The `recall` function (Task 2.7) populates these fields.
#[derive(Debug, Clone, PartialEq)]
pub struct RecallResult {
    pub episode_id:   i64,
    pub score:        f64,
    pub bm25_norm:    f64,
    pub dense_norm:   f64,
    pub decay:        f64,
    pub salience:     f64,
    pub entity_match: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(i64, f64)]) -> HashMap<i64, f64> {
        pairs.iter().cloned().collect()
    }

    /// Empty input → empty output.
    #[test]
    fn max_normalize_empty_returns_empty() {
        assert!(max_normalize(&HashMap::new()).is_empty());
    }

    /// All-zero map → all 0.0 (max ≤ 0 branch).
    #[test]
    fn max_normalize_all_zero_returns_all_zero() {
        let input = map(&[(1, 0.0), (2, 0.0), (3, 0.0)]);
        let out = max_normalize(&input);
        for &v in out.values() {
            assert_eq!(v, 0.0);
        }
        assert_eq!(out.len(), 3);
    }

    /// All-negative map → all 0.0 (max ≤ 0 + negative-clip).
    #[test]
    fn max_normalize_all_negative_returns_all_zero() {
        let input = map(&[(1, -1.0), (2, -0.5), (3, -2.0)]);
        let out = max_normalize(&input);
        for &v in out.values() {
            assert_eq!(v, 0.0, "expected 0.0 for all-negative input");
        }
    }

    /// Normal case: max gets 1.0, others scale proportionally, negatives clip to 0.
    #[test]
    fn max_normalize_normal_case() {
        let input = map(&[(1, 2.0), (2, 1.0), (3, 0.0)]);
        let out = max_normalize(&input);
        assert!((out[&1] - 1.0).abs() < 1e-12, "max key should be 1.0");
        assert!((out[&2] - 0.5).abs() < 1e-12, "half-max key should be 0.5");
        assert_eq!(out[&3], 0.0);
    }

    /// Mixed negative and positive: negative values clip to 0, positive normalised.
    #[test]
    fn max_normalize_negative_clip_with_positive_max() {
        let input = map(&[(1, 4.0), (2, -1.0), (3, 2.0)]);
        let out = max_normalize(&input);
        assert!((out[&1] - 1.0).abs() < 1e-12, "key 1 = max → 1.0");
        assert_eq!(out[&2], 0.0, "negative value clipped to 0");
        assert!((out[&3] - 0.5).abs() < 1e-12, "key 3 = 2/4 = 0.5");
    }

    /// Single-entry map → that entry normalises to 1.0.
    #[test]
    fn max_normalize_single_positive_entry() {
        let input = map(&[(42, 3.7)]);
        let out = max_normalize(&input);
        assert!((out[&42] - 1.0).abs() < 1e-12);
    }

    /// Output values are all in [0, 1].
    #[test]
    fn max_normalize_output_in_unit_range() {
        let input = map(&[(1, 10.0), (2, 5.0), (3, -3.0), (4, 0.0)]);
        let out = max_normalize(&input);
        for &v in out.values() {
            assert!(v >= 0.0 && v <= 1.0, "out of range: {v}");
        }
    }
}
