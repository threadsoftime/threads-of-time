//! Integration test: GET /health → 200 {"ok":true}
//!
//! Uses tower::ServiceExt::oneshot so no real TCP listener is needed.
//! Parity target: the live Python memory-sidecar v0.2.1 `/health` returns
//! `{"ok": true}` (NOT `{"status": "ok"}`).

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use memory_rs::{
    app::build_router,
    config::ScoringWeights,
    core::MemoryService,
    db::{self, migrate},
    embed_cache::EmbedCache,
    embeddings::EmbeddingsClient,
    pubsub::PubSub,
    state::AppState,
};
use tower::ServiceExt; // for `oneshot`

fn test_state() -> AppState {
    // Minimal state for /health — no DB or embed calls are made for health.
    // Use an in-memory tempfile so register_vec0 + open_db are valid.
    let tmp = tempfile::NamedTempFile::new().expect("tempfile");
    db::register_vec0();
    {
        let conn = db::open_db(tmp.path()).expect("open_db");
        migrate::run(
            &conn,
            std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/migrations")),
        )
        .expect("migrations");
    }
    let client = EmbeddingsClient::new("http://127.0.0.1:8081/v1", "embedding", "");
    let embed = Arc::new(EmbedCache::new(client));
    let pubsub = Arc::new(PubSub::new());
    let weights = ScoringWeights { w_rel: 0.5, w_rec: 0.2, w_imp: 0.3, tau_seconds: 604800 };
    let svc = Arc::new(MemoryService::new(
        tmp.path().to_path_buf(),
        weights,
        2000,
        embed,
        pubsub.clone(),
    ));
    // Keep tmp alive until after the test; leak it intentionally so the path
    // stays valid for the duration of the test (the DB is never written by /health).
    std::mem::forget(tmp);
    AppState::for_test(svc, pubsub)
}

#[tokio::test]
async fn health_returns_200_with_ok_true() {
    let app = build_router(test_state(), vec![]);  // disable host check in tests

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

    assert_eq!(
        body,
        serde_json::json!({"ok": true}),
        "health body must be exactly {{\"ok\":true}}"
    );
}
