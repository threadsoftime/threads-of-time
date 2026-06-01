//! Axum router factory for `memory-rs`.
//!
//! `build_router` assembles all route handlers and injects [`AppState`].
//! Called once at startup in `main.rs` and in tests via `tower::ServiceExt::oneshot`.
//!
//! Route layout (Phase 0 — health only; extended in Phases 4–7):
//! - `GET /health`     — liveness probe

use axum::{Router, routing::get};
use crate::state::AppState;
use crate::routes::health::health;

/// Build the axum [`Router`] with [`AppState`] injected.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .with_state(state)
}
