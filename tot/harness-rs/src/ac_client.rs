//! Async HTTP client to the mod-harness-bridge AC module.
//!
//! Port of `harness_daemon/ac_client.py`. Forwards parsed tool calls to
//! `POST /dispatch` on the AC bridge. Propagates the daemon's request_id
//! and identity via `X-Daemon-*` headers.

use std::time::{Duration, Instant};

use reqwest::{Client, ClientBuilder};
use serde_json::{json, Value};
use thiserror::Error;

// ── ACClientError ─────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum ACClientError {
    #[error("AC bridge transport error: {0}")]
    Transport(#[from] reqwest::Error),
}

// ── ACResponse ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ACResponse {
    pub status:        u16,
    pub body:          Value,
    pub ac_latency_ms: u64,
}

// ── ACClient ──────────────────────────────────────────────────────────────────

pub struct ACClient {
    base_url: String,
    client:   Client,
}

impl ACClient {
    /// Create an `ACClient`.
    ///
    /// `base_url` has any trailing `/` stripped.
    /// `timeout_s` sets the request timeout; pass `3.0` for the production
    /// default (mirrors `ac_client.py`).
    pub fn new(base_url: &str, timeout_s: f64) -> Self {
        let timeout = Duration::from_secs_f64(timeout_s);
        let client = ClientBuilder::new()
            .timeout(timeout)
            .build()
            .expect("reqwest client build failed");
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client,
        }
    }

    /// POST `{base_url}/dispatch` with `{"tool": tool, "args": args}`.
    ///
    /// For `gm.run_console` ONLY: clones args and injects
    /// `"request_id": request_id` into the forwarded args so the AC bridge
    /// can use the daemon-generated id as its console-capture key (spec
    /// §4.2, v1.2). The caller's `args` value is NEVER mutated.
    ///
    /// On transport error:       returns `Err(ACClientError)`
    /// On non-JSON response:     returns `Ok` with body
    ///     `{"ok":false,"error":"non_json_response","raw":<text up to 200 chars>}`
    pub async fn dispatch(
        &self,
        tool:       &str,
        args:       Value,
        request_id: &str,
        identity:   &str,
    ) -> Result<ACResponse, ACClientError> {
        // V1.2: inject request_id into forwarded args for gm.run_console only.
        let forwarded_args = if tool == "gm.run_console" {
            let mut a = match args {
                Value::Object(m) => m,
                other            => {
                    // args was not an Object (unusual) — wrap into one
                    let mut m = serde_json::Map::new();
                    m.insert("_args".to_string(), other);
                    m
                }
            };
            a.insert("request_id".to_string(), Value::String(request_id.to_string()));
            Value::Object(a)
        } else {
            args
        };

        let body = json!({
            "tool": tool,
            "args": forwarded_args,
        });

        let t0 = Instant::now();
        let resp = self
            .client
            .post(format!("{}/dispatch", self.base_url))
            .header("X-Daemon-Request-Id", request_id)
            .header("X-Daemon-Identity",   identity)
            .header("Content-Type",        "application/json")
            .json(&body)
            .send()
            .await?;

        let latency_ms = t0.elapsed().as_millis() as u64;
        let status     = resp.status().as_u16();

        // Attempt to parse the response body as JSON.
        let resp_body = match resp.text().await {
            Err(_) => json!({"ok": false, "error": "non_json_response", "raw": ""}),
            Ok(text) => {
                match serde_json::from_str::<Value>(&text) {
                    Ok(v)  => v,
                    Err(_) => {
                        // Mirror ac_client.py:75 — truncate raw to 200 chars.
                        // Be careful: Python's [:200] is Unicode code-point
                        // count, not bytes. Use chars().take(200).
                        let raw: String = text.chars().take(200).collect();
                        json!({
                            "ok":    false,
                            "error": "non_json_response",
                            "raw":   raw,
                        })
                    }
                }
            }
        };

        Ok(ACResponse {
            status,
            body: resp_body,
            ac_latency_ms: latency_ms,
        })
    }

    /// GET `{base_url}/health` → true iff the server responds with HTTP 200.
    pub async fn health(&self) -> bool {
        match self
            .client
            .get(format!("{}/health", self.base_url))
            .send()
            .await
        {
            Ok(r)  => r.status().as_u16() == 200,
            Err(_) => false,
        }
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{spawn_mock_ac, spawn_mock_ac_with_config};
    use serde_json::json;

    // ── helpers ────────────────────────────────────────────────────────────

    fn make_client(base_url: &str) -> ACClient {
        ACClient::new(base_url, 3.0)
    }

    // ── dispatch posts correct body and headers ────────────────────────────

    #[tokio::test]
    async fn dispatch_posts_tool_and_args() {
        let mock = spawn_mock_ac().await;
        let client = make_client(mock.base_url());

        let resp = client
            .dispatch("obs.ping", json!({}), "req-001", "gm.tbrack")
            .await
            .unwrap();

        assert_eq!(resp.status, 200);
        let rec = mock.last_request().unwrap();
        assert_eq!(rec.body["tool"], "obs.ping");
        assert_eq!(rec.body["args"], json!({}));
    }

    #[tokio::test]
    async fn dispatch_sends_x_daemon_headers() {
        let mock = spawn_mock_ac().await;
        let client = make_client(mock.base_url());

        client
            .dispatch("obs.ping", json!({}), "req-xyz", "identity.test")
            .await
            .unwrap();

        let rec = mock.last_request().unwrap();
        let header_val = |name: &str| -> Option<String> {
            rec.headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.clone())
        };
        assert_eq!(header_val("x-daemon-request-id").as_deref(), Some("req-xyz"));
        assert_eq!(header_val("x-daemon-identity").as_deref(), Some("identity.test"));
    }

    // ── gm.run_console injects request_id ─────────────────────────────────

    #[tokio::test]
    async fn gm_run_console_injects_request_id_into_forwarded_args() {
        let mock = spawn_mock_ac().await;
        let client = make_client(mock.base_url());

        let caller_args = json!({"command": ".lookup item 19019"});
        client
            .dispatch("gm.run_console", caller_args.clone(), "req-inject", "gm.tbrack")
            .await
            .unwrap();

        // Forwarded body should have args.request_id == "req-inject"
        let rec = mock.last_request().unwrap();
        assert_eq!(rec.body["tool"], "gm.run_console");
        assert_eq!(rec.body["args"]["request_id"], "req-inject");
        assert_eq!(rec.body["args"]["command"], ".lookup item 19019");

        // Caller's original args must be unmutated
        assert_eq!(caller_args, json!({"command": ".lookup item 19019"}));
    }

    #[tokio::test]
    async fn non_gm_run_console_does_not_inject_request_id() {
        let mock = spawn_mock_ac().await;
        let client = make_client(mock.base_url());

        let caller_args = json!({"target_guid": 12345});
        client
            .dispatch("obs.ping", caller_args.clone(), "req-no-inject", "gm.tbrack")
            .await
            .unwrap();

        let rec = mock.last_request().unwrap();
        assert_eq!(rec.body["args"]["request_id"], Value::Null);
    }

    // ── unreachable AC returns ACClientError ───────────────────────────────

    #[tokio::test]
    async fn unreachable_ac_returns_error() {
        // Port 1 is typically closed / refused — reliably unreachable
        let client = ACClient::new("http://127.0.0.1:1", 1.0);
        let result = client
            .dispatch("obs.ping", json!({}), "req-dead", "test")
            .await;
        assert!(result.is_err(), "expected ACClientError for dead port");
    }

    // ── non-JSON response is wrapped ───────────────────────────────────────

    #[tokio::test]
    async fn non_json_response_produces_envelope() {
        use crate::test_support::MockConfig;

        // Configure the mock to return a plain-text body
        let mock = spawn_mock_ac_with_config(MockConfig {
            dispatch_status: 200,
            // The mock server sends JSON, so we need a different approach:
            // We'll use a raw text response body by returning an invalid JSON
            // that serde would reject. But our mock always returns Json<Value>.
            // Instead we verify the envelope using a direct reqwest call to
            // a server that returns non-JSON.
            dispatch_body:   json!({"ok": true}), // placeholder
            health_status:   200,
        })
        .await;

        // Build a tiny one-shot axum server that returns plain text:
        let text_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let text_port = text_listener.local_addr().unwrap().port();
        // Bind happens before spawn — the OS socket is already listening, so no
        // sleep is needed before connecting (new connections queue in the kernel).
        tokio::spawn(async move {
            let text_app = axum::Router::new().route(
                "/dispatch",
                axum::routing::post(|| async {
                    axum::response::Response::builder()
                        .status(200)
                        .header("Content-Type", "text/plain")
                        .body(axum::body::Body::from("not json at all"))
                        .unwrap()
                }),
            );
            axum::serve(text_listener, text_app).await.unwrap();
        });

        let client = ACClient::new(&format!("http://127.0.0.1:{text_port}"), 3.0);
        let resp = client
            .dispatch("obs.ping", json!({}), "req-text", "test")
            .await
            .unwrap();

        assert_eq!(resp.body["error"], "non_json_response");
        assert_eq!(resp.body["ok"], false);
        assert!(resp.body["raw"].as_str().is_some());
        assert!(resp.body["raw"].as_str().unwrap().contains("not json"));

        drop(mock); // keep mock alive during test
    }

    // ── non_json_response raw is truncated to 200 chars ───────────────────

    #[tokio::test]
    async fn non_json_response_raw_truncated_to_200() {
        let long_text = "x".repeat(500);
        let long_text_clone = long_text.clone();

        let text_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let text_port = text_listener.local_addr().unwrap().port();
        // Bind happens before spawn — the OS socket is already listening, so no
        // sleep is needed before connecting (new connections queue in the kernel).
        tokio::spawn(async move {
            let text_app = axum::Router::new().route(
                "/dispatch",
                axum::routing::post(move || {
                    let body = long_text_clone.clone();
                    async move {
                        axum::response::Response::builder()
                            .status(200)
                            .header("Content-Type", "text/plain")
                            .body(axum::body::Body::from(body))
                            .unwrap()
                    }
                }),
            );
            axum::serve(text_listener, text_app).await.unwrap();
        });

        let client = ACClient::new(&format!("http://127.0.0.1:{text_port}"), 3.0);
        let resp = client
            .dispatch("obs.ping", json!({}), "req-trunc", "test")
            .await
            .unwrap();

        let raw = resp.body["raw"].as_str().unwrap();
        assert_eq!(raw.chars().count(), 200, "raw should be truncated to 200 chars");
    }

    // ── health ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn health_returns_true_when_200() {
        let mock = spawn_mock_ac().await;
        let client = make_client(mock.base_url());
        assert!(client.health().await);
    }

    #[tokio::test]
    async fn health_returns_false_when_unreachable() {
        let client = ACClient::new("http://127.0.0.1:1", 1.0);
        assert!(!client.health().await);
    }
}
