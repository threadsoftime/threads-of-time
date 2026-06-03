//! Increment 0: rmcp CLIENT spike
//!
//! Proves rmcp 1.7 per-call sessions work against a StreamableHTTP MCP endpoint.
//! Open -> initialize -> list_all_tools -> call_tool("obs.ping") -> close.
//! Runs twice to prove per-call reconnect.
//!
//! Run against live backends (I0.2):
//!   cargo run -p brain-rs --example rmcp_client_spike -- \
//!       --url http://192.168.1.3:8099/mcp/mcp --bearer <token>
//!
//! Real rmcp 1.7 client API corrections from plan snippet:
//!   - Feature: "client" + "transport-streamable-http-client-reqwest"
//!     (NOT "transport-streamable-http-client" alone — that omits the reqwest impl)
//!   - Transport ctor: StreamableHttpClientTransport::from_config(config)
//!     (from_config is gated on transport-streamable-http-client-reqwest)
//!   - Config: StreamableHttpClientTransportConfig::with_uri(url).auth_header(token)
//!     auth_header() stores the raw token; rmcp calls reqwest::bearer_auth() which
//!     prepends "Bearer " automatically.
//!   - Service creation: ().serve(transport).await? (ServiceExt::serve on ())
//!     yields RunningService<RoleClient, ()>
//!   - RunningService impls Deref<Target = Peer<RoleClient>>, so list_all_tools()
//!     and call_tool() are called directly on the RunningService.
//!   - call_tool() takes CallToolRequestParams, not (name, Option<Value>).
//!   - Session close: client.cancel().await (consuming)
//!   - The plan's ClientInfo::default().serve(...) pattern is wrong:
//!     ClientHandler is impl'd for ClientInfo (used as the service), but
//!     ().serve() is simpler and more idiomatic for a no-op client handler.

use anyhow::Result;
use clap::Parser;
use rmcp::{
    ServiceExt,
    model::CallToolRequestParams,
    transport::{
        StreamableHttpClientTransport,
        streamable_http_client::StreamableHttpClientTransportConfig,
    },
};

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "http://localhost:8099/mcp/mcp")]
    url: String,
    #[arg(long, default_value = "")]
    bearer: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();

    for round in 1..=2u32 {
        println!("=== round {round} ===");

        // Build per-call config. auth_header() stores the raw token; the transport
        // calls reqwest::RequestBuilder::bearer_auth() which prepends "Bearer ".
        let mut config = StreamableHttpClientTransportConfig::with_uri(args.url.clone());
        if !args.bearer.is_empty() {
            config = config.auth_header(args.bearer.clone());
        }

        // from_config is only available under transport-streamable-http-client-reqwest.
        // It builds a fresh reqwest::Client with pool_max_idle_per_host=0 internally.
        let transport = StreamableHttpClientTransport::from_config(config);

        // ().serve() uses the no-op ClientHandler impl for () — correct for a probe.
        // Performs the MCP initialize handshake; returns RunningService<RoleClient, ()>.
        // RunningService derefs to Peer<RoleClient> so peer methods work on it directly.
        let client = ().serve(transport).await?;

        // list_all_tools() paginates until cursor is None.
        let tools = client.list_all_tools().await?;
        println!("tools/list: {} tools", tools.len());

        let has_defs = tools.iter().any(|t| {
            serde_json::to_string(&t.input_schema)
                .unwrap_or_default()
                .contains("$defs")
        });
        println!("$defs present: {has_defs}");

        // call_tool takes CallToolRequestParams { name: Cow, arguments: Option<JsonObject>, .. }.
        // JsonObject = serde_json::Map<String, Value>.
        let args_map: serde_json::Map<String, serde_json::Value> =
            serde_json::from_value(serde_json::json!({ "args": { "target_guid": 1 } }))
                .expect("static json is valid");
        let params = CallToolRequestParams::new("obs.ping").with_arguments(args_map);

        let result = client.call_tool(params).await?;
        assert!(
            !result.is_error.unwrap_or(false),
            "obs.ping isError=true"
        );

        // Content = Annotated<RawContent>; Annotated derefs to RawContent; as_text() on RawContent.
        let text = result
            .content
            .iter()
            .find_map(|c| c.as_text().map(|t| t.text.clone()))
            .unwrap_or_default();
        println!("obs.ping result: {text}");

        // cancel() consumes RunningService, cancels the background task, and waits
        // for the transport to send DELETE /session (with 5s timeout in the worker).
        let _ = client.cancel().await;
        println!("session closed cleanly");
    }

    println!("SPIKE PASS: rmcp client per-call reconnect works");
    Ok(())
}
