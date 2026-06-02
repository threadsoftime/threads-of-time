//! Integration test: GET /health → 200 {"ok":true}
//!
//! Uses tower::ServiceExt::oneshot so no real TCP listener is needed.
//! Parity target: the live Python memory-sidecar v0.2.1 `/health` returns
//! `{"ok": true}` (NOT `{"status": "ok"}`).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use memory_rs::{app::build_router, state::{AppState, EmbedConfig}};
use std::path::PathBuf;
use tower::ServiceExt; // for `oneshot`

fn test_state() -> AppState {
    AppState {
        data_dir: PathBuf::from("/tmp/mem-test"),
        embed: EmbedConfig {
            url: "http://127.0.0.1:8081".to_string(),
            model: String::new(),
            api_key: String::new(),
        },
    }
}

#[tokio::test]
async fn health_returns_200_with_ok_true() {
    let app = build_router(test_state());

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
