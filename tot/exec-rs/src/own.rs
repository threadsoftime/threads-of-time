//! Ownership client — thin wrapper over the `bot.set_ai_enabled` harness primitive.
//!
//! `set_ai_owned(client, bot, true)`  → sends `enabled:false` (new system claims the bot)
//! `set_ai_owned(client, bot, false)` → sends `enabled:true` + `reset_on_release:true` (release)

use serde::Deserialize;
use serde_json::json;
use thiserror::Error;
use tot_harness_client::{HarnessClient, HarnessError};

#[derive(Debug, Error)]
pub enum OwnError {
    #[error("harness: {0}")]
    Harness(#[from] HarnessError),
    #[error("bot.set_ai_enabled: unexpected response shape: {0}")]
    Shape(String),
}

#[derive(Debug, Deserialize)]
struct SetAiEnabledResult {
    owned: bool,
    #[allow(dead_code)]
    reset: bool,
}

#[derive(Debug, Deserialize)]
pub struct ReviveResult {
    pub revived: bool,
    #[allow(dead_code)]
    pub was_dead: bool,
}

/// Resurrect a dead bot in place via the harness `bot.revive` primitive (server-side
/// `ResurrectPlayer` + `SpawnCorpseBones`; never touches inventory — Finding #10 safe).
/// Idempotent: a live bot returns `{revived:false, was_dead:false}`. The bot stays
/// claimed throughout (no ownership release), so there is no #12 re-mount window.
pub async fn bot_revive(client: &HarnessClient, bot_guid: u64) -> Result<ReviveResult, OwnError> {
    let result = client.call("bot.revive", json!({ "bot_guid": bot_guid as i64 })).await?;
    serde_json::from_value(result).map_err(|e| OwnError::Shape(format!("bot.revive serde: {e}")))
}

/// Claim (`owned=true`) or release (`owned=false`) external ownership of a bot.
/// On release, requests `reset_on_release:true` for a clean handback.
pub async fn set_ai_owned(client: &HarnessClient, bot_guid: u64, owned: bool) -> Result<(), OwnError> {
    let mut body = json!({ "bot_guid": bot_guid as i64, "enabled": !owned });
    if !owned {
        body["reset_on_release"] = json!(true);
    }
    let result = client.call("bot.set_ai_enabled", body).await?;
    let r: SetAiEnabledResult =
        serde_json::from_value(result).map_err(|e| OwnError::Shape(format!("serde: {e}")))?;
    if r.owned != owned {
        return Err(OwnError::Shape(format!(
            "bot.set_ai_enabled returned owned:{} but we requested owned:{}", r.owned, owned)));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::Path as AxumPath, http::StatusCode, routing::post, Json, Router};
    use serde_json::json;
    use std::time::Duration;
    use tot_harness_client::HarnessClient;

    async fn spawn_mock<F>(tool_handler: F) -> String
    where F: Fn(String, serde_json::Value) -> serde_json::Value + Send + Sync + 'static {
        let handler_arc = std::sync::Arc::new(tool_handler);
        let handler = move |AxumPath(name): AxumPath<String>, Json(args): Json<serde_json::Value>| {
            let handler_arc = handler_arc.clone();
            async move { (StatusCode::OK, Json(json!({ "ok": true, "result": handler_arc(name, args) }))) }
        };
        let app = Router::new().route("/v1/tools/:name", post(handler));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }
    fn client(base: &str) -> HarnessClient { HarnessClient::new(base, "tok", Duration::from_secs(5)) }

    #[tokio::test]
    async fn set_ai_owned_true_sends_enabled_false() {
        let received = std::sync::Arc::new(std::sync::Mutex::new(serde_json::Value::Null));
        let recv = received.clone();
        let base = spawn_mock(move |_n, args| { *recv.lock().unwrap() = args; json!({ "owned": true, "reset": false }) }).await;
        set_ai_owned(&client(&base), 1003, true).await.unwrap();
        let body = received.lock().unwrap().clone();
        assert_eq!(body["bot_guid"], 1003_i64);
        assert_eq!(body["enabled"], false, "owned=true must send enabled=false");
        assert!(body.get("reset_on_release").is_none(), "no reset_on_release when claiming");
    }

    #[tokio::test]
    async fn set_ai_owned_false_sends_enabled_true_with_reset() {
        let received = std::sync::Arc::new(std::sync::Mutex::new(serde_json::Value::Null));
        let recv = received.clone();
        let base = spawn_mock(move |_n, args| { *recv.lock().unwrap() = args; json!({ "owned": false, "reset": true }) }).await;
        set_ai_owned(&client(&base), 1003, false).await.unwrap();
        let body = received.lock().unwrap().clone();
        assert_eq!(body["enabled"], true, "owned=false must send enabled=true");
        assert_eq!(body["reset_on_release"], true, "reset_on_release true when releasing");
    }

    #[tokio::test]
    async fn set_ai_owned_wraps_harness_error() {
        let c = HarnessClient::new("http://127.0.0.1:1", "tok", Duration::from_secs(1));
        let err = set_ai_owned(&c, 42, true).await.unwrap_err();
        match err { OwnError::Harness(_) => {}, other => panic!("expected Harness, got {other:?}") }
    }

    #[tokio::test]
    async fn set_ai_owned_returns_shape_error_on_bad_response() {
        let base = spawn_mock(|_n, _a| json!({ "unexpected_key": "x" })).await;
        let err = set_ai_owned(&client(&base), 42, true).await.unwrap_err();
        match err { OwnError::Shape(_) => {}, other => panic!("expected Shape, got {other:?}") }
    }

    #[tokio::test]
    async fn set_ai_owned_shape_error_on_state_mismatch() {
        let base = spawn_mock(|_n, _a| json!({ "owned": false, "reset": false })).await; // we'll ask owned=true
        let err = set_ai_owned(&client(&base), 42, true).await.unwrap_err();
        match err { OwnError::Shape(msg) => assert!(msg.contains("owned:false")), other => panic!("expected Shape, got {other:?}") }
    }

    #[tokio::test]
    async fn bot_revive_sends_guid_and_parses_result() {
        let received = std::sync::Arc::new(std::sync::Mutex::new(serde_json::Value::Null));
        let recv = received.clone();
        let base = spawn_mock(move |_n, args| { *recv.lock().unwrap() = args; json!({ "revived": true, "was_dead": true }) }).await;
        let r = bot_revive(&client(&base), 1323).await.unwrap();
        assert!(r.revived);
        assert_eq!(received.lock().unwrap()["bot_guid"], 1323_i64);
    }
}
