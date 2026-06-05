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
//! | POST   | /mcp/mcp                     | MCP StreamableHTTP (when token file present) |
//!
//! Auth note: REST routes are **not** bearer-protected — only MCP/SSE uses the
//! TokenStore (Phase 6).  This replicates Python exactly; see `auth.rs` for
//! rationale.
//!
//! MCP is mounted only when `state.token_store` is `Some` (token file found at
//! startup).  When no token file is present, the `/mcp/mcp` path returns 404,
//! matching Python's conditional `create_app` behaviour.

use axum::{
    routing::{get, post, put},
    Router,
};
use crate::{
    mcp,
    routes::{
        events,
        goals,
        health::health,
        memory,
        personality,
    },
    state::AppState,
};

/// Build the axum [`Router`] with [`AppState`] injected.
///
/// `allowed_hosts` is forwarded to `mcp::build_mcp_service` →
/// `StreamableHttpServerConfig::with_allowed_hosts`.  Pass an empty `Vec` to
/// disable host-validation in tests; production passes the list built from
/// `["127.0.0.1", "localhost", <bind_host>]` + `MEM_EXTRA_ALLOWED_HOSTS`.
///
/// Mounts the MCP endpoint at `/mcp/mcp` when `state.token_store` is `Some`.
pub fn build_router(state: AppState, allowed_hosts: Vec<String>) -> Router {
    let rest = Router::new()
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
        .route("/memory/{memory_id}",       get(memory::get_memory))
        // Goals routes.
        .route("/goals/create",             post(goals::create))
        .route("/goals/list",               get(goals::list_goals))
        .route("/goals/update",             put(goals::update))
        .route("/goals/complete",           post(goals::complete))
        // Goals read-by-id (wildcard last).
        .route("/goals/{goal_id}",          get(goals::read))
        // SSE event stream.
        .route("/v1/events/stream",         get(events::stream_events))
        .with_state(state.clone());

    // Mount MCP only when a token store was loaded (matching Python's conditional).
    if let Some(token_store) = state.token_store.clone() {
        let mcp_svc = mcp::build_mcp_service(
            state.service.clone(),
            allowed_hosts,
            token_store,
        );
        rest.nest_service("/mcp/mcp", mcp_svc)
    } else {
        rest
    }
}
