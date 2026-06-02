//! Axum router factory for `memory-rs`.
//!
//! Stubbed during archival (Task 0.1). Domain routes archived to `_archive-768/`.
//! Routes will be rebuilt in Phases 4–7 against the v0.2.1 contract.

use axum::Router;
use crate::state::AppState;

/// Build the axum [`Router`] with [`AppState`] injected.
///
/// Currently returns an empty router. `/health` and domain routes land in
/// Tasks 0.4+ once the v0.2.1 schema and contract are in place.
pub fn build_router(state: AppState) -> Router {
    Router::new().with_state(state)
}
