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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        extract::Json as AxumJson,
        http::{HeaderMap as AxumHeaderMap, StatusCode},
        routing::post,
        Router,
    };
    use std::sync::Arc;
    use tokio::sync::Mutex;

    /// Captured request data from the mock handler.
    #[derive(Default, Clone)]
    struct Captured {
        body: serde_json::Value,
        auth_header: Option<String>,
    }

    /// Spin up a mock `/v1/embeddings` endpoint on an ephemeral port.
    ///
    /// `captured` is written by the handler so tests can assert on it after
    /// the client call returns.
    ///
    /// Returns `(base_url, Arc<Mutex<Captured>>)`.
    async fn spawn_embed_mock(
        response_vec: Vec<f32>,
    ) -> (String, Arc<Mutex<Captured>>) {
        let captured = Arc::new(Mutex::new(Captured::default()));
        let captured_clone = captured.clone();

        let handler = move |
            headers: AxumHeaderMap,
            AxumJson(body): AxumJson<serde_json::Value>,
        | {
            let captured = captured_clone.clone();
            let vec = response_vec.clone();
            async move {
                // Store what the client sent so the test can assert on it.
                let auth = headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_owned());
                {
                    let mut c = captured.lock().await;
                    c.body = body;
                    c.auth_header = auth;
                }

                // Return the OpenAI-compatible shape the client expects.
                let embedding_json: Vec<serde_json::Value> = vec
                    .iter()
                    .map(|&x| serde_json::json!(x))
                    .collect();
                (
                    StatusCode::OK,
                    AxumJson(serde_json::json!({
                        "object": "list",
                        "data": [{ "object": "embedding", "embedding": embedding_json, "index": 0 }],
                        "model": "test-model",
                    })),
                )
            }
        };

        let app = Router::new().route("/v1/embeddings", post(handler));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        (format!("http://{addr}/v1"), captured)
    }

    // --- Happy-path tests ---

    // H1: embed() returns Vec<f32> of exactly EMBEDDING_DIM length.
    #[tokio::test]
    async fn happy_returns_vec_len_768() {
        let response_vec: Vec<f32> = (0..EMBEDDING_DIM).map(|i| i as f32 * 0.001).collect();
        let (base_url, _captured) = spawn_embed_mock(response_vec.clone()).await;

        let client = EmbeddingsClient::new(&base_url, "nomic-embed-text", "");
        let result = client.embed("hello world").await.unwrap();

        assert_eq!(result.len(), EMBEDDING_DIM, "returned vec must be exactly EMBEDDING_DIM={}", EMBEDDING_DIM);
    }

    // H2: embed() sends {"model": <model>, "input": <text>} in the request body.
    #[tokio::test]
    async fn happy_sends_correct_body() {
        let response_vec: Vec<f32> = vec![0.0_f32; EMBEDDING_DIM];
        let (base_url, captured) = spawn_embed_mock(response_vec).await;

        let client = EmbeddingsClient::new(&base_url, "nomic-embed-text", "");
        client.embed("test input text").await.unwrap();

        let c = captured.lock().await;
        assert_eq!(
            c.body.get("model").and_then(|v| v.as_str()),
            Some("nomic-embed-text"),
            "body must include model field"
        );
        assert_eq!(
            c.body.get("input").and_then(|v| v.as_str()),
            Some("test input text"),
            "body must include input field"
        );
    }

    // H3: Authorization: Bearer header is present when api_key is non-empty.
    #[tokio::test]
    async fn happy_sends_bearer_when_api_key_set() {
        let response_vec: Vec<f32> = vec![0.0_f32; EMBEDDING_DIM];
        let (base_url, captured) = spawn_embed_mock(response_vec).await;

        let client = EmbeddingsClient::new(&base_url, "nomic-embed-text", "secret-key");
        client.embed("text").await.unwrap();

        let c = captured.lock().await;
        assert_eq!(
            c.auth_header.as_deref(),
            Some("Bearer secret-key"),
            "Authorization header must be 'Bearer <api_key>'"
        );
    }

    // H4: No Authorization header is sent when api_key is empty.
    #[tokio::test]
    async fn happy_no_auth_header_when_api_key_empty() {
        let response_vec: Vec<f32> = vec![0.0_f32; EMBEDDING_DIM];
        let (base_url, captured) = spawn_embed_mock(response_vec).await;

        let client = EmbeddingsClient::new(&base_url, "nomic-embed-text", "");
        client.embed("text").await.unwrap();

        let c = captured.lock().await;
        assert!(
            c.auth_header.is_none(),
            "no Authorization header must be sent when api_key is empty, got: {:?}",
            c.auth_header
        );
    }

    // H5: Trailing slashes in base_url are stripped — the URL hit is
    //     exactly {base}/embeddings, not {base}//embeddings.
    //     The mock only routes /v1/embeddings; double-slash would 404.
    #[tokio::test]
    async fn happy_trailing_slash_stripped() {
        let response_vec: Vec<f32> = vec![0.0_f32; EMBEDDING_DIM];
        // spawn_embed_mock returns a /v1 base; add trailing slashes.
        let (base_url, _) = spawn_embed_mock(response_vec).await;
        let base_url_with_slashes = format!("{}/", base_url); // e.g. http://addr/v1/

        let client = EmbeddingsClient::new(&base_url_with_slashes, "model", "");
        // Would return 404 (not 200) if the slash were not trimmed.
        client.embed("text").await.expect("trailing slash must be stripped before appending /embeddings");
    }

    // H6: Returned Vec<f32> values match the mock's output (f64→f32 round-trip).
    #[tokio::test]
    async fn happy_vec_values_match() {
        // Use a recognisable pattern so the assertion is not vacuous.
        let expected: Vec<f32> = (0..EMBEDDING_DIM).map(|i| (i as f32) / (EMBEDDING_DIM as f32)).collect();
        let (base_url, _) = spawn_embed_mock(expected.clone()).await;

        let client = EmbeddingsClient::new(&base_url, "model", "");
        let got = client.embed("text").await.unwrap();

        // The JSON round-trip (f32 → JSON f64 → as_f64() as f32) introduces
        // at most 1 ULP of error on values in [0, 1]. Use approximate equality.
        for (i, (&e, &g)) in expected.iter().zip(got.iter()).enumerate() {
            assert!(
                (e - g).abs() < 1e-5,
                "index {i}: expected {e}, got {g}"
            );
        }
    }

    // --- Error-mapping helpers ---

    /// Spawn a mock that always responds with `status` and `body` on any POST
    /// to /v1/embeddings. Used for non-2xx and malformed tests.
    async fn spawn_status_mock(status: StatusCode, body: serde_json::Value) -> String {
        let handler = move || {
            let s = status;
            let b = body.clone();
            async move { (s, AxumJson(b)) }
        };
        let app = Router::new().route("/v1/embeddings", post(handler));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}/v1")
    }

    // E1: Non-2xx (e.g. 500 Internal Server Error) → EmbedError::Http.
    //     This covers the graceful-degradation branch in write/recall.
    #[tokio::test]
    async fn error_non_2xx_maps_to_http() {
        let base_url = spawn_status_mock(
            StatusCode::INTERNAL_SERVER_ERROR,
            serde_json::json!({"error": "overloaded"}),
        )
        .await;

        let client = EmbeddingsClient::new(&base_url, "model", "");
        let err = client.embed("text").await.unwrap_err();

        match err {
            EmbedError::Http(_) => {} // correct
            other => panic!("expected EmbedError::Http, got: {other}"),
        }
    }

    // E2: 401 Unauthorized → EmbedError::Http (non-2xx, same branch as E1).
    #[tokio::test]
    async fn error_401_maps_to_http() {
        let base_url = spawn_status_mock(
            StatusCode::UNAUTHORIZED,
            serde_json::json!({"error": "unauthorized"}),
        )
        .await;

        let client = EmbeddingsClient::new(&base_url, "model", "wrong-key");
        let err = client.embed("text").await.unwrap_err();

        match err {
            EmbedError::Http(_) => {}
            other => panic!("expected EmbedError::Http on 401, got: {other}"),
        }
    }

    // E3: Wrong dimension (5 instead of 768) → EmbedError::DimMismatch{got:5,expected:768}.
    //     This is the variant that the search route treats as 503 (loud failure)
    //     while the write route treats as graceful degradation (NULL vec).
    #[tokio::test]
    async fn error_wrong_dim_maps_to_dim_mismatch() {
        // Reuse spawn_embed_mock with a 5-element vector.
        let short_vec: Vec<f32> = vec![0.1, 0.2, 0.3, 0.4, 0.5];
        let (base_url, _) = spawn_embed_mock(short_vec).await;

        let client = EmbeddingsClient::new(&base_url, "model", "");
        let err = client.embed("text").await.unwrap_err();

        match err {
            EmbedError::DimMismatch { got, expected } => {
                assert_eq!(got, 5, "got must reflect the actual short length");
                assert_eq!(expected, EMBEDDING_DIM, "expected must be EMBEDDING_DIM={}", EMBEDDING_DIM);
            }
            other => panic!("expected EmbedError::DimMismatch, got: {other}"),
        }
    }

    // E4: 2xx response but `data[0].embedding` is absent → EmbedError::Malformed.
    #[tokio::test]
    async fn error_missing_embedding_path_maps_to_malformed() {
        // Well-formed HTTP 200 but wrong payload shape — no `data` key.
        let base_url = spawn_status_mock(
            StatusCode::OK,
            serde_json::json!({"result": "ok"}),
        )
        .await;

        let client = EmbeddingsClient::new(&base_url, "model", "");
        let err = client.embed("text").await.unwrap_err();

        match err {
            EmbedError::Malformed => {}
            other => panic!("expected EmbedError::Malformed on missing data[0].embedding, got: {other}"),
        }
    }

    // E5: 2xx response where `data` exists but `data[0]` has no `embedding` key.
    #[tokio::test]
    async fn error_missing_embedding_key_maps_to_malformed() {
        let base_url = spawn_status_mock(
            StatusCode::OK,
            serde_json::json!({"data": [{"object": "embedding"}]}), // embedding key absent
        )
        .await;

        let client = EmbeddingsClient::new(&base_url, "model", "");
        let err = client.embed("text").await.unwrap_err();

        match err {
            EmbedError::Malformed => {}
            other => panic!("expected EmbedError::Malformed on absent embedding key, got: {other}"),
        }
    }

    // E6: `data` is present but empty array — index 0 does not exist.
    #[tokio::test]
    async fn error_empty_data_array_maps_to_malformed() {
        let base_url = spawn_status_mock(
            StatusCode::OK,
            serde_json::json!({"data": []}),
        )
        .await;

        let client = EmbeddingsClient::new(&base_url, "model", "");
        let err = client.embed("text").await.unwrap_err();

        match err {
            EmbedError::Malformed => {}
            other => panic!("expected EmbedError::Malformed on empty data array, got: {other}"),
        }
    }

    // E7: Connection refused (no server at that port) → EmbedError::Http.
    //     Uses port 1 which is a privileged port — the OS will refuse the connect
    //     immediately without a sleep.
    #[tokio::test]
    async fn error_connection_refused_maps_to_http() {
        // Port 1 is privileged and unbound; the connect will be refused
        // immediately (same pattern as lfg-matchmaker harness.rs T3).
        let client = EmbeddingsClient::new("http://127.0.0.1:1/v1", "model", "");
        let err = client.embed("text").await.unwrap_err();

        match err {
            EmbedError::Http(_) => {}
            other => panic!("expected EmbedError::Http on connection refused, got: {other}"),
        }
    }
}
