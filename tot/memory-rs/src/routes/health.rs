//! GET /health — liveness probe.
//!
//! Returns `{"ok": true}`, matching the Python memory-sidecar v0.2.1 contract.
//! No side effects; no database access; no state required.

use axum::Json;

/// Handler for `GET /health`.
pub async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn health_returns_ok_true() {
        let Json(body) = health().await;
        assert_eq!(body, serde_json::json!({"ok": true}));
    }
}
