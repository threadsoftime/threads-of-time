//! Shared application state injected into axum handlers via [`axum::extract::State`].
//!
//! `AppState` is `Clone` (required by axum) — all heavy fields are `Arc`-wrapped
//! so clone is O(1) reference-count increments.

use std::sync::Arc;

use crate::auth::TokenStore;
use crate::core::MemoryService;
use crate::pubsub::PubSub;

/// Axum application state — cloned into every handler invocation.
///
/// Constructed in `main.rs` after all services are wired up.
#[derive(Clone)]
pub struct AppState {
    /// The core memory/goals/personality service.
    pub service: Arc<MemoryService>,
    /// Bearer token store for MCP/SSE auth.
    ///
    /// `None` when no token file was found at startup (MCP transport is
    /// disabled; REST routes are unaffected — they are never bearer-protected).
    pub token_store: Option<Arc<TokenStore>>,
    /// Pub/sub bus shared with the SSE route (Phase 6).
    pub pubsub: Arc<PubSub>,
}

impl AppState {
    /// Construct an `AppState` for use in route handler tests.
    ///
    /// Accepts pre-built service + pubsub; token_store is always `None` in
    /// tests (REST routes are not auth-protected).
    pub fn for_test(service: Arc<MemoryService>, pubsub: Arc<PubSub>) -> Self {
        AppState {
            service,
            token_store: None,
            pubsub,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use crate::config::{RecencyBasis, ScoringWeights};
    use crate::embed_cache::EmbedCache;
    use crate::embeddings::EmbeddingsClient;

    fn make_service(db_path: PathBuf) -> Arc<MemoryService> {
        let client = EmbeddingsClient::new("http://127.0.0.1:11434/v1", "embedding", "");
        let embed = Arc::new(EmbedCache::new(client));
        let pubsub = Arc::new(PubSub::new());
        let weights = ScoringWeights {
            w_rel: 0.5,
            w_rec: 0.2,
            w_imp: 0.3,
            tau_seconds: 604800,
        };
        let mut svc = MemoryService::new(db_path, weights, 2000, embed, pubsub);
        svc.mmr_lambda = 0.7;
        svc.recency_basis = RecencyBasis::Created;
        Arc::new(svc)
    }

    /// AppState must be Clone — axum requires it.
    #[test]
    fn app_state_is_clone() {
        let tmp = tempfile::NamedTempFile::new().expect("tempfile");
        let svc = make_service(tmp.path().to_path_buf());
        let pubsub = Arc::new(PubSub::new());
        let state = AppState::for_test(svc, pubsub);
        let _cloned = state.clone();
    }

    /// token_store is None in for_test.
    #[test]
    fn for_test_has_no_token_store() {
        let tmp = tempfile::NamedTempFile::new().expect("tempfile");
        let svc = make_service(tmp.path().to_path_buf());
        let pubsub = Arc::new(PubSub::new());
        let state = AppState::for_test(svc, pubsub);
        assert!(state.token_store.is_none());
    }
}
