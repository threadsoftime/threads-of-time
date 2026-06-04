// SPDX-License-Identifier: AGPL-3.0
/// MemoryClient — thin async wrapper over the harness `memory.*` HTTP tools.
///
/// Faithful Rust port of brain_sidecar/memory_client.py MemoryClient.
///
/// Transport: plain HTTP (reqwest 0.12) to the harness REST API at
///   POST {HARNESS_BASE_URL}/v1/tools/memory.recall
///   POST {HARNESS_BASE_URL}/v1/tools/memory.write
///
/// The harness wraps responses in `{"ok": true, "result": {...}}`; this client
/// unwraps `result` for the caller. The `{"ok":false,...}` case returns Err.
///
/// This is DISTINCT from McpClient — memory_client.py uses direct HTTP, NOT MCP.
/// Matches the Python client's request shape, response unwrap, and error handling
/// exactly.
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tot_harness_client::HarnessClient;

// ---------------------------------------------------------------------------
// RecalledEpisode
// ---------------------------------------------------------------------------

/// One episode returned by `memory.recall`.
///
/// Fields mirror memory_client.py RecalledEpisode and the design subspec §10.3
/// response shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecalledEpisode {
    pub episode_id: i64,
    pub content_text: String,
    /// ISO-8601 string OR epoch ms as str — server-defined.
    pub timestamp: String,
    pub salience_score: f64,
    /// Hybrid recall_score (§6.2 of the subspec).
    pub score: f64,
}

// ---------------------------------------------------------------------------
// MemoryClient
// ---------------------------------------------------------------------------

/// Async client over the harness `memory.*` HTTP tools.
///
/// Construct with `MemoryClient::new(base_url, bearer_token)` or use
/// `MemoryClient::from_env()` in production (reads `HARNESS_BASE_URL` and
/// `HARNESS_BEARER_TOKEN`).
pub struct MemoryClient {
    client: HarnessClient,
}

