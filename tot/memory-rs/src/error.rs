//! Unified error type for all route handlers.
//!
//! All variants implement [`axum::response::IntoResponse`] and produce a
//! `{"detail": "..."}` JSON body to match the FastAPI error shape the brain-
//! sidecar and harness-daemon callers already expect.
//!
//! # Validation discipline
//!
//! Request range/length validation is done explicitly in each handler,
//! returning `AppError::BadRequest`. FastAPI's 422 Unprocessable Entity body
//! is not reproduced — this is a documented acceptable divergence (callers
//! send well-formed input; the brain-sidecar constructs requests from typed
//! Pydantic models).

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// 404 — resource not found; `msg` is the `{"detail": msg}` string.
    #[error("not found: {0}")]
    NotFound(&'static str),

    /// 400 — invalid request; `s` is the `{"detail": s}` string.
    #[error("bad request: {0}")]
    BadRequest(String),

    /// 503 — the embedding service is unavailable. Raised by write, update
    /// (text change), recall, recall_about, and search — all paths that
    /// require a live embedder. Matches Python: no try/except around embed().
    #[error("embedding service unavailable")]
    EmbeddingUnavailable,

    /// 500 — SQLite error propagated via `?`.
    #[error("db error: {0}")]
    Db(#[from] rusqlite::Error),

    /// 500 — any other internal error (e.g. spawn_blocking JoinError).
    #[error("internal error: {0}")]
    Internal(#[from] anyhow::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, detail) = match &self {
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.to_string()),
            AppError::BadRequest(s) => (StatusCode::BAD_REQUEST, s.clone()),
            AppError::EmbeddingUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "embedding_service_unavailable".to_string(),
            ),
            AppError::Db(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error".to_string(),
            ),
            AppError::Internal(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error".to_string(),
            ),
        };
        (status, Json(serde_json::json!({ "detail": detail }))).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    async fn body_json(resp: Response) -> serde_json::Value {
        let bytes = to_bytes(resp.into_body(), 4096).await.expect("body bytes");
        serde_json::from_slice(&bytes).expect("valid JSON body")
    }

    #[tokio::test]
    async fn not_found_returns_404_with_detail() {
        let resp = AppError::NotFound("episode_not_found").into_response();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let body = body_json(resp).await;
        assert_eq!(body["detail"], "episode_not_found");
    }

    #[tokio::test]
    async fn bad_request_returns_400_with_detail() {
        let resp = AppError::BadRequest("no_fields_to_update".to_string()).into_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = body_json(resp).await;
        assert_eq!(body["detail"], "no_fields_to_update");
    }

    #[tokio::test]
    async fn embedding_unavailable_returns_503() {
        let resp = AppError::EmbeddingUnavailable.into_response();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = body_json(resp).await;
        assert_eq!(body["detail"], "embedding_service_unavailable");
    }

    #[tokio::test]
    async fn db_error_returns_500_with_internal_error_detail() {
        let db_err =
            rusqlite::Error::QueryReturnedNoRows;
        let resp = AppError::Db(db_err).into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = body_json(resp).await;
        assert_eq!(body["detail"], "internal_error");
    }

    #[tokio::test]
    async fn internal_error_returns_500_with_internal_error_detail() {
        let resp =
            AppError::Internal(anyhow::anyhow!("some internal cause")).into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = body_json(resp).await;
        assert_eq!(body["detail"], "internal_error");
    }

    #[test]
    fn from_rusqlite_error_converts_via_question_mark() {
        fn may_fail() -> Result<(), AppError> {
            Err(rusqlite::Error::QueryReturnedNoRows)?;
            Ok(())
        }
        assert!(matches!(may_fail(), Err(AppError::Db(_))));
    }
}
