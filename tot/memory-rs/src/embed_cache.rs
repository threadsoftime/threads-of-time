//! LRU-cached wrapper around [`EmbeddingsClient`].
//!
//! `EmbedCache` wraps the HTTP embeddings client with an in-process LRU cache
//! keyed by text. Identical inputs short-circuit to the cached `Arc<Vec<f32>>`
//! without hitting the network. Capacity is 1024 entries (parity with Python's
//! `cache_size=1024`).
//!
//! Thread-safety: the inner `LruCache` is guarded by a `tokio::sync::Mutex`
//! so the cache can be shared across async tasks via `Arc<EmbedCache>`.

use std::num::NonZeroUsize;
use std::sync::Arc;

use lru::LruCache;
use tokio::sync::Mutex;

use crate::embeddings::{EmbedError, EmbeddingsClient};

/// LRU cache capacity — mirrors Python's `cache_size=1024`.
const CACHE_CAP: usize = 1024;

/// `EmbedCache` wraps [`EmbeddingsClient`] with an LRU cache keyed by text.
///
/// Cloning is cheap: the inner state is `Arc`-wrapped.
#[derive(Clone)]
pub struct EmbedCache {
    client: EmbeddingsClient,
    cache: Arc<Mutex<LruCache<String, Arc<Vec<f32>>>>>,
}

impl EmbedCache {
    /// Construct an `EmbedCache` from an already-built [`EmbeddingsClient`].
    pub fn new(client: EmbeddingsClient) -> Self {
        let cap = NonZeroUsize::new(CACHE_CAP).expect("CACHE_CAP must be > 0");
        Self {
            client,
            cache: Arc::new(Mutex::new(LruCache::new(cap))),
        }
    }

