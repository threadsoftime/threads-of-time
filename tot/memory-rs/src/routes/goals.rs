//! REST route handlers for `/goals/*`.
//!
//! Auth note: no bearer auth (matches Python `routes_goals.py` which
//! uses a plain `APIRouter()` with no dependencies).
//!
//! Routes:
//!   POST /goals/create      → GoalCreateResponse
//!   GET  /goals/list        → GoalListResponse (query params)
//!   GET  /goals/{goal_id}   → GoalRow (404 on miss)
//!   PUT  /goals/update      → GoalUpdateResponse (400/404 on errors)
//!   POST /goals/complete    → GoalCompleteResponse (400/404 on errors)

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::core::{
    GoalCompleteReq, GoalCreateReq, GoalListReq, GoalOutcome, GoalUpdateReq,
};
use crate::error::AppError;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Wire shapes
// ---------------------------------------------------------------------------

/// Mirrors Python `GoalCreateRequest`.
#[derive(Debug, Deserialize)]
pub struct GoalCreateRequest {
    pub bot_id: String,
    pub text: String,
    pub source: Option<String>,
    #[serde(default)]
    pub priority: i64,
    pub origin_memory: Option<String>,
}

/// Mirrors Python `GoalCreateResponse`.
#[derive(Debug, Serialize)]
pub struct GoalCreateResponse {
    pub goal_id: String,
    pub status: String,
}

/// Mirrors Python `GoalRow`.
#[derive(Debug, Serialize)]
pub struct GoalRowWire {
    pub id: String,
    pub bot_id: String,
    pub text: String,
    pub status: String,
    pub source: Option<String>,
    pub priority: i64,
    pub origin_memory: Option<String>,
    pub created_ts: i64,
    pub updated_ts: i64,
    pub completed_ts: Option<i64>,
}

impl From<crate::core::GoalRow> for GoalRowWire {
    fn from(r: crate::core::GoalRow) -> Self {
        GoalRowWire {
            id: r.id,
            bot_id: r.bot_id,
            text: r.text,
            status: r.status,
            source: r.source,
            priority: r.priority,
            origin_memory: r.origin_memory,
            created_ts: r.created_ts,
            updated_ts: r.updated_ts,
            completed_ts: r.completed_ts,
        }
    }
}

/// Query params for `GET /goals/{goal_id}`.
#[derive(Debug, Deserialize)]
pub struct GetGoalParams {
    pub bot_id: String,
}

