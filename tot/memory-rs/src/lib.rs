//! `memory-rs` — drop-in Rust replacement for the Python `memory-sidecar`.
//!
//! Provides the same 8 HTTP endpoints on port 8090, backed by per-bot SQLite
//! files in `MEMORY_DATA_DIR/<bot_guid>/memory.sqlite` with FTS5 BM25 + vec0
//! dense vector hybrid retrieval.

/// Dimension of the dense embedding vector.
/// Must match the `float[768]` in the `embeddings_vec` virtual table schema
/// and the nomic-embed-text model output.
pub const EMBEDDING_DIM: usize = 768;

pub mod app;
pub mod config;
pub mod db;
pub mod embeddings;
pub mod error;
pub mod retrieval;
pub mod routes;
pub mod state;
