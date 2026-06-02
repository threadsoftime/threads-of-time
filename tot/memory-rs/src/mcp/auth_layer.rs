//! Bearer-auth Tower layer for the MCP adapter.
//!
//! Port of harness-rs `mcp/auth_layer.rs` adapted for the memory-rs
//! `TokenStore` (which uses `.verify()` instead of `.find()` and carries a
//! simpler `TokenRecord { token, identity, scope }`).
//!
//! On each request:
//! - Read `Authorization: Bearer <tok>` header.
//! - Look up the daemon's `TokenStore`.
//! - If missing/unknown → HTTP 401 with body
//!   `{"error":"invalid_token","error_description":"Authentication required"}`
//!   and header `WWW-Authenticate: Bearer`.
//! - If found → insert the full `auth::TokenRecord` (cloned) into
//!   `req.extensions_mut()` and call the inner service.

use std::{convert::Infallible, future::Future, mem, pin::Pin, sync::Arc, task::Poll};

use axum::body::Body;
use bytes::Bytes;
use http::{Request, Response, StatusCode};
use http_body_util::{BodyExt, Full, combinators::BoxBody};
use tower::{Layer, Service};

use crate::auth::{TokenRecord, TokenStore};

// ── Wire constants ────────────────────────────────────────────────────────────

const AUTH_401_BODY: &[u8] =
    br#"{"error":"invalid_token","error_description":"Authentication required"}"#;

// ── BearerAuthLayer ───────────────────────────────────────────────────────────

/// Tower [`Layer`] that performs bearer-token auth before forwarding to
/// the inner `StreamableHttpService`.
#[derive(Clone)]
pub struct BearerAuthLayer {
    token_store: Arc<TokenStore>,
}

impl BearerAuthLayer {
    pub fn new(token_store: Arc<TokenStore>) -> Self {
        Self { token_store }
    }
}

impl<S> Layer<S> for BearerAuthLayer {
    type Service = BearerAuthService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        BearerAuthService {
            inner,
            token_store: Arc::clone(&self.token_store),
        }
    }
}

// ── BearerAuthService ─────────────────────────────────────────────────────────

/// The concrete [`Service`] produced by [`BearerAuthLayer`].
#[derive(Clone)]
pub struct BearerAuthService<S> {
    inner:       S,
    token_store: Arc<TokenStore>,
}

