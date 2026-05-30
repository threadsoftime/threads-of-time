mod api;
mod config;
mod harness;
mod matcher;
mod orchestrator;
mod queue;
mod tick;
mod types;

use crate::api::AppState;
use crate::config::Config;
use crate::harness::Harness;
use crate::queue::Queue;
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
    let cfg = Config::from_env().unwrap_or_else(|e| {
        eprintln!("config error: {e}");
        std::process::exit(2);
    });

    let harness = Harness::new(cfg.harness_base_url.clone(), cfg.harness_bearer.clone());
    let state = Arc::new(AppState { queue: Queue::new() });

    // Keep the tick JoinHandle so a tick exit/panic ends the process (the container
    // restarts it) instead of zombifying the matcher while /healthz still returns ok.
    let tick_handle = {
        let state = state.clone();
        let harness = harness.clone();
        let cfg = cfg.clone();
        tokio::spawn(async move { tick::run(state, harness, cfg).await })
    };

    let app = api::router(state);
    let listener = tokio::net::TcpListener::bind(&cfg.listen_addr).await.expect("bind");
    eprintln!("lfg-matchmaker listening on {} (tick {}s)", cfg.listen_addr, cfg.tick_secs);

    let server = axum::serve(listener, app).with_graceful_shutdown(shutdown_signal());

    // Supervise: whichever arm fires first ends the process. The tick loop runs
    // forever, so its handle only resolves on panic/exit — either way we must not
    // keep serving traffic with a dead matcher.
    tokio::select! {
        res = server => {
            if let Err(e) = res {
                eprintln!("[supervise] server error: {e}");
            } else {
                eprintln!("[supervise] server shut down gracefully");
            }
        }
        res = tick_handle => {
            match res {
                Ok(()) => eprintln!("[supervise] tick loop exited unexpectedly — terminating"),
                Err(e) => eprintln!("[supervise] tick loop panicked: {e} — terminating"),
            }
            std::process::exit(1);
        }
    }
}
