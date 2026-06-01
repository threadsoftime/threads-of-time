//! Application-level error type for the REST adapter.
//!
//! Kept minimal — the tool-call handler converts every case to a typed JSON
//! response with an explicit HTTP status rather than bubbling up an opaque 500.
//! This type covers the infrastructure failures that happen *before* dispatch.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("internal error: {0}")]
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let body = json!({"ok": false, "error": "internal", "detail": self.to_string()});
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(body),
        )
            .into_response()
    }
}