impl<S> Service<Request<Body>> for BearerAuthService<S>
where
    S: Service<Request<Body>, Response = Response<BoxBody<Bytes, Infallible>>>
        + Clone
        + Send
        + 'static,
    S::Error: Into<Infallible>,
    S::Future: Send + 'static,
{
    type Response = S::Response;
    type Error    = S::Error;
    type Future   = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut std::task::Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<Body>) -> Self::Future {
        // ── Extract bearer token ───────────────────────────────────────────
        let raw_token: Option<String> = req
            .headers()
            .get(http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "))
            .map(|s| s.to_owned());

        // ── Lookup in TokenStore (verify returns Option<&TokenRecord>) ─────
        let maybe_record: Option<TokenRecord> = raw_token
            .as_deref()
            .and_then(|tok| self.token_store.verify(tok))
            .cloned();

        if let Some(record) = maybe_record {
            // Inject the full TokenRecord into request extensions.
            req.extensions_mut().insert(record);
            // Tower readiness contract: the ready instance must be called.
            // Clone first (immutable borrow), then replace (mutable borrow).
            let fresh = self.inner.clone();
            let mut inner = mem::replace(&mut self.inner, fresh);
            Box::pin(async move { inner.call(req).await })
        } else {
            // Return 401 immediately — do NOT forward to inner service.
            Box::pin(async move {
                let response = Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .header(http::header::WWW_AUTHENTICATE, "Bearer")
                    .header(http::header::CONTENT_TYPE, "application/json")
                    .body(
                        Full::new(Bytes::from_static(AUTH_401_BODY))
                            .map_err(|e| match e {})
                            .boxed(),
                    )
                    .expect("valid 401 response shape");
                Ok(response)
            })
        }
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::convert::Infallible;
    use std::io::Write;
    use std::sync::Arc;
    use std::task::Poll;

    use axum::body::Body;
    use bytes::Bytes;
    use http::{Request, Response, StatusCode};
    use http_body_util::{BodyExt, combinators::BoxBody};
    use tower::{Layer, Service};

    use super::BearerAuthLayer;
    use crate::auth::{TokenRecord, TokenStore};

    // ── helpers ───────────────────────────────────────────────────────────────

    fn make_token_store() -> Arc<TokenStore> {
        let yaml = r#"
tokens:
  - token: "dev-all"
    identity: "dev.user"
    scope: ["memory.read", "memory.write"]
"#;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(yaml.as_bytes()).unwrap();
        Arc::new(TokenStore::load(f.path()).unwrap())
    }

    /// Minimal inner service that echoes whether it saw a `TokenRecord` in
    /// `req.extensions`. Returns `{"identity": "<id>"}` if found, else `{"identity":"MISSING"}`.
    #[derive(Clone)]
    struct EchoInner;

    impl Service<Request<Body>> for EchoInner {
        type Response = Response<BoxBody<Bytes, Infallible>>;
        type Error    = Infallible;
        type Future   = std::future::Ready<Result<Self::Response, Self::Error>>;

        fn poll_ready(&mut self, _cx: &mut std::task::Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, req: Request<Body>) -> Self::Future {
            let identity = req
                .extensions()
                .get::<TokenRecord>()
                .map(|r| r.identity.clone())
                .unwrap_or_else(|| "MISSING".to_string());

            let body_str = format!(r#"{{"identity":"{}"}}"#, identity);
            let resp = Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "application/json")
                .body(
                    http_body_util::Full::new(Bytes::from(body_str))
                        .map_err(|e| match e {})
                        .boxed(),
                )
                .unwrap();
            std::future::ready(Ok(resp))
        }
    }

    async fn body_bytes(resp: Response<BoxBody<Bytes, Infallible>>) -> Bytes {
        resp.into_body().collect().await.unwrap().to_bytes()
    }

    fn req_with_bearer(token: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/mcp/mcp")
            .header("Authorization", format!("Bearer {}", token))
            .body(Body::empty())
            .unwrap()
    }

    fn req_no_auth() -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/mcp/mcp")
            .body(Body::empty())
            .unwrap()
    }

    fn req_bad_auth(value: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/mcp/mcp")
            .header("Authorization", value)
            .body(Body::empty())
            .unwrap()
    }

    // ── 1. No Authorization header → 401 with exact body + WWW-Authenticate ──

    #[tokio::test]
    async fn no_bearer_returns_401_with_exact_body_and_header() {
        let layer = BearerAuthLayer::new(make_token_store());
        let mut svc = layer.layer(EchoInner);

        let resp = svc.call(req_no_auth()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        assert_eq!(
            resp.headers().get("WWW-Authenticate").and_then(|v| v.to_str().ok()),
            Some("Bearer"),
        );
        assert_eq!(
            resp.headers().get("content-type").and_then(|v| v.to_str().ok()),
            Some("application/json"),
        );

        let body = body_bytes(resp).await;
        assert_eq!(
            body.as_ref(),
            br#"{"error":"invalid_token","error_description":"Authentication required"}"#,
        );
    }

    // ── 2. Bad bearer (wrong token) → 401 ────────────────────────────────────

    #[tokio::test]
    async fn bad_bearer_returns_401() {
        let layer = BearerAuthLayer::new(make_token_store());
        let mut svc = layer.layer(EchoInner);

        let resp = svc.call(req_with_bearer("totally-wrong-token")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let body = body_bytes(resp).await;
        assert_eq!(
            body.as_ref(),
            br#"{"error":"invalid_token","error_description":"Authentication required"}"#,
        );
    }

    // ── 3. Malformed header (no Bearer prefix) → 401 ─────────────────────────

    #[tokio::test]
    async fn malformed_auth_header_returns_401() {
        let layer = BearerAuthLayer::new(make_token_store());
        let mut svc = layer.layer(EchoInner);

        let resp = svc.call(req_bad_auth("dev-all")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // ── 4. Valid bearer → inner sees TokenRecord in extensions ────────────────

    #[tokio::test]
    async fn valid_bearer_inner_sees_token_record() {
        let layer = BearerAuthLayer::new(make_token_store());
        let mut svc = layer.layer(EchoInner);

        let resp = svc.call(req_with_bearer("dev-all")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = body_bytes(resp).await;
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["identity"], "dev.user", "inner service must see injected identity");
    }

    // ── 5. Two-token store — second token verified correctly ──────────────────

    #[tokio::test]
    async fn two_token_store_second_token_verified() {
        let yaml = r#"
tokens:
  - token: "tok-a"
    identity: "alice"
    scope: []
  - token: "tok-b"
    identity: "bob"
    scope: []
"#;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(yaml.as_bytes()).unwrap();
        let store = Arc::new(TokenStore::load(f.path()).unwrap());

        let layer = BearerAuthLayer::new(store);
        let mut svc = layer.layer(EchoInner);

        let resp = svc.call(req_with_bearer("tok-b")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_bytes(resp).await;
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["identity"], "bob");
    }
}
