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

use serde_json::{json, Value};
use std::time::Duration;
use tot_harness_client::HarnessClient;
pub use tot_harness_client::HarnessError;

use crate::events::{GroundTruth, QueryDbResult};
use crate::resolve::GameEventInput;

// ── Harness client ────────────────────────────────────────────────────────────

/// HTTP client that speaks the `{ok,result}` harness wire protocol.
///
/// Construct with [`Harness::new`]; the underlying [`HarnessClient`] is shared
/// (connection pool reused across calls).
#[derive(Clone)]
pub struct Harness(HarnessClient);

impl Harness {
    /// Create a new client.
    ///
    /// `base_url` — e.g. `"http://192.168.1.3:8099"` (no trailing slash).
    /// `bearer`   — the raw token, WITHOUT the `Bearer ` prefix.
    pub fn new(base_url: impl Into<String>, bearer: impl Into<String>) -> Self {
        Harness(HarnessClient::new(base_url, bearer, Duration::from_secs(10)))
    }

    pub async fn call(&self, tool: &str, args: Value) -> Result<Value, HarnessError> {
        self.0.call(tool, args).await
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

    /// Call `event.start` to activate a game event on the server.
    ///
    /// `event_id` — the entry ID of the game event to start.
    ///
    /// Returns the raw `result` value from the harness envelope on success.
    /// The caller may inspect `result["is_active_now"]` if needed; the drive step
    /// only needs to know the call succeeded (no error).
    ///
    /// Wire: `POST /v1/tools/event.start` with `{ "event_id": <event_id> }`.
    /// Expected result: `{ "started": true, "event_id": <event_id>, "is_active_now": <bool> }`.
    pub async fn event_start(&self, event_id: u16) -> Result<Value, HarnessError> {
        self.call("event.start", json!({"event_id": event_id})).await
    }

    /// Call `event.stop` to deactivate a game event on the server.
    ///
    /// `event_id` — the entry ID of the game event to stop.
    ///
    /// Returns the raw `result` value from the harness envelope on success.
    ///
    /// Wire: `POST /v1/tools/event.stop` with `{ "event_id": <event_id> }`.
    /// Expected result: `{ "stopped": true, "event_id": <event_id>, "is_active_now": <bool> }`.
    pub async fn event_stop(&self, event_id: u16) -> Result<Value, HarnessError> {
        self.call("event.stop", json!({"event_id": event_id})).await
    }

    /// Fetch `obs.query_db` with the `game_event_all` template and deserialize
    /// each row to [`GameEventInput`].
    ///
    /// The real live `obs.query_db` result is an OBJECT `{ "row_count": <int>, "rows": [...] }`,
    /// NOT a bare array. We deserialize via [`QueryDbResult`] (which models only `rows`
    /// and ignores `row_count`) then map each [`SqlGameEventRow`] via `From` to
    /// [`GameEventInput`] (null timestamps become 0).
    pub async fn query_game_events(&self) -> Result<Vec<GameEventInput>, HarnessError> {
        let result = self
            .call(
                "obs.query_db",
                json!({"template_name": "game_event_all", "params": {}}),
            )
            .await?;
        let db_result: QueryDbResult = serde_json::from_value(result)
            .map_err(|e| HarnessError::Shape(format!("obs.query_db deserialize: {e}")))?;
        Ok(db_result.rows.into_iter().map(GameEventInput::from).collect())
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

    /// Fixed `obs.query_db` response — the REAL live envelope shape (2026-05-31 verified).
    ///
    /// The real `result` is an object `{ "row_count": <int>, "rows": [...] }`, NOT a bare array.
    /// Row has BOTH `start_time: null` AND `end_time: null` (holiday row, eventEntry:1).
    const QUERY_DB_RESULT: &str = r#"{
        "row_count": 1,
        "rows": [
            {"eventEntry":1,"start_time":null,"end_time":null,"occurence":525600,"length":20160,
             "holiday":341,"holidayStage":1,"description":"d","world_event":0,"announce":2}
        ]
    }"#;

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

    /// Handler: POST /v1/tools/event.start — echoes the event_id back in the result.
    async fn handle_event_start(Json(body): Json<Value>) -> Json<Value> {
        let event_id = body.get("event_id").and_then(Value::as_u64).unwrap_or(0) as u16;
        Json(json!({
            "ok": true,
            "result": {
                "started": true,
                "event_id": event_id,
                "is_active_now": true
            }
        }))
    }

    /// Handler: POST /v1/tools/event.stop — echoes the event_id back in the result.
    async fn handle_event_stop(Json(body): Json<Value>) -> Json<Value> {
        let event_id = body.get("event_id").and_then(Value::as_u64).unwrap_or(0) as u16;
        Json(json!({
            "ok": true,
            "result": {
                "stopped": true,
                "event_id": event_id,
                "is_active_now": false
            }
        }))
    }

    /// Handler: POST /v1/tools/event.start — returns `{ok:false}` for error path testing.
    async fn handle_event_start_fail() -> (axum::http::StatusCode, Json<Value>) {
        (
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "ok": false,
                "error": "event_not_found",
                "detail": "event_id 9999 does not exist"
            })),
        )
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
            .route("/v1/tools/event.start", post(handle_event_start))
            .route("/v1/tools/event.stop", post(handle_event_stop))
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

    /// Spawn a mock that routes `event.start` to a failure handler.
    async fn spawn_mock_with_start_fail() -> String {
        let state = MockState {
            game_events_result: GAME_EVENTS_RESULT.into(),
            query_db_result: QUERY_DB_RESULT.into(),
        };
        let app = Router::new()
            .route("/v1/tools/obs.game_events", post(handle_game_events))
            .route("/v1/tools/obs.query_db", post(handle_query_db))
            .route("/v1/tools/event.start", post(handle_event_start_fail))
            .route("/v1/tools/event.stop", post(handle_event_stop))
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
    async fn query_game_events_null_start_time_preserved_as_none() {
        let base_url = spawn_mock().await;
        let h = Harness::new(base_url, "test-token");
        let inputs = h.query_game_events().await.unwrap();
        // The SQL row has start_time: null → GameEventInput.start_time = None
        // (callers apply effective_start to convert None → 0)
        assert_eq!(inputs[0].start_time, None, "null start_time must be preserved as None");
    }

    #[tokio::test]
    async fn query_game_events_fields() {
        let base_url = spawn_mock().await;
        let h = Harness::new(base_url, "test-token");
        let inputs = h.query_game_events().await.unwrap();
        let inp = &inputs[0];
        assert_eq!(inp.entry, 1);
        // The mock row has end_time: null (holiday row) → preserved as None
        // (callers apply effective_end to convert None → resolve_ref + 63_072_000)
        assert_eq!(inp.end_time, None);
        assert_eq!(inp.occurence, 525600);
        assert_eq!(inp.length, 20160);
        assert_eq!(inp.holiday, 341);
        assert_eq!(inp.holiday_stage, 1);
    }

    #[tokio::test]
    async fn query_game_events_both_times_null_preserved_as_none() {
        // The real live holiday rows (e.g. eventEntry:1) have BOTH
        // start_time: null AND end_time: null. The mock QUERY_DB_RESULT uses
        // this shape — confirm both are preserved as None in GameEventInput.
        let base_url = spawn_mock().await;
        let h = Harness::new(base_url, "test-token");
        let inputs = h.query_game_events().await.unwrap();
        assert_eq!(inputs[0].start_time, None, "null start_time must be preserved as None");
        assert_eq!(inputs[0].end_time, None, "null end_time must be preserved as None");
    }

    // ── HarnessError::Tool (422 ok:false) ─────────────────────────────────────

    #[tokio::test]
    async fn tool_error_surfaces_detail() {
        let base_url = spawn_mock().await;
        let h = Harness::new(base_url, "test-token");
        let err = h.call("fail.tool", json!({})).await.expect_err("should be an error");
        match err {
            HarnessError::Tool { tool, message } => {
                assert!(message.contains("missing required param"), "detail lost: {message}");
                assert_eq!(tool, "fail.tool");
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

    // ── HarnessError::Tool (missing ok field — shared client treats ok-absent as false) ──

    #[tokio::test]
    async fn tool_error_on_missing_ok_field() {
        // The shared HarnessClient treats a missing/non-bool `ok` as false
        // (unwrap_or(false)), so {"garbage": true} surfaces as Tool, not Shape.
        let base_url = spawn_mock().await;
        let h = Harness::new(base_url, "test-token");
        let err = h.call("shape.bad", json!({})).await.expect_err("should be an error");
        assert!(
            matches!(err, HarnessError::Tool { .. }),
            "expected HarnessError::Tool, got {err:?}"
        );
    }

    // ── event_start() ─────────────────────────────────────────────────────────

    /// event_start sends the correct wire args and returns the result envelope.
    #[tokio::test]
    async fn event_start_sends_correct_args_and_returns_result() {
        let base_url = spawn_mock().await;
        let h = Harness::new(base_url, "test-token");
        let result = h.event_start(42).await.expect("event_start should succeed");
        // The mock handler echoes back event_id in the result.
        assert_eq!(
            result.get("event_id").and_then(Value::as_u64),
            Some(42),
            "result must contain the event_id we sent"
        );
        assert_eq!(
            result.get("started").and_then(Value::as_bool),
            Some(true),
            "result must contain started: true"
        );
        assert_eq!(
            result.get("is_active_now").and_then(Value::as_bool),
            Some(true),
            "mock result must report is_active_now: true after start"
        );
    }

    /// event_start propagates a HarnessError::Tool when the server returns ok:false.
    #[tokio::test]
    async fn event_start_tool_error_on_ok_false() {
        let base_url = spawn_mock_with_start_fail().await;
        let h = Harness::new(base_url, "test-token");
        let err = h.event_start(9999).await.expect_err("should return an error for ok:false");
        match err {
            HarnessError::Tool { tool, message } => {
                assert_eq!(tool, "event.start");
                assert!(
                    message.contains("9999") || message.contains("event_id") || message.contains("event_not_found"),
                    "message should mention the error: {message}"
                );
            }
            other => panic!("expected HarnessError::Tool, got {other:?}"),
        }
    }

    // ── event_stop() ──────────────────────────────────────────────────────────

    /// event_stop sends the correct wire args and returns the result envelope.
    #[tokio::test]
    async fn event_stop_sends_correct_args_and_returns_result() {
        let base_url = spawn_mock().await;
        let h = Harness::new(base_url, "test-token");
        let result = h.event_stop(17).await.expect("event_stop should succeed");
        assert_eq!(
            result.get("event_id").and_then(Value::as_u64),
            Some(17),
            "result must contain the event_id we sent"
        );
        assert_eq!(
            result.get("stopped").and_then(Value::as_bool),
            Some(true),
            "result must contain stopped: true"
        );
        assert_eq!(
            result.get("is_active_now").and_then(Value::as_bool),
            Some(false),
            "mock result must report is_active_now: false after stop"
        );
    }

    /// event_stop returns HarnessError::Http on connection refused.
    #[tokio::test]
    async fn event_stop_http_error_on_connection_refused() {
        let h = Harness::new("http://127.0.0.1:1", "test-token");
        let err = h.event_stop(1).await.expect_err("should fail on connection refused");
        assert!(
            matches!(err, HarnessError::Http(_)),
            "expected HarnessError::Http, got {err:?}"
        );
    }
}
