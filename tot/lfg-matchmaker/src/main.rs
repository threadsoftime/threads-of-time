//! Standalone binary entry point for `lfg-matchmaker`.
//!
//! Thin wrapper around the library crate.  All logic lives in `lib.rs` and its
//! submodules — the binary just reads config, wires the pieces, and serves.
//!
//! When running under `slice-host` this file is NOT used; `slice-host` composes
//! the library directly via `lfg_matchmaker::tick::run` + `lfg_matchmaker::routes`.

use lfg_matchmaker::api::{AppState, PendingPlacements};
use lfg_matchmaker::config::Config;
use lfg_matchmaker::harness::Harness;
use lfg_matchmaker::queue::Queue;
use std::sync::Arc;

/// Completes on Ctrl-C (any OS) OR SIGTERM (unix). Used both for axum's graceful
/// shutdown and to break the supervising `select!`.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.expect("install Ctrl-C handler");
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
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

#[tokio::main]
async fn main() {
    // Init the tracing subscriber for the standalone binary only — the library
    // must NOT init global state (slice-host is responsible for subscriber init
    // when composing this library; see slice-host/src/main.rs).
    //
    // NOTE: slice-host currently has NO tracing subscriber init (it uses eprintln!
    // throughout). When slice-host is migrated to tracing, it should add the
    // subscriber init there and this note should be removed.
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cfg = Config::from_env().unwrap_or_else(|e| {
        tracing::error!(error = %e, "config error");
        std::process::exit(2);
    });

    let harness = Harness::new(cfg.harness_base_url.clone(), cfg.harness_bearer.clone());
    let state = Arc::new(AppState { queue: Queue::new(), pending_placements: PendingPlacements::new() });

    // Keep the tick JoinHandle so a tick exit/panic ends the process (the container
    // restarts it) instead of zombifying the matcher while /healthz still returns ok.
    let tick_handle = {
        let state = state.clone();
        let harness = harness.clone();
        let cfg = cfg.clone();
        tokio::spawn(async move { lfg_matchmaker::tick::run(state, harness, cfg).await })
    };

    let app = lfg_matchmaker::api::router(state);
    let listener = tokio::net::TcpListener::bind(&cfg.listen_addr).await.expect("bind");
    tracing::info!(listen = %cfg.listen_addr, tick_secs = cfg.tick_secs, "lfg-matchmaker listening");

    let server = axum::serve(listener, app).with_graceful_shutdown(shutdown_signal());

    // Supervise: whichever arm fires first ends the process. The tick loop runs
    // forever, so its handle only resolves on panic/exit — either way we must not
    // keep serving traffic with a dead matcher.
    tokio::select! {
        res = server => {
            if let Err(e) = res {
                tracing::error!(error = %e, "server error");
            } else {
                tracing::info!("server shut down gracefully");
            }
        }
        res = tick_handle => {
            match res {
                Ok(()) => tracing::error!("tick loop exited unexpectedly — terminating"),
                Err(e) => tracing::error!(error = %e, "tick loop panicked — terminating"),
            }
            std::process::exit(1);
        }
    }
}
