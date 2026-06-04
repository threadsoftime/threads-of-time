//! Shared `{ok,result}` REST client for the harness daemon.
//!
//! POSTs `args` to `/v1/tools/<tool>` and unwraps `{"ok":true,"result":...}`.
//! The harness surfaces adapter rejections as NON-2xx (400 bad-args /
//! 409 validator-rejected / 422 executor-failed) with body
//! `{"ok":false,"error":"<code>","detail":"<human message>"}`. We MUST NOT
//! short-circuit on HTTP status — that would collapse every typed tool failure
//! into `HarnessError::Http` and discard `detail`. Instead we always read the
//! JSON envelope and map `ok:false`/non-2xx to a `HarnessError::Tool` that
//! preserves the human-readable detail.

use reqwest::Client;
use serde_json::Value;
use std::time::Duration;

#[derive(Clone)]
pub struct HarnessClient {
    client: Client,
    base_url: String,
    bearer: String,
}

#[derive(Debug)]
pub enum HarnessError {
    Http(reqwest::Error),
    Tool { tool: String, message: String },
    Shape(String),
}

impl std::fmt::Display for HarnessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HarnessError::Http(e) => write!(f, "http: {e}"),
            HarnessError::Tool { tool, message } => write!(f, "tool {tool}: {message}"),
            HarnessError::Shape(s) => write!(f, "shape: {s}"),
        }
    }
}
impl std::error::Error for HarnessError {}
impl From<reqwest::Error> for HarnessError {
    fn from(e: reqwest::Error) -> Self {
        HarnessError::Http(e)
    }
}

impl HarnessClient {
    /// Create a client with an explicit request timeout.
    pub fn new(base_url: impl Into<String>, bearer: impl Into<String>, timeout: Duration) -> Self {
        let client = Client::builder()
            .timeout(timeout)
            .build()
            .expect("reqwest client");
        Self { client, base_url: base_url.into(), bearer: bearer.into() }
    }

    /// POST /v1/tools/<tool> with `args`; unwrap `{"ok":true,"result":...}`.
    pub async fn call(&self, tool: &str, args: Value) -> Result<Value, HarnessError> {
        let url = format!("{}/v1/tools/{}", self.base_url.trim_end_matches('/'), tool);
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.bearer)
            .json(&args)
            .send()
            .await?;
        let status = resp.status();
        let body: Value = resp.json().await?;

        let ok = body.get("ok").and_then(Value::as_bool).unwrap_or(false);
        if !ok || !status.is_success() {
            let code = body.get("error").and_then(Value::as_str).unwrap_or("unknown");
            let detail = body.get("detail").and_then(Value::as_str).unwrap_or("");
            return Err(HarnessError::Tool {
                tool: tool.to_string(),
                message: if detail.is_empty() {
                    code.to_string()
                } else {
                    format!("{code}: {detail}")
                },
            });
        }
        body.get("result")
            .cloned()
            .ok_or_else(|| HarnessError::Shape(format!("{tool}: missing result")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::Path, http::StatusCode, routing::post, Json, Router};
    use serde_json::json;
    use std::time::Duration;

    // Mock harness: returns a 422 + {ok:false,...} for tool "fails",
    // and 200 + {ok:true,result:{...}} for anything else.
    async fn spawn_mock() -> String {
        let app = Router::new().route(
            "/v1/tools/:tool",
            post(|Path(tool): Path<String>, Json(_body): Json<Value>| async move {
                if tool == "fails" {
                    (
                        StatusCode::UNPROCESSABLE_ENTITY,
                        Json(json!({"ok": false, "error": "executor_failed", "detail": "bot not online"})),
                    )
                } else {
                    (StatusCode::OK, Json(json!({"ok": true, "result": {"echo": tool}})))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        format!("http://{addr}")
    }

    // Bug-lock (a): a 4xx carrying {ok:false,detail} surfaces the typed detail,
    // NOT a generic HTTP error. (This is the memory_client.rs error_for_status bug.)
    #[tokio::test]
    async fn non_2xx_envelope_preserves_detail() {
        let base = spawn_mock().await;
        let client = HarnessClient::new(base, "tok", Duration::from_secs(5));
        let err = client.call("fails", json!({})).await.unwrap_err();
        match err {
            HarnessError::Tool { tool, message } => {
                assert_eq!(tool, "fails");
                assert!(message.contains("executor_failed"), "got: {message}");
                assert!(message.contains("bot not online"), "detail lost: {message}");
            }
            other => panic!("expected Tool error preserving detail, got {other:?}"),
        }
    }

    // Bug-lock (b): the client carries a configured timeout and unwraps result on success.
    #[tokio::test]
    async fn success_unwraps_result() {
        let base = spawn_mock().await;
        let client = HarnessClient::new(base, "tok", Duration::from_secs(5));
        let result = client.call("obs.ping", json!({})).await.unwrap();
        assert_eq!(result, json!({"echo": "obs.ping"}));
    }
}
