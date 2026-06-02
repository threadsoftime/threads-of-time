//! Maximal Marginal Relevance — ports `search.py::mmr_select`.
//!
//! ```python
//! def mmr_select(candidates, query_emb, top_k, lam=0.7):
//!     if not candidates or top_k <= 0: return []
//!     remaining = list(candidates)
//!     selected = []
//!     relevance = {doc: _cosine(emb, query_emb) for doc, emb in remaining}
//!     while len(selected) < top_k and remaining:
//!         best_doc = None; best_score = float("-inf")
//!         for cand in remaining:
//!             cid, cemb = cand
//!             rel = relevance[cid]
//!             if not selected: penalty = 0.0
//!             else: penalty = max(_cosine(cemb, s_emb) for _, s_emb in selected)
//!             score = lam * rel - (1.0 - lam) * penalty
//!             if score > best_score:          # strict > — first-encountered wins on tie
//!                 best_score = score; best_doc = cand
//!         selected.append(best_doc)
//!         remaining.remove(best_doc)
//!     return selected
//! ```
//!
//! Parity notes:
//! - Relevance scores are pre-computed once (not recomputed each iteration).
//! - `remaining` is a `Vec` — no HashMap or HashSet — preserving insertion order.
//! - Strict `>` comparison: the first-encountered candidate wins on exact score ties,
//!   matching Python's iteration order over a `list`.
//! - The same f64-upcast `cosine` from `cosine.rs` is used throughout.

use crate::retrieval::cosine::cosine;

