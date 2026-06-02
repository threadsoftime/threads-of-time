//! rmcp 1.7.0 wire-capture spike — validates MCP transport unknowns before Phase 10 build.
//!
//! Findings targeted:
//!   (a) nest path for /mcp/mcp
//!   (b) stateful_mode=true + Python streamablehttp_client GET SSE stream
//!   (c) $ref / $defs style in schemars 1.x
//!   (d) TokenRecord extension propagation through StreamableHttpService
//!   (e) full initialize result shape
//!   (f) notifications/initialized handling
//!   (g) list_tools pagination
//!   (h) structuredContent presence
//!   (i) dotted tool names (obs.ping, bot.set_strategy)
//!   (j) 401 body and WWW-Authenticate
//!   (k) isError
//!   (l) POST framing (SSE vs JSON)
//!   (m) mcp-session-id issued
//!   (n) macro pattern confirmation

use std::{convert::Infallible, future::Future, pin::Pin, sync::Arc, task::Poll};

use axum::body::Body;
use bytes::Bytes;
use http::{Request, Response};
use http_body_util::{BodyExt, Full, combinators::BoxBody};
use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, tool::Extension},
    model::{CallToolResult, Content},
    schemars::JsonSchema,
    tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService,
        session::local::LocalSessionManager,
    },
};
use serde::{Deserialize, Serialize};
use tower::{Layer, Service};

// ---------------------------------------------------------------------------
// Auth extension — injected by bearer auth layer, read by bot.set_strategy
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct TokenRecord {
    pub identity: String,
}

// ---------------------------------------------------------------------------
// Bearer auth Tower layer
// ---------------------------------------------------------------------------

const BEARER_TOKEN: &str = "dev-all";
const AUTH_401_BODY: &[u8] =
    br#"{"error":"invalid_token","error_description":"Authentication required"}"#;

#[derive(Clone)]
pub struct BearerAuthLayer;

impl<S> Layer<S> for BearerAuthLayer {
    type Service = BearerAuthService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        BearerAuthService { inner }
    }
}

#[derive(Clone)]
pub struct BearerAuthService<S> {
    inner: S,
}

// We concrete-type the Response to BoxBody<Bytes, Infallible> since that's
// what StreamableHttpService returns. This avoids a generic Body bound on S.
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
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut std::task::Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<Body>) -> Self::Future {
        // Extract bearer token from Authorization header
        let auth_ok = req
            .headers()
            .get(http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "))
            .map(|token| token == BEARER_TOKEN)
            .unwrap_or(false);

        if !auth_ok {
            // Return 401 immediately without forwarding
            let response = Response::builder()
                .status(http::StatusCode::UNAUTHORIZED)
                .header(http::header::WWW_AUTHENTICATE, "Bearer")
                .header(http::header::CONTENT_TYPE, "application/json")
                .body(
                    Full::new(Bytes::from_static(AUTH_401_BODY))
                        .map_err(|e| match e {})
                        .boxed(),
                )
                .expect("valid 401 response");
            return Box::pin(async move { Ok(response) });
        }

        // Inject TokenRecord into request extensions so rmcp can propagate it
        // into http::request::Parts.extensions, accessible via Extension<TokenRecord>
        req.extensions_mut().insert(TokenRecord {
            identity: "dev.user".to_string(),
        });

        let mut inner = self.inner.clone();
        Box::pin(async move { inner.call(req).await })
    }
}

// ---------------------------------------------------------------------------
// Tool parameter types
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct PingWrapper {
    pub args: PingArgs,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct PingArgs {}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SetStrategyWrapper {
    pub args: SetStrategyArgs,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SetStrategyArgs {
    pub bot_guid: i64,
    pub strategy: String,
    pub bot_state: BotState,
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BotState {
    Combat,
    NonCombat,
    Dead,
    All,
}

// ---------------------------------------------------------------------------
// MCP handler
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct SpikeHandler {
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl SpikeHandler {
    /// Ping — returns {"ok":true,"result":{"pong":true}}
    #[tool(name = "obs.ping", description = "Harness liveness ping")]
    pub async fn obs_ping(
        &self,
        _params: rmcp::handler::server::wrapper::Parameters<PingWrapper>,
    ) -> CallToolResult {
        CallToolResult::success(vec![Content::text(
            r#"{"ok":true,"result":{"pong":true}}"#.to_string(),
        )])
    }

    /// Set bot strategy — also echoes the injected auth identity.
    #[tool(name = "bot.set_strategy", description = "Set a bot's strategy")]
    pub async fn bot_set_strategy(
        &self,
        _params: rmcp::handler::server::wrapper::Parameters<SetStrategyWrapper>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> CallToolResult {
        let identity = parts
            .extensions
            .get::<TokenRecord>()
            .map(|t| t.identity.as_str())
            .unwrap_or("MISSING");
        CallToolResult::success(vec![Content::text(format!(
            r#"{{"ok":true,"result":{{"identity":"{}"}}}}"#,
            identity
        ))])
    }
}

impl SpikeHandler {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }
}

// tool_handler with explicit name/version sets serverInfo in the initialize result
#[tool_handler(name = "tot-harness", version = "0.2.0")]
impl ServerHandler for SpikeHandler {}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("harness_rs=debug".parse()?)
                .add_directive("rmcp=info".parse()?),
        )
        .init();

    let session_manager = Arc::new(LocalSessionManager::default());

    // stateful_mode: true (default) — supports GET SSE stream that Python client opens.
    // allowed_hosts default already includes "localhost" and "127.0.0.1".
    // StreamableHttpServerConfig is #[non_exhaustive] — use Default + builder methods.
    let config = StreamableHttpServerConfig::default().with_stateful_mode(true);

    let svc: StreamableHttpService<SpikeHandler, LocalSessionManager> =
        StreamableHttpService::new(|| Ok(SpikeHandler::new()), session_manager, config);

    // Wrap the rmcp service in the bearer auth Tower layer.
    // Layer WRAPS the service so auth runs BEFORE rmcp consumes the body.
    // The injected TokenRecord ends up in Parts.extensions visible to tools.
    let authed_svc = BearerAuthLayer.layer(svc);

    // nest_service("/mcp/mcp", ...) strips the prefix; inner service answers /.
    // POST /mcp/mcp → POST / in StreamableHttpService — finding (a).
    let app = axum::Router::new().nest_service("/mcp/mcp", authed_svc);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:8199").await?;
    tracing::info!("rmcp spike listening on http://127.0.0.1:8199/mcp/mcp");
    eprintln!("[spike] listening on http://127.0.0.1:8199/mcp/mcp");

    axum::serve(listener, app).await?;
    Ok(())
}
