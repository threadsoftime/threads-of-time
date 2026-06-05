//! Shared bearer-auth Tower layer for the MCP transport.
//!
//! On each request: read `Authorization: Bearer <tok>`, look the token up via the
//! crate-supplied [`TokenLookup`] store, and — if found — inject the looked-up
//! record (cloned) into `req.extensions_mut()` before calling the inner service.
//! Missing/unknown tokens get an immediate HTTP 401 with body
//! `{"error":"invalid_token","error_description":"Authentication required"}` and
//! header `WWW-Authenticate: Bearer`.
//!
//! Each consuming crate keeps its own `TokenStore`/`TokenRecord` and implements
//! [`TokenLookup`] for its store; handlers extract their own concrete record type
//! from `parts.extensions` (rmcp propagates `req.extensions` into `Parts.extensions`).

use std::{convert::Infallible, future::Future, mem, pin::Pin, sync::Arc, task::Poll};

use axum::body::Body;
use bytes::Bytes;
use http::{Request, Response, StatusCode};
use http_body_util::{BodyExt, Full, combinators::BoxBody};
use tower::{Layer, Service};

const AUTH_401_BODY: &[u8] =
    br#"{"error":"invalid_token","error_description":"Authentication required"}"#;

/// A token store the [`BearerAuthLayer`] can look tokens up in. Each consuming
/// crate implements this for its own `TokenStore`, choosing its own `Record` type.
pub trait TokenLookup: Send + Sync + 'static {
    /// The record injected into request extensions on a successful lookup.
    /// Must be `Clone` (injected by value) and `Send + Sync + 'static`
    /// (`http::Extensions` requires it).
    type Record: Clone + Send + Sync + 'static;

    /// Return the record for `token`, or `None` if unknown.
    fn lookup(&self, token: &str) -> Option<&Self::Record>;
}

/// Tower [`Layer`] performing bearer-token auth before forwarding to the inner service.
pub struct BearerAuthLayer<L: TokenLookup> {
    token_store: Arc<L>,
}

// Manual Clone: cloning an Arc never requires `L: Clone`.
impl<L: TokenLookup> Clone for BearerAuthLayer<L> {
    fn clone(&self) -> Self {
        Self { token_store: Arc::clone(&self.token_store) }
    }
}

impl<L: TokenLookup> BearerAuthLayer<L> {
    pub fn new(token_store: Arc<L>) -> Self {
        Self { token_store }
    }
}

impl<L: TokenLookup, S> Layer<S> for BearerAuthLayer<L> {
    type Service = BearerAuthService<L, S>;

    fn layer(&self, inner: S) -> Self::Service {
        BearerAuthService {
            inner,
            token_store: Arc::clone(&self.token_store),
        }
    }
}

/// The concrete [`Service`] produced by [`BearerAuthLayer`].
///
/// Response is concrete-typed to `BoxBody<Bytes, Infallible>` to match what
/// `rmcp`'s `StreamableHttpService` returns (avoids a generic Body bound on `S`).
pub struct BearerAuthService<L: TokenLookup, S> {
    inner:       S,
    token_store: Arc<L>,
}

// Manual Clone: requires `S: Clone` only (Arc<L> is always Clone).
impl<L: TokenLookup, S: Clone> Clone for BearerAuthService<L, S> {
    fn clone(&self) -> Self {
        Self {
            inner:       self.inner.clone(),
            token_store: Arc::clone(&self.token_store),
        }
    }
}

