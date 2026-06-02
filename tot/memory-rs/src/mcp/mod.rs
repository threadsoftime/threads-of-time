//! MCP adapter — exposes 15 memory/goals tools over the MCP StreamableHTTP
//! transport (port 8090, path `/mcp/mcp`, bearer auth).
//!
//! Public surface:
//! - `build_mcp_service` — construct the tower service to nest at `/mcp/mcp`.
//! - `handler::MemoryMcp` — the ServerHandler (used in tests).
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
use crate::core::MemoryService;
use auth_layer::{BearerAuthLayer, BearerAuthService};
use handler::MemoryMcp;

/// Build the MCP service ready to be nested at `/mcp/mcp`.
///
/// Returns a `BearerAuthService<StreamableHttpService<MemoryMcp, LocalSessionManager>>`
/// — a tower [`Service`] that:
/// - Runs bearer-auth (from the daemon's `TokenStore`).
/// - On valid bearer: injects `auth::TokenRecord` into request extensions.
/// - Forwards to `StreamableHttpService<MemoryMcp>` in stateful mode.
///
/// Called from `app::build_router` (which receives the list from `main.rs`).
///
/// `allowed_hosts`: passed to `StreamableHttpServerConfig::with_allowed_hosts`.
/// Pass an empty `Vec` to disable host validation (tests); in production pass
/// `["127.0.0.1", "localhost", <bind_host>]` + `MEM_EXTRA_ALLOWED_HOSTS` entries.
/// The brain connects via `192.168.1.3:8090`; the Quadlet must set
/// `MEM_EXTRA_ALLOWED_HOSTS=192.168.1.3:8090,192.168.1.3` to admit that Host header.
pub fn build_mcp_service(
    service:       Arc<MemoryService>,
    allowed_hosts: Vec<String>,
    token_store:   Arc<TokenStore>,
) -> BearerAuthService<StreamableHttpService<MemoryMcp, LocalSessionManager>>
{
    let session_manager = Arc::new(LocalSessionManager::default());

    let config = if allowed_hosts.is_empty() {
        StreamableHttpServerConfig::default()
            .disable_allowed_hosts()
            .with_stateful_mode(true)
    } else {
        StreamableHttpServerConfig::default()
            .with_allowed_hosts(allowed_hosts)
            .with_stateful_mode(true)
    };

    let svc: StreamableHttpService<MemoryMcp, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok(MemoryMcp::new(Arc::clone(&service))),
            session_manager,
            config,
        );

    BearerAuthLayer::new(token_store).layer(svc)
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::Arc;

    use axum::{Router, body::Body};
    use http::Request;
    use http::StatusCode;
    use serde_json::{Value, json};
    use tower::ServiceExt as _;

    use crate::auth::TokenStore;
    use crate::config::{RecencyBasis, ScoringWeights};
    use crate::core::MemoryService;
    use crate::db::{migrate, open_db, register_vec0};
    use crate::embed_cache::EmbedCache;
    use crate::embeddings::EmbeddingsClient;
    use crate::pubsub::PubSub;

    // ── helpers ───────────────────────────────────────────────────────────────

    /// Build a `MemoryService` backed by a temporary SQLite file with migrations applied.
    async fn make_service() -> (Arc<MemoryService>, tempfile::NamedTempFile) {
        // Ensure vec0 is registered before any connection opens.
        register_vec0();

        let db_file = tempfile::NamedTempFile::new().unwrap();
        let db_path = db_file.path().to_path_buf();

        // Apply migrations.
        let migrations_dir = PathBuf::from(
            std::env::var("MIGRATIONS_DIR")
                .unwrap_or_else(|_| "migrations".to_owned()),
        );
        {
            let conn = open_db(&db_path).unwrap();
            // Best-effort: if migrations dir doesn't exist in test env, continue.
            let _ = migrate::run(&conn, &migrations_dir);
        }

        let client = EmbeddingsClient::new("http://127.0.0.1:11434/v1", "embedding", "");
        let embed = Arc::new(EmbedCache::new(client));
        let pubsub = Arc::new(PubSub::new());
        let weights = ScoringWeights {
            w_rel: 0.5,
            w_rec: 0.2,
            w_imp: 0.3,
            tau_seconds: 604800,
        };
        let mut svc = MemoryService::new(db_path, weights, 2000, embed, pubsub);
        svc.mmr_lambda = 0.7;
        svc.recency_basis = RecencyBasis::Created;
        (Arc::new(svc), db_file)
    }

    fn make_token_store_with(token: &str, identity: &str) -> Arc<TokenStore> {
        let yaml = format!(
            "tokens:\n  - token: \"{token}\"\n    identity: \"{identity}\"\n    scope: []\n"
        );
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(yaml.as_bytes()).unwrap();
        let path = f.path().to_path_buf();
        // Keep f alive long enough for load, then let it drop.
        let store = TokenStore::load(&path).unwrap();
        // SAFETY: NamedTempFile is still alive here; store is loaded.
        let _ = f; // explicit drop after load
        Arc::new(store)
    }

    async fn make_app(token: &str) -> Router {
        let (service, _db) = make_service().await;
        let token_store = make_token_store_with(token, "test.user");
        let svc = super::build_mcp_service(
            Arc::clone(&service),
            vec![],  // disable host check in tests
            Arc::clone(&token_store),
        );
        Router::new().nest_service("/mcp/mcp", svc)
    }

    async fn body_bytes(resp: axum::response::Response) -> bytes::Bytes {
        use http_body_util::BodyExt;
        resp.into_body().collect().await.unwrap().to_bytes()
    }

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
        let app = make_app("dev-all").await;
        let req = jsonrpc_req("initialize", json!({}), 1, None);
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let raw = body_bytes(resp).await;
        assert_eq!(
            raw.as_ref(),
            br#"{"error":"invalid_token","error_description":"Authentication required"}"#,
        );
    }

    // ── test 2: bad bearer → 401 ──────────────────────────────────────────────

    #[tokio::test]
    async fn mcp_bad_bearer_returns_401() {
        let app = make_app("dev-all").await;
        let req = jsonrpc_req("initialize", json!({}), 1, Some("wrong-token"));
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // ── test 3: build_mcp_service constructs without panic ───────────────────

    #[tokio::test]
    async fn build_mcp_service_constructs_without_panic() {
        // If this test passes, the service built OK.
        let _app = make_app("dev-all").await;
    }

    // ── test 4: strip_top_level_nulls parity ──────────────────────────────────

    #[test]
    fn strip_nulls_top_level_only() {
        use super::handler::strip_top_level_nulls;
        let input = json!({"a": null, "b": 1, "c": {"d": null}});
        let out = strip_top_level_nulls(input);
        assert!(out.get("a").is_none(), "'a' must be stripped");
        assert_eq!(out["b"], 1);
        assert_eq!(out["c"], json!({"d": null}), "nested null must not be removed");
    }

    // ── test 5: MemoryMcp constructs with the correct tool_router ─────────────

    #[tokio::test]
    async fn memory_mcp_constructs_ok() {
        let (service, _db) = make_service().await;
        let handler = super::handler::MemoryMcp::new(service);
        // The #[tool_router] generates a ToolRouter; construction is the test.
        let _ = handler;
    }

    // ── test 6: 15-tool count + names + args-envelope ────────────────────────
    //
    // The full count/name/envelope assertions live in
    // `mcp::handler::tests::tool_router_has_exactly_15_tools_with_args_envelope`
    // where `Self::tool_router()` is accessible (it's private to the impl block).
    //
    // Here we verify the service constructs without panic (already covered by
    // test 3) and leave the count/envelope to handler tests + schemas tests.
    #[test]
    fn service_constructs_and_build_mcp_service_compiles() {
        // This test just asserts the API surface compiles.
        // The actual 15-tool count test is in handler::tests.
        let _ = Value::Null; // prevent dead_code on the Value import
    }

    // ── test 7: allowed_hosts — LAN IP accepted, random host rejected ─────────
    //
    // Validates FIX 2 (CRITICAL-2): the dns-rebind guard must ACCEPT the brain's
    // `Host: 192.168.1.3:8090` and REJECT an arbitrary unknown host.
    //
    // rmcp's allowed-hosts middleware fires before auth (it returns 403 without
    // reading the body).  We can exercise it via axum `oneshot` with a custom
    // Host header.  The bearer token is still required for 401 checks to pass,
    // so here we confirm:
    //   - LAN IP host + valid bearer  → NOT 403 (auth layer responds instead, 200 or 401 etc.)
    //   - Unknown host + valid bearer → 403
    //
    // Note: when allowed_hosts is EMPTY the middleware is disabled (harness + tests use
    // this path), so we build a separate service with the LAN IP explicitly allowed.

    async fn make_app_with_allowed_hosts(token: &str, allowed_hosts: Vec<String>) -> Router {
        let (service, _db) = make_service().await;
        let token_store = make_token_store_with(token, "test.user");
        let svc = super::build_mcp_service(
            Arc::clone(&service),
            allowed_hosts,
            Arc::clone(&token_store),
        );
        Router::new().nest_service("/mcp/mcp", svc)
    }

    #[tokio::test]
    async fn lan_ip_host_not_rejected_when_in_allowed_list() {
        // Build with the LAN IP in the allowed list (mirrors Quadlet config).
        let allowed = vec![
            "127.0.0.1".to_string(),
            "localhost".to_string(),
            "192.168.1.3:8090".to_string(),
            "192.168.1.3".to_string(),
        ];
        let app = make_app_with_allowed_hosts("dev-all", allowed).await;

        let body = json!({
            "jsonrpc": "2.0",
            "method": "initialize",
            "params": {},
            "id": 1,
        });
        let req = http::Request::builder()
            .method("POST")
            .uri("/mcp/mcp")
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("Authorization", "Bearer dev-all")
            .header("Host", "192.168.1.3:8090")  // LAN IP — must NOT be 403
            .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_ne!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "LAN IP host must not be rejected (was 403); brain would be blocked at deploy"
        );
    }

    #[tokio::test]
    async fn random_host_rejected_when_allowed_list_non_empty() {
        // Build with the LAN IP in the allowed list.
        let allowed = vec![
            "127.0.0.1".to_string(),
            "localhost".to_string(),
            "192.168.1.3:8090".to_string(),
            "192.168.1.3".to_string(),
        ];
        let app = make_app_with_allowed_hosts("dev-all", allowed).await;

        let body = json!({
            "jsonrpc": "2.0",
            "method": "initialize",
            "params": {},
            "id": 1,
        });
        let req = http::Request::builder()
            .method("POST")
            .uri("/mcp/mcp")
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("Authorization", "Bearer dev-all")
            .header("Host", "evil.attacker.example.com")  // NOT in allowed list
            .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "Unknown host must be rejected with 403 by dns-rebind protection"
        );
    }

}
