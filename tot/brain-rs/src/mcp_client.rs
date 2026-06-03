// SPDX-License-Identifier: AGPL-3.0
/// McpClient — per-call reconnecting rmcp 1.7 client.
///
/// Faithful Rust port of brain_sidecar/mcp_clients.py McpClient.
///
/// Design note: opens a FRESH rmcp session per call() and list_tools() to avoid
/// the mcp idle-timeout teardown that kills long-lived sessions. Each call:
///   open → initialize → call/list → cancel (sends DELETE /session)
///
/// Both harness-daemon and memory-sidecar use FastMCP with a handler signature
/// `_handler(ctx, args: SchemaModel)`. FastMCP registers the Python parameter
/// "args" as the sole top-level input property, so MCP call arguments must be
/// wrapped: {"args": <actual-params>}. This matches Python exactly.
///
/// Implements McpCallable (defined in personality.rs) so it can be injected
/// into dispatch/decide/personality without those callers knowing about rmcp.
///
/// NOTE ON DUAL REQWEST: This crate depends on reqwest 0.12 directly; rmcp
/// internally uses reqwest 0.13. This is a known deferred split. We never
/// name reqwest::Client in type positions here — all rmcp transport types are
/// handled by full inference so the 0.13 types remain opaque to this crate.
use anyhow::Context as _;
use rmcp::{
    ServiceExt,
    model::CallToolRequestParams,
    transport::{
        StreamableHttpClientTransport,
        streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde_json::Value;

use crate::personality::McpCallable;

// ---------------------------------------------------------------------------
// McpClient
// ---------------------------------------------------------------------------

/// Reconnecting MCP client — opens a fresh session per call().
///
/// Holds only the URL and bearer; does NOT maintain a long-lived session.
#[derive(Debug, Clone)]
pub struct McpClient {
    pub url: String,
    pub bearer: String,
}

impl McpClient {
    /// Construct a new McpClient. `bearer` is the raw token (without "Bearer " prefix).
    pub fn new(url: impl Into<String>, bearer: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            bearer: bearer.into(),
        }
    }

    /// Open a fresh session, call tool_name with args wrapped in the FastMCP
    /// envelope `{"args": <args>}`, close the session, return the parsed result.
    pub async fn call(&self, tool_name: &str, args: &Value) -> anyhow::Result<Value> {
        // Per-call session: open → initialize → call → cancel.
        // IMPORTANT: do NOT annotate the transport or client types — the rmcp
        // transport uses reqwest 0.13 (rmcp-internal), but this crate's direct
        // dep is 0.12. Type inference keeps them separate; naming them would
        // cause a version-mismatch compile error.
        let mut config = StreamableHttpClientTransportConfig::with_uri(self.url.clone());
        if !self.bearer.is_empty() {
            config = config.auth_header(self.bearer.clone());
        }
        let transport = StreamableHttpClientTransport::from_config(config);
        let client = ().serve(transport).await.context("McpClient initialize")?;

        // FastMCP envelope: wrap actual args under "args" key.
        let wrapped = wrap_args(args);
        let params = CallToolRequestParams::new(tool_name.to_string()).with_arguments(wrapped);
        let result = client
            .call_tool(params)
            .await
            .context("McpClient call_tool")?;

        let _ = client.cancel().await;

        extract_content(tool_name, &result)
    }

    /// Open a fresh session, list tools, close the session.
    /// Called only at startup by schema_builder; per-call overhead is immaterial.
    pub async fn list_tools(&self) -> anyhow::Result<Vec<rmcp::model::Tool>> {
        let mut config = StreamableHttpClientTransportConfig::with_uri(self.url.clone());
        if !self.bearer.is_empty() {
            config = config.auth_header(self.bearer.clone());
        }
        let transport = StreamableHttpClientTransport::from_config(config);
        let client = ().serve(transport).await.context("McpClient initialize")?;

        let tools = client
            .list_all_tools()
            .await
            .context("McpClient list_all_tools")?;

        let _ = client.cancel().await;
        Ok(tools)
    }
}

// ---------------------------------------------------------------------------
// McpCallable impl — connects McpClient to the trait used by dispatch/decide/personality
// ---------------------------------------------------------------------------

impl McpCallable for McpClient {
    fn call<'a>(
        &'a self,
        tool: &'a str,
        args: Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Value, anyhow::Error>> + Send + 'a>,
    > {
        Box::pin(async move { self.call(tool, &args).await })
    }
}

// ---------------------------------------------------------------------------
// Pure helpers — factored out for unit testing without network
// ---------------------------------------------------------------------------

/// Wrap args in the FastMCP envelope: `{"args": <args>}`.
/// This is what both harness-daemon and memory-sidecar expect.
pub(crate) fn wrap_args(args: &Value) -> serde_json::Map<String, Value> {
    let mut map = serde_json::Map::new();
    map.insert("args".to_string(), args.clone());
    map
}

/// Extract the first JSON text content block from a tool call result.
/// Returns Err if `is_error` is true, or if no text block is found.
pub(crate) fn extract_content(
    tool_name: &str,
    result: &rmcp::model::CallToolResult,
) -> anyhow::Result<Value> {
    if result.is_error.unwrap_or(false) {
        let text: String = result
            .content
            .iter()
            .filter_map(|c| c.as_text().map(|t| t.text.clone()))
            .collect::<Vec<_>>()
            .join("");
        return Err(anyhow::anyhow!("MCP tool {} error: {}", tool_name, text));
    }
    // Convention: harness/memory tools return a single JSON content block.
    for block in &result.content {
        if let Some(text_block) = block.as_text() {
            return serde_json::from_str(&text_block.text)
                .with_context(|| format!("McpClient: failed to parse JSON from tool {tool_name}"));
        }
    }
    Ok(Value::Object(serde_json::Map::new()))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        extract::Request,
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::{delete, post},
    };
    use rmcp::model::CallToolResult;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    // -----------------------------------------------------------------------
    // Pure-logic unit tests (no network)
    // -----------------------------------------------------------------------

    #[test]
    fn test_wrap_args_envelopes_object() {
        let args = serde_json::json!({"target_guid": 1});
        let wrapped = wrap_args(&args);
        assert_eq!(
            wrapped.get("args").unwrap(),
            &serde_json::json!({"target_guid": 1})
        );
        assert_eq!(wrapped.len(), 1, "only 'args' key");
    }

    #[test]
    fn test_wrap_args_envelopes_null() {
        let args = Value::Null;
        let wrapped = wrap_args(&args);
        assert_eq!(wrapped.get("args").unwrap(), &Value::Null);
    }

    #[test]
    fn test_extract_content_parses_json_text() {
        let result = make_call_result(r#"{"pong":true}"#, false);
        let val = extract_content("obs.ping", &result).unwrap();
        assert_eq!(val, serde_json::json!({"pong": true}));
    }

    #[test]
    fn test_extract_content_is_error_returns_err() {
        let result = make_call_result("tool exploded", true);
        let err = extract_content("obs.ping", &result).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("obs.ping"), "error includes tool name");
        assert!(msg.contains("tool exploded"), "error includes message");
    }

    #[test]
    fn test_extract_content_empty_content_returns_empty_object() {
        // Build via serde round-trip to avoid #[non_exhaustive] struct literal restriction
        let json = serde_json::json!({
            "content": [],
            "isError": false
        });
        let result: CallToolResult = serde_json::from_value(json).unwrap();
        let val = extract_content("obs.ping", &result).unwrap();
        assert_eq!(val, serde_json::json!({}));
    }

    #[test]
    fn test_extract_content_bad_json_returns_err() {
        let result = make_call_result("not-json!!", false);
        let err = extract_content("obs.ping", &result).unwrap_err();
        assert!(err.to_string().contains("obs.ping"));
    }

    // -----------------------------------------------------------------------
    // Mock MCP server helpers
    //
    // Implements just enough of the MCP Streamable HTTP protocol to let
    // rmcp's StreamableHttpClientTransport complete its handshake and
    // exchange a single tools/call or tools/list request.
    //
    // Protocol skeleton (per MCP spec + rmcp source):
    //   1. POST /mcp  {method:"initialize", ...}  → 200 JSON init result
    //      + Mcp-Session-Id response header
    //   2. POST /mcp  {method:"notifications/initialized"} → 202 Accepted
    //   3. POST /mcp  {method:"tools/list" | "tools/call", ...} → 200 JSON result
    //   4. DELETE /mcp  → 200 (session cleanup)
    // -----------------------------------------------------------------------

    /// Spawn a minimal MCP-shaped axum server. Returns the base URL `/mcp` URL.
    async fn spawn_mock_mcp(
        tool_call_response: serde_json::Value,
        is_error: bool,
        init_counter: Option<Arc<AtomicUsize>>,
        captured_headers: Option<Arc<tokio::sync::Mutex<Option<HeaderMap>>>>,
    ) -> String {
        let counter = init_counter.unwrap_or_default();

        let app = Router::new()
            .route(
                "/mcp",
                post({
                    let counter = counter.clone();
                    let captured = captured_headers;
                    let tool_call_response = tool_call_response.clone();
                    move |headers: HeaderMap, req: Request| {
                        let counter = counter.clone();
                        let captured = captured.clone();
                        let tool_call_response = tool_call_response.clone();
                        async move {
                            if let Some(ref cap) = captured {
                                let mut guard = cap.lock().await;
                                *guard = Some(headers.clone());
                            }
                            let body_bytes =
                                axum::body::to_bytes(req.into_body(), 64 * 1024).await.unwrap();
                            let body: serde_json::Value =
                                serde_json::from_slice(&body_bytes).unwrap_or(serde_json::json!({}));
                            let method = body
                                .get("method")
                                .and_then(|m| m.as_str())
                                .unwrap_or("")
                                .to_string();
                            let id = body.get("id").cloned();

                            match method.as_str() {
                                "initialize" => {
                                    counter.fetch_add(1, Ordering::SeqCst);
                                    let resp_id = id.unwrap_or(serde_json::json!(1));
                                    let response = serde_json::json!({
                                        "jsonrpc": "2.0",
                                        "id": resp_id,
                                        "result": {
                                            "protocolVersion": "2025-11-25",
                                            "capabilities": {"tools": {}},
                                            "serverInfo": {"name": "mock-mcp", "version": "0.0.1"}
                                        }
                                    });
                                    (
                                        StatusCode::OK,
                                        [
                                            ("content-type", "application/json"),
                                            ("mcp-session-id", "test-session-id"),
                                        ],
                                        serde_json::to_string(&response).unwrap(),
                                    )
                                        .into_response()
                                }
                                "notifications/initialized" => {
                                    (StatusCode::ACCEPTED, "").into_response()
                                }
                                "tools/list" => {
                                    let resp_id = id.unwrap_or(serde_json::json!(1));
                                    let response = serde_json::json!({
                                        "jsonrpc": "2.0",
                                        "id": resp_id,
                                        "result": {
                                            "tools": [
                                                {
                                                    "name": "obs.ping",
                                                    "description": "ping the server",
                                                    "inputSchema": {
                                                        "type": "object",
                                                        "properties": {}
                                                    }
                                                }
                                            ]
                                        }
                                    });
                                    (
                                        StatusCode::OK,
                                        [("content-type", "application/json")],
                                        serde_json::to_string(&response).unwrap(),
                                    )
                                        .into_response()
                                }
                                "tools/call" => {
                                    let resp_id = id.unwrap_or(serde_json::json!(1));
                                    let text_content =
                                        serde_json::to_string(&tool_call_response).unwrap();
                                    let response = serde_json::json!({
                                        "jsonrpc": "2.0",
                                        "id": resp_id,
                                        "result": {
                                            "content": [
                                                {"type": "text", "text": text_content}
                                            ],
                                            "isError": is_error
                                        }
                                    });
                                    (
                                        StatusCode::OK,
                                        [("content-type", "application/json")],
                                        serde_json::to_string(&response).unwrap(),
                                    )
                                        .into_response()
                                }
                                _ => (StatusCode::ACCEPTED, "").into_response(),
                            }
                        }
                    }
                }),
            )
            .route(
                "/mcp",
                delete(|| async { (StatusCode::OK, "").into_response() }),
            );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://127.0.0.1:{}/mcp", addr.port())
    }

    // -----------------------------------------------------------------------
    // Network tests (mock MCP server)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_call_returns_parsed_json_text_content() {
        let url = spawn_mock_mcp(serde_json::json!({"pong": true}), false, None, None).await;
        let client = McpClient::new(url, "");
        let result = client
            .call("obs.ping", &serde_json::json!({"target_guid": 1}))
            .await
            .expect("call should succeed");
        assert_eq!(result, serde_json::json!({"pong": true}));
    }

    #[tokio::test]
    async fn test_list_tools_returns_tool_vec() {
        let url = spawn_mock_mcp(serde_json::json!({}), false, None, None).await;
        let client = McpClient::new(url, "");
        let tools = client.list_tools().await.expect("list_tools should succeed");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name.as_ref(), "obs.ping");
    }

    #[tokio::test]
    async fn test_bearer_header_sent_in_request() {
        let captured: Arc<tokio::sync::Mutex<Option<HeaderMap>>> =
            Arc::new(tokio::sync::Mutex::new(None));
        let url = spawn_mock_mcp(
            serde_json::json!({"pong": true}),
            false,
            None,
            Some(captured.clone()),
        )
        .await;

        let client = McpClient::new(url, "test-token");
        let _ = client
            .call("obs.ping", &serde_json::json!({}))
            .await
            .expect("call should succeed");

        let guard = captured.lock().await;
        let headers = guard.as_ref().expect("headers should have been captured");
        let auth = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(auth, "Bearer test-token");
    }

    #[tokio::test]
    async fn test_is_error_propagates_as_err() {
        let url = spawn_mock_mcp(
            serde_json::json!({"detail": "boom"}),
            true, // is_error
            None,
            None,
        )
        .await;
        let client = McpClient::new(url, "");
        let err = client
            .call("obs.ping", &serde_json::json!({}))
            .await
            .expect_err("should propagate as Err");
        assert!(
            err.to_string().contains("obs.ping"),
            "error message includes tool name: {err}"
        );
    }

    #[tokio::test]
    async fn test_per_call_sessions_independent() {
        let counter = Arc::new(AtomicUsize::new(0));
        let url = spawn_mock_mcp(
            serde_json::json!({"pong": true}),
            false,
            Some(counter.clone()),
            None,
        )
        .await;

        let client = McpClient::new(url, "");

        // First call
        let _ = client
            .call("obs.ping", &serde_json::json!({}))
            .await
            .expect("first call");
        // Second call — must open a fresh session (new initialize)
        let _ = client
            .call("obs.ping", &serde_json::json!({}))
            .await
            .expect("second call");

        let init_count = counter.load(Ordering::SeqCst);
        assert_eq!(
            init_count,
            2,
            "two calls must trigger two initialize handshakes; got {init_count}"
        );
    }

    // -----------------------------------------------------------------------
    // Helper — build CallToolResult via serde round-trip to avoid
    // the #[non_exhaustive] struct literal restriction
    // -----------------------------------------------------------------------

    fn make_call_result(text: &str, is_error: bool) -> CallToolResult {
        let json = serde_json::json!({
            "content": [{"type": "text", "text": text}],
            "isError": is_error
        });
        serde_json::from_value(json).expect("static json is valid")
    }
}
