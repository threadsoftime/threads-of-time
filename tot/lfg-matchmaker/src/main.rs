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

#[tokio::main]
async fn main() {
    let cfg = Config::from_env().unwrap_or_else(|e| {
        eprintln!("config error: {e}");
        std::process::exit(2);
    });

    let harness = Harness::new(cfg.harness_base_url.clone(), cfg.harness_bearer.clone());
    let state = Arc::new(AppState { queue: Queue::new() });

    {
        let state = state.clone();
        let harness = harness.clone();
        let cfg = cfg.clone();
        tokio::spawn(async move { tick::run(state, harness, cfg).await });
    }

    let app = api::router(state);
    let listener = tokio::net::TcpListener::bind(&cfg.listen_addr).await.expect("bind");
    eprintln!("lfg-matchmaker listening on {} (tick {}s)", cfg.listen_addr, cfg.tick_secs);
    axum::serve(listener, app).await.expect("serve");
}
