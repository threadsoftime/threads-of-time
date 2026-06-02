//! Reciprocal Rank Fusion — ports `search.py::rrf_fuse`.
//!
//! ```python
//! def rrf_fuse(ranked_lists, k=60):
//!     scores: dict[str, float] = {}
//!     for lst in ranked_lists:
//!         for rank, doc in enumerate(lst):
//!             scores[doc] = scores.get(doc, 0.0) + 1.0 / (k + rank)
//!     return scores
//! ```
//!
//! Parity notes:
//! - rank is 0-indexed (first element of each list → rank=0 → score += 1/(k+0)).
//! - Insertion-stable output: documents appear in the order they were FIRST
//!   seen across all ranked lists, NOT sorted by score (the Python dict preserves
//!   insertion order in CPython 3.7+).
//! - The caller sorts by score; this function just accumulates and preserves order.
//! - Vec-based index-ordered map (no HashMap) — insertion order is deterministic.

/// Reciprocal Rank Fusion across multiple ranked document lists.
///
/// For each ranked list, for each 0-indexed `(rank, doc_id)`:
///   `score[doc_id] += 1.0 / (k + rank)`
///
/// Returns `(doc_id, fused_score)` pairs in **insertion order** (the order in
/// which each document ID was first encountered across the lists), matching
/// CPython's `dict` insertion-order semantics.
///
/// Higher scores indicate stronger aggregate rank.
pub fn rrf_fuse(lists: &[Vec<String>], k: i32) -> Vec<(String, f64)> {
    // Parallel arrays: doc_id order and accumulated scores.
    // Using Vec (not HashMap) guarantees insertion-stable ordering.
    let mut keys: Vec<String> = Vec::new();
    let mut scores: Vec<f64> = Vec::new();

    for lst in lists {
        for (rank, doc) in lst.iter().enumerate() {
            let contribution = 1.0 / (k as f64 + rank as f64);
            // Find existing entry or append a new one.
            if let Some(idx) = keys.iter().position(|k| k == doc) {
                scores[idx] += contribution;
            } else {
                keys.push(doc.clone());
                scores.push(contribution);
            }
        }
    }

    keys.into_iter().zip(scores).collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Canonical example from the spec:
    /// lists = [["a","b"], ["b","c"]], k=60
    /// a: 1/60
    /// b: 1/61 + 1/60   (appears in both lists)
    /// c: 1/61
    /// b must be the highest score.
    #[test]
    fn canonical_two_lists() {
        let lists = vec![
            vec!["a".to_string(), "b".to_string()],
            vec!["b".to_string(), "c".to_string()],
        ];
        let result = rrf_fuse(&lists, 60);

        // Collect into a map for assertions.
        let map: std::collections::HashMap<&str, f64> =
            result.iter().map(|(k, v)| (k.as_str(), *v)).collect();

        let a = map["a"];
        let b = map["b"];
        let c = map["c"];

        assert!((a - 1.0 / 60.0).abs() < 1e-15, "a: expected 1/60, got {a}");
        assert!((b - (1.0 / 61.0 + 1.0 / 60.0)).abs() < 1e-15,
            "b: expected 1/61+1/60, got {b}");
        assert!((c - 1.0 / 61.0).abs() < 1e-15, "c: expected 1/61, got {c}");
        assert!(b > a, "b should be highest");
        assert!(b > c, "b should beat c");
    }

    /// Insertion-stable ordering: documents appear in first-encountered order.
    #[test]
    fn insertion_stable_order() {
        let lists = vec![
            vec!["a".to_string(), "b".to_string()],
            vec!["b".to_string(), "c".to_string()],
        ];
        let result = rrf_fuse(&lists, 60);
        // a was seen first (list 0 pos 0), then b (list 0 pos 1), then c (list 1 pos 1).
        assert_eq!(result[0].0, "a");
        assert_eq!(result[1].0, "b");
        assert_eq!(result[2].0, "c");
    }

    /// Single list — rank 0 scores 1/k.
    #[test]
    fn single_list() {
        let lists = vec![vec!["x".to_string(), "y".to_string(), "z".to_string()]];
        let result = rrf_fuse(&lists, 60);
        assert!((result[0].1 - 1.0 / 60.0).abs() < 1e-15);
        assert!((result[1].1 - 1.0 / 61.0).abs() < 1e-15);
        assert!((result[2].1 - 1.0 / 62.0).abs() < 1e-15);
    }

    /// Empty lists → empty result.
    #[test]
    fn empty_lists() {
        let result = rrf_fuse(&[], 60);
        assert!(result.is_empty());
    }

    /// A single empty list → empty result.
    #[test]
    fn one_empty_list() {
        let lists: Vec<Vec<String>> = vec![vec![]];
        let result = rrf_fuse(&lists, 60);
        assert!(result.is_empty());
    }

    /// Scores accumulate correctly across 3 lists when a doc appears in all 3.
    #[test]
    fn three_lists_accumulate() {
        let lists = vec![
            vec!["a".to_string()],
            vec!["a".to_string()],
            vec!["a".to_string()],
        ];
        let result = rrf_fuse(&lists, 60);
        assert_eq!(result.len(), 1);
        // Each list contributes 1/60 → total = 3/60.
        assert!((result[0].1 - 3.0 / 60.0).abs() < 1e-15);
    }
}
