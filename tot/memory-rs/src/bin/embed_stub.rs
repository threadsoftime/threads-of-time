// SPDX-License-Identifier: GPL-2.0-or-later
//! Deterministic embedding stub server — a parity/test utility.
//!
//! Serves an OpenAI-compatible `POST /embeddings` endpoint that maps any input
//! text to a fixed 768-dim L2-normalized f32 vector via SHA-256. Identical text
//! always yields the identical vector, with no randomness and no external model.
//!
//! This is the HTTP counterpart of the `stub_embed` used in
//! `tests/quality_gate.rs` and of Python's `_stub_embed` in
//! `tests/eval/test_quality_gate.py` — byte-for-byte the same algorithm. The
//! cross-implementation parity harness (`tests/parity.rs`) points BOTH the Rust
//! router (`AppState.embed.url`) and the Python parity sidecar
//! (`BRAIN_EMBEDDINGS_URL`) at this stub so both embed each query text into the
//! same vector. That byte-identical query vector is the precondition for
//! dense-score parity within EPS.
//!
//! NOT a runtime component: this binary is excluded from the container image
//! (the Containerfile ships only the `memory-rs` binary).
//!
//! Run:
//!   cargo run -p memory-rs --bin embed-stub
//!   # binds 127.0.0.1:8099 by default; override with EMBED_STUB_BIND
//!
//! Smoke test:
//!   curl -s -XPOST 127.0.0.1:8099/embeddings -d '{"model":"x","input":"hello"}'

use std::net::SocketAddr;

use axum::body::Bytes;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{routing::post, Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use memory_rs::EMBEDDING_DIM;

/// Default bind address when `EMBED_STUB_BIND` is unset.
const DEFAULT_BIND: &str = "127.0.0.1:8099";

/// SHA-256 → 768-dim L2-normalized f32 vector.
///
/// Mirrors `stub_embed` in `tests/quality_gate.rs` (and Python's `_stub_embed`)
/// exactly:
///   1. Hash text with SHA-256 (32 bytes).
///   2. Cycle the digest to fill EMBEDDING_DIM (768 / 32 = 24 reps).
///   3. Map each byte b → (b as f64 - 128.0) / 128.0.
///   4. L2-normalize.
///   5. Cast to f32.
fn stub_embed(text: &str) -> Vec<f32> {
    let digest = Sha256::digest(text.as_bytes());
    let h: &[u8] = &digest;
    let repeated: Vec<u8> = h.iter().copied().cycle().take(EMBEDDING_DIM).collect();

    let mut vec: Vec<f64> = repeated
        .iter()
        .map(|&b| (b as f64 - 128.0) / 128.0)
        .collect();
    let norm = vec.iter().map(|v| v * v).sum::<f64>().sqrt();
    if norm > 0.0 {
        vec.iter_mut().for_each(|v| *v /= norm);
    }
    vec.iter().map(|&v| v as f32).collect()
}

/// Request body — the OpenAI/`EmbeddingsClient` shape: `{"model":..,"input":..}`.
///
/// `model` is accepted and ignored (the stub is model-agnostic). `input` is the
/// text to embed; non-string inputs are coerced to their empty form.
#[derive(Debug, Deserialize)]
struct EmbedRequest {
    #[allow(dead_code)]
    #[serde(default)]
    model: Value,
    #[serde(default)]
    input: String,
}

/// `POST /embeddings` — returns `{"data":[{"embedding":[<768 floats>]}]}`.
///
/// Parses the JSON body from raw bytes (rather than via the `Json` extractor)
/// so the stub is content-type agnostic: it accepts `curl -d '{...}'` (which
/// defaults to `application/x-www-form-urlencoded`) just as readily as a
/// reqwest `.json()` call that sets `application/json`. The OpenAI-compatible
/// `EmbeddingsClient` sends the header; ad-hoc smoke tests usually don't.
async fn embeddings(body: Bytes) -> Response {
    let req: EmbedRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, format!("invalid JSON body: {e}")).into_response();
        }
    };
    let vec = stub_embed(&req.input);
    Json(json!({
        "data": [{ "object": "embedding", "embedding": vec, "index": 0 }],
        "model": "embed-stub",
        "object": "list",
    }))
    .into_response()
}

#[tokio::main]
async fn main() {
    let bind = std::env::var("EMBED_STUB_BIND").unwrap_or_else(|_| DEFAULT_BIND.to_string());
    let addr: SocketAddr = bind
        .parse()
        .unwrap_or_else(|e| panic!("invalid EMBED_STUB_BIND {bind:?}: {e}"));

    let app = Router::new().route("/embeddings", post(embeddings));

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|e| panic!("embed-stub failed to bind {addr}: {e}"));
    let local = listener.local_addr().unwrap_or(addr);

    // Unique startup string (mirrors main.rs convention).
    eprintln!("embed-stub listening on {local}");

    axum::serve(listener, app)
        .await
        .expect("embed-stub server error");
}
