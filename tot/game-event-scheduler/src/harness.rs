//! `{ok,result}` HTTP client for the harness wire.
//!
//! All harness calls go through [`Harness::call`], which POST-s to
//! `POST <base_url>/v1/tools/<tool_name>` with a JSON body, reads the envelope,
//! and maps errors to [`HarnessError`].
//!
//! Two typed methods sit on top of `call`:
//! - [`Harness::game_events`] — fetches `obs.game_events`, deserializes to [`GroundTruth`].
//! - [`Harness::query_game_events`] — fetches `obs.query_db` with the
//!   `game_event_all` template, deserializes each row as [`SqlGameEventRow`] → [`GameEventInput`].
//!
//! # Wire contract (harness v1)
//!
//! ```text
//! POST /v1/tools/<tool_name>
//! Authorization: Bearer <token>
//! Content-Type: application/json
//!
//! { <args> }
//! ```
//!
//! Success response (any 2xx):
//! ```json
//! { "ok": true, "result": <payload> }
//! ```
//!
//! Tool-level failure (any 4xx/5xx, or `ok:false` on a 2xx):
//! ```json
//! { "ok": false, "error": "<code>", "detail": "<human string>" }
//! ```
//!
//! [`HarnessError::Tool`] preserves the `detail` string so callers can log it.

use reqwest::Client;
use serde_json::{json, Value};

use crate::events::{GroundTruth, SqlGameEventRow};
use crate::resolve::GameEventInput;

// ── Error type ────────────────────────────────────────────────────────────────

/// Errors surfaced by the harness client.
#[derive(Debug)]
pub enum HarnessError {
    /// The HTTP transport failed (connection refused, timeout, DNS, TLS…).
    Http(reqwest::Error),
    /// The harness returned `ok:false` (tool-level error).
    /// The `detail` field from the JSON envelope is preserved.
    Tool { detail: String },
    /// The JSON response shape was unexpected (parse/type mismatch).
    Shape(String),
}

impl std::fmt::Display for HarnessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http(e) => write!(f, "harness HTTP error: {e}"),
            Self::Tool { detail } => write!(f, "harness tool error: {detail}"),
            Self::Shape(msg) => write!(f, "harness shape error: {msg}"),
        }
    }
}

impl std::error::Error for HarnessError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Http(e) => Some(e),
            _ => None,
        }
    }
}

impl From<reqwest::Error> for HarnessError {
    fn from(e: reqwest::Error) -> Self {
        Self::Http(e)
    }
}

// ── Harness client ────────────────────────────────────────────────────────────

/// HTTP client that speaks the `{ok,result}` harness wire protocol.
///
/// Construct with [`Harness::new`]; the underlying [`reqwest::Client`] is shared
/// (connection pool reused across calls).
#[derive(Clone)]
pub struct Harness {
    client: Client,
    base_url: String,
    bearer: String,
}

