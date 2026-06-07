/// TriageGate — decide whether the LLM (C4) should run this tick.
///
/// Faithful Rust port of brain_sidecar/triage.py TriageGate.
///
/// Priority order (from Python evaluate()):
///   1. last_tick_ms == 0         → "first_tick"       (always fire)
///   2. sse_inputs.fresh_chat     → skip chat poll; fan-out obs+combat;
///                                   return "fresh_chat"
///   3. Fan-out all 4 parallel    → obs.get_state, obs.get_combat_log,
///                                   memory.search (if no sse chat), goals.list
///   4. pending_invite present    → "party_invite_received"
///   5. Any None result           → "triage_timeout"   (no decide)
///   6. Filtered chat items found → "fresh_chat"
///   7. combat_events present     → "combat_event"
///   8. next_wakeup_at_ms expired → "organic_wakeup"
///   9. Nothing changed           → "no_change"        (no decide)
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::models::{TickState, TriageResult};
use crate::personality::McpCallable;

// ---------------------------------------------------------------------------
// Constants (public — used by SseConsumer and tests)
// ---------------------------------------------------------------------------

/// Substrings that identify a memory row as originating from a real player
/// interaction. Matches Python `_PLAYER_CHAT_SIGNALS` / `CHAT_PREFIXES`.
pub const CHAT_PREFIXES: &[&str] = &[
    "received whisper",
    "chat_received",
    "received party",
    "received say",
    "received guild",
    "whisper from",
];

/// Prefixes written by the dispatcher itself. These must NOT trigger fresh_chat
/// even if dense retrieval happens to match them against "chat OR whisper".
/// Matches Python `_BRAIN_OUTCOME_PREFIXES`.
pub const _BRAIN_OUTCOME_PREFIXES: &[&str] = &[
    "brain_no_op:",
    "action_taken:",
    "tool_call_failed:",
    "pending_confirmation:",
    "blocked_cross_bot:",
    "decision_invalid_tool:",
];

// ---------------------------------------------------------------------------
// is_player_chat (public — tested directly in tier1_triage)
// ---------------------------------------------------------------------------