/// Select up to `top_k` candidates using Maximal Marginal Relevance.
///
/// Arguments:
/// * `candidates` — ordered list of `(doc_id, embedding)` pairs.
/// * `query_emb`  — the query embedding.
/// * `top_k`      — maximum number of results to return.
/// * `lam`        — trade-off: 1.0 = pure relevance, 0.0 = pure diversity.
///
/// Returns a `Vec<(String, Vec<f32>)>` containing the selected candidates in
/// selection order.  Shorter than `top_k` only when `candidates` is exhausted.
pub fn mmr_select(
    candidates: &[(String, Vec<f32>)],
    query_emb: &[f32],
    top_k: usize,
    lam: f64,
) -> Vec<(String, Vec<f32>)> {
    if candidates.is_empty() || top_k == 0 {
        return Vec::new();
    }

    // `remaining` mirrors Python's `remaining = list(candidates)`.
    // Each entry is an index into `candidates` — avoids cloning the embeddings.
    let mut remaining: Vec<usize> = (0..candidates.len()).collect();

    // Pre-compute relevance scores once, keyed by position in `candidates`.
    let relevance: Vec<f64> = candidates
        .iter()
        .map(|(_, emb)| cosine(emb, query_emb))
        .collect();

    // `selected` holds indices of already-chosen candidates (in order).
    let mut selected_indices: Vec<usize> = Vec::with_capacity(top_k);

    while selected_indices.len() < top_k && !remaining.is_empty() {
        let mut best_idx_in_remaining: Option<usize> = None;
        let mut best_score = f64::NEG_INFINITY;

        for (pos_in_remaining, &cand_idx) in remaining.iter().enumerate() {
            let rel = relevance[cand_idx];
            let penalty = if selected_indices.is_empty() {
                0.0
            } else {
                selected_indices
                    .iter()
                    .map(|&sel_idx| cosine(&candidates[cand_idx].1, &candidates[sel_idx].1))
                    .fold(f64::NEG_INFINITY, f64::max)
            };
            let score = lam * rel - (1.0 - lam) * penalty;
            // Strict > : first-encountered wins on exact ties (Python semantics).
            if score > best_score {
                best_score = score;
                best_idx_in_remaining = Some(pos_in_remaining);
            }
        }

        let pos = best_idx_in_remaining.expect("remaining is non-empty so a best must exist");
        let cand_idx = remaining.remove(pos);
        selected_indices.push(cand_idx);
    }

    selected_indices
        .into_iter()
        .map(|idx| candidates[idx].clone())
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(id: &str, emb: Vec<f32>) -> (String, Vec<f32>) {
        (id.to_string(), emb)
    }

    /// 3 candidates with distinct relevance, λ=0.7, top_k=2.
    /// The highest-relevance candidate is selected first, then the most diverse.
    #[test]
    fn highest_relevance_first_then_diverse() {
        // query = [1,0,0]
        // a = [1,0,0]  → cos(a,q)=1.0   (most relevant)
        // b = [0,1,0]  → cos(b,q)=0.0
        // c = [-1,0,0] → cos(c,q)=-1.0  (least relevant, but diverse from a)
        let query = vec![1.0f32, 0.0, 0.0];
        let candidates = vec![
            mk("a", vec![1.0, 0.0, 0.0]),
            mk("b", vec![0.0, 1.0, 0.0]),
            mk("c", vec![-1.0, 0.0, 0.0]),
        ];
        let result = mmr_select(&candidates, &query, 2, 0.7);
        assert_eq!(result.len(), 2);
        // First pick: a (highest relevance = 1.0, no penalty yet).
        assert_eq!(result[0].0, "a");
        // Second pick: c is most diverse from a (cos(c,a)=-1 → penalty=-1 → MMR score higher)
        // vs b: cos(b,a)=0 → penalty=0.
        // c: score = 0.7*(-1) - 0.3*(-1) = -0.7 + 0.3 = -0.4
        // b: score = 0.7*0.0 - 0.3*0.0  = 0.0
        // b should be chosen (0.0 > -0.4)
        assert_eq!(result[1].0, "b");
    }

    /// Empty candidates → empty result.
    #[test]
    fn empty_candidates_returns_empty() {
        let result = mmr_select(&[], &[1.0f32, 0.0], 5, 0.7);
        assert!(result.is_empty());
    }

    /// top_k=0 → empty result.
    #[test]
    fn top_k_zero_returns_empty() {
        let candidates = vec![mk("x", vec![1.0, 0.0])];
        let result = mmr_select(&candidates, &[1.0f32, 0.0], 0, 0.7);
        assert!(result.is_empty());
    }

    /// top_k > candidates.len() → return all candidates.
    #[test]
    fn top_k_larger_than_candidates() {
        let candidates = vec![mk("a", vec![1.0, 0.0]), mk("b", vec![0.0, 1.0])];
        let result = mmr_select(&candidates, &[1.0f32, 0.0], 10, 0.7);
        assert_eq!(result.len(), 2);
    }

    /// Single candidate is always returned (when top_k >= 1).
    #[test]
    fn single_candidate_returned() {
        let candidates = vec![mk("only", vec![1.0, 0.0])];
        let result = mmr_select(&candidates, &[1.0f32, 0.0], 1, 0.7);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "only");
    }

    /// Exact-tie tie-break: first-encountered wins (strict > semantics).
    /// Both a and b have identical embeddings and query alignment.
    #[test]
    fn tie_break_first_encountered_wins() {
        // a and b are identical vectors → same relevance, same penalty for any selection.
        let emb = vec![1.0f32, 0.0];
        let query = vec![1.0f32, 0.0];
        let candidates = vec![
            mk("first", emb.clone()),
            mk("second", emb.clone()),
        ];
        let result = mmr_select(&candidates, &query, 1, 0.7);
        // "first" is iterated first in `remaining`; strict > means it wins on tie.
        assert_eq!(result[0].0, "first");
    }

    /// λ=1.0 (pure relevance) → candidates sorted purely by cosine similarity.
    #[test]
    fn lambda_one_pure_relevance_order() {
        let query = vec![1.0f32, 0.0, 0.0];
        // Relevances: a=1.0, b=0.5, c=0.0 (b has intermediate angle)
        let b_emb: Vec<f32> = {
            let v = vec![1.0f32, 1.0, 0.0];
            let norm = (2.0f32).sqrt();
            v.iter().map(|x| x / norm).collect()
        };
        let candidates = vec![
            mk("a", vec![1.0, 0.0, 0.0]),
            mk("c", vec![0.0, 1.0, 0.0]),
            mk("b", b_emb),
        ];
        let result = mmr_select(&candidates, &query, 3, 1.0);
        // With λ=1.0 penalty term is zero, so order is descending by relevance.
        assert_eq!(result[0].0, "a");  // cos≈1.0
        assert_eq!(result[1].0, "b");  // cos≈0.707
        assert_eq!(result[2].0, "c");  // cos=0.0
    }

    /// Results contain exactly top_k items when candidates >= top_k.
    #[test]
    fn result_has_exactly_top_k_items() {
        let query = vec![1.0f32, 0.0];
        let candidates = vec![
            mk("a", vec![1.0, 0.0]),
            mk("b", vec![0.8f32, 0.6]),
            mk("c", vec![0.6f32, 0.8]),
            mk("d", vec![0.0, 1.0]),
        ];
        let result = mmr_select(&candidates, &query, 2, 0.7);
        assert_eq!(result.len(), 2);
    }
}
