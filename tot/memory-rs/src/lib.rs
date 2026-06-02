//! `memory-rs` — stub scaffold for v0.2.1 re-target.
//!
//! Domain modules archived to `_archive-768/`.
//! Phases 1–7 will rebuild them against the live memory-sidecar v0.2.1 contract.

/// Dimension of the dense embedding vector.
///
/// 384 = bge-small-en-v1.5 (the live memory-sidecar v0.2.1 model).
/// Parity-critical: the embed stub, the LRU cache, and the Python
/// `EmbeddingClient` all use this exact value.
pub const EMBEDDING_DIM: usize = 384;

pub mod app;
pub mod auth;
pub mod config;
pub mod core;
pub mod db;
pub mod embed_cache;
pub mod embeddings;
pub mod error;
pub mod ids;
pub mod pubsub;
pub mod retrieval;
pub mod routes;
pub mod state;