impl<L, S> Service<Request<Body>> for BearerAuthService<L, S>
where
    L: TokenLookup,
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
        let raw_token: Option<String> = req
            .headers()
            .get(http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "))
            .map(|s| s.to_owned());

        let maybe_record: Option<L::Record> = raw_token
            .as_deref()
            .and_then(|tok| self.token_store.lookup(tok))
            .cloned();

        if let Some(record) = maybe_record {
            req.extensions_mut().insert(record);
            // Tower readiness contract: call the inner instance poll_ready drove
            // to readiness. Two-step (clone, then replace) to satisfy the borrow
            // checker (single-expression mem::replace(&mut self.inner, self.inner.clone())
            // is rejected, E0502).
            let fresh = self.inner.clone();
            let mut inner = mem::replace(&mut self.inner, fresh);
            Box::pin(async move { inner.call(req).await })
        } else {
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::convert::Infallible;
    use std::sync::Arc;
    use std::task::Poll;

    use axum::body::Body;
    use bytes::Bytes;
    use http::{Request, Response, StatusCode};
    use http_body_util::{BodyExt, combinators::BoxBody};
    use tower::{Layer, Service};

    use super::{BearerAuthLayer, TokenLookup};

    // A minimal record + store implementing TokenLookup (stands in for each
    // crate's TokenStore/TokenRecord).
    #[derive(Clone)]
    struct TestRecord { identity: String }

    struct TestStore { by_token: HashMap<String, TestRecord> }

    impl TokenLookup for TestStore {
        type Record = TestRecord;
        fn lookup(&self, token: &str) -> Option<&TestRecord> {
            self.by_token.get(token)
        }
    }

    fn make_store() -> Arc<TestStore> {
        let mut by_token = HashMap::new();
        by_token.insert("tok-a".to_string(), TestRecord { identity: "alice".to_string() });
        by_token.insert("tok-b".to_string(), TestRecord { identity: "bob".to_string() });
        Arc::new(TestStore { by_token })
    }

    // Inner service that echoes the injected record's identity, or "MISSING".
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
                .get::<TestRecord>()
                .map(|r| r.identity.clone())
                .unwrap_or_else(|| "MISSING".to_string());
            let body_str = format!(r#"{{"identity":"{}"}}"#, identity);
            let resp = Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "application/json")
                .body(http_body_util::Full::new(Bytes::from(body_str)).map_err(|e| match e {}).boxed())
                .unwrap();
            std::future::ready(Ok(resp))
        }
    }

    async fn body_bytes(resp: Response<BoxBody<Bytes, Infallible>>) -> Bytes {
        resp.into_body().collect().await.unwrap().to_bytes()
    }
    fn req_bearer(token: &str) -> Request<Body> {
        Request::builder().method("POST").uri("/mcp/mcp")
            .header("Authorization", format!("Bearer {}", token))
            .body(Body::empty()).unwrap()
    }
    fn req_no_auth() -> Request<Body> {
        Request::builder().method("POST").uri("/mcp/mcp").body(Body::empty()).unwrap()
    }
    fn req_bad(value: &str) -> Request<Body> {
        Request::builder().method("POST").uri("/mcp/mcp")
            .header("Authorization", value).body(Body::empty()).unwrap()
    }

    #[tokio::test]
    async fn no_bearer_returns_401_with_exact_body_and_header() {
        let mut svc = BearerAuthLayer::new(make_store()).layer(EchoInner);
        let resp = svc.call(req_no_auth()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(resp.headers().get("WWW-Authenticate").and_then(|v| v.to_str().ok()), Some("Bearer"));
        assert_eq!(resp.headers().get("content-type").and_then(|v| v.to_str().ok()), Some("application/json"));
        let body = body_bytes(resp).await;
        assert_eq!(body.as_ref(), br#"{"error":"invalid_token","error_description":"Authentication required"}"#);
    }

    #[tokio::test]
    async fn bad_bearer_returns_401() {
        let mut svc = BearerAuthLayer::new(make_store()).layer(EchoInner);
        let resp = svc.call(req_bearer("totally-wrong")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn malformed_auth_header_returns_401() {
        let mut svc = BearerAuthLayer::new(make_store()).layer(EchoInner);
        let resp = svc.call(req_bad("tok-a")).await.unwrap(); // no "Bearer " prefix
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn valid_bearer_injects_record() {
        let mut svc = BearerAuthLayer::new(make_store()).layer(EchoInner);
        let resp = svc.call(req_bearer("tok-b")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_bytes(resp).await;
        assert_eq!(body.as_ref(), br#"{"identity":"bob"}"#);
    }
}