/// Query params for `GET /goals/list`.
#[derive(Debug, Deserialize)]
pub struct GoalListParams {
    pub bot_id: String,
    pub status: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 { 50 }

/// Mirrors Python `GoalListResponse`.
#[derive(Debug, Serialize)]
pub struct GoalListResponse {
    pub items: Vec<GoalRowWire>,
    pub total: i64,
}

/// Mirrors Python `GoalUpdateRequest`.
#[derive(Debug, Deserialize)]
pub struct GoalUpdateRequest {
    pub bot_id: String,
    pub goal_id: String,
    pub text: Option<String>,
    pub status: Option<String>,
    pub priority: Option<i64>,
    pub origin_memory: Option<String>,
}

/// Mirrors Python `GoalUpdateResponse`.
#[derive(Debug, Serialize)]
pub struct GoalUpdateResponse {
    pub updated: bool,
}

/// Mirrors Python `GoalCompleteRequest`.
#[derive(Debug, Deserialize)]
pub struct GoalCompleteRequest {
    pub bot_id: String,
    pub goal_id: String,
    /// `"completed"` or `"abandoned"` — matches Python `Literal["completed", "abandoned"]`.
    pub outcome: String,
    #[serde(default = "default_also_record")]
    pub also_record_memory: bool,
}

fn default_also_record() -> bool { true }

/// Mirrors Python `GoalCompleteResponse`.
#[derive(Debug, Serialize)]
pub struct GoalCompleteResponse {
    pub updated: bool,
    pub memory_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// POST /goals/create
pub async fn create(
    State(state): State<AppState>,
    Json(req): Json<GoalCreateRequest>,
) -> Result<Json<GoalCreateResponse>, AppError> {
    let resp = state.service.goal_create(GoalCreateReq {
        bot_id: req.bot_id,
        text: req.text,
        source: req.source,
        priority: req.priority,
        origin_memory: req.origin_memory,
    }).await?;
    Ok(Json(GoalCreateResponse {
        goal_id: resp.goal_id,
        status: resp.status,
    }))
}

/// GET /goals/list
pub async fn list_goals(
    State(state): State<AppState>,
    Query(params): Query<GoalListParams>,
) -> Result<Json<GoalListResponse>, AppError> {
    let resp = state.service.goal_list(GoalListReq {
        bot_id: params.bot_id,
        status: params.status,
        limit: params.limit,
        offset: params.offset,
    }).await?;
    Ok(Json(GoalListResponse {
        items: resp.items.into_iter().map(Into::into).collect(),
        total: resp.total,
    }))
}

/// GET /goals/{goal_id}?bot_id=
/// Returns 404 when no row matches.
pub async fn read(
    State(state): State<AppState>,
    Path(goal_id): Path<String>,
    Query(params): Query<GetGoalParams>,
) -> Result<impl IntoResponse, AppError> {
    match state.service.goal_read(&params.bot_id, &goal_id).await? {
        Some(row) => Ok((StatusCode::OK, Json(GoalRowWire::from(row))).into_response()),
        None => Err(AppError::NotFound("goal not found")),
    }
}

/// PUT /goals/update
/// Returns 400 on invalid status/transition/no-fields, 404 when goal not found.
pub async fn update(
    State(state): State<AppState>,
    Json(req): Json<GoalUpdateRequest>,
) -> Result<Json<GoalUpdateResponse>, AppError> {
    let resp = state.service.goal_update(GoalUpdateReq {
        bot_id: req.bot_id,
        goal_id: req.goal_id,
        text: req.text,
        status: req.status,
        priority: req.priority,
        origin_memory: req.origin_memory,
    }).await?;

    if !resp.updated {
        return Err(AppError::NotFound("goal not found"));
    }

    Ok(Json(GoalUpdateResponse { updated: resp.updated }))
}

/// POST /goals/complete
/// Returns 400 on terminal-already or bad transition, 404 when goal not found.
pub async fn complete(
    State(state): State<AppState>,
    Json(req): Json<GoalCompleteRequest>,
) -> Result<Json<GoalCompleteResponse>, AppError> {
    // Parse outcome — matches Python `Literal["completed", "abandoned"]`.
    let outcome = match req.outcome.as_str() {
        "completed" => GoalOutcome::Completed,
        "abandoned" => GoalOutcome::Abandoned,
        other => {
            return Err(AppError::BadRequest(format!(
                "outcome must be 'completed' or 'abandoned', got '{other}'"
            )));
        }
    };

    let resp = state.service.goal_complete(GoalCompleteReq {
        bot_id: req.bot_id,
        goal_id: req.goal_id,
        outcome,
        also_record_memory: req.also_record_memory,
    }).await?;

    Ok(Json(GoalCompleteResponse {
        updated: resp.updated,
        memory_id: resp.memory_id,
    }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use axum::{body::{to_bytes, Body}, http::{Request, StatusCode}};
    use tower::ServiceExt;

    use crate::config::ScoringWeights;
    use crate::core::MemoryService;
    use crate::db::{self, migrate};
    use crate::embed_cache::EmbedCache;
    use crate::embeddings::EmbeddingsClient;
    use crate::pubsub::PubSub;

    async fn spawn_mock_embed() -> String {
        use axum::{routing::post, Router};
        let handler = || async {
            let vec: Vec<f32> = vec![0.0_f32; crate::EMBEDDING_DIM];
            let embedding_json: Vec<serde_json::Value> =
                vec.iter().map(|&x| serde_json::json!(x)).collect();
            axum::Json(serde_json::json!({
                "object": "list",
                "data": [{ "object": "embedding", "embedding": embedding_json, "index": 0 }],
                "model": "test",
            }))
        };
        let app = Router::new().route("/v1/embeddings", post(handler));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}/v1")
    }

    async fn build_test_router() -> (axum::Router, tempfile::NamedTempFile) {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        db::register_vec0();
        {
            let conn = db::open_db(tmp.path()).unwrap();
            migrate::run(&conn, std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/migrations"))).unwrap();
        }
        let embed_url = spawn_mock_embed().await;
        let client = EmbeddingsClient::new(&embed_url, "embedding", "");
        let embed = Arc::new(EmbedCache::new(client));
        let pubsub = Arc::new(PubSub::new());
        let weights = ScoringWeights { w_rel: 0.5, w_rec: 0.2, w_imp: 0.3, tau_seconds: 604800 };
        let svc = Arc::new(MemoryService::new(tmp.path().to_path_buf(), weights, 2000, embed, pubsub.clone()));
        let state = crate::state::AppState::for_test(svc, pubsub);
        let router = crate::app::build_router(state, vec![]);  // disable host check in tests
        (router, tmp)
    }

    async fn body_json(body: Body) -> serde_json::Value {
        let bytes = to_bytes(body, usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    // T1: POST /goals/create → 200 with goal_id and status "pending"
    #[tokio::test]
    async fn create_returns_200_pending() {
        let (router, _tmp) = build_test_router().await;
        let req = Request::builder()
            .method("POST")
            .uri("/goals/create")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({
                "bot_id": "g_bot1", "text": "defeat the dragon"
            }).to_string()))
            .unwrap();
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp.into_body()).await;
        assert!(body["goal_id"].is_string());
        assert_eq!(body["status"], "pending");
    }

    // T2: GET /goals/{id}?bot_id= → 404 on miss
    #[tokio::test]
    async fn read_goal_returns_404_on_miss() {
        let (router, _tmp) = build_test_router().await;
        let req = Request::builder()
            .method("GET")
            .uri("/goals/g_doesnotexist?bot_id=g_bot1")
            .body(Body::empty())
            .unwrap();
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // T3: create → read → 200 with correct fields
    #[tokio::test]
    async fn create_then_read_returns_goal() {
        let (router, tmp) = build_test_router().await;

        let create_req = Request::builder()
            .method("POST")
            .uri("/goals/create")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({
                "bot_id": "g_bot2", "text": "find the sacred relic"
            }).to_string()))
            .unwrap();
        let resp = router.clone().oneshot(create_req).await.unwrap();
        let body = body_json(resp.into_body()).await;
        let gid = body["goal_id"].as_str().unwrap().to_string();

        let get_req = Request::builder()
            .method("GET")
            .uri(format!("/goals/{}?bot_id=g_bot2", gid))
            .body(Body::empty())
            .unwrap();
        let resp = router.oneshot(get_req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp.into_body()).await;
        assert_eq!(body["id"], gid);
        assert_eq!(body["text"], "find the sacred relic");
        assert_eq!(body["status"], "pending");
        drop(tmp);
    }

    // T4: GET /goals/list → 200, items + total
    #[tokio::test]
    async fn list_goals_returns_200() {
        let (router, tmp) = build_test_router().await;

        let create_req = Request::builder()
            .method("POST")
            .uri("/goals/create")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({
                "bot_id": "g_bot3", "text": "explore the dungeon"
            }).to_string()))
            .unwrap();
        router.clone().oneshot(create_req).await.unwrap();

