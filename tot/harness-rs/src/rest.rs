//! REST adapter — `POST /v1/tools/{name}`, `POST /v1/observations/{name}`,
//! `GET /v1/health`, `GET /v1/audit`.
//!
//! Port of `harness_daemon/app.py:211-362` (`build_app` + `_handle_tool_call`).
//! The MCP adapter (Phase 10/11) is NOT mounted here yet.

use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use axum::{
    Router,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, Request, StatusCode, header::AUTHORIZATION},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json,
};
use http_body_util::BodyExt;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::ac_client::ACClient;
use crate::audit::{AuditEvent, AuditLogger};
use crate::auth::{AuthError, TokenStore, authenticate_bearer};
use crate::db_client::DbClient;
use crate::dispatch::dispatch_tool;
use crate::registry::Registry;

// ── AppState ──────────────────────────────────────────────────────────────────

pub struct AppState {
    pub token_store: TokenStore,
    pub registry:    Registry,
    pub ac_client:   ACClient,
    pub db_client:   Option<DbClient>,
    pub audit:       AuditLogger,
    pub audit_path:  String,
}

pub type SharedState = Arc<AppState>;

// ── helpers ───────────────────────────────────────────────────────────────────

fn unix_now_f64() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Generate a request_id if the `X-Request-Id` header is absent.
///
/// Mirrors Python: `f"req_{uuid.uuid4().hex[:12]}"`.
fn gen_request_id() -> String {
    let id = Uuid::new_v4().simple().to_string();
    // uuid simple() = 32 lowercase hex ASCII chars; [..12] is always in-bounds + char-safe
    format!("req_{}", &id[..12])
}

/// Build a plain JSON error body + response for auth and parse failures.
/// The `detail` field is always an empty string for unauthorized (spec: no
/// information leakage). For bad_request it carries the parse error message.
fn json_error_response(status: StatusCode, error: &str, detail: &str) -> Response {
    let body = json!({"ok": false, "error": error, "detail": detail});
    (status, Json(body)).into_response()
}

// ── shared tool-call handler ──────────────────────────────────────────────────

/// Core handler called by both `/v1/tools/{name}` and `/v1/observations/{name}`.
///
/// Mirrors `app.py:261-328` step-for-step.
async fn handle_tool_call(
    state:   &AppState,
    headers: &HeaderMap,
    name:    &str,
    body:    bytes::Bytes,
) -> Response {
    let t0 = Instant::now();

    // ── request_id ────────────────────────────────────────────────────────────
    let request_id: String = headers
        .get("X-Request-Id")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_owned())
        .unwrap_or_else(gen_request_id);

    // ── 1. Auth ───────────────────────────────────────────────────────────────
    let auth_header: Option<&str> = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    let auth = match authenticate_bearer(&state.token_store, auth_header) {
        Ok(a) => a,
        Err(AuthError::Unauthorized) => {
            let latency_ms = t0.elapsed().as_millis() as u64;
            let _ = state.audit.write(&AuditEvent {
                ts: unix_now_f64(),
                request_id: request_id.clone(),
                identity:   "unknown".to_string(),
                tool:       name.to_string(),
                args_body:  json!({}),
                outcome:    "unauthorized".to_string(),
                status:     401,
                latency_ms,
                ac_latency_ms: Some(0),
                error_detail: String::new(),
                transport:  "http".to_string(),
            });
            return json_error_response(StatusCode::UNAUTHORIZED, "unauthorized", "");
        }
    };

    // ── 2. Body parse ─────────────────────────────────────────────────────────
    let args: Value = if body.is_empty() {
        json!({})
    } else {
        match serde_json::from_slice::<Value>(&body) {
            Ok(v) => v,
            Err(e) => {
                let latency_ms = t0.elapsed().as_millis() as u64;
                let detail = format!("json parse: {e}");
                let _ = state.audit.write(&AuditEvent {
                    ts: unix_now_f64(),
                    request_id: request_id.clone(),
                    identity:   auth.identity.clone(),
                    tool:       name.to_string(),
                    args_body:  json!({}),
                    outcome:    "bad_request".to_string(),
                    status:     400,
                    latency_ms,
                    ac_latency_ms: Some(0),
                    error_detail: detail.clone(),
                    transport:  "http".to_string(),
                });
                return json_error_response(StatusCode::BAD_REQUEST, "bad_request", &e.to_string());
            }
        }
    };

    // ── 3. Dispatch ───────────────────────────────────────────────────────────
    let outcome = dispatch_tool(
        name,
        &args,
        &auth,
        &request_id,
        &state.registry,
        &state.ac_client,
        state.db_client.as_ref(),
    )
    .await;

    let latency_ms = t0.elapsed().as_millis() as u64;

    // ── 4. Audit ──────────────────────────────────────────────────────────────
    let _ = state.audit.write(&AuditEvent {
        ts: unix_now_f64(),
        request_id: request_id.clone(),
        identity:   auth.identity.clone(),
        tool:       name.to_string(),
        args_body:  args.clone(),
        outcome:    outcome.audit_outcome.clone(),
        status:     outcome.status,
        latency_ms,
        ac_latency_ms: outcome.ac_latency_ms,
        error_detail: outcome.error_detail.clone(),
        transport:  "http".to_string(),
    });

    // ── 5. Response ───────────────────────────────────────────────────────────
    let http_status = StatusCode::from_u16(outcome.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

    if outcome.audit_outcome == "ok" || outcome.audit_outcome == "ac_error" {
        // Merge meta fields into the body (V1 wire contract).
        let mut body_val = outcome.body.clone();
        if let Some(obj) = body_val.as_object_mut() {
            obj.insert("request_id".to_string(),   Value::String(request_id.clone()));
            obj.insert("identity".to_string(),     Value::String(auth.identity.clone()));
            obj.insert("latency_ms".to_string(),   json!(latency_ms));
            obj.insert("ac_latency_ms".to_string(), match outcome.ac_latency_ms {
                Some(v) => json!(v),
                None    => Value::Null,
            });
        } else {
            tracing::warn!(tool = %name, "dispatch body is not a JSON object; meta fields omitted");
        }
        return (http_status, Json(body_val)).into_response();
    }

    // Error path: raw body, optional Retry-After.
    if outcome.audit_outcome == "ac_unreachable" {
        let mut resp = (http_status, Json(outcome.body)).into_response();
        resp.headers_mut().insert(
            "Retry-After",
            "1".parse().expect("valid header value"),
        );
        return resp;
    }

    (http_status, Json(outcome.body)).into_response()
}

