//! Axum router factory for `memory-rs`.
//!
//! Wires all REST route handlers into the [`Router`] and injects [`AppState`].
//!
//! Route table (mirrors Python `create_app` in `main.py`):
//!
//! | Method | Path                         | Handler                    |
//! |--------|------------------------------|----------------------------|
//! | GET    | /health                      | health                     |
//! | POST   | /memory/remember             | memory::remember           |
//! | POST   | /memory/forget               | memory::forget             |
//! | POST   | /memory/recall               | memory::recall             |
//! | POST   | /memory/recall_about         | memory::recall_about       |
//! | POST   | /memory/search               | memory::search             |
//! | GET    | /memory/list                 | memory::list_memories      |
//! | GET    | /memory/:memory_id           | memory::get_memory         |
//! | PUT    | /memory/update               | memory::update             |
//! | POST   | /memory/personality/get      | personality::personality_get |
//! | POST   | /memory/personality/set      | personality::personality_set |
//! | POST   | /goals/create                | goals::create              |
//! | GET    | /goals/list                  | goals::list_goals          |
//! | GET    | /goals/:goal_id              | goals::read                |
//! | PUT    | /goals/update                | goals::update              |
//! | POST   | /goals/complete              | goals::complete            |
//!
//! Auth note: REST routes are **not** bearer-protected — only MCP/SSE uses the
//! TokenStore (Phase 6).  This replicates Python exactly; see `auth.rs` for
//! rationale.

use axum::{
    routing::{get, post, put},
    Router,
};
use crate::{
    routes::{
        goals,
        health::health,
        memory,
        personality,
    },
    state::AppState,
};

/// Build the axum [`Router`] with [`AppState`] injected.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        // Liveness probe.
        .route("/health", get(health))
        // Memory routes.
        .route("/memory/remember",          post(memory::remember))
        .route("/memory/forget",            post(memory::forget))
        .route("/memory/recall",            post(memory::recall))
        .route("/memory/recall_about",      post(memory::recall_about))
        .route("/memory/search",            post(memory::search))
        .route("/memory/list",              get(memory::list_memories))
        .route("/memory/update",            put(memory::update))
        // Personality routes — registered BEFORE the wildcard /:memory_id
        // so axum's router matches the literal prefix first.
        .route("/memory/personality/get",   post(personality::personality_get))
        .route("/memory/personality/set",   post(personality::personality_set))
        // Memory read-by-id (wildcard last).
        .route("/memory/:memory_id",        get(memory::get_memory))
        // Goals routes.
        .route("/goals/create",             post(goals::create))
        .route("/goals/list",               get(goals::list_goals))
        .route("/goals/update",             put(goals::update))
        .route("/goals/complete",           post(goals::complete))
        // Goals read-by-id (wildcard last).
        .route("/goals/:goal_id",           get(goals::read))
        .with_state(state)
}
