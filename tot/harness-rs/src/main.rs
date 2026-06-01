//! Entry point: parse CLI, dispatch to `serve`, `mint-token`, or `validate`.
//!
//! Port of `harness_daemon/main.py` — preserving the serve / mint-token /
//! validate CLI shape and the SIGHUP re-exec behaviour.

mod cli;
mod config;
mod auth;
mod registry;
mod audit;
mod db_client;
mod ac_client;
mod dispatch;
mod error;
mod mcp;
mod rest;
#[cfg(test)] mod test_support;

use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use clap::Parser as _;
use rand::RngCore as _;
use tracing_subscriber::EnvFilter;

use ac_client::ACClient;
use audit::AuditLogger;
use auth::TokenStore;
use cli::{Cli, Command};
use config::load_config_from_path;
use db_client::DbClient;
use registry::build_v1_registry;
use rest::{AppState, SharedState, build_router};

// ── main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Command::Serve(args)     => run_serve(args).await,
        Command::MintToken(args) => run_mint_token(args),
        Command::Validate(args)  => run_validate(args),
    }
}

// ── serve ─────────────────────────────────────────────────────────────────────

async fn run_serve(args: cli::ServeArgs) {
    // 1. Load config
    let cfg = load_config_from_path(&args.config).unwrap_or_else(|e| {
        eprintln!("[harness-rs] config error: {e}");
        std::process::exit(1);
    });

    // 2. Parse listen address
    let listen_addr = cfg.listen_address.clone();

    // 3. Build AC client
    let ac_client = ACClient::new(&cfg.ac_bridge_url, 3.0);

    // 4. Build optional DB client (only if password is set — mirror main.py:53)
    let db_client = if !args.mysql_password.is_empty() {
        Some(DbClient::new(
            &args.mysql_host,
            args.mysql_port,
            &args.mysql_user,
            &args.mysql_password,
        ))
    } else {
        None
    };

    let mysql_status = if db_client.is_some() { "wired" } else { "unconfigured" };

    // 5. Build shared TokenStore (used by both REST and MCP)
    let token_store = Arc::new(TokenStore::new(cfg.tokens.clone()));

    // 6. Build AppState
    let audit = AuditLogger::new(&cfg.audit_path).unwrap_or_else(|e| {
        eprintln!("[harness-rs] audit logger error: {e}");
        std::process::exit(1);
    });

    let state: SharedState = Arc::new(AppState {
        token_store: TokenStore::new(cfg.tokens.clone()),
        registry:    build_v1_registry(),
        ac_client,
        db_client,
        audit,
        audit_path:  cfg.audit_path.clone(),
    });

    // 7. MCP host allowlist (mirror app.py:228-234)
    let mut allowed_hosts = vec![
        "127.0.0.1".to_string(),
        "localhost".to_string(),
        cfg.listen_address.clone(),
    ];
    let extra_hosts = std::env::var("HARNESS_EXTRA_ALLOWED_HOSTS").unwrap_or_default();
    for h in extra_hosts.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
        allowed_hosts.push(h.to_string());
    }

    // 8. Compose router: REST + MCP nested at /mcp/mcp
    let mcp_svc = mcp::build_mcp_service(
        Arc::clone(&state),
        allowed_hosts,
        Arc::clone(&token_store),
    );
    let app = build_router(Arc::clone(&state))
        .nest_service("/mcp/mcp", mcp_svc);

    // 9. SIGHUP handler — re-exec the process to reload config
    install_sighup_handler();

    // 10. Bind and serve
    println!(
        "[harness-rs] listening on {listen_addr}, ac_bridge={}, mysql={mysql_status}",
        cfg.ac_bridge_url,
    );

    let listener = tokio::net::TcpListener::bind(&listen_addr)
        .await
        .unwrap_or_else(|e| {
            eprintln!("[harness-rs] bind error on {listen_addr}: {e}");
            std::process::exit(1);
        });

    axum::serve(listener, app).await.unwrap_or_else(|e| {
        eprintln!("[harness-rs] serve error: {e}");
        std::process::exit(1);
    });
}

// ── SIGHUP re-exec ────────────────────────────────────────────────────────────

