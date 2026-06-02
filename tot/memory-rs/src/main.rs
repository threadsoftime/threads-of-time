//! Standalone binary entry point for `memory-rs`.
//!
//! Reads [`Settings`] from environment variables, registers the sqlite-vec
//! auto-extension, builds the core [`MemoryService`], runs DB migrations,
//! wires [`AppState`], builds the axum [`Router`], binds the TCP listener,
//! and serves with graceful shutdown on SIGTERM / Ctrl-C.
//!
//! All logic lives in `lib.rs` and its submodules. This file is intentionally
//! thin: config → services → state → router → serve.

use std::sync::Arc;

use memory_rs::{
    app::build_router,
    auth::TokenStore,
    config::Settings,
    core::MemoryService,
    db::{self, migrate, register_vec0},
    embed_cache::EmbedCache,
    embeddings::EmbeddingsClient,
    pubsub::PubSub,
    state::AppState,
};
use tokio::net::TcpListener;

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("install Ctrl-C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c    => {},
        _ = terminate => {},
    }
}

#[tokio::main]
async fn main() {
    // Initialise structured logging.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("memory_rs=info".parse().expect("directive")),
        )
        .init();

    // Read configuration; exit(2) on any missing required var.
    let settings = Settings::from_env().unwrap_or_else(|e| {
        eprintln!("[memory-rs] config error: {e}");
        std::process::exit(2);
    });

    // Register vec0 ONCE before any connection is opened.
    // sqlite3_auto_extension is process-global and idempotent.
    register_vec0();

    // Run migrations synchronously at startup (blocking is fine: we haven't
    // started serving yet).
    {
        let conn = db::open_db(&settings.db_path).unwrap_or_else(|e| {
            eprintln!("[memory-rs] failed to open DB at {:?}: {e}", settings.db_path);
            std::process::exit(2);
        });
        migrate::run(&conn, &settings.migrations_dir).unwrap_or_else(|e| {
            eprintln!("[memory-rs] migration error: {e}");
            std::process::exit(2);
        });
        eprintln!("[memory-rs] migrations applied (or already up to date)");
    }

    // Build shared services.
    let client = EmbeddingsClient::new(&settings.embed_endpoint, "embedding", "");
    let embed = Arc::new(EmbedCache::new(client));
    let pubsub = Arc::new(PubSub::new());

    let mut service = MemoryService::new(
        settings.db_path.clone(),
        settings.weights.clone(),
        settings.cap_per_bot,
        Arc::clone(&embed),
        Arc::clone(&pubsub),
    );
    // Wire mmr_lambda and recency_basis from Settings.
    service.mmr_lambda = settings.mmr_lambda;
    service.recency_basis = settings.recency_basis.clone();
    let service = Arc::new(service);

    // Load token store (None if file missing — MCP transport disabled).
    let token_store: Option<Arc<TokenStore>> = settings
        .token_store
        .as_deref()
        .and_then(|p| TokenStore::load(p))
        .map(Arc::new);

    if token_store.is_some() {
        eprintln!("[memory-rs] token store loaded (MCP auth enabled)");
    } else {
        eprintln!("[memory-rs] no token store; MCP auth disabled");
    }

    let bind_addr = format!("{}:{}", settings.bind_host, settings.bind_port);
    let state = AppState { service, token_store, pubsub };
    let app = build_router(state);

    let listener = TcpListener::bind(&bind_addr).await.unwrap_or_else(|e| {
        eprintln!("[memory-rs] failed to bind {bind_addr}: {e}");
        std::process::exit(2);
    });

    eprintln!("[memory-rs] listening on {bind_addr}");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap_or_else(|e| eprintln!("[memory-rs] server error: {e}"));

    eprintln!("[memory-rs] shut down cleanly");
}
