//! Standalone binary entry point for the game-event-scheduler.
//!
//! This thin wrapper is kept for Inc-1/Inc-2 deploy compatibility (the path
//! `tot/game-event-scheduler` is referenced in existing deploy docs).  Production
//! hosting will use `slice-host` (T32 onward).

use std::sync::Arc;
use tokio::net::TcpListener;

use game_event_scheduler::api;
use game_event_scheduler::config::Config;
use game_event_scheduler::harness::Harness;
use game_event_scheduler::tick;

#[tokio::main]
async fn main() {
    let cfg = match Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[ges] configuration error: {e}");
            std::process::exit(2);
        }
    };

    eprintln!(
        "[ges] starting — listen={} harness={} tick={}s drive={}",
        cfg.listen_addr, cfg.harness_base_url, cfg.tick_secs, cfg.drive
    );

    let harness = Harness::new(cfg.harness_base_url.clone(), cfg.harness_bearer.clone());
    let state = Arc::new(api::AppState::new());

    // Build the HTTP router (includes /healthz, /report, /report/history).
    let app = api::router(state.clone());

    // Bind the listener.
    let listener = match TcpListener::bind(&cfg.listen_addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[ges] failed to bind {}: {e}", cfg.listen_addr);
            std::process::exit(2);
        }
    };

    eprintln!("[ges] listening on {}", cfg.listen_addr);

    // Graceful shutdown signal: first Ctrl-C or SIGTERM.
    let shutdown_signal = async {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            let mut sigterm =
                signal(SignalKind::terminate()).expect("failed to register SIGTERM handler");
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    eprintln!("[ges] received SIGINT — shutting down");
                }
                _ = sigterm.recv() => {
                    eprintln!("[ges] received SIGTERM — shutting down");
                }
            }
        }
        #[cfg(not(unix))]
        {
            tokio::signal::ctrl_c().await.expect("failed to listen for Ctrl-C");
            eprintln!("[ges] received Ctrl-C — shutting down");
        }
    };

    // Spawn the tick loop; keep the JoinHandle so we can monitor it.
    let tick_handle = tokio::spawn(tick::run(state.clone(), harness, cfg.clone()));

    // Axum server future with graceful shutdown wired to the OS signal.
    let server = axum::serve(listener, app).with_graceful_shutdown(shutdown_signal);

    // Supervise: if the tick handle resolves (panic or unexpected return),
    // log and exit so the process supervisor can restart us.
    // If the server returns (graceful shutdown completed), log and exit cleanly.
    tokio::select! {
        res = server => {
            match res {
                Ok(()) => eprintln!("[ges] server shut down cleanly"),
                Err(e) => eprintln!("[ges] server error: {e}"),
            }
        }
        res = tick_handle => {
            match res {
                Ok(()) => eprintln!("[ges] tick loop returned unexpectedly — exiting"),
                Err(e) => eprintln!("[ges] tick loop panicked or was aborted: {e}"),
            }
            std::process::exit(1);
        }
    }
}
