//! Unified error type with axum IntoResponse. Full implementation in Task 0.4.
use axum::{http::StatusCode, response::{IntoResponse, Response}, Json};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("not found")]
    NotFound(&'static str),
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("embedding service unavailable")]
    EmbeddingUnavailable,
    #[error("db: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("internal: {0}")]
    Internal(#[from] anyhow::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        todo!("Task 0.4")
    }
}