impl Harness {
    /// Create a new client.
    ///
    /// `base_url` — e.g. `"http://192.168.1.3:8099"` (no trailing slash).
    /// `bearer`   — the raw token, WITHOUT the `Bearer ` prefix.
    pub fn new(base_url: impl Into<String>, bearer: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.into(),
            bearer: bearer.into(),
        }
    }

    /// Call a harness tool and return the `result` value on success.
    ///
    /// `tool`  — e.g. `"obs.game_events"`.
    /// `args`  — a JSON object that is the POST body (use `json!({})` for empty).
    ///
    /// Maps harness failures to typed [`HarnessError`] variants without
    /// short-circuiting on HTTP status — reads the JSON envelope first and
    /// maps `ok:false` to [`HarnessError::Tool`] (preserving `detail`).
    /// Non-2xx responses that are NOT JSON are surfaced as [`HarnessError::Shape`].
    pub async fn call(&self, tool: &str, args: Value) -> Result<Value, HarnessError> {
        let url = format!("{}/v1/tools/{}", self.base_url, tool);
        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.bearer))
            .json(&args)
            .send()
            .await?;

        // Read the body regardless of HTTP status; the envelope determines success.
        let body: Value = resp
            .json()
            .await
            .map_err(|e| HarnessError::Shape(format!("failed to parse response JSON: {e}")))?;

        let ok = body
            .get("ok")
            .and_then(Value::as_bool)
            .ok_or_else(|| HarnessError::Shape("response missing boolean 'ok' field".to_string()))?;

        if !ok {
            let detail = body
                .get("detail")
                .and_then(Value::as_str)
                .unwrap_or("<no detail>")
                .to_string();
            return Err(HarnessError::Tool { detail });
        }

        body.get("result")
            .cloned()
            .ok_or_else(|| HarnessError::Shape("response missing 'result' field".to_string()))
    }

    // ── Typed methods ─────────────────────────────────────────────────────────

    /// Fetch `obs.game_events` and deserialize to [`GroundTruth`].
    ///
    /// Returns the full server-resolved snapshot: events, active set, holidays
    /// dump, server gametime, and resolve reference.
    pub async fn game_events(&self) -> Result<GroundTruth, HarnessError> {
        let result = self.call("obs.game_events", json!({})).await?;
        serde_json::from_value(result)
            .map_err(|e| HarnessError::Shape(format!("obs.game_events deserialize: {e}")))
    }

    /// Fetch `obs.query_db` with the `game_event_all` template and deserialize
    /// each row to [`GameEventInput`].
    ///
    /// The raw SQL rows are deserialized as [`SqlGameEventRow`] then mapped via
    /// `From` to [`GameEventInput`] (null timestamps become 0).
    pub async fn query_game_events(&self) -> Result<Vec<GameEventInput>, HarnessError> {
        let result = self
            .call(
                "obs.query_db",
                json!({"template_name": "game_event_all", "params": {}}),
            )
            .await?;
        let rows: Vec<SqlGameEventRow> = serde_json::from_value(result)
            .map_err(|e| HarnessError::Shape(format!("obs.query_db deserialize: {e}")))?;
        Ok(rows.into_iter().map(GameEventInput::from).collect())
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, extract::State, routing::post, Json};
    use std::sync::Arc;
    use tokio::net::TcpListener;

    // ── Mock server helpers ───────────────────────────────────────────────────

    /// Fixed `obs.game_events` response matching the REAL live payload shape
    /// (2026-05-31 Heimdal verification).
    const GAME_EVENTS_RESULT: &str = r#"{
        "events":[
            {"entry":1,"start":1782000000,"end":1843316906,"occurence":525600,"length":20160,
             "holiday":341,"holiday_stage":1,"is_active":false,"next_start":0,"state":0}
        ],
        "active_event_list":[16,20,62],
        "holidays":[
            {"holiday_id":62,"calendar_filter_type":-1,"looping":0,"region":1,
             "date":[425781248,442560512,459325440,476106752,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],
             "duration":[18,0,0,0,0,0,0,0,0,0]}
        ],
        "server_gametime":1780245388,
        "resolve_reference_unixtime":1780244906,
        "server_tz_offset_secs":0
    }"#;

    /// Fixed `obs.query_db` response — one row with null `start_time`.
    const QUERY_DB_RESULT: &str = r#"[
        {"eventEntry":1,"start_time":null,"end_time":1843316906,"occurence":525600,"length":20160,
         "holiday":341,"holidayStage":1,"description":"d","world_event":0,"announce":2}
    ]"#;

    /// State shared by the mock axum handlers.
    #[derive(Clone)]
    struct MockState {
        /// JSON string to return as `result` for `obs.game_events`.
        game_events_result: Arc<str>,
        /// JSON string to return as `result` for `obs.query_db`.
        query_db_result: Arc<str>,
    }

    /// Handler: POST /v1/tools/obs.game_events
    async fn handle_game_events(
        State(s): State<MockState>,
    ) -> Json<Value> {
        let result: Value = serde_json::from_str(&s.game_events_result).unwrap();
        Json(json!({"ok": true, "result": result}))
    }

    /// Handler: POST /v1/tools/obs.query_db
    async fn handle_query_db(
        State(s): State<MockState>,
    ) -> Json<Value> {
        let result: Value = serde_json::from_str(&s.query_db_result).unwrap();
        Json(json!({"ok": true, "result": result}))
    }

    /// Handler: POST /v1/tools/fail.tool — returns `{ok:false}` with detail.
    async fn handle_fail() -> (axum::http::StatusCode, Json<Value>) {
        (
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "ok": false,
                "error": "validation_error",
                "detail": "missing required param"
            })),
        )
    }

    /// Handler: POST /v1/tools/shape.bad — returns JSON without `ok` field.
    async fn handle_shape_bad() -> Json<Value> {
        Json(json!({"garbage": true}))
    }

    /// Spawn a local axum mock server; return its base URL.
    async fn spawn_mock() -> String {
        let state = MockState {
            game_events_result: GAME_EVENTS_RESULT.into(),
            query_db_result: QUERY_DB_RESULT.into(),
        };
        let app = Router::new()
            .route("/v1/tools/obs.game_events", post(handle_game_events))
            .route("/v1/tools/obs.query_db", post(handle_query_db))
            .route("/v1/tools/fail.tool", post(handle_fail))
            .route("/v1/tools/shape.bad", post(handle_shape_bad))
            .with_state(state);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{}", addr)
    }

    // ── game_events() tests ───────────────────────────────────────────────────

    #[tokio::test]
    async fn game_events_deserializes_event_count() {
        let base_url = spawn_mock().await;
        let h = Harness::new(base_url, "test-token");
        let gt = h.game_events().await.expect("game_events should succeed");
        assert_eq!(gt.events.len(), 1);
    }

    #[tokio::test]
    async fn game_events_event_fields() {
        let base_url = spawn_mock().await;
        let h = Harness::new(base_url, "test-token");
        let gt = h.game_events().await.unwrap();
        let ev = &gt.events[0];
        assert_eq!(ev.entry, 1);
        assert_eq!(ev.start, 1782000000);
        assert_eq!(ev.end, 1843316906);
        assert_eq!(ev.occurence, 525600);
        assert_eq!(ev.length, 20160);
        assert_eq!(ev.holiday, 341);
        assert_eq!(ev.holiday_stage, 1);
        assert!(!ev.is_active);
        assert_eq!(ev.next_start, 0);
    }

    #[tokio::test]
    async fn game_events_active_event_list() {
        let base_url = spawn_mock().await;
        let h = Harness::new(base_url, "test-token");
        let gt = h.game_events().await.unwrap();
        assert_eq!(gt.active_event_list, vec![16u16, 20, 62]);
    }

    #[tokio::test]
    async fn game_events_server_timestamps() {
        let base_url = spawn_mock().await;
        let h = Harness::new(base_url, "test-token");
        let gt = h.game_events().await.unwrap();
        assert_eq!(gt.server_gametime, 1780245388);
        assert_eq!(gt.resolve_reference_unixtime, 1780244906);
        assert_eq!(gt.server_tz_offset_secs, 0);
    }

    #[tokio::test]
    async fn game_events_holidays_count() {
        let base_url = spawn_mock().await;
        let h = Harness::new(base_url, "test-token");
        let gt = h.game_events().await.unwrap();
        assert_eq!(gt.holidays.len(), 1);
    }

    // ── query_game_events() tests ─────────────────────────────────────────────

    #[tokio::test]
    async fn query_game_events_returns_game_event_inputs() {
        let base_url = spawn_mock().await;
        let h = Harness::new(base_url, "test-token");
        let inputs = h.query_game_events().await.expect("query_game_events should succeed");
        assert_eq!(inputs.len(), 1);
    }

    #[tokio::test]
    async fn query_game_events_null_start_time_becomes_zero() {
        let base_url = spawn_mock().await;
        let h = Harness::new(base_url, "test-token");
        let inputs = h.query_game_events().await.unwrap();
        // The SQL row has start_time: null → GameEventInput.start_time = 0
        assert_eq!(inputs[0].start_time, 0, "null start_time should map to 0");
    }

    #[tokio::test]
    async fn query_game_events_fields() {
        let base_url = spawn_mock().await;
        let h = Harness::new(base_url, "test-token");
        let inputs = h.query_game_events().await.unwrap();
        let inp = &inputs[0];
        assert_eq!(inp.entry, 1);
        assert_eq!(inp.end_time, 1843316906);
        assert_eq!(inp.occurence, 525600);
        assert_eq!(inp.length, 20160);
        assert_eq!(inp.holiday, 341);
        assert_eq!(inp.holiday_stage, 1);
    }

    // ── HarnessError::Tool (422 ok:false) ─────────────────────────────────────

    #[tokio::test]
    async fn tool_error_surfaces_detail() {
        let base_url = spawn_mock().await;
        let h = Harness::new(base_url, "test-token");
        let err = h.call("fail.tool", json!({})).await.expect_err("should be an error");
        match err {
            HarnessError::Tool { detail } => {
                assert_eq!(detail, "missing required param");
            }
            other => panic!("expected HarnessError::Tool, got {other:?}"),
        }
    }

    // ── HarnessError::Http (connection refused) ───────────────────────────────

    #[tokio::test]
    async fn http_error_on_connection_refused() {
        // Port 1 is non-routable / refused on all platforms in CI.
        let h = Harness::new("http://127.0.0.1:1", "test-token");
        let err = h.call("obs.game_events", json!({})).await.expect_err("should fail");
        assert!(
            matches!(err, HarnessError::Http(_)),
            "expected HarnessError::Http, got {err:?}"
        );
    }

    // ── HarnessError::Shape (missing ok field) ────────────────────────────────

    #[tokio::test]
    async fn shape_error_on_missing_ok_field() {
        let base_url = spawn_mock().await;
        let h = Harness::new(base_url, "test-token");
        let err = h.call("shape.bad", json!({})).await.expect_err("should be a shape error");
        assert!(
            matches!(err, HarnessError::Shape(_)),
            "expected HarnessError::Shape, got {err:?}"
        );
    }
}
