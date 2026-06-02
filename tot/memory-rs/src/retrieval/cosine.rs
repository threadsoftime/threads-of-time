//! f64-upcast cosine similarity — parity with numpy's `np.dot / (norm(a)*norm(b))`.
//!
//! Mirrors `recall.py::cosine`:
//! ```python
//! def cosine(a, b):
//!     na = float(np.linalg.norm(a)); nb = float(np.linalg.norm(b))
//!     if na == 0.0 or nb == 0.0: return 0.0
//!     return float(np.dot(a, b) / (na * nb))
//! ```
//!
//! Design notes:
//! - f32 inputs upcast to f64 element-by-element, matching numpy's default
//!   behaviour when dot() is called on float32 arrays (accumulates in f64).
//! - Sequential summation only — NO SIMD — to preserve identical
//!   float-accumulation order with the Python reference.
//! - Zero-vector guard returns 0.0 (not NaN).

/// Cosine similarity between two f32 slices, computed in f64.
///
/// Returns 0.0 if either vector is the zero vector.
/// Panics if `a.len() != b.len()`.
pub fn cosine(a: &[f32], b: &[f32]) -> f64 {
    assert_eq!(a.len(), b.len(), "cosine: vectors must have the same length");
    let mut dot = 0f64;
    let mut na = 0f64;
    let mut nb = 0f64;
    for i in 0..a.len() {
        let x = a[i] as f64;
        let y = b[i] as f64;
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    let na = na.sqrt();
    let nb = nb.sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// A vector with itself is exactly 1.0.
    #[test]
    fn identical_vectors_return_one() {
        let a = vec![1.0f32, 2.0, 3.0];
        let r = cosine(&a, &a);
        assert!((r - 1.0).abs() < 1e-12, "cosine(a,a) should be ~1.0, got {r}");
    }

    /// Orthogonal vectors have cosine 0.
    #[test]
    fn orthogonal_vectors_return_zero() {
        let a = vec![1.0f32, 0.0, 0.0];
        let b = vec![0.0f32, 1.0, 0.0];
        let r = cosine(&a, &b);
        assert!((r).abs() < 1e-12, "orthogonal cosine should be 0.0, got {r}");
    }

    /// Zero vector (left) → 0.0 without NaN.
    #[test]
    fn zero_vector_a_returns_zero() {
        let a = vec![0.0f32, 0.0, 0.0];
        let b = vec![1.0f32, 2.0, 3.0];
        let r = cosine(&a, &b);
        assert_eq!(r, 0.0, "zero-vector a should give 0.0, got {r}");
    }

    /// Zero vector (right) → 0.0 without NaN.
    #[test]
    fn zero_vector_b_returns_zero() {
        let a = vec![1.0f32, 2.0, 3.0];
        let b = vec![0.0f32, 0.0, 0.0];
        let r = cosine(&a, &b);
        assert_eq!(r, 0.0, "zero-vector b should give 0.0, got {r}");
    }

    /// Both zero → 0.0.
    #[test]
    fn both_zero_returns_zero() {
        let z = vec![0.0f32; 8];
        assert_eq!(cosine(&z, &z), 0.0);
    }

    /// Anti-parallel vectors → -1.0.
    #[test]
    fn anti_parallel_returns_minus_one() {
        let a = vec![1.0f32, 0.0];
        let b = vec![-1.0f32, 0.0];
        let r = cosine(&a, &b);
        assert!((r + 1.0).abs() < 1e-12, "anti-parallel should be ~-1.0, got {r}");
    }

    /// 384-dim self-similarity smoke test (representative of real embeddings).
    #[test]
    fn self_similarity_384_dim() {
        let a: Vec<f32> = (0..384).map(|i| (i as f32) / 384.0).collect();
        let r = cosine(&a, &a);
        assert!((r - 1.0).abs() < 1e-10, "384-dim self-sim should be ~1.0, got {r}");
    }
}
