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

/// Successful chat-completion result.
#[derive(Debug)]
pub struct LlmResponse {
    pub parsed: Option<Value>,
    pub raw: String,
    pub latency_ms: f64,
    /// True when a `json_schema` request returned 400 and the json_object retry was issued.
    pub json_schema_fell_back: bool,
}

/// Classified transport/HTTP failure of a chat call.
#[derive(Debug, Clone)]
pub enum LlmErrorClass {
    Timeout,
    Transport,
    HttpStatus(u16),
}
impl LlmErrorClass {
    pub fn as_str(&self) -> String {
        match self {
            LlmErrorClass::Timeout => "timeout".to_string(),
            LlmErrorClass::Transport => "transport".to_string(),
            LlmErrorClass::HttpStatus(c) => format!("http_{c}"),
        }
    }
    fn from_reqwest(e: &reqwest::Error) -> Self {
        if e.is_timeout() {
            LlmErrorClass::Timeout
        } else if let Some(s) = e.status() {
            LlmErrorClass::HttpStatus(s.as_u16())
        } else {
            LlmErrorClass::Transport
        }
    }
}

/// Error returned by `chat_completion_json`, carrying timing + fallback context.
#[derive(Debug, Clone)]
pub struct LlmCallError {
    pub class: LlmErrorClass,
    pub elapsed_ms: f64,
    pub json_schema_fell_back: bool,
}
impl std::fmt::Display for LlmCallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "llm_call_error class={} elapsed_ms={:.0} json_schema_fell_back={}",
            self.class.as_str(), self.elapsed_ms, self.json_schema_fell_back
        )
    }
}
impl std::error::Error for LlmCallError {}

impl LlmClient {
    /// Call /v1/chat/completions. Returns `LlmResponse` on success or a classified
    /// `LlmCallError` (with elapsed + fallback context) on transport/HTTP failure.
    ///
    /// On HTTP 400 with json_schema active: logs a warning and retries ONCE with
    /// `response_format = {type:"json_object"}`, setting `json_schema_fell_back`.
    pub async fn chat_completion_json(
        &self,
        system: &str,
        user: &str,
        max_tokens: u32,
        temperature: f64,
        json_schema: Option<&Value>,
    ) -> Result<LlmResponse, LlmCallError> {
        let t0 = Instant::now();
        let mut fell_back = false;
        let mk_err = |e: &reqwest::Error, fb: bool| LlmCallError {
            class: LlmErrorClass::from_reqwest(e),
            elapsed_ms: t0.elapsed().as_secs_f64() * 1000.0,
            json_schema_fell_back: fb,
        };

        let client = Client::builder()
            .timeout(std::time::Duration::from_secs_f64(self.timeout_s))
            .build()
            .map_err(|e| mk_err(&e, fell_back))?;

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
        let url = format!("{}/v1/chat/completions", self.base_url.trim_end_matches('/'));
        let resp = client.post(&url).json(&payload).send().await.map_err(|e| mk_err(&e, fell_back))?;

        let resp = if resp.status() == 400 && json_schema.is_some() {
            tracing::warn!("llm_client_json_schema_rejected_falling_back status=400");
            fell_back = true;
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
            client.post(&url).json(&fallback_payload).send().await.map_err(|e| mk_err(&e, fell_back))?
        } else {
            resp
        };

        let data: Value = resp
            .error_for_status()
            .map_err(|e| mk_err(&e, fell_back))?
            .json()
            .await
            .map_err(|e| mk_err(&e, fell_back))?;
        let latency_ms = t0.elapsed().as_secs_f64() * 1000.0;
        let raw = data["choices"][0]["message"]["content"].as_str().unwrap_or("").to_string();
        let parsed = serde_json::from_str(&raw).ok();
        Ok(LlmResponse { parsed, raw, latency_ms, json_schema_fell_back: fell_back })
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
        let client = LlmClient { base_url, model: "test".into(), timeout_s: 5.0 };
        let r = client.chat_completion_json("system", "user", 400, 0.5, None).await.unwrap();
        assert!(r.parsed.is_some());
        assert!(r.raw.contains("no_op"));
        assert!(r.latency_ms > 0.0);
        assert!(!r.json_schema_fell_back);
    }

    #[tokio::test]
    async fn test_chat_completion_unparseable_returns_none_raw() {
        let resp = make_llm_response("not json at all");
        let base_url = spawn_mock_llm(resp).await;
        let client = LlmClient { base_url, model: "test".into(), timeout_s: 5.0 };
        let r = client.chat_completion_json("system", "user", 400, 0.5, None).await.unwrap();
        assert!(r.parsed.is_none());
        assert_eq!(r.raw, "not json at all");
    }

    #[tokio::test]
    async fn test_400_fallback_sets_fell_back_flag() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;
        let count = Arc::new(AtomicU32::new(0));
        let count2 = count.clone();
        let app = Router::new().route(
            "/v1/chat/completions",
            post(move |Json(_b): Json<serde_json::Value>| {
                let count = count2.clone();
                async move {
                    let n = count.fetch_add(1, Ordering::SeqCst);
                    if n == 0 {
                        (axum::http::StatusCode::BAD_REQUEST, "json_schema rejected").into_response()
                    } else {
                        Json(serde_json::json!({"choices":[{"message":{"content":
                            "{\"kind\":\"no_op\",\"tool\":null,\"args\":null,\"confidence\":0.3,\"reasoning\":\"fb\"}"}}]}))
                            .into_response()
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let base_url = format!("http://127.0.0.1:{}", addr.port());
        let client = LlmClient { base_url, model: "test".into(), timeout_s: 5.0 };
        let schema = serde_json::json!({"oneOf": []});
        let r = client.chat_completion_json("system", "user", 400, 0.5, Some(&schema)).await.unwrap();
        assert!(r.parsed.is_some(), "fallback should succeed");
        assert!(r.json_schema_fell_back, "json_schema_fell_back must be true");
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_http_status_error_classified_with_latency() {
        let app = Router::new().route(
            "/v1/chat/completions",
            post(|| async { (axum::http::StatusCode::BAD_REQUEST, "nope").into_response() }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let base_url = format!("http://127.0.0.1:{}", addr.port());
        let client = LlmClient { base_url, model: "test".into(), timeout_s: 5.0 };
        let schema = serde_json::json!({"oneOf": []});
        let err = client.chat_completion_json("s", "u", 400, 0.5, Some(&schema)).await.unwrap_err();
        assert_eq!(err.class.as_str(), "http_400");
        assert!(err.elapsed_ms >= 0.0);
        assert!(err.json_schema_fell_back, "json_schema 400 happened before fallback also 400'd");
    }

    #[tokio::test]
    async fn test_transport_error_when_unreachable() {
        let client = LlmClient {
            base_url: "http://127.0.0.1:1".into(), model: "test".into(), timeout_s: 2.0,
        };
        let err = client.chat_completion_json("s", "u", 400, 0.5, None).await.unwrap_err();
        assert_eq!(err.class.as_str(), "transport");
    }
}
