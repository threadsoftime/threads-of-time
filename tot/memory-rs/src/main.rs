//! Standalone binary entry point for `memory-rs`.
//!
//! Reads [`Settings`] from environment variables, registers the sqlite-vec
//! auto-extension, builds the axum [`Router`], binds the TCP listener, and
//! serves with graceful shutdown on SIGTERM / Ctrl-C.
//!
//! All logic lives in `lib.rs` and its submodules. This file is intentionally
//! thin: config → state → router → serve.
//!
//! NOTE (Task 0.1): domain routes are archived; boot tests re-land in Task 0.4
//! once `/health` is restored.

use memory_rs::{
    app::build_router,
    config::Settings,
    db::register_vec0,
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

    let bind_addr = format!("{}:{}", settings.bind_host, settings.bind_port);
    let state = AppState::from_settings(&settings);
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
