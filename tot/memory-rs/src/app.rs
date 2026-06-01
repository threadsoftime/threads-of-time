//! Axum router factory for `memory-rs`.
//!
//! `build_router` assembles all route handlers and injects [`AppState`].
//! Called once at startup in `main.rs` and in tests via `tower::ServiceExt::oneshot`.
//!
//! Route layout (extended across Phases 4–7):
//! - `GET  /health`                                          — liveness probe
//! - `POST /v1/memory/:bot_guid/episodes`                   — write an episode (Phase 4)
//! - `GET  /v1/memory/:bot_guid/episodes/:episode_id`       — read an episode (Phase 4)

use axum::{Router, routing::{get, post}};
use crate::state::AppState;
use crate::routes::health::health;
use crate::routes::write::write_episode;
use crate::routes::read::read_episode;

/// Build the axum [`Router`] with [`AppState`] injected.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/memory/:bot_guid/episodes", post(write_episode))
        .route("/v1/memory/:bot_guid/episodes/:episode_id", get(read_episode))
        .with_state(state)
}