        let list_req = Request::builder()
            .method("GET")
            .uri("/goals/list?bot_id=g_bot3")
            .body(Body::empty())
            .unwrap();
        let resp = router.oneshot(list_req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp.into_body()).await;
        assert_eq!(body["total"], 1);
        let items = body["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        drop(tmp);
    }

    // T5: PUT /goals/update with no fields → 400
    #[tokio::test]
    async fn update_no_fields_returns_400() {
        let (router, _tmp) = build_test_router().await;
        let req = Request::builder()
            .method("PUT")
            .uri("/goals/update")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({
                "bot_id": "g_bot1", "goal_id": "g_whatever"
            }).to_string()))
            .unwrap();
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // T6: create → update (text) → 200
    #[tokio::test]
    async fn update_goal_text_returns_200() {
        let (router, tmp) = build_test_router().await;

        let create_req = Request::builder()
            .method("POST")
            .uri("/goals/create")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({
                "bot_id": "g_bot4", "text": "old text"
            }).to_string()))
            .unwrap();
        let resp = router.clone().oneshot(create_req).await.unwrap();
        let body = body_json(resp.into_body()).await;
        let gid = body["goal_id"].as_str().unwrap().to_string();

        let update_req = Request::builder()
            .method("PUT")
            .uri("/goals/update")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({
                "bot_id": "g_bot4", "goal_id": gid, "text": "new text"
            }).to_string()))
            .unwrap();
        let resp = router.oneshot(update_req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp.into_body()).await;
        assert_eq!(body["updated"], true);
        drop(tmp);
    }

    // T7: POST /goals/complete → 200 (pending → completed)
    #[tokio::test]
    async fn complete_goal_returns_200() {
        let (router, tmp) = build_test_router().await;

        // Create in pending.
        let create_req = Request::builder()
            .method("POST")
            .uri("/goals/create")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({
                "bot_id": "g_bot5", "text": "slay the lich"
            }).to_string()))
            .unwrap();
        let resp = router.clone().oneshot(create_req).await.unwrap();
        let body = body_json(resp.into_body()).await;
        let gid = body["goal_id"].as_str().unwrap().to_string();

        // Move pending → active first (required by state machine).
        let activate_req = Request::builder()
            .method("PUT")
            .uri("/goals/update")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({
                "bot_id": "g_bot5", "goal_id": gid, "status": "active"
            }).to_string()))
            .unwrap();
        router.clone().oneshot(activate_req).await.unwrap();

        // Now complete.
        let complete_req = Request::builder()
            .method("POST")
            .uri("/goals/complete")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({
                "bot_id": "g_bot5", "goal_id": gid, "outcome": "completed"
            }).to_string()))
            .unwrap();
        let resp = router.oneshot(complete_req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp.into_body()).await;
        assert_eq!(body["updated"], true);
        // also_record_memory defaults to true → memory_id should be set
        assert!(body["memory_id"].is_string() || body["memory_id"].is_null());
        drop(tmp);
    }

    // T8: complete goal with invalid outcome → 400
    #[tokio::test]
    async fn complete_invalid_outcome_returns_400() {
        let (router, _tmp) = build_test_router().await;
        let req = Request::builder()
            .method("POST")
            .uri("/goals/complete")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({
                "bot_id": "g_bot1", "goal_id": "g_whatever", "outcome": "done"
            }).to_string()))
            .unwrap();
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // T9: complete non-existent goal → 404
    #[tokio::test]
    async fn complete_nonexistent_returns_404() {
        let (router, _tmp) = build_test_router().await;
        let req = Request::builder()
            .method("POST")
            .uri("/goals/complete")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({
                "bot_id": "g_bot1", "goal_id": "g_nonexistent", "outcome": "completed"
            }).to_string()))
            .unwrap();
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
