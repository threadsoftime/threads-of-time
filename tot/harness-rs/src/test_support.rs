//! Shared test helpers for ac_client and dispatch tests.
//!
//! `spawn_mock_ac()` starts a real axum server on an ephemeral port and
//! returns a `MockAcHandle` that lets tests inspect received requests and
//! configure canned responses.

use std::sync::{Arc, Mutex};

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use tokio::net::TcpListener;

// ── RecordedRequest ───────────────────────────────────────────────────────────

/// The last request body + headers recorded by POST /dispatch.
#[derive(Debug, Clone, Default)]
pub struct RecordedRequest {
    pub headers: Vec<(String, String)>,
    pub body:    Value,
}

// ── MockConfig — what the server returns ─────────────────────────────────────

#[derive(Debug, Clone)]
pub struct MockConfig {
    /// HTTP status to return from POST /dispatch
    pub dispatch_status: u16,
    /// Body to return from POST /dispatch
    pub dispatch_body:   Value,
    /// HTTP status to return from GET /health
    pub health_status:   u16,
}

impl Default for MockConfig {
    fn default() -> Self {
        Self {
            dispatch_status: 200,
            dispatch_body:   json!({"ok": true, "result": {"pong": true}}),
            health_status:   200,
        }
    }
}

// ── shared state ──────────────────────────────────────────────────────────────

#[derive(Clone)]
struct AppState {
    last:   Arc<Mutex<Option<RecordedRequest>>>,
    config: Arc<Mutex<MockConfig>>,
}

// ── handlers ──────────────────────────────────────────────────────────────────

async fn handle_dispatch(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let recorded = RecordedRequest {
        headers: headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("<non-utf8>").to_string()))
            .collect(),
        body: body.clone(),
    };
    *state.last.lock().unwrap() = Some(recorded);

    let cfg = state.config.lock().unwrap().clone();
    let status = StatusCode::from_u16(cfg.dispatch_status).unwrap_or(StatusCode::OK);
    (status, Json(cfg.dispatch_body.clone()))
}

async fn handle_health(
    State(state): State<AppState>,
) -> (StatusCode, Json<Value>) {
    let cfg = state.config.lock().unwrap().clone();
    let status = StatusCode::from_u16(cfg.health_status).unwrap_or(StatusCode::OK);
    (status, Json(json!({"ok": true})))
}

// ── MockAcHandle ──────────────────────────────────────────────────────────────

/// Handle returned by `spawn_mock_ac`. Tests use it to:
/// - read the last received request via `last_request()`
/// - change the canned response via `set_config(MockConfig { … })`
/// - obtain the base URL via `base_url()`
pub struct MockAcHandle {
    base_url: String,
    last:     Arc<Mutex<Option<RecordedRequest>>>,
    config:   Arc<Mutex<MockConfig>>,
}

impl MockAcHandle {
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Return the last request recorded by POST /dispatch, or None if no
    /// request has been received yet.
    pub fn last_request(&self) -> Option<RecordedRequest> {
        self.last.lock().unwrap().clone()
    }

    /// Replace the canned response for subsequent requests.
    pub fn set_config(&self, cfg: MockConfig) {
        *self.config.lock().unwrap() = cfg;
    }

    /// Clear the recorded request so the test starts fresh.
    pub fn reset(&self) {
        *self.last.lock().unwrap() = None;
    }
}

// ── spawn_mock_ac ─────────────────────────────────────────────────────────────

/// Bind to `127.0.0.1:0` (ephemeral port), spawn the mock AC server as a
/// background tokio task, and return a `MockAcHandle`.
///
/// The server lives until the test binary exits (task is detached). Each call
/// creates an independent server on its own port.
pub async fn spawn_mock_ac() -> MockAcHandle {
    spawn_mock_ac_with_config(MockConfig::default()).await
}

pub async fn spawn_mock_ac_with_config(initial_config: MockConfig) -> MockAcHandle {
    let last:   Arc<Mutex<Option<RecordedRequest>>> = Arc::new(Mutex::new(None));
    let config: Arc<Mutex<MockConfig>>              = Arc::new(Mutex::new(initial_config));

    let state = AppState {
        last:   Arc::clone(&last),
        config: Arc::clone(&config),
    };

    let app = Router::new()
        .route("/dispatch", post(handle_dispatch))
        .route("/health",   get(handle_health))
        .with_state(state);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr     = listener.local_addr().unwrap();
    let base_url = format!("http://127.0.0.1:{}", addr.port());

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    MockAcHandle { base_url, last, config }
}
