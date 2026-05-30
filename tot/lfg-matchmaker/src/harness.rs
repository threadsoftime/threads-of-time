use crate::config::Dungeon;
use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Clone)]
pub struct Harness {
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

impl Harness {
    pub fn new(base_url: String, bearer: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("reqwest client");
        Self { client, base_url, bearer }
    }

    /// POST /v1/tools/<tool> with `args`; unwrap `{"ok":true,"result":...}`.
    ///
    /// The harness surfaces adapter rejections as NON-2xx (400 bad-args /
    /// 409 validator-rejected / 422 executor-failed) with body
    /// `{"ok":false,"error":"<code>","detail":"<human message>"}`. We must NOT
    /// short-circuit on status (e.g. `error_for_status`) — that would collapse
    /// every typed tool failure into `HarnessError::Http` and discard `detail`.
    /// Instead we always read the JSON envelope and map `ok:false`/non-2xx to a
    /// `HarnessError::Tool` that preserves the human-readable detail.
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
                message: if detail.is_empty() { code.to_string() } else { format!("{code}: {detail}") },
            });
        }
        body.get("result")
            .cloned()
            .ok_or_else(|| HarnessError::Shape(format!("{tool}: missing result")))
    }

    pub async fn invite_to_group(&self, bot: u64, target: u64) -> Result<Value, HarnessError> {
        self.call("bot.invite_to_group", json!({ "bot_guid": bot, "target_guid": target })).await
    }

    pub async fn accept_invite(&self, bot: u64) -> Result<Value, HarnessError> {
        self.call("bot.accept_invite", json!({ "bot_guid": bot })).await
    }

    /// Remove `bot` from its group. The data-plane adapter routes a
    /// CMSG_GROUP_DISBAND, which the server handles as leave-or-disband (the
    /// leader leaving tears down the whole group). Used to roll back a partially
    /// formed group on a mid-formation failure.
    pub async fn leave_group(&self, bot: u64) -> Result<Value, HarnessError> {
        self.call("bot.leave_group", json!({ "bot_guid": bot })).await
    }

    pub async fn enter_instance_direct(&self, bot: u64, dungeon: &Dungeon) -> Result<Value, HarnessError> {
        self.call(
            "bot.enter_instance",
            json!({
                "bot_guid": bot,
                "mode": "direct",
                "map_id": dungeon.map_id,
                "x": dungeon.x,
                "y": dungeon.y,
                "z": dungeon.z,
                "orientation": dungeon.o,
            }),
        )
        .await
    }

    #[allow(dead_code)] // scaffolding for the deferred obs pre-poll reconciliation step
    pub async fn get_group(&self, target: u64) -> Result<Value, HarnessError> {
        self.call("obs.get_group", json!({ "target_guid": target })).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::Path, http::StatusCode, routing::post, Json, Router};

    // Mock harness:
    //   - "bot.fail422" → HTTP 422 with {ok:false,error,detail} (real-wire shape:
    //     adapter rejections come back as NON-2xx + typed envelope).
    //   - "bot.fail"    → HTTP 200 with {ok:false,error} (legacy 200-body case).
    //   - anything else → HTTP 200 {ok:true, result:{tool, args}} (echo).
    async fn spawn_mock() -> String {
        async fn handler(
            Path(name): Path<String>,
            Json(args): Json<serde_json::Value>,
        ) -> (StatusCode, Json<serde_json::Value>) {
            if name == "bot.fail422" {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(serde_json::json!({
                        "ok": false,
                        "error": "executor_failed",
                        "detail": "target is already in a group",
                    })),
                );
            }
            if name == "bot.fail" {
                return (StatusCode::OK, Json(serde_json::json!({ "ok": false, "error": "boom" })));
            }
            (StatusCode::OK, Json(serde_json::json!({ "ok": true, "result": { "tool": name, "args": args } })))
        }
        let app = Router::new().route("/v1/tools/:name", post(handler));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn unwraps_ok_result() {
        let base = spawn_mock().await;
        let h = Harness::new(base, "tok".into());
        let res = h.invite_to_group(1, 2).await.unwrap();
        assert_eq!(res["tool"], "bot.invite_to_group");
        assert_eq!(res["args"]["bot_guid"], 1);
        assert_eq!(res["args"]["target_guid"], 2);
    }

    // T1 (C1): a 422 typed tool-failure MUST surface as HarnessError::Tool with
    // the human-readable `detail` preserved — not collapsed into HarnessError::Http.
    #[tokio::test]
    async fn surfaces_typed_tool_failure_on_422() {
        let base = spawn_mock().await;
        let h = Harness::new(base, "tok".into());
        let err = h.call("bot.fail422", serde_json::json!({})).await.unwrap_err();
        match err {
            HarnessError::Tool { tool, message } => {
                assert_eq!(tool, "bot.fail422");
                assert!(message.contains("already in a group"), "message lost detail: {message}");
            }
            other => panic!("expected Tool error, got {other:?}"),
        }
    }

    // Legacy shape: HTTP 200 with {ok:false} must also surface as a Tool error.
    #[tokio::test]
    async fn surfaces_tool_failure_on_200_ok_false() {
        let base = spawn_mock().await;
        let h = Harness::new(base, "tok".into());
        let err = h.call("bot.fail", serde_json::json!({})).await.unwrap_err();
        match err {
            HarnessError::Tool { tool, message } => {
                assert_eq!(tool, "bot.fail");
                assert_eq!(message, "boom");
            }
            other => panic!("expected Tool error, got {other:?}"),
        }
    }

    // T3: a transport-level failure (closed/unused port) surfaces as HarnessError::Http,
    // NOT a Tool error. Fast — no sleep, the connect refusal is immediate.
    #[tokio::test]
    async fn connection_error_surfaces_as_http() {
        let h = Harness::new("http://127.0.0.1:1".into(), "tok".into());
        let err = h.call("bot.invite_to_group", serde_json::json!({})).await.unwrap_err();
        match err {
            HarnessError::Http(_) => {}
            other => panic!("expected Http error, got {other:?}"),
        }
    }
}