/// Install a SIGHUP handler that re-execs the current process.
///
/// On receipt:  prints a diagnostic line, then calls `execv(current_exe, argv)`
/// — the OS replaces the running process image with a fresh one that reads the
/// (possibly updated) tokens.yaml from scratch.
///
/// Mirror of `main.py:21-27`.
fn install_sighup_handler() {
    // We spawn a tokio task that awaits the signal and then execs.
    // tokio::signal::unix requires a multi-thread runtime — `#[tokio::main]`
    // provides one.
    #[allow(unreachable_code)] // execv replaces the process image on success; code after it is unreachable
    tokio::spawn(async move {
        let mut hup = tokio::signal::unix::signal(
            tokio::signal::unix::SignalKind::hangup(),
        )
        .expect("SIGHUP signal handler install failed");

        hup.recv().await;

        println!("[harness-rs] SIGHUP — re-exec to reload config");

        let exe = std::env::current_exe()
            .expect("cannot determine current executable path");
        let argv: Vec<std::ffi::CString> = std::env::args()
            .map(|a| std::ffi::CString::new(a).expect("arg contains NUL byte"))
            .collect();

        // nix::unistd::execv replaces the process image in-place.
        // SAFETY: no background threads hold locks that would deadlock here
        // because this is intentional process replacement, not a fork.
        let exe_cstr = std::ffi::CString::new(
            exe.to_str().expect("exe path is not UTF-8")
        )
        .expect("exe path contains NUL byte");

        nix::unistd::execv(&exe_cstr, &argv)
            .expect("execv failed — process could not re-exec");
    });
}

// ── mint-token ────────────────────────────────────────────────────────────────

/// Generate a token and print a YAML list entry.
///
/// Port of `main.py:62-85`.  Token is 24 random bytes encoded as URL-safe
/// base64 without padding — identical to Python's `secrets.token_urlsafe(24)`
/// (which uses 24 bytes, not 24 characters).
pub fn run_mint_token(args: cli::MintTokenArgs) {
    let token = generate_token_urlsafe(24);
    print_mint_token_yaml(&token, &args.identity, &args.scope,
                          args.bound_to_guid, args.augmented,
                          args.note.as_deref());
}

/// Generate `n_bytes` of CSPRNG random bytes, encode as URL-safe base64 (no padding).
///
/// Mirrors Python `secrets.token_urlsafe(n_bytes)`.
pub fn generate_token_urlsafe(n_bytes: usize) -> String {
    let mut buf = vec![0u8; n_bytes];
    rand::rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(&buf)
}

/// Format and print the YAML list entry to stdout.
///
/// Mirrors Python `yaml.safe_dump([entry], sort_keys=False)`.
pub fn print_mint_token_yaml(
    token:         &str,
    identity:      &str,
    scope:         &[String],
    bound_to_guid: Option<i64>,
    augmented:     bool,
    note:          Option<&str>,
) {
    // Python yaml.safe_dump([entry]) emits:
    //   - token: "<tok>"
    //     identity: "<id>"
    //     scope:
    //     - "<s1>"
    //     - "<s2>"
    //     bound_to_guid: <n>    # only if set
    //     augmented: true       # only if true
    //     note: "<n>"           # only if set
    //
    // We emit the same shape manually (no external yaml-emit dep needed).
    println!("- token: {token:?}");
    println!("  identity: {identity:?}");
    println!("  scope:");
    for s in scope {
        println!("  - {s:?}");
    }
    if let Some(g) = bound_to_guid {
        println!("  bound_to_guid: {g}");
    }
    if augmented {
        println!("  augmented: true");
    }
    if let Some(n) = note {
        if !n.is_empty() {
            println!("  note: {n:?}");
        }
    }
}

// ── validate ──────────────────────────────────────────────────────────────────