impl MemoryClient {
    /// Construct a new MemoryClient.
    /// `base_url` is the harness base URL, e.g. `http://192.168.1.3:8099`.
    /// `bearer_token` is the raw bearer token (without "Bearer " prefix).
    pub fn new(
        base_url: impl Into<String>,
        bearer_token: impl Into<String>,
        timeout_s: f64,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            client: HarnessClient::new(base_url, bearer_token, Duration::from_secs_f64(timeout_s)),
        })
    }

    /// Build a client from `HARNESS_BASE_URL` + `HARNESS_BEARER_TOKEN` env vars.
    /// `HARNESS_BASE_URL` is required; `HARNESS_BEARER_TOKEN` defaults to "".
    pub fn from_env() -> anyhow::Result<Self> {
        let base_url = std::env::var("HARNESS_BASE_URL").map_err(|_| {
            anyhow::anyhow!(
                "HARNESS_BASE_URL is not set; MemoryClient cannot reach the harness daemon."
            )
        })?;
        let bearer = std::env::var("HARNESS_BEARER_TOKEN").unwrap_or_default();
        Self::new(base_url, bearer, 10.0)
    }

    // -----------------------------------------------------------------------
    // Tool calls
    // -----------------------------------------------------------------------

    /// Call `memory.recall` and return the top-K hybrid-scored episodes.
    ///
    /// Request shape (mirrors Python exactly):
    ///   `{"bot_guid": <str>, "query_text": <str>, "top_k": <int>}`
    ///
    /// Response shape: `{"ok": true, "result": {"results": [<episode>,...]}}`
    pub async fn recall(
        &self,
        bot_guid: &str,
        query_text: &str,
        top_k: usize,
    ) -> anyhow::Result<Vec<RecalledEpisode>> {
        let body = serde_json::json!({
            "bot_guid": bot_guid,
            "query_text": query_text,
            "top_k": top_k,
        });
        let result = self
            .client
            .call("memory.recall", body)
            .await
            .map_err(|e| anyhow::anyhow!("MemoryClient.recall: {e}"))?;
        let results = result
            .get("results")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow::anyhow!("MemoryClient.recall: missing 'results' array"))?;

        results
            .iter()
            .map(|item| {
                Ok(RecalledEpisode {
                    episode_id: item["episode_id"]
                        .as_i64()
                        .ok_or_else(|| anyhow::anyhow!("missing episode_id"))?,
                    content_text: item["content_text"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("missing content_text"))?
                        .to_string(),
                    timestamp: item["timestamp"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("missing timestamp"))?
                        .to_string(),
                    salience_score: item["salience_score"]
                        .as_f64()
                        .ok_or_else(|| anyhow::anyhow!("missing salience_score"))?,
                    score: item["score"]
                        .as_f64()
                        .ok_or_else(|| anyhow::anyhow!("missing score"))?,
                })
            })
            .collect()
    }

    /// Call `memory.write` and return the newly-created `episode_id`.
    ///
    /// Request shape (mirrors Python exactly):
    ///   `{"bot_guid": str, "content_text": str, "episode_type": str,
    ///     "timestamp": str, "salience_score": f64, "entities": []}`
    ///
    /// Response shape: `{"ok": true, "result": {"episode_id": <int>}}`
    pub async fn write_episode(
        &self,
        bot_guid: &str,
        content_text: &str,
        episode_type: &str,
        timestamp_iso: &str,
        salience_score: f64,
    ) -> anyhow::Result<i64> {
        let body = serde_json::json!({
            "bot_guid": bot_guid,
            "content_text": content_text,
            "episode_type": episode_type,
            "timestamp": timestamp_iso,
            "salience_score": salience_score,
            "entities": [],
        });
        let result = self
            .client
            .call("memory.write", body)
            .await
            .map_err(|e| anyhow::anyhow!("MemoryClient.write_episode: {e}"))?;
        result["episode_id"]
            .as_i64()
            .ok_or_else(|| anyhow::anyhow!("MemoryClient.write_episode: missing episode_id"))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, routing::post};
    use std::sync::Arc;

    // -----------------------------------------------------------------------
    // Helper — spawn mock harness REST server
    // -----------------------------------------------------------------------

    async fn spawn_mock_harness(
        recall_response: serde_json::Value,
        write_response: serde_json::Value,
        captured_recall: Option<Arc<tokio::sync::Mutex<Option<serde_json::Value>>>>,
        captured_write: Option<Arc<tokio::sync::Mutex<Option<serde_json::Value>>>>,
    ) -> String {
        let app = Router::new()
            .route(
                "/v1/tools/memory.recall",
                post({
                    let resp = recall_response.clone();
                    let captured = captured_recall;
                    move |Json(body): Json<serde_json::Value>| {
                        let resp = resp.clone();
                        let captured = captured.clone();
                        async move {
                            if let Some(ref cap) = captured {
                                let mut guard = cap.lock().await;
                                *guard = Some(body);
                            }
                            Json(resp)
                        }
                    }
                }),
            )
            .route(
                "/v1/tools/memory.write",
                post({
                    let resp = write_response.clone();
                    let captured = captured_write;
                    move |Json(body): Json<serde_json::Value>| {
                        let resp = resp.clone();
                        let captured = captured.clone();
                        async move {
                            if let Some(ref cap) = captured {
                                let mut guard = cap.lock().await;
                                *guard = Some(body);
                            }
                            Json(resp)
                        }
                    }
                }),
            );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://127.0.0.1:{}", addr.port())
    }

    // -----------------------------------------------------------------------
    // recall() tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_recall_returns_episodes() {
        let response = serde_json::json!({
            "ok": true,
            "result": {
                "results": [
                    {
                        "episode_id": 42,
                        "content_text": "Arthas defeated Mal'Ganis at Stratholme",
                        "timestamp": "2026-01-01T00:00:00Z",
                        "salience_score": 0.9,
                        "score": 0.85
                    }
                ]
            }
        });
        let url = spawn_mock_harness(response, serde_json::json!({}), None, None).await;
        let client = MemoryClient::new(url, "", 5.0).unwrap();
        let episodes = client
            .recall("bot-123", "Arthas", 5)
            .await
            .expect("recall should succeed");
        assert_eq!(episodes.len(), 1);
        let ep = &episodes[0];
        assert_eq!(ep.episode_id, 42);
        assert_eq!(ep.content_text, "Arthas defeated Mal'Ganis at Stratholme");
        assert_eq!(ep.timestamp, "2026-01-01T00:00:00Z");
        assert!((ep.salience_score - 0.9).abs() < 1e-9);
        assert!((ep.score - 0.85).abs() < 1e-9);
    }

    #[tokio::test]
    async fn test_recall_empty_results() {
        let response = serde_json::json!({
            "ok": true,
            "result": { "results": [] }
        });
        let url = spawn_mock_harness(response, serde_json::json!({}), None, None).await;
        let client = MemoryClient::new(url, "", 5.0).unwrap();
        let episodes = client.recall("bot-1", "query", 5).await.unwrap();
        assert!(episodes.is_empty());
    }

    #[tokio::test]
    async fn test_recall_request_shape() {
        let captured: Arc<tokio::sync::Mutex<Option<serde_json::Value>>> =
            Arc::new(tokio::sync::Mutex::new(None));
        let response = serde_json::json!({
            "ok": true,
            "result": { "results": [] }
        });
        let url = spawn_mock_harness(response, serde_json::json!({}), Some(captured.clone()), None)
            .await;

        let client = MemoryClient::new(url, "", 5.0).unwrap();
        let _ = client.recall("bot-abc", "what happened", 3).await.unwrap();

        let guard = captured.lock().await;
        let body = guard.as_ref().unwrap();
        assert_eq!(body["bot_guid"], "bot-abc");
        assert_eq!(body["query_text"], "what happened");
        assert_eq!(body["top_k"], 3);
    }

    // -----------------------------------------------------------------------
    // write_episode() tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_write_episode_returns_episode_id() {
        let response = serde_json::json!({
            "ok": true,
            "result": { "episode_id": 99 }
        });
        let url = spawn_mock_harness(serde_json::json!({}), response, None, None).await;
        let client = MemoryClient::new(url, "", 5.0).unwrap();
        let id = client
            .write_episode("bot-1", "I killed a dragon", "combat", "2026-01-01T00:00:00Z", 0.7)
            .await
            .expect("write should succeed");
        assert_eq!(id, 99);
    }

    #[tokio::test]
    async fn test_write_episode_request_shape() {
        let captured: Arc<tokio::sync::Mutex<Option<serde_json::Value>>> =
            Arc::new(tokio::sync::Mutex::new(None));
        let response = serde_json::json!({
            "ok": true,
            "result": { "episode_id": 1 }
        });
        let url = spawn_mock_harness(serde_json::json!({}), response, None, Some(captured.clone()))
            .await;

        let client = MemoryClient::new(url, "", 5.0).unwrap();
        let _ = client
            .write_episode("bot-xyz", "text body", "action", "2026-01-01T12:00:00Z", 0.5)
            .await
            .unwrap();

        let guard = captured.lock().await;
        let body = guard.as_ref().unwrap();
        assert_eq!(body["bot_guid"], "bot-xyz");
        assert_eq!(body["content_text"], "text body");
        assert_eq!(body["episode_type"], "action");
        assert_eq!(body["timestamp"], "2026-01-01T12:00:00Z");
        assert!((body["salience_score"].as_f64().unwrap() - 0.5).abs() < 1e-9);
        // entities must be empty list (faithful to Python)
        assert_eq!(body["entities"], serde_json::json!([]));
    }

    // -----------------------------------------------------------------------
    // Error handling
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_recall_ok_false_returns_err() {
        let response = serde_json::json!({
            "ok": false,
            "error": "bot not found"
        });
        let url = spawn_mock_harness(response, serde_json::json!({}), None, None).await;
        let client = MemoryClient::new(url, "", 5.0).unwrap();
        let err = client.recall("bot-1", "q", 5).await.unwrap_err();
        // HarnessClient surfaces ok:false as a tool error; the anyhow context
        // wraps it so the error string mentions the tool or the error code.
        assert!(
            err.to_string().contains("bot not found") || err.to_string().contains("memory.recall"),
            "error message: {err}"
        );
    }

    // -----------------------------------------------------------------------
    // Bearer token
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_bearer_token_sent_in_header() {
        use axum::http::HeaderMap;
        let captured_headers: Arc<tokio::sync::Mutex<Option<HeaderMap>>> =
            Arc::new(tokio::sync::Mutex::new(None));

        let response = serde_json::json!({
            "ok": true,
            "result": { "results": [] }
        });

        // Spawn server that captures request headers
        let cap = captured_headers.clone();
        let app = Router::new().route(
            "/v1/tools/memory.recall",
            post(move |headers: HeaderMap, Json(_body): Json<serde_json::Value>| {
                let cap = cap.clone();
                let resp = response.clone();
                async move {
                    let mut guard = cap.lock().await;
                    *guard = Some(headers);
                    Json(resp)
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let url = format!("http://127.0.0.1:{}", addr.port());

        let client = MemoryClient::new(url, "my-secret", 5.0).unwrap();
        let _ = client.recall("bot-1", "q", 5).await.unwrap();

        let guard = captured_headers.lock().await;
        let headers = guard.as_ref().unwrap();
        let auth = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(auth, "Bearer my-secret");
    }

}
