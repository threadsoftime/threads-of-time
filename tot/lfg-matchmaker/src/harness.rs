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
    pub async fn call(&self, tool: &str, args: Value) -> Result<Value, HarnessError> {
        let url = format!("{}/v1/tools/{}", self.base_url.trim_end_matches('/'), tool);
        let body: Value = self
            .client
            .post(&url)
            .bearer_auth(&self.bearer)
            .json(&args)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let ok = body.get("ok").and_then(Value::as_bool).unwrap_or(false);
        if !ok {
            let message = body
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            return Err(HarnessError::Tool { tool: tool.to_string(), message });
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

    pub async fn enter_instance_direct(
        &self,
        bot: u64,
        map_id: u32,
        x: f64,
        y: f64,
        z: f64,
        o: f64,
    ) -> Result<Value, HarnessError> {
        self.call(
            "bot.enter_instance",
            json!({ "bot_guid": bot, "mode": "direct", "map_id": map_id, "x": x, "y": y, "z": z, "orientation": o }),
        )
        .await
    }

    pub async fn get_group(&self, target: u64) -> Result<Value, HarnessError> {
        self.call("obs.get_group", json!({ "target_guid": target })).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::Path, routing::post, Json, Router};

    // Mock harness: always wraps the echoed tool name in {ok:true, result:{...}},
    // except for a tool literally named "bot.fail" which returns {ok:false}.
    async fn spawn_mock() -> String {
        async fn handler(Path(name): Path<String>, Json(args): Json<serde_json::Value>) -> Json<serde_json::Value> {
            if name == "bot.fail" {
                return Json(serde_json::json!({ "ok": false, "error": "boom" }));
            }
            Json(serde_json::json!({ "ok": true, "result": { "tool": name, "args": args } }))
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

    #[tokio::test]
    async fn surfaces_tool_failure() {
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
}
