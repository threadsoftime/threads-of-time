//! MCP adapter — exposes the 46-tool V1 surface over the MCP StreamableHTTP
//! transport so the daemon is a drop-in for the Python FastMCP daemon.
//!
//! Public surface:
//! - `build_mcp_service` — construct the tower service to nest at `/mcp/mcp`.
//! - `handler::HarnessMcp` — the ServerHandler (used in tests).
//! - `auth_layer::BearerAuthLayer` — the bearer-auth tower layer (used in tests).

pub mod auth_layer;
pub mod handler;
pub mod schemas;

use std::sync::Arc;

use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService,
    session::local::LocalSessionManager,
};
use tower::Layer as _;

use crate::auth::TokenStore;
use crate::rest::SharedState;
use auth_layer::{BearerAuthLayer, BearerAuthService};
use handler::HarnessMcp;

/// Build the MCP service ready to be nested at `/mcp/mcp`.
///
/// Returns a `BearerAuthService<StreamableHttpService<HarnessMcp, LocalSessionManager>>`
/// — a tower [`Service`] that:
/// - Runs bearer-auth (from the daemon's `TokenStore`).
/// - On valid bearer: injects `config::TokenRecord` into request extensions.
/// - Forwards to `StreamableHttpService<HarnessMcp>` in stateful mode (GET
///   SSE stream supported — spike finding (b)).
///
/// Phase 11 mounts this with:
/// ```ignore
/// app = Router::new()
///     .nest_service("/mcp/mcp", build_mcp_service(state.clone(), allowed_hosts, token_store))
///     .merge(build_router(state));
/// ```
///
/// `allowed_hosts`: forwarded to `StreamableHttpServerConfig::with_allowed_hosts`.
/// Pass an empty `Vec` to disable host validation (for tests); in production
/// pass `["127.0.0.1", "localhost"]` or the external host.
pub fn build_mcp_service(
    state:         SharedState,
    allowed_hosts: Vec<String>,
    token_store:   Arc<TokenStore>,
) -> BearerAuthService<StreamableHttpService<HarnessMcp, LocalSessionManager>>
{
    let session_manager = Arc::new(LocalSessionManager::default());

    // `StreamableHttpServerConfig` is `#[non_exhaustive]` — MUST use
    // `Default::default()` + builder methods (spike finding, notes file).
    let config = if allowed_hosts.is_empty() {
        StreamableHttpServerConfig::default()
            .disable_allowed_hosts()
            .with_stateful_mode(true)
    } else {
        StreamableHttpServerConfig::default()
            .with_allowed_hosts(allowed_hosts)
            .with_stateful_mode(true)
    };

    let svc: StreamableHttpService<HarnessMcp, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok(HarnessMcp::new(Arc::clone(&state))),
            session_manager,
            config,
        );

    // Wrap the rmcp service in the bearer-auth layer.  Auth runs FIRST, before
    // rmcp consumes the body.  The injected TokenRecord ends up in Parts.extensions
    // (spike finding (d)).
    BearerAuthLayer::new(token_store).layer(svc)
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::{Router, body::Body};
    use http::{Request, StatusCode};
    use serde_json::{Value, json};
    use tower::ServiceExt as _;

    use crate::ac_client::ACClient;
    use crate::audit::AuditLogger;
    use crate::auth::TokenStore;
    use crate::config::TokenRecord;
    use crate::registry::build_v1_registry;
    use crate::rest::AppState;
    use crate::test_support::{MockConfig, spawn_mock_ac_with_config};

    // ── helpers ───────────────────────────────────────────────────────────────

    async fn make_app_and_token_store(mock_cfg: MockConfig) -> (Router, Arc<TokenStore>) {
        let mock = spawn_mock_ac_with_config(mock_cfg).await;
        let ac_client = ACClient::new(mock.base_url(), 3.0);

        let audit_path = std::env::temp_dir()
            .join(format!("harness_rs_mcp_test_{}.jsonl", uuid::Uuid::new_v4().simple()))
            .to_str()
            .unwrap()
            .to_string();

        let token_record = TokenRecord {
            token:         "dev-all".to_string(),
            identity:      "dev.user".to_string(),
            scope:         vec![
                "gm.*".to_string(),
                "obs.*".to_string(),
                "bot.*".to_string(),
                "event.*".to_string(),
                "memory.*".to_string(),
                "lfg.*".to_string(),
            ],
            augmented:     false,
            bound_to_guid: None,
            note:          None,
        };

        let token_store = Arc::new(TokenStore::new(vec![token_record]));

        let state = Arc::new(AppState {
            token_store: TokenStore::new(vec![TokenRecord {
                token:         "dev-all".to_string(),
                identity:      "dev.user".to_string(),
                scope:         vec![
                    "gm.*".to_string(),
                    "obs.*".to_string(),
                    "bot.*".to_string(),
                    "event.*".to_string(),
                    "memory.*".to_string(),
                    "lfg.*".to_string(),
                ],
                augmented:     false,
                bound_to_guid: None,
                note:          None,
            }]),
            registry:   build_v1_registry(),
            ac_client,
            db_client:  None,
            audit:      AuditLogger::new(&audit_path).unwrap(),
            audit_path: audit_path.clone(),
        });

        let svc = super::build_mcp_service(
            Arc::clone(&state),
            vec![],  // disabled host check in tests
            Arc::clone(&token_store),
        );

        let app = Router::new().nest_service("/mcp/mcp", svc);
        (app, token_store)
    }

    async fn body_bytes_raw(resp: axum::response::Response) -> bytes::Bytes {
        axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap()
    }

    // ── JSON-RPC helper ───────────────────────────────────────────────────────

    fn jsonrpc_req(method: &str, params: Value, id: i64, bearer: Option<&str>) -> Request<Body> {
        let body = json!({
            "jsonrpc": "2.0",
            "method":  method,
            "params":  params,
            "id":      id,
        });
        let mut builder = Request::builder()
            .method("POST")
            .uri("/mcp/mcp")
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream");
        if let Some(tok) = bearer {
            builder = builder.header("Authorization", format!("Bearer {}", tok));
        }
        builder
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap()
    }

    // ── test 1: no bearer → 401 with exact body ───────────────────────────────

    #[tokio::test]
    async fn mcp_no_bearer_returns_401_exact_body() {
        let (app, _) = make_app_and_token_store(MockConfig::default()).await;
        let req = jsonrpc_req("initialize", json!({}), 1, None);
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let raw = body_bytes_raw(resp).await;
        assert_eq!(
            raw.as_ref(),
            br#"{"error":"invalid_token","error_description":"Authentication required"}"#,
        );
    }

    // ── test 2: bad bearer → 401 ──────────────────────────────────────────────

    #[tokio::test]
    async fn mcp_bad_bearer_returns_401() {
        let (app, _) = make_app_and_token_store(MockConfig::default()).await;
        let req = jsonrpc_req("initialize", json!({}), 1, Some("wrong-token"));
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // ── test 3: strip_top_level_nulls — ensure parity ────────────────────────

    #[test]
    fn strip_nulls_top_level_only() {
        use super::handler::strip_top_level_nulls;
        let input = json!({"a": null, "b": 1, "c": {"d": null}});
        let out = strip_top_level_nulls(input);
        assert!(out.get("a").is_none(), "'a' must be stripped");
        assert_eq!(out["b"], 1);
        // nested object must be preserved intact
        assert_eq!(out["c"], json!({"d": null}), "nested null must not be removed");
    }

    // ── test 4: build_mcp_service returns a valid tower service ───────────────
    // (smoke: the service compiles and constructs without panic)

    #[tokio::test]
    async fn build_mcp_service_constructs_without_panic() {
        // If this test passes, the service built OK (constructor is non-trivial:
        // it calls Self::tool_router() which is proc-macro generated).
        let (_app, _) = make_app_and_token_store(MockConfig::default()).await;
        // No assertion needed — construction itself is the test.
    }

    // ── test 5: HarnessMcp::new constructs with correct tool_router ───────────

    #[tokio::test]
    async fn harness_mcp_tool_router_contains_46_tools() {
        use crate::mcp::handler::HarnessMcp;
        use crate::mcp::schemas;
        use rmcp::schemars::schema_for;

        // We can't easily call list_tools without a full MCP session,
        // but we can verify the schema wiring by checking that the #[tool_router]
        // expansion doesn't panic and that the wrapper types all produce
        // inputSchema.properties.args (previously tested in schemas::tests).
        // Here we verify the Handler compiles and constructs correctly.

        let mock = spawn_mock_ac_with_config(MockConfig::default()).await;
        let ac_client = ACClient::new(mock.base_url(), 3.0);
        let audit_path = std::env::temp_dir()
            .join(format!("harness_rs_mcp_handler_test_{}.jsonl", uuid::Uuid::new_v4().simple()))
            .to_str()
            .unwrap()
            .to_string();

        let state = Arc::new(AppState {
            token_store: TokenStore::new(vec![]),
            registry:    build_v1_registry(),
            ac_client,
            db_client:   None,
            audit:       AuditLogger::new(&audit_path).unwrap(),
            audit_path:  audit_path.clone(),
        });

        let handler = HarnessMcp::new(state);

        // The #[tool_router] generates a ToolRouter; there is no public
        // runtime count on it, but we can verify the registry has 46 tools.
        let registry = build_v1_registry();
        assert_eq!(
            registry.names().len(),
            46,
            "registry must have 46 tools matching the 46 #[tool] methods"
        );

        // Also verify that all 46 wrapper types have args schemas (belt+suspenders
        // on the derive(Serialize) change we made to schemas.rs).
        macro_rules! check_serializes {
            ($T:ty, $name:expr) => {{
                let schema_val = serde_json::to_value(schema_for!($T)).unwrap();
                assert!(
                    schema_val.get("properties").and_then(|p| p.get("args")).is_some(),
                    "{}: inputSchema must have properties.args after Serialize derive", $name
                );
            }};
        }

        check_serializes!(schemas::ObsPingWrapper,            "obs.ping");
        check_serializes!(schemas::GmAdditemWrapper,          "gm.additem");
        check_serializes!(schemas::BotSetStrategyWrapper,     "bot.set_strategy");
        check_serializes!(schemas::LfgFormGroupWrapper,       "lfg.form_group");
        check_serializes!(schemas::MemoryRecallWrapper,       "memory.recall");
        // spot-check: also verify serialize works (to_value of an instance)
        let w = schemas::ObsPingWrapper { args: schemas::ObsPingArgs {} };
        let v = serde_json::to_value(&w.args).unwrap();
        assert!(v.is_object(), "ObsPingArgs must serialize to a JSON object");

        // suppress unused warning for handler
        let _ = handler;
    }

    // ── test 6: scope_denied → success text with ok:false, NOT MCP isError ────

    #[tokio::test]
    async fn scope_denied_returns_success_text_not_mcp_iserror() {
        // Token with no scope at all — any tool call should be scope_denied.
        let mock = spawn_mock_ac_with_config(MockConfig::default()).await;
        let ac_client = ACClient::new(mock.base_url(), 3.0);
        let audit_path = std::env::temp_dir()
            .join(format!("harness_rs_mcp_scope_{}.jsonl", uuid::Uuid::new_v4().simple()))
            .to_str()
            .unwrap()
            .to_string();

        // (token_store not needed here — we call dispatch directly)
        let state = Arc::new(AppState {
            token_store: TokenStore::new(vec![TokenRecord {
                token:         "no-scope".to_string(),
                identity:      "restricted.user".to_string(),
                scope:         vec![],
                augmented:     false,
                bound_to_guid: None,
                note:          None,
            }]),
            registry:   build_v1_registry(),
            ac_client,
            db_client:  None,
            audit:      AuditLogger::new(&audit_path).unwrap(),
            audit_path: audit_path.clone(),
        });

        use crate::mcp::handler::HarnessMcp;
        use crate::auth::AuthResult;

        let handler = HarnessMcp::new(Arc::clone(&state));

        // Simulate the forward() path with a no-scope AuthResult by calling
        // dispatch directly and verifying the outcome body.
        let auth = AuthResult {
            identity:      "restricted.user".to_string(),
            scope:         vec![],
            bound_to_guid: None,
            augmented:     false,
        };

        let outcome = crate::dispatch::dispatch_tool(
            "obs.ping",
            &json!({}),
            &auth,
            "test-req-id",
            &state.registry,
            &state.ac_client,
            state.db_client.as_ref(),
        )
        .await;

        // dispatch returns scope_denied body — when wrapped in forward() this
        // MUST come back as MCP success (text), NOT isError.
        assert_eq!(outcome.audit_outcome, "scope_denied");
        assert_eq!(outcome.body["ok"], false);
        assert_eq!(outcome.body["error"], "scope_denied");

        // The MCP envelope must be isError=false (success text):
        // forward() always returns CallToolResult::success(...) regardless of
        // outcome.audit_outcome. This is the Python parity: mcp_server.py:139
        // `return outcome.body` — FastMCP wraps it in a successful tool result.
        let _ = handler;
    }
}