// ── axum handlers ─────────────────────────────────────────────────────────────

async fn tools_endpoint(
    State(state): State<SharedState>,
    Path(name):   Path<String>,
    headers:      HeaderMap,
    req:          Request<Body>,
) -> Response {
    let body = match req.into_body().collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(_) => {
            return json_error_response(
                StatusCode::BAD_REQUEST,
                "bad_request",
                "body read error",
            );
        }
    };
    handle_tool_call(&state, &headers, &name, body).await
}

async fn observations_endpoint(
    State(state): State<SharedState>,
    Path(name):   Path<String>,
    headers:      HeaderMap,
    req:          Request<Body>,
) -> Response {
    let body = match req.into_body().collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(_) => {
            return json_error_response(
                StatusCode::BAD_REQUEST,
                "bad_request",
                "body read error",
            );
        }
    };
    handle_tool_call(&state, &headers, &name, body).await
}

async fn health_endpoint(
    State(state): State<SharedState>,
) -> impl IntoResponse {
    let ac_ok = state.ac_client.health().await;
    Json(json!({"ok": true, "ac_bridge_reachable": ac_ok}))
}

#[derive(Deserialize)]
struct AuditQuery {
    #[serde(default)]
    since: f64,
}

/// Intentionally unauthenticated — matches the Python daemon (`app.py:343`
/// `audit_replay` has no bearer check); access is gated by the pod-local network.
async fn audit_endpoint(
    State(state): State<SharedState>,
    Query(params): Query<AuditQuery>,
) -> impl IntoResponse {
    let path = state.audit_path.clone();

    // Read the JSONL on a blocking thread — std::fs I/O must not run on the
    // tokio executor.  A read error or NotFound → treat as "no events".
    let contents: String = match tokio::task::spawn_blocking(move || {
        std::fs::read_to_string(&path)
    })
    .await
    {
        Ok(Ok(c))  => c,
        // File not found, permission error, or spawn_blocking JoinError → empty.
        _ => return Json(json!({"events": []})),
    };

    let events: Vec<Value> = contents
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|rec| {
            rec.get("ts").and_then(|v| v.as_f64()).unwrap_or(0.0) >= params.since
        })
        .collect();

    Json(json!({"events": events}))
}

// ── build_router ──────────────────────────────────────────────────────────────