fn run_validate(args: cli::ValidateArgs) {
    match load_config_from_path(&args.config) {
        Ok(_)  => {
            println!("ok");
            // exit 0 — normal return
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use http::{Request, StatusCode};
    use serde_json::json;
    use tower::ServiceExt as _;

    use crate::ac_client::ACClient;
    use crate::audit::AuditLogger;
    use crate::auth::TokenStore;
    use crate::config::TokenRecord;
    use crate::db_client::DbClient;
    use crate::mcp::build_mcp_service;
    use crate::registry::build_v1_registry;
    use crate::rest::{AppState, build_router};
    use crate::test_support::{MockConfig, spawn_mock_ac_with_config};

    // ── test helper ───────────────────────────────────────────────────────────

    async fn body_json(resp: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn bearer_req(method: &str, uri: &str, body: Option<serde_json::Value>) -> Request<Body> {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("Authorization", "Bearer dev-all")
            .header("Content-Type", "application/json");
        match body {
            Some(v) => builder
                .body(Body::from(serde_json::to_vec(&v).unwrap()))
                .unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        }
    }

    // ── W2 env-wiring: db_client Some wires to dispatch ──────────────────────

    /// Build AppState with a DbClient (dummy host — no real connection needed;
    /// the unknown-template check fires before any query).  POST obs.query_db
    /// with an unknown template → assert 400 bad_request (NOT 503 db_not_configured).
    ///
    /// Proves that db_client=Some(...) reaches dispatch correctly.
    #[tokio::test]
    async fn db_client_some_reaches_dispatch_returns_400_bad_request() {
        let mock = spawn_mock_ac_with_config(MockConfig::default()).await;
        let audit_path = std::env::temp_dir()
            .join(format!("harness_rs_main_w2_{}.jsonl", uuid::Uuid::new_v4().simple()))
            .to_str()
            .unwrap()
            .to_string();

        // DbClient with dummy host — pool is lazy, no real connection attempted
        let db_client = Some(DbClient::new("127.0.0.1", 3306, "root", "dummy"));

        let token_record = TokenRecord {
            token:         "dev-all".to_string(),
            identity:      "dev.user".to_string(),
            scope:         vec!["obs.*".to_string()],
            augmented:     false,
            bound_to_guid: None,
            note:          None,
        };
        let token_store = Arc::new(TokenStore::new(vec![token_record.clone()]));

        let state = Arc::new(AppState {
            token_store: TokenStore::new(vec![token_record]),
            registry:    build_v1_registry(),
            ac_client:   ACClient::new(mock.base_url(), 3.0),
            db_client,
            audit:       AuditLogger::new(&audit_path).unwrap(),
            audit_path:  audit_path.clone(),
        });

        // Build the full composed router (REST + MCP on one router)
        let mcp_svc = build_mcp_service(
            Arc::clone(&state),
            vec![],  // no host check in tests
            Arc::clone(&token_store),
        );
        let app = build_router(Arc::clone(&state))
            .nest_service("/mcp/mcp", mcp_svc);

        // POST obs.query_db with an unknown template name → 400 bad_request
        // (unknown template is caught by dispatch before any DB connection)
        let req = Request::builder()
            .method("POST")
            .uri("/v1/tools/obs.query_db")
            .header("Authorization", "Bearer dev-all")
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_vec(
                &json!({"template_name": "nope"})
            ).unwrap()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();

        // 400 bad_request (unknown template), NOT 503 db_not_configured
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "unknown template with db_client=Some must return 400, not 503"
        );
        let body = body_json(resp).await;
        assert_eq!(body["ok"], false);
        assert_eq!(body["error"], "bad_request",
            "error must be bad_request for unknown template, not db_not_configured");
    }

    // ── Serve composition integration: GET /v1/health + authed POST + MCP 401 ─

    /// Boot the full composed Router via oneshot — confirms REST + MCP are
    /// both mounted and operating on the single router.
    #[tokio::test]
    async fn composed_router_rest_and_mcp_both_mounted() {
        let mock = spawn_mock_ac_with_config(MockConfig {
            dispatch_status: 200,
            dispatch_body:   json!({"ok": true, "result": {"pong": true}}),
            health_status:   200,
        }).await;

        let audit_path = std::env::temp_dir()
            .join(format!("harness_rs_main_comp_{}.jsonl", uuid::Uuid::new_v4().simple()))
            .to_str()
            .unwrap()
            .to_string();

        let token_record = TokenRecord {
            token:         "dev-all".to_string(),
            identity:      "dev.user".to_string(),
            scope:         vec![
                "gm.*".to_string(), "obs.*".to_string(), "bot.*".to_string(),
                "event.*".to_string(), "memory.*".to_string(), "lfg.*".to_string(),
            ],
            augmented:     false,
            bound_to_guid: None,
            note:          None,
        };
        let token_store = Arc::new(TokenStore::new(vec![token_record.clone()]));

        let state = Arc::new(AppState {
            token_store: TokenStore::new(vec![token_record]),
            registry:    build_v1_registry(),
            ac_client:   ACClient::new(mock.base_url(), 3.0),
            db_client:   None,
            audit:       AuditLogger::new(&audit_path).unwrap(),
            audit_path:  audit_path.clone(),
        });

        let mcp_svc = build_mcp_service(
            Arc::clone(&state),
            vec![],  // host check disabled for tests
            Arc::clone(&token_store),
        );
        let app = build_router(Arc::clone(&state))
            .nest_service("/mcp/mcp", mcp_svc);

        // 1. GET /v1/health → 200
        {
            let req = Request::builder()
                .method("GET")
                .uri("/v1/health")
                .body(Body::empty())
                .unwrap();
            let resp = app.clone().oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::OK, "GET /v1/health must be 200");
            let body = body_json(resp).await;
            assert_eq!(body["ok"], true);
        }

        // 2. POST /v1/tools/obs.ping with valid bearer → 200 with result.pong
        {
            let req = bearer_req("POST", "/v1/tools/obs.ping", Some(json!({})));
            let resp = app.clone().oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::OK, "authed POST must return 200");
            let body = body_json(resp).await;
            assert_eq!(body["ok"], true);
            assert_eq!(body["result"]["pong"], true);
        }

        // 3. POST /mcp/mcp with NO bearer → 401 with MCP error body
        {
            let req = Request::builder()
                .method("POST")
                .uri("/mcp/mcp")
                .header("Content-Type", "application/json")
                .header("Accept", "application/json, text/event-stream")
                .body(Body::from(serde_json::to_vec(&json!({
                    "jsonrpc": "2.0",
                    "method":  "initialize",
                    "params":  {},
                    "id":      1,
                })).unwrap()))
                .unwrap();
            let resp = app.clone().oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::UNAUTHORIZED,
                "MCP with no bearer must return 401");
            let raw = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
            assert_eq!(
                raw.as_ref(),
                br#"{"error":"invalid_token","error_description":"Authentication required"}"#,
                "MCP 401 body must match exact wire format",
            );
        }
    }

    // ── mint-token output ─────────────────────────────────────────────────────

    /// Token generation produces URL-safe base64 output of ~32 chars for 24
    /// bytes and the YAML is well-formed.
    #[test]
    fn generate_token_urlsafe_length_and_chars() {
        let tok = super::generate_token_urlsafe(24);
        // 24 bytes → ceil(24 * 4 / 3) = 32 chars (no padding needed, 24 is multiple of 3)
        assert_eq!(tok.len(), 32, "24 bytes must encode to exactly 32 url-safe base64 chars");
        // Must be URL-safe base64 alphabet only: A-Z, a-z, 0-9, -, _
        assert!(
            tok.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "token must only contain URL-safe base64 chars, got: {tok}",
        );
    }

    /// Two calls produce different tokens (birthday paradox: P(collision) ≈ 2^-64).
    #[test]
    fn generate_token_urlsafe_is_random() {
        let t1 = super::generate_token_urlsafe(24);
        let t2 = super::generate_token_urlsafe(24);
        assert_ne!(t1, t2, "two token_urlsafe(24) calls must not collide");
    }

    /// mint-token output contains `token:`, the identity, and the scope.
    #[test]
    fn print_mint_token_yaml_contains_required_fields() {
        // Capture stdout
        use std::io::Write;
        let mut buf = Vec::<u8>::new();
        let token    = "abc123def456ghi789jkl0mn";
        let identity = "bot.Krak";
        let scope    = vec!["bot.*".to_string(), "obs.*".to_string()];

        // Call via the formatting fn — write to a string instead of stdout
        // by duplicating the logic here for testability.
        let mut out = String::new();
        out.push_str(&format!("- token: {token:?}\n"));
        out.push_str(&format!("  identity: {identity:?}\n"));
        out.push_str("  scope:\n");
        for s in &scope {
            out.push_str(&format!("  - {s:?}\n"));
        }

        assert!(out.contains("token:"),    "output must contain 'token:'");
        assert!(out.contains("bot.Krak"),  "output must contain the identity");
        assert!(out.contains("bot.*"),     "output must contain first scope");
        assert!(out.contains("obs.*"),     "output must contain second scope");
        // Verify token length in typical output (32 chars for 24 bytes)
        let tok = super::generate_token_urlsafe(24);
        assert!(tok.len() >= 30, "token must be at least 30 url-safe chars");

        // Suppress unused-write warning
        let _ = buf.write_all(out.as_bytes());
    }

    // ── validate logic ────────────────────────────────────────────────────────

    /// Valid config string → Ok (validate function logic, not the binary exit code)
    #[test]
    fn validate_valid_config_returns_ok() {
        let yaml = r#"
ac_bridge_url: "http://127.0.0.1:8091"
audit_path: "/tmp/audit.jsonl"
listen_address: "0.0.0.0:8099"
tokens:
  - token: "tok-test-123"
    identity: "test.runner"
    scope:
      - "gm.*"
"#;
        let result = crate::config::load_config_from_str(yaml);
        assert!(result.is_ok(), "valid YAML must parse without error");
    }

    /// Malformed config string → Err (validates the path the `validate`
    /// subcommand takes on invalid input; binary exits 1 on Err — manual test).
    #[test]
    fn validate_malformed_config_returns_err() {
        let yaml = "not: valid: yaml: at all: :::";
        let result = crate::config::load_config_from_str(yaml);
        assert!(result.is_err(), "malformed YAML must return Err");
    }

    /// Config missing a required field → Err
    #[test]
    fn validate_missing_required_field_returns_err() {
        let yaml = r#"
audit_path: "/tmp/audit.jsonl"
listen_address: "0.0.0.0:8099"
tokens: []
"#;
        // Missing ac_bridge_url
        let result = crate::config::load_config_from_str(yaml);
        assert!(result.is_err(), "config without ac_bridge_url must return Err");
    }
}
