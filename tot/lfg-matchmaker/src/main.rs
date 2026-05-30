mod api;
mod config;
mod harness;
mod matcher;
mod queue;
mod types;

use axum::{routing::get, Router};

#[tokio::main]
async fn main() {
    let app = Router::new().route("/healthz", get(|| async { "ok" }));
    let addr = std::env::var("LFG_LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:8095".into());
    let listener = tokio::net::TcpListener::bind(&addr).await.expect("bind");
    eprintln!("lfg-matchmaker listening on {addr}");
    axum::serve(listener, app).await.expect("serve");
}
