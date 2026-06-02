//! REST route handlers for `/memory/*` (everything except personality and goals).
//!
//! Auth note: the Python `routes_memory.py` has **no bearer auth** applied —
//! the `build_router(state)` function uses a plain `APIRouter()` with no
//! dependencies.  These Rust handlers are similarly unauthenticated; bearer
//! auth is only applied to the MCP/SSE transport (Phase 6).
//!
//! Route map (mirrors Python exactly):
//!   POST /memory/remember        → write  → RememberResponse
//!   POST /memory/forget          → forget → ForgetResponse
//!   POST /memory/recall          → recall → RecallResponse
//!   POST /memory/recall_about    → recall_about → RecallAboutResponse
//!   POST /memory/search          → search → SearchResponse
//!   GET  /memory/list            → list   → ListResponse (query params)
//!   GET  /memory/{memory_id}     → read   → MemoryRow (404 on miss)
//!   PUT  /memory/update          → update → UpdateResponse

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::core::{
    ForgetReq, ListReq, RecallAboutReq, RecallReq, SearchReq, UpdateReq, WriteReq,
};
use crate::error::AppError;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Wire shapes — JSON request / response types (mirrors Python pydantic models)
// ---------------------------------------------------------------------------

/// Mirrors Python `RememberRel`.
#[derive(Debug, Deserialize)]
pub struct RememberRelWire {
    pub src: String,
    pub rel: String,
    pub dst: String,
}

/// Mirrors Python `RememberRequest`.
#[derive(Debug, Deserialize)]
pub struct RememberRequest {
    pub bot_id: String,
    pub text: String,
    #[serde(default)]
    pub entities: Vec<String>,
    pub salience: f32,
    #[serde(default)]
    pub relations: Vec<RememberRelWire>,
}

/// Mirrors Python `RememberResponse`.
#[derive(Debug, Serialize)]
pub struct RememberResponse {
    pub memory_id: String,
    pub evicted: usize,
}

/// Mirrors Python `ForgetRequest`.
#[derive(Debug, Deserialize)]
pub struct ForgetRequest {
    pub bot_id: String,
    pub memory_id: String,
}

/// Mirrors Python `ForgetResponse`.
#[derive(Debug, Serialize)]
pub struct ForgetResponse {
    pub forgotten: bool,
}

/// Mirrors Python `RecallRequest`.
#[derive(Debug, Deserialize)]
pub struct RecallRequest {
    pub bot_id: String,
    pub query: String,
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    pub since_ts: Option<i64>,
    pub until_ts: Option<i64>,
    pub memory_type: Option<String>,
}

fn default_top_k() -> usize { 5 }
fn default_top_k_3() -> usize { 3 }
fn default_max_hops() -> usize { 2 }

/// Mirrors Python `RecalledMemory`.
#[derive(Debug, Serialize)]
pub struct RecalledMemory {
    pub memory_id: String,
    pub text: String,
    pub score: f64,
    pub ts: i64,
}

/// Mirrors Python `RecallResponse`.
#[derive(Debug, Serialize)]
pub struct RecallResponse {
    pub memories: Vec<RecalledMemory>,
}

/// Mirrors Python `RecallAboutRequest`.
#[derive(Debug, Deserialize)]
pub struct RecallAboutRequest {
    pub bot_id: String,
    pub entity: String,
    #[serde(default = "default_max_hops")]
    pub max_hops: usize,
    #[serde(default = "default_top_k_3")]
    pub top_k: usize,
    pub since_ts: Option<i64>,
    pub until_ts: Option<i64>,
    pub memory_type: Option<String>,
}

/// Mirrors Python `RecallAboutResponse`.
#[derive(Debug, Serialize)]
pub struct RecallAboutResponse {
    pub hints: Vec<String>,
}

/// Mirrors Python `SearchRequest`.
#[derive(Debug, Deserialize)]
pub struct SearchRequest {
    pub bot_id: String,
    pub query: String,
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    pub since_ts: Option<i64>,
    pub until_ts: Option<i64>,
    pub memory_type: Option<String>,
}

