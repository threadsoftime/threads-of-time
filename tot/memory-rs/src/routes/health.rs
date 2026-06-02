//! GET /health — liveness probe.
//!
//! Returns `{"status": "ok"}`, matching the Python FastAPI health body exactly.
//! No side effects; no database access; no state required.

use axum::Json;

/// Handler for `GET /health`.
pub async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn health_returns_status_ok() {
        let Json(body) = health().await;
        assert_eq!(body["status"], "ok");
    }
}