/// Wire the 4 REST routes onto an axum `Router`.
///
/// The MCP adapter (Phase 10/11) is not mounted here — call this from
/// `main.rs` and compose the MCP mount separately.
pub fn build_router(state: SharedState) -> Router {
    Router::new()
        .route("/v1/tools/{name}",        post(tools_endpoint))
        .route("/v1/observations/{name}", post(observations_endpoint))
        .route("/v1/health",              get(health_endpoint))
        .route("/v1/audit",               get(audit_endpoint))
        .with_state(state)
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use http::{Request, StatusCode};
    use serde_json::json;
    use tower::ServiceExt as _;

    use crate::ac_client::ACClient;
    use crate::audit::AuditLogger;
    use crate::auth::TokenStore;
    use crate::config::TokenRecord;
    use crate::db_client::DbClient;
    use crate::registry::build_v1_registry;
    use crate::rest::{AppState, build_router};
    use crate::test_support::{MockConfig, spawn_mock_ac, spawn_mock_ac_with_config};

    // ── test AppState builder ─────────────────────────────────────────────────

    async fn make_state(mock_cfg: MockConfig) -> (Arc<AppState>, crate::test_support::MockAcHandle) {
        let mock = spawn_mock_ac_with_config(mock_cfg).await;
        let ac_client = ACClient::new(mock.base_url(), 3.0);

        let audit_path = {
            let p = std::env::temp_dir().join(format!(
                "harness_rs_rest_test_{}.jsonl",
                uuid::Uuid::new_v4().simple()
            ));
            p.to_str().unwrap().to_string()
        };

        let state = Arc::new(AppState {
            token_store: TokenStore::new(vec![
                TokenRecord {
                    token:          "dev-all".to_string(),
                    identity:       "dev.user".to_string(),
                    scope:          vec!["gm.*".to_string(), "obs.*".to_string(), "bot.*".to_string(), "event.*".to_string(), "memory.*".to_string(), "lfg.*".to_string()],
                    augmented:      false,
                    bound_to_guid:  None,
                    note:           None,
                },
            ]),
            registry:   build_v1_registry(),
            ac_client,
            db_client:  None,
            audit:      AuditLogger::new(&audit_path).unwrap(),
            audit_path: audit_path.clone(),
        });

        (state, mock)
    }

    async fn make_dead_ac_state() -> Arc<AppState> {
        let audit_path = std::env::temp_dir()
            .join(format!("harness_rs_rest_dead_{}.jsonl", uuid::Uuid::new_v4().simple()))
            .to_str()
            .unwrap()
            .to_string();

        Arc::new(AppState {
            token_store: TokenStore::new(vec![
                TokenRecord {
                    token:         "dev-all".to_string(),
                    identity:      "dev.user".to_string(),
                    scope:         vec!["obs.*".to_string()],
                    augmented:     false,
                    bound_to_guid: None,
                    note:          None,
                },
            ]),
            registry:   build_v1_registry(),
            ac_client:  ACClient::new("http://127.0.0.1:1", 0.5),
            db_client:  None,
            audit:      AuditLogger::new(&audit_path).unwrap(),
            audit_path: audit_path.clone(),
        })
    }

    // ── helpers ───────────────────────────────────────────────────────────────

    async fn body_json(resp: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn bearer_req(method: &str, uri: &str, body: Option<serde_json::Value>) -> Request<Body> {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("Authorization", "Bearer dev-all")
            .header("Content-Type", "application/json");
        match body {
            Some(v) => builder
                .body(Body::from(serde_json::to_vec(&v).unwrap()))
                .unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        }
    }

    // ── 1. tools endpoint happy path — meta fields injected ───────────────────

    #[tokio::test]
    async fn tools_endpoint_ok_wraps_meta_fields() {
        let (state, _mock) = make_state(MockConfig {
            dispatch_status: 200,
            dispatch_body:   json!({"ok": true, "result": {"pong": true}}),
            health_status:   200,
        })
        .await;

        let app = build_router(state);
        let req = bearer_req("POST", "/v1/tools/obs.ping", Some(json!({})));
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp).await;
        assert_eq!(body["ok"], true);
        assert_eq!(body["result"]["pong"], true);
        assert_eq!(body["identity"], "dev.user");
        assert!(body["request_id"].as_str().is_some(), "request_id must be present");
        assert!(body["latency_ms"].as_u64().is_some(), "latency_ms must be a u64");
        // ac_latency_ms is Some on a successful forward — must be a number
        assert!(
            body["ac_latency_ms"].is_number(),
            "ac_latency_ms must be a number on ok: {:?}",
            body["ac_latency_ms"]
        );
    }

    // ── 2. missing bearer → 401, no detail ───────────────────────────────────

    #[tokio::test]
    async fn missing_bearer_returns_401() {
        let (state, _mock) = make_state(MockConfig::default()).await;
        let app = build_router(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/tools/obs.ping")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let body = body_json(resp).await;
        assert_eq!(body["ok"], false);
        assert_eq!(body["error"], "unauthorized");
        assert_eq!(body["detail"], "");
    }

    // ── 3. invalid JSON body → 400 bad_request ────────────────────────────────

    #[tokio::test]
    async fn invalid_json_body_returns_400() {
        let (state, _mock) = make_state(MockConfig::default()).await;
        let app = build_router(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/tools/obs.ping")
            .header("Authorization", "Bearer dev-all")
            .header("Content-Type", "application/json")
            .body(Body::from(b"{not valid json".as_slice()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = body_json(resp).await;
        assert_eq!(body["ok"], false);
        assert_eq!(body["error"], "bad_request");
    }

    // ── 4. ac_unreachable → 503 + Retry-After: 1 + raw body (no meta) ────────

    #[tokio::test]
    async fn ac_unreachable_returns_503_with_retry_after() {
        let state = make_dead_ac_state().await;
        let app   = build_router(state);
        let req   = bearer_req("POST", "/v1/tools/obs.ping", Some(json!({})));
        let resp  = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            resp.headers().get("Retry-After").and_then(|v| v.to_str().ok()),
            Some("1")
        );
        let body = body_json(resp).await;
        assert_eq!(body["ok"], false);
        assert_eq!(body["error"], "unavailable");
        // Raw body — meta fields must NOT be present
        assert!(body["request_id"].is_null(), "request_id must not be in error body");
        assert!(body["identity"].is_null(),   "identity must not be in error body");
    }

    // ── 5. observations endpoint behaves like tools ───────────────────────────

    #[tokio::test]
    async fn observations_endpoint_works_like_tools() {
        let (state, _mock) = make_state(MockConfig {
            dispatch_status: 200,
            dispatch_body:   json!({"ok": true, "result": {"state": "ok"}}),
            health_status:   200,
        })
        .await;

        let app = build_router(state);
        let req = bearer_req("POST", "/v1/observations/obs.ping", Some(json!({})));
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp).await;
        assert_eq!(body["ok"], true);
        assert_eq!(body["identity"], "dev.user");
    }

    // ── 6. /v1/health shape ───────────────────────────────────────────────────

    #[tokio::test]
    async fn health_endpoint_shape() {
        let (state, _mock) = make_state(MockConfig::default()).await;
        let app = build_router(state);
        let req = Request::builder()
            .method("GET")
            .uri("/v1/health")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp).await;
        assert_eq!(body["ok"], true);
        assert!(body["ac_bridge_reachable"].is_boolean(), "ac_bridge_reachable must be bool");
    }

    // ── 7. /v1/audit filter by since ─────────────────────────────────────────

    #[tokio::test]
    async fn audit_endpoint_filters_by_since() {
        let audit_path = std::env::temp_dir()
            .join(format!("harness_rs_audit_filter_{}.jsonl", uuid::Uuid::new_v4().simple()))
            .to_str()
            .unwrap()
            .to_string();

        // Write two synthetic JSONL records — ts=1000.0 and ts=2000.0
        {
            use std::fs::OpenOptions;
            use std::io::Write;
            let mut f = OpenOptions::new().create(true).append(true)
                .open(&audit_path).unwrap();
            writeln!(f, r#"{{"ts":1000.0,"request_id":"r1","identity":"a","tool":"obs.ping","args_sha256":"x","outcome":"ok","status":200,"latency_ms":1,"ac_latency_ms":null,"transport":"http"}}"#).unwrap();
            writeln!(f, r#"{{"ts":2000.0,"request_id":"r2","identity":"a","tool":"obs.ping","args_sha256":"y","outcome":"ok","status":200,"latency_ms":1,"ac_latency_ms":null,"transport":"http"}}"#).unwrap();
        }

        let (state, _mock) = make_state(MockConfig::default()).await;
        // Build a new state with the seeded audit path.
        let state = Arc::new(AppState {
            token_store: Arc::try_unwrap(state)
                .map(|s| {
                    // We can't easily move fields out; rebuild directly.
                    s.token_store
                })
                .unwrap_or_else(|arc| {
                    // Arc still shared — just use a fresh token store.
                    TokenStore::new(vec![TokenRecord {
                        token:         "dev-all".to_string(),
                        identity:      "dev.user".to_string(),
                        scope:         vec!["obs.*".to_string()],
                        augmented:     false,
                        bound_to_guid: None,
                        note:          None,
                    }])
                }),
            registry:   build_v1_registry(),
            ac_client:  ACClient::new("http://127.0.0.1:1", 0.5),
            db_client:  None,
            audit:      AuditLogger::new(&audit_path).unwrap(),
            audit_path: audit_path.clone(),
        });

        let app = build_router(state);

        // Query with since=1500.0 — should return only ts=2000.0
        let req = Request::builder()
            .method("GET")
            .uri("/v1/audit?since=1500.0")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp).await;
        let events = body["events"].as_array().unwrap();
        assert_eq!(events.len(), 1, "should return 1 event with ts>=1500");
        assert_eq!(events[0]["request_id"], "r2");
    }

    // ── 8b. /v1/audit record with no `ts` is included when since==0.0 ─────────
    // Python: rec.get("ts", 0) >= since — a missing `ts` is treated as 0.

    #[tokio::test]
    async fn audit_missing_ts_included_when_since_zero() {
        let audit_path = std::env::temp_dir()
            .join(format!("harness_rs_audit_no_ts_{}.jsonl", uuid::Uuid::new_v4().simple()))
            .to_str()
            .unwrap()
            .to_string();

        // One record with no `ts` field at all.
        {
            use std::fs::OpenOptions;
            use std::io::Write;
            let mut f = OpenOptions::new().create(true).append(true)
                .open(&audit_path).unwrap();
            writeln!(f, r#"{{"request_id":"r_no_ts","identity":"a","tool":"obs.ping","outcome":"ok","status":200,"latency_ms":1}}"#).unwrap();
        }

        let state = Arc::new(AppState {
            token_store: TokenStore::new(vec![]),
            registry:    build_v1_registry(),
            ac_client:   ACClient::new("http://127.0.0.1:1", 0.5),
            db_client:   None,
            audit:       AuditLogger::new(&audit_path).unwrap(),
            audit_path:  audit_path.clone(),
        });
        let app = build_router(state);

        // since defaults to 0.0 when omitted; a missing ts (treated as 0.0) satisfies 0.0 >= 0.0
        let req = Request::builder()
            .method("GET")
            .uri("/v1/audit")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp).await;
        let events = body["events"].as_array().unwrap();
        assert_eq!(events.len(), 1, "record with missing ts should be included when since==0.0");
        assert_eq!(events[0]["request_id"], "r_no_ts");
    }

    // ── 9. empty request body → valid dispatch (I3: empty-body Ok path unchanged) ──
    //
    // Simulating a mid-stream body-read failure via oneshot is impractical, so
    // this test covers the other half of the I3 contract: an Ok response with 0
    // bytes must still reach dispatch as `{}` args (not be rejected as a 400).

    #[tokio::test]
    async fn empty_body_dispatches_with_empty_args() {
        let (state, _mock) = make_state(MockConfig {
            dispatch_status: 200,
            dispatch_body:   json!({"ok": true, "result": {"pong": true}}),
            health_status:   200,
        })
        .await;

        let app = build_router(state);
        // No body at all — Body::empty() → Ok(0 bytes) in collect().
        let req = Request::builder()
            .method("POST")
            .uri("/v1/tools/obs.ping")
            .header("Authorization", "Bearer dev-all")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        // Must succeed — NOT a 400.
        assert_eq!(resp.status(), StatusCode::OK, "empty body must not be rejected as bad_request");
        let body = body_json(resp).await;
        assert_eq!(body["ok"], true);
        assert_eq!(body["result"]["pong"], true);
        // Meta fields should be injected (confirms we reached dispatch, not an early error return).
        assert!(body["identity"].as_str().is_some(), "identity meta must be present");
    }

    // ── 8. /v1/audit missing file → empty events ─────────────────────────────

    #[tokio::test]
    async fn audit_missing_file_returns_empty() {
        let audit_path = "/tmp/harness_rs_noexist_audit_XYZ_123456.jsonl".to_string();
        let state = Arc::new(AppState {
            token_store: TokenStore::new(vec![]),
            registry:    build_v1_registry(),
            ac_client:   ACClient::new("http://127.0.0.1:1", 0.5),
            db_client:   None,
            audit:       AuditLogger::new(&audit_path).unwrap(),
            audit_path:  audit_path.clone(),
        });
        let app = build_router(state);
        let req = Request::builder()
            .method("GET")
            .uri("/v1/audit")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp).await;
        assert_eq!(body["events"], json!([]));
    }
}
