//! Axum router factory for `memory-rs`.
//!
//! Task 0.4: wires the `/health` liveness probe.
//! Domain routes (write, read, list, update, delete, recall, search) land in
//! Phases 4–7 against the v0.2.1 contract.

use axum::{routing::get, Router};
use crate::{routes::health::health, state::AppState};

/// Build the axum [`Router`] with [`AppState`] injected.
///
/// Routes wired so far:
/// - `GET /health` → `{"ok": true}` (liveness probe; parity with Python v0.2.1)
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .with_state(state)
}
