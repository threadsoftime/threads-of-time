//! Hybrid retrieval pipeline: BM25 + dense + decay + entity filter + rerank.
//! Full implementation in Phase 3.
pub mod bm25;
pub mod decay;
pub mod dense;
pub mod entity;
pub mod hybrid;
pub mod rerank;
