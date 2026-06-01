//! Axum router factory for `memory-rs`.
//!
//! `build_router` assembles all route handlers and injects [`AppState`].
//! Called once at startup in `main.rs` and in tests via `tower::ServiceExt::oneshot`.
//!
//! Route layout (extended across Phases 4–7):
//! - `GET    /health`                                          — liveness probe
//! - `POST   /v1/memory/:bot_guid/episodes`                   — write an episode (Phase 4)
//! - `GET    /v1/memory/:bot_guid/episodes`                   — list episodes (Phase 4)
//! - `GET    /v1/memory/:bot_guid/episodes/:episode_id`       — read an episode (Phase 4)
//! - `PATCH  /v1/memory/:bot_guid/episodes/:episode_id`       — update an episode (Phase 5)
//! - `DELETE /v1/memory/:bot_guid/episodes/:episode_id`       — delete an episode (Phase 5)
//! - `POST   /v1/memory/:bot_guid/recall`                     — hybrid recall (Phase 5)
//! - `POST   /v1/memory/:bot_guid/search`                     — dense-only search (Phase 5)

use axum::{Router, routing::{get, post}};
use crate::state::AppState;
use crate::routes::health::health;
use crate::routes::write::write_episode;
use crate::routes::read::read_episode;
use crate::routes::list::list_episodes;
use crate::routes::recall::recall_handler;
use crate::routes::search::search_handler;
use crate::routes::update::update_handler;
use crate::routes::delete::delete_handler;

/// Build the axum [`Router`] with [`AppState`] injected.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route(
            "/v1/memory/:bot_guid/episodes",
            post(write_episode).get(list_episodes),
        )
        .route(
            "/v1/memory/:bot_guid/episodes/:episode_id",
            get(read_episode).patch(update_handler).delete(delete_handler),
        )
        .route("/v1/memory/:bot_guid/recall", post(recall_handler))
        .route("/v1/memory/:bot_guid/search", post(search_handler))
        .with_state(state)
}