/// Return `true` if the memory item looks like a real player chat message.
///
/// Mirrors Python `_is_player_chat`:
///   1. Extract text from `text` or `content` field.
///   2. Fast-reject: if text starts with any `_BRAIN_OUTCOME_PREFIXES` → false.
///   3. Accept: if text contains any `CHAT_PREFIXES` signal → true.
pub fn is_player_chat(item: &Value) -> bool {
    let text = item.get("text")
        .or_else(|| item.get("content"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let lower = text.to_lowercase();
    // Fast reject: known brain-outcome prefixes are never player chat.
    for prefix in _BRAIN_OUTCOME_PREFIXES {
        if lower.starts_with(prefix) {
            return false;
        }
    }
    // Accept: must contain at least one player-chat signal.
    for signal in CHAT_PREFIXES {
        if lower.contains(signal) {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// TriageGate
// ---------------------------------------------------------------------------

pub struct TriageGate {
    pub harness_mcp: Arc<dyn McpCallable>,
    pub memory_mcp: Arc<dyn McpCallable>,
    pub per_call_timeout_s: f64,
}

impl TriageGate {
    pub fn new(harness_mcp: Arc<dyn McpCallable>, memory_mcp: Arc<dyn McpCallable>) -> Self {
        Self {
            harness_mcp,
            memory_mcp,
            per_call_timeout_s: 5.0,
        }
    }

    /// Constructor variant used in tests — identical to `new`.
    pub fn new_with_mocks(
        harness_mcp: Arc<dyn McpCallable>,
        memory_mcp: Arc<dyn McpCallable>,
    ) -> Self {
        Self::new(harness_mcp, memory_mcp)
    }

    /// Wrap an MCP call in a timeout; unwrap `{"ok": true, "result": {...}}`
    /// envelope. Returns `None` on timeout or any error — matches Python
    /// `_safe_call` returning `None` on `(asyncio.TimeoutError, Exception)`.
    async fn _safe_call(&self, mcp: &Arc<dyn McpCallable>, tool: &str, args: Value) -> Option<Value> {
        let timeout = Duration::from_secs_f64(self.per_call_timeout_s);
        let fut = mcp.call(tool, args);
        match tokio::time::timeout(timeout, fut).await {
            Ok(Ok(raw)) => {
                // Unwrap the FastMCP/harness envelope: {"ok": true, "result": {...}}
                if let Some(result) = raw.get("result") {
                    Some(result.clone())
                } else {
                    Some(raw)
                }
            }
            Ok(Err(_)) => None,
            Err(_timeout) => None,
        }
    }

    /// Evaluate whether the LLM (C4) should run this tick.
    ///
    /// `sse_inputs`: optional map from SseConsumer; may contain "fresh_chat"
    ///   key with a list of memory-row Value objects.
    /// `next_wakeup_at_ms`: optional self-paced wakeup timer.
    pub async fn evaluate(
        &self,
        bot_guid: i64,
        last_state: &TickState,
        now_ms: i64,
        sse_inputs: Option<&HashMap<String, Value>>,
        next_wakeup_at_ms: Option<i64>,
    ) -> TriageResult {
        // Gate 1: first tick after enroll — fire only when NO wakeup was seeded.
        // M2 slice P seeds a jittered next_wakeup_ms at enroll; in that case we skip
        // first_tick (which carries empty hot_inputs and emits no goal) and let the
        // seeded organic_wakeup be the bot's first decision — it populates
        // state_summary so the goal IS emitted. The un-seeded path (next_wakeup_at_ms
        // == None) is byte-identical to before → parity preserved.
        if last_state.last_tick_ms == 0 && next_wakeup_at_ms.is_none() {
            return TriageResult {
                should_decide: true,
                reason: "first_tick".to_string(),
                hot_inputs: HashMap::new(),
            };
        }

        // Extract SSE-delivered fresh_chat (if any).
        let sse_fresh_chat: Vec<Value> = sse_inputs
            .and_then(|m| m.get("fresh_chat"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let has_sse_chat = !sse_fresh_chat.is_empty();

        // memory-sidecar stores created_ts as Unix seconds.
        // last_tick_ms is milliseconds — divide by 1000 (integer div = floor).
        let since_ts_s = last_state.last_tick_ms / 1000;

        // Fan-out parallel observation queries.
        // If SSE already delivered chat rows, skip the memory.search chat poll.
        let harness = &self.harness_mcp;
        let memory = &self.memory_mcp;

        let obs_state_fut = self._safe_call(
            harness,
            "obs.get_state",
            serde_json::json!({ "target_guid": bot_guid }),
        );
        let combat_fut = self._safe_call(
            harness,
            "obs.get_combat_log",
            serde_json::json!({
                "target_guid": bot_guid,
                "since_ts_ms": last_state.last_tick_ms
            }),
        );
        let chat_fut: std::pin::Pin<Box<dyn std::future::Future<Output = Option<Value>> + Send>> =
            if has_sse_chat {
                // SSE fast-path: skip chat poll — return synthetic empty result.
                Box::pin(async { Some(serde_json::json!({ "items": [] })) })
            } else {
                Box::pin(self._safe_call(
                    memory,
                    "memory.search",
                    serde_json::json!({
                        "bot_id": bot_guid.to_string(),
                        "query": "chat OR whisper",
                        "since_ts": since_ts_s,
                        "top_k": 5
                    }),
                ))
            };
        let goals_fut = self._safe_call(
            memory,
            "goals.list",
            serde_json::json!({
                "bot_id": bot_guid.to_string(),
                "status": "active"
            }),
        );

        let (obs_state, combat, chat, goals) =
            tokio::join!(obs_state_fut, combat_fut, chat_fut, goals_fut);

        // SSE fast-path: return immediately with the SSE-delivered items.
        // Include state_summary so decide.py can derive at_cap (V3.6.1 fix).
        if has_sse_chat {
            return TriageResult {
                should_decide: true,
                reason: "fresh_chat".to_string(),
                hot_inputs: {
                    let mut m = HashMap::new();
                    m.insert(
                        "fresh_chat".to_string(),
                        serde_json::json!(sse_fresh_chat),
                    );
                    m.insert(
                        "state_summary".to_string(),
                        obs_state.unwrap_or_else(|| serde_json::json!({})),
                    );
                    m
                },
            };
        }

        // Gate: PARTY_INVITE_RECEIVED — pending group invite in obs_state.
        // Field path: obs_state["self"]["pending_group_invite"].
        let pending_invite = obs_state
            .as_ref()
            .and_then(|s| s.get("self"))
            .and_then(|slf| slf.get("pending_group_invite"))
            .filter(|v| !v.is_null())
            .cloned();
        if let Some(invite) = pending_invite {
            return TriageResult {
                should_decide: true,
                reason: "party_invite_received".to_string(),
                hot_inputs: {
                    let mut m = HashMap::new();
                    m.insert("pending_invite".to_string(), invite);
                    m.insert(
                        "state_summary".to_string(),
                        obs_state.unwrap_or_else(|| serde_json::json!({})),
                    );
                    m
                },
            };
        }

        // Gate: triage_timeout — any required result is None.
        if obs_state.is_none() || combat.is_none() || chat.is_none() || goals.is_none() {
            return TriageResult {
                should_decide: false,
                reason: "triage_timeout".to_string(),
                hot_inputs: HashMap::new(),
            };
        }

        // Freshness + relevance filter on polled chat items.
        let raw_chat_items = chat
            .as_ref()
            .and_then(|c| c.get("items"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let fresh_chat: Vec<Value> = raw_chat_items
            .into_iter()
            .filter(|it| {
                it.get("ts")
                    .and_then(|v| v.as_i64())
                    .map(|ts| ts > since_ts_s)
                    .unwrap_or(false)
                    && is_player_chat(it)
            })
            .collect();
        if !fresh_chat.is_empty() {
            return TriageResult {
                should_decide: true,
                reason: "fresh_chat".to_string(),
                hot_inputs: {
                    let mut m = HashMap::new();
                    m.insert("fresh_chat".to_string(), serde_json::json!(fresh_chat));
                    m.insert(
                        "state_summary".to_string(),
                        obs_state.unwrap_or_else(|| serde_json::json!({})),
                    );
                    m
                },
            };
        }

        // Gate: combat_events.
        let combat_events = combat
            .as_ref()
            .and_then(|c| c.get("events"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if !combat_events.is_empty() {
            return TriageResult {
                should_decide: true,
                reason: "combat_event".to_string(),
                hot_inputs: {
                    let mut m = HashMap::new();
                    m.insert("combat_events".to_string(), serde_json::json!(combat_events));
                    m.insert(
                        "state_summary".to_string(),
                        obs_state.unwrap_or_else(|| serde_json::json!({})),
                    );
                    m
                },
            };
        }

        // Gate: organic_wakeup — self-paced timer expired.
        if let Some(wakeup_at) = next_wakeup_at_ms {
            if now_ms >= wakeup_at {
                return TriageResult {
                    should_decide: true,
                    reason: "organic_wakeup".to_string(),
                    hot_inputs: {
                        let mut m = HashMap::new();
                        m.insert(
                            "state_summary".to_string(),
                            obs_state.unwrap_or_else(|| serde_json::json!({})),
                        );
                        m
                    },
                };
            }
        }

        // Default: nothing changed.
        TriageResult {
            should_decide: false,
            reason: "no_change".to_string(),
            hot_inputs: HashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::TickState;
    use std::sync::Mutex;

    // -----------------------------------------------------------------------
    // MockMcp — returns a fixed JSON response; records calls.
    // -----------------------------------------------------------------------

    struct MockMcp {
        response: Value,
        calls: Mutex<Vec<String>>,
    }

    impl MockMcp {
        fn always_ok(response: Value) -> Arc<Self> {
            Arc::new(Self {
                response,
                calls: Mutex::new(Vec::new()),
            })
        }

    }

    impl McpCallable for MockMcp {
        fn call<'a>(
            &'a self,
            tool: &'a str,
            _args: Value,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Value, anyhow::Error>> + Send + 'a>,
        > {
            let response = self.response.clone();
            self.calls.lock().unwrap().push(tool.to_string());
            Box::pin(async move { Ok(response) })
        }
    }

    struct TimeoutMcp;

    impl McpCallable for TimeoutMcp {
        fn call<'a>(
            &'a self,
            _tool: &'a str,
            _args: Value,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Value, anyhow::Error>> + Send + 'a>,
        > {
            Box::pin(async move {
                Err(anyhow::anyhow!("simulated timeout / error"))
            })
        }
    }

    fn make_gate(harness: Arc<dyn McpCallable>, memory: Arc<dyn McpCallable>) -> TriageGate {
        TriageGate::new_with_mocks(harness, memory)
    }

    fn harness_no_combat_no_invite() -> Arc<dyn McpCallable> {
        // Returns the same value for both obs.get_state and obs.get_combat_log.
        // TriageGate unwraps "result" key → the inner object.
        MockMcp::always_ok(serde_json::json!({
            "result": {
                "self": { "pending_group_invite": null },
                "level": 30,
                "events": []
            }
        }))
    }

    fn memory_no_chat() -> Arc<dyn McpCallable> {
        MockMcp::always_ok(serde_json::json!({
            "result": { "items": [], "goals": [] },
        }))
    }

    // -----------------------------------------------------------------------
    // Unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_is_player_chat_accepts_whisper() {
        let item = serde_json::json!({"text": "received whisper from Alice: hello"});
        assert!(is_player_chat(&item));
    }

    #[test]
    fn test_is_player_chat_rejects_brain_no_op() {
        let item = serde_json::json!({"text": "brain_no_op: decided to wait"});
        assert!(!is_player_chat(&item));
    }

    #[test]
    fn test_is_player_chat_rejects_action_taken() {
        let item = serde_json::json!({"text": "action_taken: sent chat"});
        assert!(!is_player_chat(&item));
    }

    #[test]
    fn test_is_player_chat_rejects_unknown_text() {
        let item = serde_json::json!({"text": "some random log line"});
        assert!(!is_player_chat(&item));
    }

    #[test]
    fn test_is_player_chat_accepts_chat_received() {
        let item = serde_json::json!({"content": "chat_received from Bob: hey"});
        assert!(is_player_chat(&item));
    }

    #[test]
    fn test_chat_prefixes_count() {
        assert_eq!(CHAT_PREFIXES.len(), 6);
        assert_eq!(_BRAIN_OUTCOME_PREFIXES.len(), 6);
    }

    #[tokio::test]
    async fn test_first_tick_always_fires() {
        let gate = make_gate(harness_no_combat_no_invite(), memory_no_chat());
        let state = TickState::new(1001); // last_tick_ms = 0
        let result = gate.evaluate(1001, &state, 0, None, None).await;
        assert!(result.should_decide);
        assert_eq!(result.reason, "first_tick");
    }

    #[tokio::test]
    async fn test_triage_timeout_when_mcp_errors() {
        let timeout_mcp = Arc::new(TimeoutMcp) as Arc<dyn McpCallable>;
        // Create a gate with tiny timeout so errors are quick.
        let mut gate = TriageGate::new(timeout_mcp.clone(), timeout_mcp.clone());
        gate.per_call_timeout_s = 0.01;
        let state = TickState { bot_guid: 1001, last_tick_ms: 1000, last_decision_id: None };
        let result = gate.evaluate(1001, &state, 2000, None, None).await;
        assert!(!result.should_decide);
        assert_eq!(result.reason, "triage_timeout");
    }

    #[tokio::test]
    async fn test_fresh_chat_via_sse_inputs() {
        let gate = make_gate(harness_no_combat_no_invite(), memory_no_chat());
        let state = TickState { bot_guid: 1001, last_tick_ms: 1000, last_decision_id: None };
        let mut sse = HashMap::new();
        sse.insert(
            "fresh_chat".to_string(),
            serde_json::json!([{"text": "received whisper from Alice: hi", "memory_id": "abc"}]),
        );
        let result = gate.evaluate(1001, &state, 2000, Some(&sse), None).await;
        assert!(result.should_decide);
        assert_eq!(result.reason, "fresh_chat");
        assert!(result.hot_inputs.contains_key("fresh_chat"));
        assert!(result.hot_inputs.contains_key("state_summary"));
    }

    #[tokio::test]
    async fn test_organic_wakeup_fires_when_timer_expired() {
        let gate = make_gate(harness_no_combat_no_invite(), memory_no_chat());
        let state = TickState { bot_guid: 1001, last_tick_ms: 1000, last_decision_id: None };
        // next_wakeup_at_ms = 1500, now_ms = 2000 → wakeup expired
        let result = gate.evaluate(1001, &state, 2000, None, Some(1500)).await;
        assert!(result.should_decide);
        assert_eq!(result.reason, "organic_wakeup");
        assert!(result.hot_inputs.contains_key("state_summary"));
    }

    #[tokio::test]
    async fn test_organic_wakeup_does_not_fire_when_not_yet_expired() {
        let gate = make_gate(harness_no_combat_no_invite(), memory_no_chat());
        let state = TickState { bot_guid: 1001, last_tick_ms: 1000, last_decision_id: None };
        // now_ms (1200) < next_wakeup_at_ms (1500)
        let result = gate.evaluate(1001, &state, 1200, None, Some(1500)).await;
        assert!(!result.should_decide);
        assert_eq!(result.reason, "no_change");
    }

    #[tokio::test]
    async fn test_no_change_when_nothing_new() {
        let gate = make_gate(harness_no_combat_no_invite(), memory_no_chat());
        let state = TickState { bot_guid: 1001, last_tick_ms: 1000, last_decision_id: None };
        let result = gate.evaluate(1001, &state, 2000, None, None).await;
        assert!(!result.should_decide);
        assert_eq!(result.reason, "no_change");
    }

    #[tokio::test]
    async fn test_party_invite_fires_before_timeout() {
        // obs_state returns pending_group_invite; no timeout path.
        let harness = MockMcp::always_ok(serde_json::json!({
            "result": {
                "self": {
                    "pending_group_invite": {"from_name": "Alice", "group_type": "party"}
                },
                "level": 30,
                "events": []
            }
        }));
        let gate = make_gate(harness, memory_no_chat());
        let state = TickState { bot_guid: 1001, last_tick_ms: 1000, last_decision_id: None };
        let result = gate.evaluate(1001, &state, 2000, None, None).await;
        assert!(result.should_decide);
        assert_eq!(result.reason, "party_invite_received");
        assert!(result.hot_inputs.contains_key("pending_invite"));
    }

    #[tokio::test]
    async fn test_combat_event_fires() {
        let harness = MockMcp::always_ok(serde_json::json!({
            "result": {
                "self": { "pending_group_invite": null },
                "level": 30,
                "events": [{"type": "damage", "amount": 50}]
            }
        }));
        let gate = make_gate(harness, memory_no_chat());
        let state = TickState { bot_guid: 1001, last_tick_ms: 1000, last_decision_id: None };
        let result = gate.evaluate(1001, &state, 2000, None, None).await;
        assert!(result.should_decide);
        assert_eq!(result.reason, "combat_event");
        assert!(result.hot_inputs.contains_key("combat_events"));
    }

    #[tokio::test]
    async fn test_polled_fresh_chat_fires() {
        // Chat items returned by memory.search that pass is_player_chat and ts filter.
        let harness = harness_no_combat_no_invite();
        let since_ts_s: i64 = 1; // last_tick_ms=1000 / 1000 = 1
        let memory = MockMcp::always_ok(serde_json::json!({
            "result": {
                "items": [
                    {"text": "received whisper from Bob: yo", "ts": since_ts_s + 1}
                ],
                "goals": []
            }
        }));
        let gate = make_gate(harness, memory);
        let state = TickState { bot_guid: 1001, last_tick_ms: 1000, last_decision_id: None };
        let result = gate.evaluate(1001, &state, 5000, None, None).await;
        assert!(result.should_decide);
        assert_eq!(result.reason, "fresh_chat");
    }

    #[tokio::test]
    async fn test_polled_chat_filtered_by_brain_outcome_prefix() {
        // Brain-outcome rows must NOT trigger fresh_chat.
        let harness = harness_no_combat_no_invite();
        let since_ts_s: i64 = 1;
        let memory = MockMcp::always_ok(serde_json::json!({
            "result": {
                "items": [
                    {"text": "brain_no_op: decided to idle", "ts": since_ts_s + 1}
                ],
                "goals": []
            }
        }));
        let gate = make_gate(harness, memory);
        let state = TickState { bot_guid: 1001, last_tick_ms: 1000, last_decision_id: None };
        let result = gate.evaluate(1001, &state, 5000, None, None).await;
        assert!(!result.should_decide);
        assert_eq!(result.reason, "no_change");
    }

    /// When a bot is enrolled with a SEEDED (un-expired) next_wakeup_ms, the first
    /// tick must NOT fire first_tick — it waits for the seeded organic_wakeup.
    #[tokio::test]
    async fn test_first_tick_suppressed_when_wakeup_seeded_unexpired() {
        let gate = make_gate(harness_no_combat_no_invite(), memory_no_chat());
        let state = TickState::new(1001); // last_tick_ms = 0
        // seeded wakeup in the future (now=1000 < wakeup=9000) → first_tick suppressed,
        // organic_wakeup not yet due → no decision this tick.
        let result = gate.evaluate(1001, &state, 1000, None, Some(9000)).await;
        assert!(!result.should_decide, "seeded+unexpired first tick must not decide");
        assert_ne!(result.reason, "first_tick", "first_tick must be suppressed when seeded");
    }

    /// When the seeded wakeup has expired, the bot's FIRST decision is organic_wakeup
    /// (which populates state_summary so the goal can be emitted).
    #[tokio::test]
    async fn test_first_decision_is_organic_when_seed_expired() {
        let gate = make_gate(harness_no_combat_no_invite(), memory_no_chat());
        let state = TickState::new(1001); // last_tick_ms = 0
        // now=9000 >= seeded wakeup=8000 → organic_wakeup fires on the first tick.
        let result = gate.evaluate(1001, &state, 9000, None, Some(8000)).await;
        assert!(result.should_decide);
        assert_eq!(result.reason, "organic_wakeup");
        assert!(result.hot_inputs.contains_key("state_summary"));
    }
}