    /// Return the embedding for `text`.
    ///
    /// - Cache hit: returns the cached `Arc<Vec<f32>>` without any network call.
    /// - Cache miss: delegates to [`EmbeddingsClient::embed`], stores the result,
    ///   and returns a new `Arc<Vec<f32>>`.
    ///
    /// Errors from the underlying client are propagated as [`EmbedError`] without
    /// mutating the cache (a failed embed is not stored).
    pub async fn embed(&self, text: &str) -> Result<Arc<Vec<f32>>, EmbedError> {
        // Lock scope: check cache, unlock before the await.
        {
            let mut guard = self.cache.lock().await;
            if let Some(cached) = guard.get(text) {
                return Ok(Arc::clone(cached));
            }
        }

        // Cache miss — call the HTTP client (lock released during await).
        let vec = self.client.embed(text).await?;
        let arc = Arc::new(vec);

        // Re-acquire lock to store the result.
        {
            let mut guard = self.cache.lock().await;
            guard.put(text.to_owned(), Arc::clone(&arc));
        }

        Ok(arc)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        extract::Json as AxumJson,
        http::StatusCode,
        routing::post,
        Router,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Mutex as TokioMutex;

    use crate::EMBEDDING_DIM;

    // --- Mock helpers ---

    /// Spin up a mock `/v1/embeddings` endpoint on an ephemeral port.
    ///
    /// Returns `(base_url, request_count)`. `request_count` increments on every
    /// POST so tests can assert on cache hit behaviour.
    async fn spawn_counting_mock(response_vec: Vec<f32>) -> (String, Arc<AtomicUsize>) {
        let count = Arc::new(AtomicUsize::new(0));
        let count_clone = Arc::clone(&count);

        let handler = move |AxumJson(_body): AxumJson<serde_json::Value>| {
            let cnt = Arc::clone(&count_clone);
            let vec = response_vec.clone();
            async move {
                cnt.fetch_add(1, Ordering::SeqCst);
                let embedding_json: Vec<serde_json::Value> =
                    vec.iter().map(|&x| serde_json::json!(x)).collect();
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

        (format!("http://{addr}/v1"), count)
    }

    /// Spin up a counting mock that also captures the last request body.
    async fn spawn_counting_capturing_mock(
        response_vec: Vec<f32>,
    ) -> (String, Arc<AtomicUsize>, Arc<TokioMutex<serde_json::Value>>) {
        let count = Arc::new(AtomicUsize::new(0));
        let count_clone = Arc::clone(&count);
        let captured = Arc::new(TokioMutex::new(serde_json::Value::Null));
        let captured_clone = Arc::clone(&captured);

        let handler = move |AxumJson(body): AxumJson<serde_json::Value>| {
            let cnt = Arc::clone(&count_clone);
            let cap = Arc::clone(&captured_clone);
            let vec = response_vec.clone();
            async move {
                cnt.fetch_add(1, Ordering::SeqCst);
                *cap.lock().await = body;
                let embedding_json: Vec<serde_json::Value> =
                    vec.iter().map(|&x| serde_json::json!(x)).collect();
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

        (format!("http://{addr}/v1"), count, captured)
    }

    // --- C0: EMBEDDING_DIM must be exactly 384 (bge-small-en-v1.5 / parity contract) ---
    //
    // This test is the TDD tripwire: it must FAIL before the 384 re-target and
    // PASS after. If EMBEDDING_DIM is still 768 (the archived value) this panics.

    #[test]
    fn embedding_dim_is_384() {
        assert_eq!(
            EMBEDDING_DIM,
            384,
            "EMBEDDING_DIM must be 384 (bge-small-en-v1.5); got {}",
            EMBEDDING_DIM
        );
    }

    // --- C1: happy path — embed("hello") returns Arc<Vec<f32>> of len EMBEDDING_DIM ---

    #[tokio::test]
    async fn cache_embed_returns_correct_dim() {
        let response_vec: Vec<f32> = (0..EMBEDDING_DIM).map(|i| i as f32 * 0.001).collect();
        let (base_url, _count) = spawn_counting_mock(response_vec).await;

        let client = EmbeddingsClient::new(&base_url, "embedding", "");
        let cache = EmbedCache::new(client);

        let result = cache.embed("hello").await.unwrap();
        assert_eq!(
            result.len(),
            EMBEDDING_DIM,
            "embed must return exactly EMBEDDING_DIM={EMBEDDING_DIM} floats"
        );
    }

    // --- C2: request body is {"input":"hello","model":"embedding"} ---

    #[tokio::test]
    async fn cache_embed_sends_correct_body() {
        let response_vec: Vec<f32> = vec![0.0_f32; EMBEDDING_DIM];
        let (base_url, _count, captured) =
            spawn_counting_capturing_mock(response_vec).await;

        let client = EmbeddingsClient::new(&base_url, "embedding", "");
        let cache = EmbedCache::new(client);
        cache.embed("hello").await.unwrap();

        let body = captured.lock().await;
        assert_eq!(
            body.get("input").and_then(|v| v.as_str()),
            Some("hello"),
            "body.input must be the text passed to embed()"
        );
        assert_eq!(
            body.get("model").and_then(|v| v.as_str()),
            Some("embedding"),
            "body.model must be \"embedding\" (parity with Python)"
        );
    }

    // --- C3: second call with same text → only ONE upstream request (cache hit) ---

    #[tokio::test]
    async fn cache_hit_does_not_call_upstream_twice() {
        let response_vec: Vec<f32> = vec![0.1_f32; EMBEDDING_DIM];
        let (base_url, count) = spawn_counting_mock(response_vec).await;

        let client = EmbeddingsClient::new(&base_url, "embedding", "");
        let cache = EmbedCache::new(client);

        let r1 = cache.embed("hello").await.unwrap();
        let r2 = cache.embed("hello").await.unwrap();

        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "second embed(\"hello\") must hit the cache — upstream should be called exactly once"
        );
        // The two Arcs are logically equal (same backing data).
        assert_eq!(r1.as_ref(), r2.as_ref(), "cached value must equal original");
    }

    // --- C4: different texts each cause a separate upstream request ---

    #[tokio::test]
    async fn different_texts_each_call_upstream() {
        let response_vec: Vec<f32> = vec![0.2_f32; EMBEDDING_DIM];
        let (base_url, count) = spawn_counting_mock(response_vec).await;

        let client = EmbeddingsClient::new(&base_url, "embedding", "");
        let cache = EmbedCache::new(client);

        cache.embed("alice").await.unwrap();
        cache.embed("bob").await.unwrap();

        assert_eq!(
            count.load(Ordering::SeqCst),
            2,
            "two distinct texts must each produce one upstream request"
        );
    }

    // --- C5: wrong-dimension response → EmbedError::DimMismatch ---

    #[tokio::test]
    async fn wrong_dimension_response_is_error() {
        // The mock returns only 5 elements — should trigger DimMismatch.
        let short_vec: Vec<f32> = vec![0.1, 0.2, 0.3, 0.4, 0.5];
        let (base_url, _count) = spawn_counting_mock(short_vec).await;

        let client = EmbeddingsClient::new(&base_url, "embedding", "");
        let cache = EmbedCache::new(client);

        let err = cache.embed("text").await.unwrap_err();
        match err {
            EmbedError::DimMismatch { got, expected } => {
                assert_eq!(got, 5);
                assert_eq!(expected, EMBEDDING_DIM);
            }
            other => panic!("expected DimMismatch, got: {other}"),
        }
    }
}
