// brain-rs entry point — CLI + tokio runtime.
//
// Faithful port of brain_sidecar/__main__.py:
//   uvicorn.run("brain_sidecar.app:create_app", factory=True,
//               host=s.bind_host, port=s.bind_port, log_level="info")
//
// This binary:
//   1. Parses optional --host / --port CLI overrides (or BRAIN_BIND_{HOST,PORT} env).
//   2. Calls setup_tracing.
//   3. Reads Settings from env.
//   4. Calls create_app(settings) → Router.
//   5. Binds tokio::net::TcpListener and serves.

use clap::Parser;
use brain_rs::app::create_app;
use brain_rs::config::Settings;
use brain_rs::logging::setup_tracing;

/// brain-rs — Rust port of brain-sidecar.
#[derive(Parser)]
#[command(name = "brain-rs", version, about = "Rust port of the brain-sidecar decision loop")]
struct Args {
    /// Override bind address (env: BRAIN_BIND_HOST)
    #[arg(long, env = "BRAIN_BIND_HOST")]
    host: Option<String>,
    /// Override bind port (env: BRAIN_BIND_PORT)
    #[arg(long, env = "BRAIN_BIND_PORT")]
    port: Option<u16>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    setup_tracing("INFO");

    let mut settings = Settings::from_env_with_defaults_checked()
        .map_err(|e| anyhow::anyhow!(e))?;

    // CLI overrides take precedence over env vars.
    if let Some(host) = args.host {
        settings.bind_host = host;
    }
    if let Some(port) = args.port {
        settings.bind_port = port;
    }

    let addr = format!("{}:{}", settings.bind_host, settings.bind_port);
    tracing::info!("brain-rs listening on {}", addr);

    let app = create_app(settings).await?;
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
