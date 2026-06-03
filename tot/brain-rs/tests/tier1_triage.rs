/// Tier-1 parity tests for triage.rs — verifies reason strings, gate priority
/// order, CHAT_PREFIXES, _BRAIN_OUTCOME_PREFIXES, and hot_inputs assembly.
///
/// All tests use injected MockMcp — no real network; no global state mutation.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use brain_rs::models::TickState;
use brain_rs::personality::McpCallable;
use brain_rs::triage::{TriageGate, CHAT_PREFIXES, _BRAIN_OUTCOME_PREFIXES, is_player_chat};

// ---------------------------------------------------------------------------
// MockMcp helpers
// ---------------------------------------------------------------------------

struct MockMcp {
    response: serde_json::Value,
    calls: Mutex<Vec<String>>,
}

impl MockMcp {
    fn with_response(response: serde_json::Value) -> Arc<Self> {
        Arc::new(Self {
            response,
            calls: Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl McpCallable for MockMcp {
    fn call<'a>(
        &'a self,
        tool: &'a str,
        _args: serde_json::Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<serde_json::Value, anyhow::Error>> + Send + 'a>,
    > {
        let response = self.response.clone();
        self.calls.lock().unwrap().push(tool.to_string());
        Box::pin(async move { Ok(response) })
    }
}

struct ErrorMcp;

impl McpCallable for ErrorMcp {
    fn call<'a>(
        &'a self,
        _tool: &'a str,
        _args: serde_json::Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<serde_json::Value, anyhow::Error>> + Send + 'a>,
    > {
        Box::pin(async move { Err(anyhow::anyhow!("simulated error")) })
    }
}

/// State with no combat, no pending invite.
fn no_combat() -> serde_json::Value {
    serde_json::json!({
        "result": {
            "self": { "pending_group_invite": null },
            "level": 30,
            "events": []
        }
    })
}

/// Alias: same as no_combat but named to match plan helper names.
fn no_combat_no_invite() -> serde_json::Value {
    no_combat()
}

fn mock_harness_always_ok() -> Arc<dyn McpCallable> {
    MockMcp::with_response(no_combat())
}

fn mock_harness_with_state(state: serde_json::Value) -> Arc<dyn McpCallable> {
    MockMcp::with_response(state)
}

fn mock_harness_timeout() -> Arc<dyn McpCallable> {
    Arc::new(ErrorMcp) as Arc<dyn McpCallable>
}

fn mock_memory_always_ok() -> Arc<dyn McpCallable> {
    MockMcp::with_response(serde_json::json!({
        "result": { "items": [], "goals": [] }
    }))
}

fn mock_memory_timeout() -> Arc<dyn McpCallable> {
    Arc::new(ErrorMcp) as Arc<dyn McpCallable>
}

fn mock_memory_no_calls() -> Arc<dyn McpCallable> {
    // Returns valid empty response; tests verify memory was not called for SSE path.
    MockMcp::with_response(serde_json::json!({
        "result": { "items": [], "goals": [] }
    }))
}

fn mock_memory_no_chat() -> Arc<dyn McpCallable> {
    MockMcp::with_response(serde_json::json!({
        "result": { "items": [], "goals": [] }
    }))
}

fn mock_memory_no_chat_no_goals() -> Arc<dyn McpCallable> {
    mock_memory_no_chat()
}

// ---------------------------------------------------------------------------
// Constant parity tests
// ---------------------------------------------------------------------------

#[test]
fn test_chat_prefixes_exact() {
    // CHAT_PREFIXES must contain exactly these strings (from triage.py).
    let expected = [
        "received whisper",
        "chat_received",
        "received party",
        "received say",
        "received guild",
        "whisper from",
    ];
    for p in &expected {
        assert!(
            CHAT_PREFIXES.iter().any(|cp| cp == p),
            "missing prefix: {p}"
        );
    }
    assert_eq!(
        CHAT_PREFIXES.len(),
        expected.len(),
        "CHAT_PREFIXES length mismatch"
    );
}

#[test]
fn test_brain_outcome_prefixes_exact() {
    let expected = [
        "brain_no_op:",
        "action_taken:",
        "tool_call_failed:",
        "pending_confirmation:",
        "blocked_cross_bot:",
        "decision_invalid_tool:",
    ];
    for p in &expected {
        assert!(
            _BRAIN_OUTCOME_PREFIXES.iter().any(|cp| cp == p),
            "missing brain outcome prefix: {p}"
        );
    }
    assert_eq!(
        _BRAIN_OUTCOME_PREFIXES.len(),
        expected.len(),
        "_BRAIN_OUTCOME_PREFIXES length mismatch"
    );
}

#[test]
fn test_is_player_chat_rejects_brain_outcome_prefixes() {
    for prefix in _BRAIN_OUTCOME_PREFIXES.iter() {
        let item =
            serde_json::json!({"text": format!("{} some context", prefix)});
        assert!(!is_player_chat(&item), "should reject brain outcome: {prefix}");
    }
}

#[test]
fn test_is_player_chat_accepts_whisper() {
    let item = serde_json::json!({"text": "received whisper from Alice: hello"});
    assert!(is_player_chat(&item));
}

#[test]
fn test_is_player_chat_accepts_all_prefixes() {
    for prefix in CHAT_PREFIXES.iter() {
        let item = serde_json::json!({"text": format!("{} some message", prefix)});
        assert!(is_player_chat(&item), "should accept player chat prefix: {prefix}");
    }
}

// ---------------------------------------------------------------------------
// Gate / reason string parity tests (exact byte-match to triage.py)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_first_tick_always_fires() {
    // last_tick_ms == 0 → should_decide=true, reason="first_tick"
    let gate = TriageGate::new_with_mocks(mock_harness_always_ok(), mock_memory_always_ok());
    let state = TickState::new(1001); // last_tick_ms=0
    let result = gate.evaluate(1001, &state, 0, None, None).await;
    assert!(result.should_decide);
    assert_eq!(result.reason, "first_tick");
}

#[tokio::test]
async fn test_triage_timeout_when_mcp_returns_error() {
    // Any MCP call returning Err → triage_timeout (no decide).
    let gate = TriageGate::new_with_mocks(mock_harness_timeout(), mock_memory_timeout());
    let state = TickState { bot_guid: 1001, last_tick_ms: 1000, last_decision_id: None };
    let result = gate.evaluate(1001, &state, 2000, None, None).await;
    assert!(!result.should_decide);
    assert_eq!(result.reason, "triage_timeout");
}

#[tokio::test]
async fn test_fresh_chat_via_sse_inputs_skips_poll() {
    // sse_inputs with fresh_chat → should_decide=true, reason="fresh_chat"
    let memory_mock = MockMcp::with_response(serde_json::json!({
        "result": { "items": [], "goals": [] }
    }));
    let gate = TriageGate::new_with_mocks(
        mock_harness_with_state(no_combat()),
        memory_mock.clone(),
    );
    let state = TickState { bot_guid: 1001, last_tick_ms: 1000, last_decision_id: None };
    let sse_val = serde_json::json!([{
        "text": "received whisper from Alice: hi",
        "memory_id": "abc"
    }]);
    let mut map = HashMap::new();
    map.insert("fresh_chat".to_string(), sse_val);
    let result = gate.evaluate(1001, &state, 2000, Some(&map), None).await;
    assert!(result.should_decide);
    assert_eq!(result.reason, "fresh_chat");
    // hot_inputs must include both fresh_chat and state_summary.
    assert!(result.hot_inputs.contains_key("fresh_chat"));
    assert!(result.hot_inputs.contains_key("state_summary"));
    // memory.search must NOT have been called (SSE fast-path skips poll).
    let calls = memory_mock.calls();
    assert!(
        !calls.iter().any(|c| c == "memory.search"),
        "memory.search should be skipped on SSE fast-path; got calls: {calls:?}"
    );
}

#[tokio::test]
async fn test_organic_wakeup_fires_when_timer_expired() {
    let gate = TriageGate::new_with_mocks(
        mock_harness_with_state(no_combat_no_invite()),
        mock_memory_no_chat(),
    );
    let state = TickState { bot_guid: 1001, last_tick_ms: 1000, last_decision_id: None };
    // next_wakeup_at_ms = 1500, now_ms = 2000 → wakeup expired
    let result = gate.evaluate(1001, &state, 2000, None, Some(1500)).await;
    assert!(result.should_decide);
    assert_eq!(result.reason, "organic_wakeup");
    assert!(result.hot_inputs.contains_key("state_summary"));
}

#[tokio::test]
async fn test_organic_wakeup_does_not_fire_when_not_expired() {
    let gate = TriageGate::new_with_mocks(
        mock_harness_with_state(no_combat_no_invite()),
        mock_memory_no_chat_no_goals(),
    );
    let state = TickState { bot_guid: 1001, last_tick_ms: 1000, last_decision_id: None };
    // now_ms (1200) < next_wakeup_at_ms (1500) → not expired
    let result = gate.evaluate(1001, &state, 1200, None, Some(1500)).await;
    assert!(!result.should_decide);
    assert_eq!(result.reason, "no_change");
}

#[tokio::test]
async fn test_no_change_when_nothing_new() {
    let gate = TriageGate::new_with_mocks(
        mock_harness_with_state(no_combat_no_invite()),
        mock_memory_no_chat_no_goals(),
    );
    let state = TickState { bot_guid: 1001, last_tick_ms: 1000, last_decision_id: None };
    let result = gate.evaluate(1001, &state, 2000, None, None).await;
    assert!(!result.should_decide);
    assert_eq!(result.reason, "no_change");
}

#[tokio::test]
async fn test_party_invite_received() {
    let harness = MockMcp::with_response(serde_json::json!({
        "result": {
            "self": {
                "pending_group_invite": {"from_name": "Alice", "group_type": "party"}
            },
            "level": 30,
            "events": []
        }
    }));
    let gate = TriageGate::new_with_mocks(harness, mock_memory_no_chat());
    let state = TickState { bot_guid: 1001, last_tick_ms: 1000, last_decision_id: None };
    let result = gate.evaluate(1001, &state, 2000, None, None).await;
    assert!(result.should_decide);
    assert_eq!(result.reason, "party_invite_received");
    assert!(result.hot_inputs.contains_key("pending_invite"));
    assert!(result.hot_inputs.contains_key("state_summary"));
}

#[tokio::test]
async fn test_combat_event_fires() {
    let harness = MockMcp::with_response(serde_json::json!({
        "result": {
            "self": { "pending_group_invite": null },
            "level": 30,
            "events": [{"type": "damage", "target": "mob", "amount": 120}]
        }
    }));
    let gate = TriageGate::new_with_mocks(harness, mock_memory_no_chat());
    let state = TickState { bot_guid: 1001, last_tick_ms: 1000, last_decision_id: None };
    let result = gate.evaluate(1001, &state, 2000, None, None).await;
    assert!(result.should_decide);
    assert_eq!(result.reason, "combat_event");
    assert!(result.hot_inputs.contains_key("combat_events"));
    assert!(result.hot_inputs.contains_key("state_summary"));
}

#[tokio::test]
async fn test_polled_fresh_chat_fires_with_watermark_filter() {
    // Items with ts > since_ts_s AND is_player_chat → "fresh_chat"
    // since_ts_s = last_tick_ms / 1000 = 1000 / 1000 = 1
    let since_ts_s: i64 = 1;
    let harness = mock_harness_with_state(no_combat_no_invite());
    let memory = MockMcp::with_response(serde_json::json!({
        "result": {
            "items": [
                {"text": "received whisper from Bob: yo", "ts": since_ts_s + 1}
            ],
            "goals": []
        }
    }));
    let gate = TriageGate::new_with_mocks(harness, memory);
    let state = TickState { bot_guid: 1001, last_tick_ms: 1000, last_decision_id: None };
    let result = gate.evaluate(1001, &state, 5000, None, None).await;
    assert!(result.should_decide);
    assert_eq!(result.reason, "fresh_chat");
}

#[tokio::test]
async fn test_polled_chat_filtered_brain_outcome_does_not_fire() {
    // Brain-outcome rows with ts > since_ts_s must NOT trigger fresh_chat.
    let since_ts_s: i64 = 1;
    let harness = mock_harness_with_state(no_combat_no_invite());
    let memory = MockMcp::with_response(serde_json::json!({
        "result": {
            "items": [
                {"text": "brain_no_op: idle decision", "ts": since_ts_s + 1}
            ],
            "goals": []
        }
    }));
    let gate = TriageGate::new_with_mocks(harness, memory);
    let state = TickState { bot_guid: 1001, last_tick_ms: 1000, last_decision_id: None };
    let result = gate.evaluate(1001, &state, 5000, None, None).await;
    assert!(!result.should_decide);
    assert_eq!(result.reason, "no_change");
}

#[tokio::test]
async fn test_polled_chat_filtered_by_watermark() {
    // Items with ts <= since_ts_s must be excluded regardless of content.
    let since_ts_s: i64 = 5;
    let harness = mock_harness_with_state(no_combat_no_invite());
    let memory = MockMcp::with_response(serde_json::json!({
        "result": {
            "items": [
                // ts == since_ts_s: not strictly greater → excluded
                {"text": "received whisper from Alice: old", "ts": since_ts_s}
            ],
            "goals": []
        }
    }));
    let gate = TriageGate::new_with_mocks(harness, memory);
    // last_tick_ms = since_ts_s * 1000 = 5000
    let state = TickState { bot_guid: 1001, last_tick_ms: 5000, last_decision_id: None };
    let result = gate.evaluate(1001, &state, 10000, None, None).await;
    assert!(!result.should_decide);
    assert_eq!(result.reason, "no_change");
}
