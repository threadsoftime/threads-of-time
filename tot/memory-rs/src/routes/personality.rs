//! REST route handlers for `/memory/personality/*`.
//!
//! Auth note: no bearer auth (matches Python `routes_personality.py` which
//! uses a plain `APIRouter()` with no dependencies).
//!
//! Routes:
//!   POST /memory/personality/get → PersonalityGetResponse (404 if no persona)
//!   POST /memory/personality/set → PersonalitySetResponse (400 if > 4000 chars)

use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::state::AppState;

const PERSONA_MAX_CHARS: usize = 4000;

// ---------------------------------------------------------------------------
// Wire shapes
// ---------------------------------------------------------------------------

/// Mirrors Python `PersonalityGetRequest`.
#[derive(Debug, Deserialize)]
pub struct PersonalityGetRequest {
    pub bot_id: String,
}

/// Mirrors Python `PersonalityGetResponse`.
#[derive(Debug, Serialize)]
pub struct PersonalityGetResponse {
    pub persona: String,
}

/// Mirrors Python `PersonalitySetRequest`.
#[derive(Debug, Deserialize)]
pub struct PersonalitySetRequest {
    pub bot_id: String,
    pub persona: String,
}

/// Mirrors Python `PersonalitySetResponse`.
#[derive(Debug, Serialize)]
pub struct PersonalitySetResponse {
    pub ok: bool,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// POST /memory/personality/get
///
/// Returns 404 when no persona has been set for the bot — matches Python:
/// ```python
/// if not row or row[0] is None:
///     raise HTTPException(status_code=404, detail="no persona for this bot")
/// ```
pub async fn personality_get(
    State(state): State<AppState>,
    Json(req): Json<PersonalityGetRequest>,
) -> Result<Json<PersonalityGetResponse>, AppError> {
    match state.service.personality_get(&req.bot_id).await? {
        Some(persona) => Ok(Json(PersonalityGetResponse { persona })),
        None => Err(AppError::NotFound("no persona for this bot")),
    }
}

/// POST /memory/personality/set
///
/// Returns 400 when persona exceeds 4000 chars — matches Python:
/// ```python
/// if len(req.persona) > PERSONA_MAX_CHARS:
///     raise HTTPException(status_code=400, ...)
/// ```
pub async fn personality_set(
    State(state): State<AppState>,
    Json(req): Json<PersonalitySetRequest>,
) -> Result<Json<PersonalitySetResponse>, AppError> {
    if req.persona.len() > PERSONA_MAX_CHARS {
        return Err(AppError::BadRequest(
            format!("persona exceeds {PERSONA_MAX_CHARS} chars"),
        ));
    }
    state.service.personality_set(&req.bot_id, req.persona).await?;
    Ok(Json(PersonalitySetResponse { ok: true }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
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

    // T1: get on bot with no persona → 404
    #[tokio::test]
    async fn personality_get_no_persona_returns_404() {
        let (router, _tmp) = build_test_router().await;
        let req = Request::builder()
            .method("POST")
            .uri("/memory/personality/get")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({ "bot_id": "botP1" }).to_string()))
            .unwrap();
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // T2: set persona → 200 ok: true → get → 200 with correct persona
    #[tokio::test]
    async fn personality_set_then_get_returns_persona() {
        let (router, tmp) = build_test_router().await;

        let set_req = Request::builder()
            .method("POST")
            .uri("/memory/personality/set")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({
                "bot_id": "botP2",
                "persona": "A fierce warrior from the north."
            }).to_string()))
            .unwrap();
        let resp = router.clone().oneshot(set_req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp.into_body()).await;
        assert_eq!(body["ok"], true);

        let get_req = Request::builder()
            .method("POST")
            .uri("/memory/personality/get")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({ "bot_id": "botP2" }).to_string()))
            .unwrap();
        let resp = router.oneshot(get_req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp.into_body()).await;
        assert_eq!(body["persona"], "A fierce warrior from the north.");
        drop(tmp);
    }

    // T3: set persona > 4000 chars → 400
    #[tokio::test]
    async fn personality_set_too_long_returns_400() {
        let (router, _tmp) = build_test_router().await;
        let long_persona = "x".repeat(4001);
        let req = Request::builder()
            .method("POST")
            .uri("/memory/personality/set")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({
                "bot_id": "botP3",
                "persona": long_persona
            }).to_string()))
            .unwrap();
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}
