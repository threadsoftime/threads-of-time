// SPDX-License-Identifier: AGPL-3.0
//! Bearer-auth middleware for the brain-rs axum app.
//!
//! Faithful port of the `_check_auth` FastAPI dependency in `brain_sidecar/api.py`:
//!
//! - `brain_bearer` empty → auth disabled; all requests pass through.
//! - Missing or malformed `Authorization` header → HTTP 401.
//! - Token mismatch → HTTP 403.
//! - Correct token → request passes through.
//!
//! Implemented as an axum `middleware::from_fn_with_state(brain_bearer, check_bearer)`.
//! Wire it with `Router::layer(middleware::from_fn_with_state(...))` on any
//! route that requires auth.

use axum::{
    body::Body,
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};

// ---------------------------------------------------------------------------
// Middleware function
// ---------------------------------------------------------------------------

/// Axum middleware function for bearer-token auth.
///
/// State is the `brain_bearer` string from `Settings`.
/// Empty bearer → bypass (dev mode, matching Python `if not brain_bearer: return`).
pub async fn check_bearer(
    State(brain_bearer): State<String>,
    req: Request<Body>,
    next: Next,
) -> Response {
    if brain_bearer.is_empty() {
        // Auth disabled — pass through.
        return next.run(req).await;
    }

    let auth_header = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    match auth_header {
        None => (StatusCode::UNAUTHORIZED, "missing bearer").into_response(),
        Some(h) if !h.starts_with("Bearer ") => {
            (StatusCode::UNAUTHORIZED, "missing bearer").into_response()
        }
        Some(h) => {
            let token = h["Bearer ".len()..].trim();
            if token == brain_bearer {
                next.run(req).await
            } else {
                (StatusCode::FORBIDDEN, "invalid bearer").into_response()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        middleware,
        response::IntoResponse,
        routing::get,
        Router,
    };
    use tower::ServiceExt;

    /// Build a router with a single GET /test route, optionally protected by
    /// bearer auth.
    fn build_app(bearer: &str) -> Router {
        let route = Router::new().route("/test", get(|| async { (StatusCode::OK, "ok") }));
        if bearer.is_empty() {
            // No auth — skip middleware (matches "auth disabled" semantics for tests
            // where we want zero state)
            route
        } else {
            route.layer(middleware::from_fn_with_state(
                bearer.to_string(),
                check_bearer,
            ))
        }
    }

    /// Build a router that ALWAYS applies the middleware (even with empty bearer),
    /// testing the "auth disabled" passthrough path inside the middleware itself.
    fn build_app_always_middleware(bearer: &str) -> Router {
        Router::new()
            .route("/test", get(|| async { (StatusCode::OK, "ok") }))
            .layer(middleware::from_fn_with_state(
                bearer.to_string(),
                check_bearer,
            ))
    }

    async fn parse_status(resp: axum::response::Response) -> StatusCode {
        resp.status()
    }

    #[tokio::test]
    async fn test_auth_disabled_when_bearer_empty() {
        let app = build_app_always_middleware("");
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(parse_status(resp).await, StatusCode::OK);
    }

    #[tokio::test]
    async fn test_auth_passes_with_correct_bearer() {
        let app = build_app_always_middleware("secret");
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .header("Authorization", "Bearer secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(parse_status(resp).await, StatusCode::OK);
    }

    #[tokio::test]
    async fn test_auth_401_missing_bearer() {
        let app = build_app_always_middleware("secret");
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(parse_status(resp).await, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_auth_403_wrong_bearer() {
        let app = build_app_always_middleware("secret");
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .header("Authorization", "Bearer wrong")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(parse_status(resp).await, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_auth_401_malformed_no_bearer_prefix() {
        let app = build_app_always_middleware("secret");
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .header("Authorization", "Basic secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(parse_status(resp).await, StatusCode::UNAUTHORIZED);
    }
}
