/// Tier-1 integration tests for dispatch.rs and tool_policy.rs.
///
/// All async tests use injected mock McpCallable implementations —
/// no real network calls; no global state mutation.
use std::sync::{Arc, Mutex};

use brain_rs::dispatch::{
    Dispatcher, _BOT_OWN_KEYS, _OUTCOME_SALIENCE, HIGH_RISK_THRESHOLD,
    LOW_RISK_THRESHOLD, RISK_TABLE, mcp_for_tool,
};
use brain_rs::models::{Decision, DecisionKind};
use brain_rs::personality::McpCallable;

// ---------------------------------------------------------------------------
// MockMcpClient — captures calls, returns Ok(json!({})) by default.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct RecordedCall {
    pub tool: String,
    pub args: serde_json::Value,
}

struct MockMcp {
    calls: Mutex<Vec<RecordedCall>>,
    /// If Some, always return Err with this message.
    fail_with: Option<String>,
}

impl MockMcp {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            fail_with: None,
        })
    }

    fn failing(msg: &str) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            fail_with: Some(msg.to_string()),
        })
    }

    fn calls(&self) -> Vec<RecordedCall> {
        self.calls.lock().unwrap().clone()
    }
}

impl McpCallable for MockMcp {
    fn call<'a>(
        &'a self,
        tool: &'a str,
        args: serde_json::Value,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<serde_json::Value, anyhow::Error>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            self.calls.lock().unwrap().push(RecordedCall {
                tool: tool.to_string(),
                args: args.clone(),
            });
            if let Some(ref msg) = self.fail_with {
                Err(anyhow::anyhow!("{}", msg))
            } else {
                Ok(serde_json::json!({"ok": true}))
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_dispatcher(
    harness: Arc<MockMcp>,
    memory: Arc<MockMcp>,
    policy: Option<brain_rs::tool_policy::ToolPolicyEnforcer>,
) -> Dispatcher {
    Dispatcher::new(harness, memory, "whisper", policy)
}

fn action(tool: &str, args: serde_json::Value, confidence: f64) -> Decision {
    Decision {
        kind: DecisionKind::Action,
        tool: Some(tool.to_string()),
        args: Some(args),
        confidence,
        reasoning: "test reasoning".to_string(),
        wakeup_in_ms: None,
    }
}

// ---------------------------------------------------------------------------
// Static-data tests (no async)
// ---------------------------------------------------------------------------

#[test]
fn test_mcp_routing_bot_prefix_goes_to_harness() {
    assert_eq!(mcp_for_tool("bot.send_chat"), "harness");
    assert_eq!(mcp_for_tool("obs.get_state"), "harness");
    assert_eq!(mcp_for_tool("gm.teleport"), "harness");
    assert_eq!(mcp_for_tool("bot_guid_something"), "harness");
    assert_eq!(mcp_for_tool("obs_get_state"), "harness");
    assert_eq!(mcp_for_tool("gm_something"), "harness");
}

#[test]
fn test_mcp_routing_memory_prefix_goes_to_memory() {
    assert_eq!(mcp_for_tool("memory.write"), "memory");
    assert_eq!(mcp_for_tool("goals.create"), "memory");
    assert_eq!(mcp_for_tool("memory.recall_about"), "memory");
}

#[test]
fn test_risk_table_thresholds() {
    assert_eq!(LOW_RISK_THRESHOLD, 0.7f64);
    assert_eq!(HIGH_RISK_THRESHOLD, 0.95f64);
    assert_eq!(RISK_TABLE.get("bot.send_chat"), Some(&"low"));
    assert_eq!(RISK_TABLE.get("bot.invite_to_group"), Some(&"high"));
    assert_eq!(RISK_TABLE.get("obs.get_state"), Some(&"low"));
    // Unknown tool defaults to high — not in map
    assert_eq!(RISK_TABLE.get("unknown_tool"), None);
}

#[test]
fn test_bot_own_keys_contains_bot_guid_and_bot_id() {
    assert!(_BOT_OWN_KEYS.contains("bot_guid"));
    assert!(_BOT_OWN_KEYS.contains("bot_id"));
    // Other keys are NOT restricted
    assert!(!_BOT_OWN_KEYS.contains("target_guid"));
    assert!(!_BOT_OWN_KEYS.contains("player_guid"));
}

#[test]
fn test_outcome_salience_grades() {
    assert_eq!(*_OUTCOME_SALIENCE.get("action_taken").unwrap(), 0.5f64);
    assert_eq!(*_OUTCOME_SALIENCE.get("tool_call_failed").unwrap(), 0.8f64);
    assert_eq!(*_OUTCOME_SALIENCE.get("brain_no_op").unwrap(), 0.2f64);
    assert_eq!(*_OUTCOME_SALIENCE.get("pending_confirmation").unwrap(), 0.6f64);
    assert_eq!(*_OUTCOME_SALIENCE.get("blocked_cross_bot").unwrap(), 0.7f64);
}

// ---------------------------------------------------------------------------
// Async dispatch tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_no_op_decision_writes_outcome_memory_and_returns_no_op() {
    let harness = MockMcp::new();
    let memory = MockMcp::new();
    let dispatcher = make_dispatcher(harness.clone(), memory.clone(), None);

    let d = Decision {
        kind: DecisionKind::NoOp,
        tool: None,
        args: None,
        confidence: 0.0,
        reasoning: "idle".to_string(),
        wakeup_in_ms: None,
    };
    let result = dispatcher.dispatch(1001, &d, "full").await.unwrap();
    assert_eq!(result.disposition, "no_op");
    // memory.write must have been called with brain_no_op memory_type
    let mem_calls = memory.calls();
    assert!(
        mem_calls.iter().any(|c| c.tool == "memory.write"),
        "expected memory.write call for no_op outcome, got: {:?}",
        mem_calls
    );
}

#[tokio::test]
async fn test_cross_bot_violation_blocks_dispatch() {
    let harness = MockMcp::new();
    let memory = MockMcp::new();
    let dispatcher = make_dispatcher(harness.clone(), memory.clone(), None);

    let d = action(
        "bot.send_chat",
        serde_json::json!({"bot_guid": 9999}), // wrong bot
        0.99,
    );
    let result = dispatcher.dispatch(1001, &d, "full").await.unwrap();
    assert_eq!(result.disposition, "blocked_cross_bot");
}

#[tokio::test]
async fn test_cross_bot_bot_id_violation() {
    let harness = MockMcp::new();
    let memory = MockMcp::new();
    let dispatcher = make_dispatcher(harness.clone(), memory.clone(), None);

    // bot_id is also a guarded key (memory.* family)
    let d = action(
        "memory.write",
        serde_json::json!({"bot_id": "9999"}), // wrong bot as string
        0.99,
    );
    let result = dispatcher.dispatch(1001, &d, "full").await.unwrap();
    assert_eq!(result.disposition, "blocked_cross_bot");
}

#[tokio::test]
async fn test_cross_bot_garbage_value_is_violation() {
    let harness = MockMcp::new();
    let memory = MockMcp::new();
    let dispatcher = make_dispatcher(harness.clone(), memory.clone(), None);

    let d = action(
        "memory.write",
        serde_json::json!({"bot_id": null}), // null = garbage
        0.99,
    );
    let result = dispatcher.dispatch(1001, &d, "full").await.unwrap();
    assert_eq!(result.disposition, "blocked_cross_bot");
}

#[tokio::test]
async fn test_confirmation_emitted_for_high_risk_below_threshold() {
    let harness = MockMcp::new();
    let memory = MockMcp::new();
    let dispatcher = make_dispatcher(harness.clone(), memory.clone(), None);

    let d = Decision {
        kind: DecisionKind::Action,
        tool: Some("bot.invite_to_group".to_string()), // high risk
        args: Some(serde_json::json!({"bot_guid": 1001, "target_guid": 2000})),
        confidence: 0.8, // below 0.95
        reasoning: "want to invite".to_string(),
        wakeup_in_ms: None,
    };
    let result = dispatcher.dispatch(1001, &d, "full").await.unwrap();
    assert_eq!(result.disposition, "confirmation_emitted");

    // bot.send_chat must have been called with confirmation text
    let harness_calls = harness.calls();
    let chat_calls: Vec<_> = harness_calls
        .iter()
        .filter(|c| c.tool == "bot.send_chat")
        .collect();
    assert!(
        !chat_calls.is_empty(),
        "confirmation chat must be sent via bot.send_chat"
    );
    let msg = chat_calls[0].args["message"].as_str().unwrap();
    assert!(
        msg.contains("want me to invite someone to group"),
        "confirmation text mismatch: {msg}"
    );
    // Also verify pending_confirmation memory was written
    let mem_calls = memory.calls();
    assert!(
        mem_calls.iter().any(|c| c.tool == "memory.write"),
        "pending_confirmation memory.write must be called"
    );
}

#[tokio::test]
async fn test_confirmation_text_no_reasoning() {
    let harness = MockMcp::new();
    let memory = MockMcp::new();
    let dispatcher = make_dispatcher(harness.clone(), memory.clone(), None);

    let d = Decision {
        kind: DecisionKind::Action,
        tool: Some("bot.invite_to_group".to_string()),
        args: Some(serde_json::json!({"bot_guid": 1001})),
        confidence: 0.8,
        reasoning: String::new(), // empty reasoning
        wakeup_in_ms: None,
    };
    let result = dispatcher.dispatch(1001, &d, "full").await.unwrap();
    assert_eq!(result.disposition, "confirmation_emitted");

    let harness_calls = harness.calls();
    let msg = harness_calls
        .iter()
        .find(|c| c.tool == "bot.send_chat")
        .unwrap()
        .args["message"]
        .as_str()
        .unwrap()
        .to_string();
    // Without reasoning: "want me to invite someone to group?"
    assert_eq!(msg, "want me to invite someone to group?");
}

#[tokio::test]
async fn test_invalid_tool_blocked_before_cross_bot() {
    let harness = MockMcp::new();
    let memory = MockMcp::new();
    let dispatcher = make_dispatcher(harness.clone(), memory.clone(), None);

    let d = Decision {
        kind: DecisionKind::Action,
        tool: Some("not_a_real_tool".to_string()),
        args: Some(serde_json::json!({"bot_guid": 9999})), // would be cross-bot if tool real
        confidence: 0.99,
        reasoning: "test".to_string(),
        wakeup_in_ms: None,
    };
    let result = dispatcher.dispatch(1001, &d, "full").await.unwrap();
    assert_eq!(
        result.disposition, "invalid_tool",
        "unknown tool must produce invalid_tool not blocked_cross_bot"
    );
}

#[tokio::test]
async fn test_reduced_tier_blocks_bot_invite() {
    let harness = MockMcp::new();
    let memory = MockMcp::new();
    let policy = brain_rs::tool_policy::ToolPolicyEnforcer;
    let dispatcher = make_dispatcher(harness.clone(), memory.clone(), Some(policy));

    let d = Decision {
        kind: DecisionKind::Action,
        tool: Some("bot.invite_to_group".to_string()),
        args: Some(serde_json::json!({"bot_guid": 1001, "target_guid": 2000})),
        confidence: 0.99, // high confidence — should not matter; tier blocks it
        reasoning: "test".to_string(),
        wakeup_in_ms: None,
    };
    let result = dispatcher.dispatch(1001, &d, "reduced").await.unwrap();
    assert_eq!(result.disposition, "policy_denied");
}

#[tokio::test]
async fn test_reduced_tier_allows_set_strategy() {
    let harness = MockMcp::new();
    let memory = MockMcp::new();
    let policy = brain_rs::tool_policy::ToolPolicyEnforcer;
    let dispatcher = make_dispatcher(harness.clone(), memory.clone(), Some(policy));

    let d = Decision {
        kind: DecisionKind::Action,
        tool: Some("bot.set_strategy".to_string()),
        args: Some(serde_json::json!({"bot_guid": 1001, "strategy": "grind"})),
        confidence: 0.99,
        reasoning: "test".to_string(),
        wakeup_in_ms: None,
    };
    let result = dispatcher.dispatch(1001, &d, "reduced").await.unwrap();
    assert_eq!(result.disposition, "executed");
}

#[tokio::test]
async fn test_low_risk_above_threshold_executes() {
    let harness = MockMcp::new();
    let memory = MockMcp::new();
    let dispatcher = make_dispatcher(harness.clone(), memory.clone(), None);

    // bot.send_chat is low-risk; 0.75 >= 0.7 → execute
    let d = action(
        "bot.send_chat",
        serde_json::json!({"bot_guid": 1001, "channel": "say", "message": "hello"}),
        0.75,
    );
    let result = dispatcher.dispatch(1001, &d, "full").await.unwrap();
    assert_eq!(result.disposition, "executed");

    // harness.call("bot.send_chat", ...) must have been called
    let calls = harness.calls();
    assert!(
        calls.iter().any(|c| c.tool == "bot.send_chat"),
        "harness must have been called with bot.send_chat"
    );
    // outcome memory action_taken
    let mem = memory.calls();
    assert!(mem.iter().any(|c| c.tool == "memory.write"));
}

#[tokio::test]
async fn test_low_risk_below_threshold_emits_confirmation() {
    let harness = MockMcp::new();
    let memory = MockMcp::new();
    let dispatcher = make_dispatcher(harness.clone(), memory.clone(), None);

    // bot.send_chat is low-risk; 0.5 < 0.7 → confirmation
    let d = action(
        "bot.send_chat",
        serde_json::json!({"bot_guid": 1001, "channel": "say", "message": "hello"}),
        0.5,
    );
    let result = dispatcher.dispatch(1001, &d, "full").await.unwrap();
    assert_eq!(result.disposition, "confirmation_emitted");
}

#[tokio::test]
async fn test_tool_call_failed_path() {
    let harness = MockMcp::failing("network error");
    let memory = MockMcp::new();
    let dispatcher = make_dispatcher(harness.clone(), memory.clone(), None);

    // bot.send_chat low-risk, high confidence → reaches execute → harness fails
    let d = action(
        "bot.send_chat",
        serde_json::json!({"bot_guid": 1001, "channel": "say", "message": "hello"}),
        0.99,
    );
    let result = dispatcher.dispatch(1001, &d, "full").await.unwrap();
    assert_eq!(result.disposition, "tool_call_failed");
    assert!(result.error.contains("network error"));

    // tool_call_failed memory must be written
    let mem = memory.calls();
    assert!(
        mem.iter().any(|c| c.tool == "memory.write"),
        "tool_call_failed outcome memory must be written"
    );
}

#[tokio::test]
async fn test_routing_memory_tool_uses_memory_mcp() {
    let harness = MockMcp::new();
    let memory = MockMcp::new();
    let dispatcher = make_dispatcher(harness.clone(), memory.clone(), None);

    // memory.write is a memory-routed tool; confidence 0.99 > 0.7 (low risk)
    let d = action(
        "memory.write",
        serde_json::json!({"bot_id": "1001", "text": "test", "salience": 0.5, "entities": [], "relations": []}),
        0.99,
    );
    let result = dispatcher.dispatch(1001, &d, "full").await.unwrap();
    assert_eq!(result.disposition, "executed");

    // The execute-phase call must land on memory MCP, not harness
    let mem_calls = memory.calls();
    // First call should be the tool execution, second may be the outcome memory
    let exec_calls: Vec<_> = mem_calls
        .iter()
        .filter(|c| c.tool == "memory.write")
        .collect();
    assert!(
        exec_calls.len() >= 2,
        "expected at least 2 memory.write calls (execution + outcome), got {}",
        exec_calls.len()
    );
    // harness should NOT have been called for execution
    let harness_calls = harness.calls();
    assert!(
        harness_calls.is_empty(),
        "harness must not be called for memory.write routing"
    );
}

#[tokio::test]
async fn test_outcome_memory_payload_shape_for_no_op() {
    let harness = MockMcp::new();
    let memory = MockMcp::new();
    let dispatcher = make_dispatcher(harness.clone(), memory.clone(), None);

    let d = Decision {
        kind: DecisionKind::NoOp,
        tool: None,
        args: None,
        confidence: 0.0,
        reasoning: "just chilling".to_string(),
        wakeup_in_ms: None,
    };
    dispatcher.dispatch(1001, &d, "full").await.unwrap();

    let mem_calls = memory.calls();
    let write_call = mem_calls
        .iter()
        .find(|c| c.tool == "memory.write")
        .expect("memory.write must be called");

    let args = &write_call.args;
    assert_eq!(args["bot_id"].as_str().unwrap(), "1001");
    assert_eq!(args["salience"].as_f64().unwrap(), 0.2); // brain_no_op salience
    assert!(args["text"].as_str().unwrap().starts_with("brain_no_op:"));
    assert_eq!(args["relations"], serde_json::json!([]));
    assert_eq!(args["memory_type"].as_str().unwrap(), "brain_no_op");
}

#[tokio::test]
async fn test_outcome_memory_entities_include_bot_guid_and_tool() {
    let harness = MockMcp::new();
    let memory = MockMcp::new();
    let dispatcher = make_dispatcher(harness.clone(), memory.clone(), None);

    let d = action(
        "bot.send_chat",
        serde_json::json!({"bot_guid": 1001, "channel": "say", "message": "hi"}),
        0.99,
    );
    dispatcher.dispatch(1001, &d, "full").await.unwrap();

    let mem_calls = memory.calls();
    // The outcome memory (action_taken) should have entities = ["1001", "bot.send_chat"]
    // That is the LAST memory.write call (execution outcome, not confirmation)
    let write_calls: Vec<_> = mem_calls.iter().filter(|c| c.tool == "memory.write").collect();
    let outcome_call = write_calls.last().unwrap();
    let entities = outcome_call.args["entities"].as_array().unwrap();
    let entity_strs: Vec<&str> = entities.iter().filter_map(|v| v.as_str()).collect();
    assert!(entity_strs.contains(&"1001"), "entities must contain bot_guid as string");
    assert!(entity_strs.contains(&"bot.send_chat"), "entities must contain tool name");
}

#[tokio::test]
async fn test_no_policy_does_not_block_any_bot_tool() {
    // Dispatcher without tool_policy — policy gate is skipped entirely
    let harness = MockMcp::new();
    let memory = MockMcp::new();
    let dispatcher = make_dispatcher(harness.clone(), memory.clone(), None); // no policy

    let d = Decision {
        kind: DecisionKind::Action,
        tool: Some("bot.invite_to_group".to_string()),
        args: Some(serde_json::json!({"bot_guid": 1001, "target_guid": 2000})),
        confidence: 0.99,
        reasoning: "test".to_string(),
        wakeup_in_ms: None,
    };
    let result = dispatcher.dispatch(1001, &d, "reduced").await.unwrap();
    // Without tool_policy, the policy gate is skipped; passes through to execute
    assert_eq!(result.disposition, "executed");
}
