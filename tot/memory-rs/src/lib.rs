//! `memory-rs` — stub scaffold for v0.2.1 re-target.
//!
//! Domain modules archived to `_archive-768/`.
//! Phases 1–7 will rebuild them against the live memory-sidecar v0.2.1 contract.

/// Dimension of the dense embedding vector.
/// NOTE: v0.2.1 target uses 384-dim (all-MiniLM-L6-v2); this constant will be
/// updated in Task 0.2 / Phase 1. Kept at 768 for now so embed_stub compiles
/// unchanged during the archival stub phase.
pub const EMBEDDING_DIM: usize = 768;

pub mod app;
pub mod config;
pub mod db;
pub mod embeddings;
pub mod error;
pub mod routes;
pub mod state;
