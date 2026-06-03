/// Thin async HTTP client for the llama-server OpenAI-compatible chat API.
/// Faithful port of brain_sidecar/llm_client.py.
use reqwest::Client;
use serde_json::Value;
use std::time::Instant;

pub struct LlmClient {
    pub base_url: String,
    pub model: String,
    pub timeout_s: f64,
}

impl LlmClient {
    /// Call /v1/chat/completions. Returns (parsed_json|None, raw_text, latency_ms).
    ///
    /// When `json_schema` is provided: uses
    ///   `response_format = {type:"json_schema", json_schema:{name:"decision", schema:<schema>, strict:true}}`.
    /// When omitted: `{type:"json_object"}`.
    ///
    /// On HTTP 400 with json_schema active: logs a warning and retries ONCE
    /// with `response_format = {type:"json_object"}` then calls `error_for_status`.
    ///
    /// Mirrors llm_client.py `chat_completion_json` exactly.
    pub async fn chat_completion_json(
        &self,
        system: &str,
        user: &str,
        max_tokens: u32,
        temperature: f64,
        json_schema: Option<&Value>,
    ) -> Result<(Option<Value>, String, f64), reqwest::Error> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs_f64(self.timeout_s))
            .build()?;
        let response_format = if let Some(schema) = json_schema {
            serde_json::json!({
                "type": "json_schema",
                "json_schema": {"name": "decision", "schema": schema, "strict": true}
            })
        } else {
            serde_json::json!({"type": "json_object"})
        };
        let payload = serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user}
            ],
            "max_tokens": max_tokens,
            "temperature": temperature,
            "response_format": response_format
        });
        let t0 = Instant::now();
        let url = format!("{}/v1/chat/completions", self.base_url.trim_end_matches('/'));
        let resp = client.post(&url).json(&payload).send().await?;
        // 400 fallback: retry once with json_object (mirrors Python raise-on-400 guard)
        let resp = if resp.status() == 400 && json_schema.is_some() {
            tracing::warn!("llm_client_json_schema_rejected_falling_back status=400");
            let fallback_payload = serde_json::json!({
                "model": self.model,
                "messages": [
                    {"role": "system", "content": system},
                    {"role": "user", "content": user}
                ],
                "max_tokens": max_tokens,
                "temperature": temperature,
                "response_format": {"type": "json_object"}
            });
            client.post(&url).json(&fallback_payload).send().await?
        } else {
            resp
        };
        let data: Value = resp.error_for_status()?.json().await?;
        let latency_ms = t0.elapsed().as_secs_f64() * 1000.0;
        let raw = data["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let parsed = serde_json::from_str(&raw).ok();
        Ok((parsed, raw, latency_ms))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::post, Json, Router, response::IntoResponse};

    /// Spawn a local axum mock LLM server; return its base URL.
    async fn spawn_mock_llm(response_body: serde_json::Value) -> String {
        let app = Router::new().route(
            "/v1/chat/completions",
            post(move || async move { Json(response_body.clone()).into_response() }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://127.0.0.1:{}", addr.port())
    }

    fn make_llm_response(content: &str) -> serde_json::Value {
        serde_json::json!({
            "choices": [{"message": {"content": content}}]
        })
    }

    #[tokio::test]
    async fn test_chat_completion_returns_parsed_json() {
        let resp = make_llm_response(
            r#"{"kind":"no_op","tool":null,"args":null,"confidence":0.5,"reasoning":"idle"}"#,
        );
        let base_url = spawn_mock_llm(resp).await;
        let client = LlmClient {
            base_url,
            model: "test".into(),
            timeout_s: 5.0,
        };
        let (parsed, raw, latency_ms) = client
            .chat_completion_json("system", "user", 400, 0.5, None)
            .await
            .unwrap();
        assert!(parsed.is_some());
        assert!(raw.contains("no_op"));
        assert!(latency_ms > 0.0);
    }

    #[tokio::test]
    async fn test_chat_completion_unparseable_returns_none_raw() {
        let resp = make_llm_response("not json at all");
        let base_url = spawn_mock_llm(resp).await;
        let client = LlmClient {
            base_url,
            model: "test".into(),
            timeout_s: 5.0,
        };
        let (parsed, raw, _) = client
            .chat_completion_json("system", "user", 400, 0.5, None)
            .await
            .unwrap();
        assert!(parsed.is_none());
        assert_eq!(raw, "not json at all");
    }

    #[tokio::test]
    async fn test_400_fallback_to_json_object_mode() {
        // First request returns 400; second (fallback) returns 200 with valid JSON.
        // Test that the client retries with response_format = json_object.
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;
        let count = Arc::new(AtomicU32::new(0));
        let count2 = count.clone();
        let app = Router::new().route(
            "/v1/chat/completions",
            post(move |Json(_body): Json<serde_json::Value>| {
                let count = count2.clone();
                async move {
                    let n = count.fetch_add(1, Ordering::SeqCst);
                    if n == 0 {
                        // First call: simulate 400 (json_schema rejected)
                        (axum::http::StatusCode::BAD_REQUEST, "json_schema rejected")
                            .into_response()
                    } else {
                        // Second call: return valid JSON with json_object format
                        Json(serde_json::json!({
                            "choices": [{"message": {"content": "{\"kind\":\"no_op\",\"tool\":null,\"args\":null,\"confidence\":0.3,\"reasoning\":\"fallback\"}"}}]
                        }))
                        .into_response()
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let base_url = format!("http://127.0.0.1:{}", addr.port());
        let client = LlmClient {
            base_url,
            model: "test".into(),
            timeout_s: 5.0,
        };
        let schema = serde_json::json!({"oneOf": []});
        let (parsed, _, _) = client
            .chat_completion_json("system", "user", 400, 0.5, Some(&schema))
            .await
            .unwrap();
        assert!(parsed.is_some(), "fallback should succeed");
        assert_eq!(count.load(Ordering::SeqCst), 2, "must have made exactly 2 calls");
    }
}