/// Mirrors Python `SearchSignals`.
#[derive(Debug, Serialize)]
pub struct SearchSignals {
    pub bm25_rank: Option<usize>,
    pub dense_rank: Option<usize>,
    pub entity_rank: Option<usize>,
}

/// Mirrors Python `SearchItem`.
#[derive(Debug, Serialize)]
pub struct SearchItem {
    pub memory_id: String,
    pub text: String,
    pub score: f64,
    pub ts: i64,
    pub signals: SearchSignals,
}

/// Mirrors Python `SearchResponse`.
#[derive(Debug, Serialize)]
pub struct SearchResponse {
    pub items: Vec<SearchItem>,
    pub total_candidates: usize,
}

/// Query params for `GET /memory/list` (mirrors Python handler signature).
#[derive(Debug, Deserialize)]
pub struct ListParams {
    pub bot_id: String,
    pub memory_type: Option<String>,
    pub source: Option<String>,
    pub since_ts: Option<i64>,
    pub until_ts: Option<i64>,
    #[serde(default = "default_list_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_list_limit() -> i64 { 50 }

/// Mirrors Python `MemoryRow`.
#[derive(Debug, Serialize)]
pub struct MemoryRowWire {
    pub id: String,
    pub bot_id: String,
    pub text: String,
    pub salience: f32,
    pub memory_type: String,
    pub source: Option<String>,
    pub created_ts: i64,
    pub last_recalled_ts: i64,
}

/// Mirrors Python `ListResponse`.
#[derive(Debug, Serialize)]
pub struct ListResponse {
    pub items: Vec<MemoryRowWire>,
    pub total: i64,
}

/// Query params for `GET /memory/{memory_id}`.
#[derive(Debug, Deserialize)]
pub struct GetMemoryParams {
    pub bot_id: String,
}

/// Mirrors Python `UpdateRequest`.
#[derive(Debug, Deserialize)]
pub struct UpdateRequest {
    pub bot_id: String,
    pub memory_id: String,
    pub text: Option<String>,
    pub salience: Option<f32>,
    pub memory_type: Option<String>,
    pub source: Option<String>,
}

/// Mirrors Python `UpdateResponse`.
#[derive(Debug, Serialize)]
pub struct UpdateResponse {
    pub updated: bool,
    pub re_embedded: bool,
}

// ---------------------------------------------------------------------------
// Conversion helpers: wire → core request types
// ---------------------------------------------------------------------------

impl From<RememberRelWire> for crate::core::RelationTriple {
    fn from(w: RememberRelWire) -> Self {
        crate::core::RelationTriple { src: w.src, rel: w.rel, dst: w.dst }
    }
}

impl From<crate::core::MemoryRow> for MemoryRowWire {
    fn from(r: crate::core::MemoryRow) -> Self {
        MemoryRowWire {
            id: r.id,
            bot_id: r.bot_id,
            text: r.text,
            salience: r.salience,
            memory_type: r.memory_type,
            source: r.source,
            created_ts: r.created_ts,
            last_recalled_ts: r.last_recalled_ts,
        }
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// POST /memory/remember
pub async fn remember(
    State(state): State<AppState>,
    Json(req): Json<RememberRequest>,
) -> Result<Json<RememberResponse>, AppError> {
    let core_req = WriteReq {
        bot_id: req.bot_id,
        text: req.text,
        salience: req.salience,
        entities: req.entities,
        relations: req.relations.into_iter().map(Into::into).collect(),
        memory_type: None,
        source: None,
    };
    let resp = state.service.write(core_req).await?;
    Ok(Json(RememberResponse {
        memory_id: resp.memory_id,
        evicted: resp.evicted,
    }))
}

/// POST /memory/forget
pub async fn forget(
    State(state): State<AppState>,
    Json(req): Json<ForgetRequest>,
) -> Result<Json<ForgetResponse>, AppError> {
    let resp = state.service.forget(ForgetReq {
        bot_id: req.bot_id,
        memory_id: req.memory_id,
    }).await?;
    Ok(Json(ForgetResponse { forgotten: resp.forgotten }))
}

/// POST /memory/recall
pub async fn recall(
    State(state): State<AppState>,
    Json(req): Json<RecallRequest>,
) -> Result<Json<RecallResponse>, AppError> {
    let resp = state.service.recall(RecallReq {
        bot_id: req.bot_id,
        query: req.query,
        top_k: req.top_k,
        since_ts: req.since_ts,
        until_ts: req.until_ts,
        memory_type: req.memory_type,
    }).await?;
    Ok(Json(RecallResponse {
        memories: resp.memories.into_iter().map(|m| RecalledMemory {
            memory_id: m.memory_id,
            text: m.text,
            score: m.score,
            ts: m.ts,
        }).collect(),
    }))
}

/// POST /memory/recall_about
pub async fn recall_about(
    State(state): State<AppState>,
    Json(req): Json<RecallAboutRequest>,
) -> Result<Json<RecallAboutResponse>, AppError> {
    let resp = state.service.recall_about(RecallAboutReq {
        bot_id: req.bot_id,
        entity: req.entity,
        max_hops: req.max_hops,
        top_k: req.top_k,
        since_ts: req.since_ts,
        until_ts: req.until_ts,
        memory_type: req.memory_type,
    }).await?;
    Ok(Json(RecallAboutResponse { hints: resp.hints }))
}

/// POST /memory/search
pub async fn search(
    State(state): State<AppState>,
    Json(req): Json<SearchRequest>,
) -> Result<Json<SearchResponse>, AppError> {
    let resp = state.service.search(SearchReq {
        bot_id: req.bot_id,
        query: req.query,
        top_k: req.top_k,
        since_ts: req.since_ts,
        until_ts: req.until_ts,
        memory_type: req.memory_type,
    }).await?;
    Ok(Json(SearchResponse {
        items: resp.items.into_iter().map(|i| SearchItem {
            memory_id: i.memory_id,
            text: i.text,
            score: i.score,
            ts: i.ts,
            signals: SearchSignals {
                bm25_rank: i.signals.bm25_rank,
                dense_rank: i.signals.dense_rank,
                entity_rank: i.signals.entity_rank,
            },
        }).collect(),
        total_candidates: resp.total_candidates,
    }))
}

/// GET /memory/list
pub async fn list_memories(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Result<Json<ListResponse>, AppError> {
    let resp = state.service.list(ListReq {
        bot_id: params.bot_id,
        memory_type: params.memory_type,
        source: params.source,
        since_ts: params.since_ts,
        until_ts: params.until_ts,
        limit: params.limit,
        offset: params.offset,
    }).await?;
    Ok(Json(ListResponse {
        items: resp.items.into_iter().map(Into::into).collect(),
        total: resp.total,
    }))
}

/// GET /memory/{memory_id}?bot_id=
/// Returns 404 when no row matches.
pub async fn get_memory(
    State(state): State<AppState>,
    Path(memory_id): Path<String>,
    Query(params): Query<GetMemoryParams>,
) -> Result<impl IntoResponse, AppError> {
    match state.service.read(&params.bot_id, &memory_id).await? {
        Some(row) => Ok((StatusCode::OK, Json(MemoryRowWire::from(row))).into_response()),
        None => Err(AppError::NotFound("memory not found")),
    }
}

/// PUT /memory/update
/// Returns 400 when no field is provided; 404 when memory not found.
pub async fn update(
    State(state): State<AppState>,
    Json(req): Json<UpdateRequest>,
) -> Result<Json<UpdateResponse>, AppError> {
    // Replicate Python's 400 guard: "no updatable field provided"
    if req.text.is_none()
        && req.salience.is_none()
        && req.memory_type.is_none()
        && req.source.is_none()
    {
        return Err(AppError::BadRequest("no updatable field provided".to_string()));
    }

    let resp = state.service.update(UpdateReq {
        bot_id: req.bot_id,
        memory_id: req.memory_id,
        text: req.text,
        salience: req.salience,
        memory_type: req.memory_type,
        source: req.source,
    }).await?;

    // `update` returns `updated: false` when the row doesn't exist.
    if !resp.updated {
        return Err(AppError::NotFound("memory not found for this bot"));
    }

    Ok(Json(UpdateResponse {
        updated: resp.updated,
        re_embedded: resp.re_embedded,
    }))
}

// ---------------------------------------------------------------------------
// Tests (oneshot via tower::ServiceExt against a tempfile-backed router)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use axum::{body::to_bytes, body::Body, http::Request};
    use tower::ServiceExt;

    use crate::config::ScoringWeights;
    use crate::core::MemoryService;
    use crate::db::{self, migrate};
    use crate::embed_cache::EmbedCache;
    use crate::embeddings::EmbeddingsClient;
    use crate::pubsub::PubSub;

    // ---- Mock embed server ----

    async fn spawn_mock_embed() -> String {
        use axum::{routing::post, Router};
        let handler = || async {
            let vec: Vec<f32> = (0..crate::EMBEDDING_DIM).map(|i| i as f32 * 0.001).collect();
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

    // ---- Router builder for tests ----

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
        let state = AppState::for_test(svc, pubsub);
        let router = crate::app::build_router(state);
        (router, tmp)
    }

    async fn body_json(body: Body) -> serde_json::Value {
        let bytes = to_bytes(body, usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    // T1: POST /memory/remember → 200, memory_id + evicted
    #[tokio::test]
    async fn remember_returns_200() {
        let (router, _tmp) = build_test_router().await;
        let req = Request::builder()
            .method("POST")
            .uri("/memory/remember")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({
                "bot_id": "bot1", "text": "hello world", "salience": 0.8
            }).to_string()))
            .unwrap();
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp.into_body()).await;
        assert!(body["memory_id"].is_string(), "memory_id must be a string");
        assert_eq!(body["evicted"], 0);
    }

    // T2: POST /memory/forget → 200, forgotten: false (not in DB)
    #[tokio::test]
    async fn forget_nonexistent_returns_200_forgotten_false() {
        let (router, _tmp) = build_test_router().await;
        let req = Request::builder()
            .method("POST")
            .uri("/memory/forget")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({
                "bot_id": "bot1", "memory_id": "m_doesnotexist"
            }).to_string()))
            .unwrap();
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp.into_body()).await;
        assert_eq!(body["forgotten"], false);
    }

