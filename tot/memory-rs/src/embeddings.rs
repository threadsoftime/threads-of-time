//! HTTP client for the OpenAI-compatible embeddings endpoint.

use crate::EMBEDDING_DIM;
use reqwest::{
    header::{self, HeaderMap, HeaderValue},
    Client,
};
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EmbedError {
    /// Network error or non-2xx HTTP status from the embeddings endpoint.
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    /// The endpoint returned a vector whose length differs from EMBEDDING_DIM.
    /// Routes that are "loud" (search, update) surface this as 503; routes that
    /// are "graceful" (write, recall) log and continue with a NULL vector.
    #[error("embedding dimension mismatch: got {got}, expected {expected}")]
    DimMismatch { got: usize, expected: usize },

    /// The response JSON was well-formed HTTP 2xx but lacked `data[0].embedding`.
    #[error("malformed embeddings response: missing data[0].embedding")]
    Malformed,
}

/// OpenAI-compatible `/v1/embeddings` client.
///
/// Mirrors `EmbeddingsClient` in `tot_memory/embeddings/client.py`:
/// - `base_url` trailing slashes are trimmed.
/// - Bearer token is sent only when `api_key` is non-empty.
/// - Per-request timeout is 30 s (same as Python's `httpx.AsyncClient(timeout=30.0)`).
///
/// The client is `Clone`; `reqwest::Client` holds an inner `Arc` so cloning is cheap.
#[derive(Clone)]
pub struct EmbeddingsClient {
    base_url: String, // trailing slash stripped
    model: String,
    http: Client,
}

impl EmbeddingsClient {
    /// Build the client. Panics only if reqwest cannot build a TLS stack (should
    /// never happen in practice with the bundled rustls backend).
    pub fn new(base_url: &str, model: &str, api_key: &str) -> Self {
        let base_url = base_url.trim_end_matches('/').to_owned();

        let mut default_headers = HeaderMap::new();
        if !api_key.is_empty() {
            // "Bearer <key>" — exact format the Python httpx client uses.
            let mut value = HeaderValue::from_str(&format!("Bearer {api_key}"))
                .expect("api_key must be a valid header value");
            value.set_sensitive(true);
            default_headers.insert(header::AUTHORIZATION, value);
        }

        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .default_headers(default_headers)
            .build()
            .expect("failed to build reqwest client");

        Self { base_url, model: model.to_owned(), http }
    }

    /// Embed `text` via the configured endpoint.
    ///
    /// Returns `Vec<f32>` of length `EMBEDDING_DIM` on success.
    ///
    /// Error mapping (load-bearing — routes branch on these variants):
    /// - Any transport error or non-2xx status → `EmbedError::Http`
    /// - 2xx but wrong vector length → `EmbedError::DimMismatch { got, expected }`
    /// - 2xx but `data[0].embedding` absent or not an array → `EmbedError::Malformed`
    ///
    /// JSON floats are `f64` by default in serde; we collect as `f32` (round-to-
    /// nearest) matching Python's `struct.pack('f', ...)` on storage.
    pub async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        let url = format!("{}/embeddings", self.base_url);

        let resp = self
            .http
            .post(&url)
            .json(&serde_json::json!({
                "model": self.model,
                "input": text,
            }))
            .send()
            .await?;

        // Converts non-2xx into reqwest::Error → EmbedError::Http via #[from].
        let resp = resp.error_for_status()?;

        let payload: serde_json::Value = resp.json().await?;

        // Navigate data[0].embedding — same index path as Python.
        let embedding_array = payload
            .get("data")
            .and_then(|d| d.get(0))
            .and_then(|item| item.get("embedding"))
            .and_then(|e| e.as_array())
            .ok_or(EmbedError::Malformed)?;

        // Collect as f32 (JSON numbers are f64; as_f64().unwrap_or(0.0) as f32
        // mirrors Python struct.pack round-to-nearest).
        let vec: Vec<f32> = embedding_array
            .iter()
            .map(|v| v.as_f64().unwrap_or(0.0) as f32)
            .collect();

        if vec.len() != EMBEDDING_DIM {
            return Err(EmbedError::DimMismatch {
                got: vec.len(),
                expected: EMBEDDING_DIM,
            });
        }

        Ok(vec)
    }
}