    // T3: GET /memory/{id}?bot_id= → 404 when not found
    #[tokio::test]
    async fn get_memory_returns_404_on_miss() {
        let (router, _tmp) = build_test_router().await;
        let req = Request::builder()
            .method("GET")
            .uri("/memory/m_doesnotexist?bot_id=bot1")
            .body(Body::empty())
            .unwrap();
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // T4: PUT /memory/update with no fields → 400
    #[tokio::test]
    async fn update_no_fields_returns_400() {
        let (router, _tmp) = build_test_router().await;
        let req = Request::builder()
            .method("PUT")
            .uri("/memory/update")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({
                "bot_id": "bot1", "memory_id": "m_abc"
            }).to_string()))
            .unwrap();
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // T5: remember → recall → assert at least 1 memory returned
    #[tokio::test]
    async fn recall_returns_results_after_write() {
        let (router, tmp) = build_test_router().await;

        // Write a memory.
        let write_req = Request::builder()
            .method("POST")
            .uri("/memory/remember")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({
                "bot_id": "bot2", "text": "the dragon ate the village", "salience": 0.9
            }).to_string()))
            .unwrap();
        router.clone().oneshot(write_req).await.unwrap();

        // Recall.
        let recall_req = Request::builder()
            .method("POST")
            .uri("/memory/recall")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({
                "bot_id": "bot2", "query": "dragon"
            }).to_string()))
            .unwrap();
        let resp = router.oneshot(recall_req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp.into_body()).await;
        let memories = body["memories"].as_array().unwrap();
        assert!(!memories.is_empty(), "must recall at least 1 memory");
        drop(tmp);
    }

    // T6: remember → search → assert items returned
    #[tokio::test]
    async fn search_returns_items_after_write() {
        let (router, tmp) = build_test_router().await;

        let write_req = Request::builder()
            .method("POST")
            .uri("/memory/remember")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({
                "bot_id": "bot3", "text": "the wizard cast a spell", "salience": 0.7
            }).to_string()))
            .unwrap();
        router.clone().oneshot(write_req).await.unwrap();

        let search_req = Request::builder()
            .method("POST")
            .uri("/memory/search")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({
                "bot_id": "bot3", "query": "wizard spell"
            }).to_string()))
            .unwrap();
        let resp = router.oneshot(search_req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp.into_body()).await;
        assert!(body["items"].is_array(), "items must be an array");
        drop(tmp);
    }

    // T7: GET /memory/list → 200, items array + total
    #[tokio::test]
    async fn list_returns_200() {
        let (router, tmp) = build_test_router().await;

        // Write one memory.
        let write_req = Request::builder()
            .method("POST")
            .uri("/memory/remember")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({
                "bot_id": "bot4", "text": "a quiet morning", "salience": 0.5
            }).to_string()))
            .unwrap();
        router.clone().oneshot(write_req).await.unwrap();

        let list_req = Request::builder()
            .method("GET")
            .uri("/memory/list?bot_id=bot4")
            .body(Body::empty())
            .unwrap();
        let resp = router.oneshot(list_req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp.into_body()).await;
        assert_eq!(body["total"], 1);
        assert_eq!(body["items"].as_array().unwrap().len(), 1);
        drop(tmp);
    }

    // T8: remember → get_memory → 200 with correct fields
    #[tokio::test]
    async fn get_memory_returns_200_after_write() {
        let (router, tmp) = build_test_router().await;

        // Write.
        let write_req = Request::builder()
            .method("POST")
            .uri("/memory/remember")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({
                "bot_id": "bot5", "text": "test memory text", "salience": 0.6
            }).to_string()))
            .unwrap();
        let resp = router.clone().oneshot(write_req).await.unwrap();
        let body = body_json(resp.into_body()).await;
        let mid = body["memory_id"].as_str().unwrap().to_string();

        // Read.
        let get_req = Request::builder()
            .method("GET")
            .uri(format!("/memory/{}?bot_id=bot5", mid))
            .body(Body::empty())
            .unwrap();
        let resp = router.oneshot(get_req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp.into_body()).await;
        assert_eq!(body["id"], mid);
        assert_eq!(body["text"], "test memory text");
        drop(tmp);
    }

    // T9: remember → update (text only) → 200
    #[tokio::test]
    async fn update_text_returns_200() {
        let (router, tmp) = build_test_router().await;

        let write_req = Request::builder()
            .method("POST")
            .uri("/memory/remember")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({
                "bot_id": "bot6", "text": "original text", "salience": 0.5
            }).to_string()))
            .unwrap();
        let resp = router.clone().oneshot(write_req).await.unwrap();
        let body = body_json(resp.into_body()).await;
        let mid = body["memory_id"].as_str().unwrap().to_string();

        let update_req = Request::builder()
            .method("PUT")
            .uri("/memory/update")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({
                "bot_id": "bot6", "memory_id": mid, "text": "updated text"
            }).to_string()))
            .unwrap();
        let resp = router.oneshot(update_req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp.into_body()).await;
        assert_eq!(body["updated"], true);
        assert_eq!(body["re_embedded"], true);
        drop(tmp);
    }

    // T10: update on nonexistent memory → 404
    #[tokio::test]
    async fn update_nonexistent_returns_404() {
        let (router, _tmp) = build_test_router().await;
        let req = Request::builder()
            .method("PUT")
            .uri("/memory/update")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({
                "bot_id": "bot1", "memory_id": "m_nonexistent", "text": "new text"
            }).to_string()))
            .unwrap();
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // T11: remember → forget (existing) → 200 forgotten: true
    #[tokio::test]
    async fn forget_existing_returns_forgotten_true() {
        let (router, tmp) = build_test_router().await;

        let write_req = Request::builder()
            .method("POST")
            .uri("/memory/remember")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({
                "bot_id": "bot7", "text": "to be forgotten", "salience": 0.3
            }).to_string()))
            .unwrap();
        let resp = router.clone().oneshot(write_req).await.unwrap();
        let body = body_json(resp.into_body()).await;
        let mid = body["memory_id"].as_str().unwrap().to_string();

        let forget_req = Request::builder()
            .method("POST")
            .uri("/memory/forget")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({
                "bot_id": "bot7", "memory_id": mid
            }).to_string()))
            .unwrap();
        let resp = router.oneshot(forget_req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp.into_body()).await;
        assert_eq!(body["forgotten"], true);
        drop(tmp);
    }
}
