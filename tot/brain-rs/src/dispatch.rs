// SPDX-License-Identifier: AGPL-3.0
/// C5: tool dispatcher — risk gate + cross-bot guard + MCP call + outcome memory.
///
/// Faithful Rust port of brain_sidecar/dispatch.py.
/// Gate order (must not change):
///   1. NO_OP → write brain_no_op memory → return "no_op"
///   2. Policy gate (bot.* tools only, if tool_policy present)
///   3. Unknown-tool guard (BEFORE cross-bot) → "invalid_tool"
///   4. Cross-bot guard → "blocked_cross_bot"
///   5. Risk gate → if below threshold: "confirmation_emitted"
///   6. Execute → "executed" or "tool_call_failed"
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use once_cell::sync::Lazy;
use tracing::warn;

use crate::decide::KNOWN_TOOLS;
use crate::models::{Decision, DecisionKind};
use crate::personality::McpCallable;
use crate::tool_policy::ToolPolicyEnforcer;

// ---------------------------------------------------------------------------
// RISK_TABLE — verbatim from dispatch.py (last write wins for duplicate keys)
// ---------------------------------------------------------------------------

/// Risk classification for tools. Anything not in this map defaults to "high".
/// low  = executable at confidence >= LOW_RISK_THRESHOLD (0.7)
/// high = executable at confidence >= HIGH_RISK_THRESHOLD (0.95), else confirmation
pub static RISK_TABLE: Lazy<HashMap<&'static str, &'static str>> = Lazy::new(|| {
    // Python dict with duplicate keys: last entry wins. We preserve that.
    let mut m = HashMap::new();
    // bot.* actions
    m.insert("bot.set_strategy", "low");
    m.insert("bot.get_strategies", "low");
    m.insert("bot.send_chat", "low");
    m.insert("bot.follow", "low");
    m.insert("bot.stop", "low");
    m.insert("bot.set_goal", "low");
    // memory.* (write/update/delete)
    m.insert("memory.write", "low");
    m.insert("memory_write", "low");
    m.insert("memory.goals.create", "low");
    m.insert("goals.create", "low");   // dot-notation form — was missing; defaulted to "high"
    m.insert("goals_create", "low");
    m.insert("memory.goals.update", "low");
    m.insert("goals_update", "low");
    m.insert("memory.update", "low");
    m.insert("memory_update", "low");
    m.insert("memory.delete", "high");
    m.insert("memory_delete", "high");
    m.insert("memory.goals.complete", "high");
    m.insert("goals_complete", "high");
    // obs.* reads — inherently safe, low
    m.insert("obs.get_state", "low");
    m.insert("obs.get_inventory", "low");
    m.insert("obs.get_money", "low");
    m.insert("obs.get_position", "low");
    m.insert("obs.get_combat_log", "low");
    m.insert("obs.get_quest_log", "low");
    m.insert("obs.get_rpg_status", "low");
    m.insert("obs.get_talents", "low");
    m.insert("obs.get_xp", "low");
    m.insert("obs.get_auras", "low");
    m.insert("obs.get_group", "low");
    m.insert("obs.ping", "low");
    m.insert("obs.query_db", "low");
    // V1.5 grouping/dungeon tools
    m.insert("bot.invite_to_group", "high");
    m.insert("bot.accept_invite", "high");
    m.insert("bot.leave_group", "high");
    m.insert("bot.set_role", "low");
    m.insert("bot.queue_for_dungeon", "high");
    m.insert("bot.enter_instance", "high");
    // Memory reads — low
    m.insert("memory.read", "low");
    m.insert("memory_read", "low");
    m.insert("memory.recall", "low");
    m.insert("memory_recall", "low");
    m.insert("memory.recall_about", "low");
    m.insert("memory_recall_about", "low");
    m.insert("memory.search", "low");
    m.insert("memory_search", "low");
    m.insert("memory.list", "low");
    m.insert("memory_list", "low");
    m.insert("memory.personality.get", "low");
    m.insert("memory_personality_get", "low");
    m.insert("memory.goals.read", "low");
    m.insert("goals_read", "low");
    m.insert("memory.goals.list", "low");
    m.insert("goals_list", "low");
    // Personality SET stays high
    m.insert("memory.personality.set", "high");
    m.insert("memory_personality_set", "high");
    // Duplicate of memory.personality.get (Python has it twice — last write wins = "low")
    m.insert("memory.personality.get", "low");
    m.insert("memory_personality_get", "low");
    m.insert("memory.personality_get", "low");
    // goals.read / goals.update (dot-notation forms)
    m.insert("goals.read", "low");
    m.insert("goals_read", "low");
    m.insert("goals.update", "low");
    m.insert("goals_update", "low");
    m
});

/// Confidence threshold for "low"-risk tools.
pub const LOW_RISK_THRESHOLD: f64 = 0.7;
/// Confidence threshold for "high"-risk tools.
pub const HIGH_RISK_THRESHOLD: f64 = 0.95;

// ---------------------------------------------------------------------------
// _BOT_OWN_KEYS — keys whose values must match the dispatching bot_guid
// ---------------------------------------------------------------------------

/// Keys in tool args that MUST equal the dispatching bot_guid.
/// Only these two — target_guid, player_guid, etc. are intentionally excluded.
pub static _BOT_OWN_KEYS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    let mut s = HashSet::new();
    s.insert("bot_guid");
    s.insert("bot_id");
    s
});

// ---------------------------------------------------------------------------
// _TOOL_DISPLAY_NAMES — human-readable action descriptions for confirmation chat
// ---------------------------------------------------------------------------

static TOOL_DISPLAY_NAMES: Lazy<HashMap<&'static str, &'static str>> = Lazy::new(|| {
    let mut m = HashMap::new();
    m.insert("bot.set_strategy", "change my strategy");
    m.insert("bot.get_strategies", "check my strategies");
    m.insert("bot.send_chat", "send a chat message");
    m.insert("bot.follow", "follow you");
    m.insert("bot.stop", "stop what I'm doing");
    m.insert("bot.set_goal", "set a new goal");
    m.insert("memory.write", "remember that");
    m.insert("memory_write", "remember that");
    m.insert("memory.delete", "forget that");
    m.insert("memory_delete", "forget that");
    m.insert("memory.update", "update that memory");
    m.insert("memory_update", "update that memory");
    m.insert("memory.goals.create", "set that as a goal");
    m.insert("goals.create", "set that as a goal");  // dot-notation form — was missing
    m.insert("goals_create", "set that as a goal");
    m.insert("memory.goals.update", "update that goal");
    m.insert("goals_update", "update that goal");
    m.insert("memory.goals.complete", "mark that goal complete");
    m.insert("goals_complete", "mark that goal complete");
    // V1.5 grouping/dungeon tools
    m.insert("bot.invite_to_group", "invite someone to group");
    m.insert("bot.accept_invite", "accept the group invite");
    m.insert("bot.leave_group", "leave the group");
    m.insert("bot.set_role", "set my group role");
    m.insert("bot.queue_for_dungeon", "queue for a dungeon");
    m.insert("bot.enter_instance", "enter the dungeon");
    m
});

// ---------------------------------------------------------------------------
// _OUTCOME_SALIENCE — memory salience grades per outcome type
// ---------------------------------------------------------------------------

/// Salience grades for outcome memory types.
/// Higher = more important to retain.
pub static _OUTCOME_SALIENCE: Lazy<HashMap<&'static str, f64>> = Lazy::new(|| {
    let mut m = HashMap::new();
    m.insert("action_taken", 0.5f64);
    m.insert("brain_no_op", 0.2f64);
    m.insert("tool_call_failed", 0.8f64);
    m.insert("pending_confirmation", 0.6f64);
    m.insert("blocked_cross_bot", 0.7f64);
    m.insert("decision_invalid_tool", 0.6f64);
    m
});

// ---------------------------------------------------------------------------
// mcp_for_tool — route tool name to "harness" or "memory"
// ---------------------------------------------------------------------------

/// Return "harness" if the tool belongs to the harness MCP, else "memory".
/// Mirrors Python: startswith(("bot.", "obs.", "gm.", "bot_", "obs_", "gm_"))
pub fn mcp_for_tool(tool: &str) -> &'static str {
    if tool.starts_with("bot.")
        || tool.starts_with("obs.")
        || tool.starts_with("gm.")
        || tool.starts_with("bot_")
        || tool.starts_with("obs_")
        || tool.starts_with("gm_")
    {
        "harness"
    } else {
        "memory"
    }
}

/// Return the human-readable display name for a tool, or the tool name itself.
pub fn display_name(tool: &str) -> &str {
    TOOL_DISPLAY_NAMES.get(tool).copied().unwrap_or(tool)
}

// ---------------------------------------------------------------------------
// DispatchResult
// ---------------------------------------------------------------------------

/// Outcome of a dispatch() call.
/// disposition ∈ {"no_op", "policy_denied", "invalid_tool", "blocked_cross_bot",
///                "confirmation_emitted", "executed", "tool_call_failed"}
#[derive(Debug, Clone)]
pub struct DispatchResult {
    pub disposition: String,
    pub tool: Option<String>,
    pub result: Option<serde_json::Value>,
    pub error: String,
}

// ---------------------------------------------------------------------------
// Dispatcher
// ---------------------------------------------------------------------------

/// Stateless dispatcher. Owns Arc references to harness and memory MCP clients.
pub struct Dispatcher {
    pub harness_mcp: Arc<dyn McpCallable>,
    pub memory_mcp: Arc<dyn McpCallable>,
    pub confirmation_channel: String,
    pub tool_policy: Option<ToolPolicyEnforcer>,
}

impl Dispatcher {
    pub fn new(
        harness_mcp: Arc<dyn McpCallable>,
        memory_mcp: Arc<dyn McpCallable>,
        confirmation_channel: &str,
        tool_policy: Option<ToolPolicyEnforcer>,
    ) -> Self {
        Self {
            harness_mcp,
            memory_mcp,
            confirmation_channel: confirmation_channel.to_string(),
            tool_policy,
        }
    }

    /// Returns violations: list of (key, value) where bot-ownership arg mismatches bot_guid.
    fn cross_bot_violations(
        &self,
        args: &serde_json::Value,
        bot_guid: i64,
    ) -> Vec<(String, serde_json::Value)> {
        let mut bad = Vec::new();
        let obj = match args.as_object() {
            Some(o) => o,
            None => return bad,
        };
        for key in _BOT_OWN_KEYS.iter() {
            let Some(val) = obj.get(*key) else {
                continue;
            };
            // Try to parse as integer for comparison.
            let parsed = if let Some(n) = val.as_i64() {
                Some(n)
            } else if let Some(s) = val.as_str() {
                s.parse::<i64>().ok()
            } else {
                None
            };
            match parsed {
                Some(n) if n == bot_guid => {} // matches — OK
                Some(_) => bad.push((key.to_string(), val.clone())), // mismatch
                None => bad.push((key.to_string(), val.clone())),    // garbage value
            }
        }
        bad
    }

    /// Main dispatch entry point. Follows exact Python gate order.
    pub async fn dispatch(
        &self,
        bot_guid: i64,
        decision: &Decision,
        tier: &str,
    ) -> Result<DispatchResult, anyhow::Error> {
        let ts_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        // ── Gate 1: NO_OP ────────────────────────────────────────────────────
        if decision.kind == DecisionKind::NoOp {
            self.write_outcome_memory(
                bot_guid,
                ts_ms,
                "brain_no_op",
                serde_json::json!({ "reasoning": decision.reasoning }),
            )
            .await;
            return Ok(DispatchResult {
                disposition: "no_op".to_string(),
                tool: None,
                result: None,
                error: String::new(),
            });
        }

        let tool = decision.tool.as_deref().unwrap_or("").to_string();
        let args = decision.args.clone().unwrap_or(serde_json::json!({}));

        // ── Gate 2: Policy gate (bot.* only) ─────────────────────────────────
        if let Some(ref policy) = self.tool_policy {
            if tool.starts_with("bot.") && !policy.is_allowed(&tool, tier) {
                tracing::info!(
                    "policy_denied tool={} tier={} bot_guid={}",
                    tool,
                    tier,
                    bot_guid
                );
                return Ok(DispatchResult {
                    disposition: "policy_denied".to_string(),
                    tool: Some(tool.clone()),
                    result: None,
                    error: format!("tier={} does not permit {}", tier, tool),
                });
            }
        }

        // ── Gate 3: Unknown-tool guard (BEFORE cross-bot) ────────────────────
        let known: HashSet<&str> = KNOWN_TOOLS.iter().copied().collect();
        if !known.contains(tool.as_str()) {
            self.write_outcome_memory(
                bot_guid,
                ts_ms,
                "decision_invalid_tool",
                serde_json::json!({
                    "tool": tool,
                    "args": args,
                    "reasoning": decision.reasoning
                }),
            )
            .await;
            return Ok(DispatchResult {
                disposition: "invalid_tool".to_string(),
                tool: Some(tool.clone()),
                result: None,
                error: format!("unknown_tool: {}", tool),
            });
        }

        // ── Gate 4: Cross-bot guard ───────────────────────────────────────────
        let violations = self.cross_bot_violations(&args, bot_guid);
        if !violations.is_empty() {
            let viol_json: Vec<serde_json::Value> = violations
                .iter()
                .map(|(k, v)| serde_json::json!([k, v]))
                .collect();
            self.write_outcome_memory(
                bot_guid,
                ts_ms,
                "blocked_cross_bot",
                serde_json::json!({
                    "tool": tool,
                    "args": args,
                    "reasoning": decision.reasoning,
                    "violations": viol_json
                }),
            )
            .await;
            return Ok(DispatchResult {
                disposition: "blocked_cross_bot".to_string(),
                tool: Some(tool.clone()),
                result: None,
                error: format!("cross-bot args: {:?}", violations),
            });
        }

        // ── Gate 5: Risk gate ─────────────────────────────────────────────────
        let risk = RISK_TABLE.get(tool.as_str()).copied().unwrap_or("high");
        let confidence_ok = match risk {
            "high" => decision.confidence >= HIGH_RISK_THRESHOLD,
            "low" => decision.confidence >= LOW_RISK_THRESHOLD,
            _ => decision.confidence >= HIGH_RISK_THRESHOLD, // unknown risk class = high
        };
        if !confidence_ok {
            self.emit_confirmation(bot_guid, decision, ts_ms).await;
            return Ok(DispatchResult {
                disposition: "confirmation_emitted".to_string(),
                tool: Some(tool.clone()),
                result: None,
                error: String::new(),
            });
        }

        // ── Gate 6: Execute ───────────────────────────────────────────────────
        let mcp: &Arc<dyn McpCallable> = if mcp_for_tool(&tool) == "harness" {
            &self.harness_mcp
        } else {
            &self.memory_mcp
        };

        match mcp.call(&tool, args.clone()).await {
            Ok(result) => {
                self.write_outcome_memory(
                    bot_guid,
                    ts_ms,
                    "action_taken",
                    serde_json::json!({
                        "tool": tool,
                        "args": args,
                        "result": result,
                        "reasoning": decision.reasoning
                    }),
                )
                .await;
                Ok(DispatchResult {
                    disposition: "executed".to_string(),
                    tool: Some(tool),
                    result: Some(result),
                    error: String::new(),
                })
            }
            Err(e) => {
                let err_str = e.to_string();
                self.write_outcome_memory(
                    bot_guid,
                    ts_ms,
                    "tool_call_failed",
                    serde_json::json!({
                        "tool": tool,
                        "args": args,
                        "error": err_str
                    }),
                )
                .await;
                Ok(DispatchResult {
                    disposition: "tool_call_failed".to_string(),
                    tool: Some(tool),
                    result: None,
                    error: err_str,
                })
            }
        }
    }

    /// Emit a confirmation chat message and write pending_confirmation memory.
    async fn emit_confirmation(&self, bot_guid: i64, decision: &Decision, ts_ms: i64) {
        let tool_name = decision.tool.as_deref().unwrap_or("");
        let disp = display_name(tool_name);
        // Python: f"want me to {display}? ({decision.reasoning})" if reasoning else f"want me to {display}?"
        let chat_text = if decision.reasoning.is_empty() {
            format!("want me to {}?", disp)
        } else {
            format!("want me to {}? ({})", disp, decision.reasoning)
        };
        // Truncate to 200 chars
        let chat_text = truncate_to_200(&chat_text);

        let send_result = self
            .harness_mcp
            .call(
                "bot.send_chat",
                serde_json::json!({
                    "bot_guid": bot_guid,
                    "channel": self.confirmation_channel,
                    "message": chat_text
                }),
            )
            .await;
        if let Err(e) = send_result {
            warn!(
                "confirmation_chat_failed bot={} tool={} err={}",
                bot_guid, tool_name, e
            );
        }

        self.write_outcome_memory(
            bot_guid,
            ts_ms,
            "pending_confirmation",
            serde_json::json!({
                "tool": decision.tool,
                "args": decision.args,
                "confidence": decision.confidence,
                "reasoning": decision.reasoning,
                "confirms_by_ts_ms": ts_ms + 60_000
            }),
        )
        .await;
    }

    /// Write an outcome memory via memory.write.
    /// Mirrors Python _write_outcome_memory exactly.
    async fn write_outcome_memory(
        &self,
        bot_guid: i64,
        _ts_ms: i64,
        memory_type: &str,
        payload: serde_json::Value,
    ) {
        let tool = payload.get("tool").and_then(|v| v.as_str()).unwrap_or("");
        let reasoning = payload
            .get("reasoning")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let error = payload.get("error").and_then(|v| v.as_str()).unwrap_or("");

        // Build text — mirrors Python exactly
        let text = if !tool.is_empty() {
            let mut t = format!("{}: {}", memory_type, tool);
            if !reasoning.is_empty() {
                t.push_str(&format!(" \u{2014} {}", reasoning)); // em-dash
            }
            if !error.is_empty() {
                t.push_str(&format!(" (error: {})", error));
            }
            t
        } else {
            format!("{}: {}", memory_type, if reasoning.is_empty() {
                // payload.to_string() equivalent
                payload.to_string()
            } else {
                reasoning.to_string()
            })
        };
        let text = truncate_to_200(&text);

        // Entities: bot_guid always; tool name appended if present
        let mut entities: Vec<serde_json::Value> = vec![serde_json::Value::String(bot_guid.to_string())];
        if !tool.is_empty() {
            entities.push(serde_json::Value::String(tool.to_string()));
        }

        let salience = *_OUTCOME_SALIENCE.get(memory_type).unwrap_or(&0.5);

        let write_result = self
            .memory_mcp
            .call(
                "memory.write",
                serde_json::json!({
                    "bot_id": bot_guid.to_string(),
                    "text": text,
                    "salience": salience,
                    "entities": entities,
                    "relations": [],
                    "memory_type": memory_type,
                    "metadata": payload
                }),
            )
            .await;

        if let Err(e) = write_result {
            tracing::error!(
                "memory_write failed for outcome bot={} type={} err={}",
                bot_guid,
                memory_type,
                e
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Helper: truncate to 200 chars on a char boundary
// ---------------------------------------------------------------------------

/// Truncate a string to at most 200 Unicode chars (mirrors Python `[:200]`).
fn truncate_to_200(s: &str) -> String {
    s.chars().take(200).collect()
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_routing_bot_prefix() {
        assert_eq!(mcp_for_tool("bot.send_chat"), "harness");
        assert_eq!(mcp_for_tool("obs.get_state"), "harness");
        assert_eq!(mcp_for_tool("gm.teleport"), "harness");
        assert_eq!(mcp_for_tool("bot_guid_something"), "harness");
        assert_eq!(mcp_for_tool("obs_get_state"), "harness");
        assert_eq!(mcp_for_tool("gm_something"), "harness");
    }

    #[test]
    fn test_mcp_routing_memory_prefix() {
        assert_eq!(mcp_for_tool("memory.write"), "memory");
        assert_eq!(mcp_for_tool("goals.create"), "memory");
        assert_eq!(mcp_for_tool("memory.recall_about"), "memory");
    }

    #[test]
    fn test_risk_table_key_values() {
        assert_eq!(*RISK_TABLE.get("bot.send_chat").unwrap(), "low");
        assert_eq!(*RISK_TABLE.get("bot.invite_to_group").unwrap(), "high");
        assert_eq!(*RISK_TABLE.get("obs.get_state").unwrap(), "low");
        assert_eq!(*RISK_TABLE.get("memory.delete").unwrap(), "high");
        assert_eq!(*RISK_TABLE.get("memory.personality.set").unwrap(), "high");
        assert_eq!(*RISK_TABLE.get("memory.personality.get").unwrap(), "low");
        // Unknown defaults to "high" — not in map
        assert!(RISK_TABLE.get("unknown_tool").is_none());
    }

    #[test]
    fn test_thresholds() {
        assert_eq!(LOW_RISK_THRESHOLD, 0.7f64);
        assert_eq!(HIGH_RISK_THRESHOLD, 0.95f64);
    }

    #[test]
    fn test_bot_own_keys() {
        assert!(_BOT_OWN_KEYS.contains("bot_guid"));
        assert!(_BOT_OWN_KEYS.contains("bot_id"));
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
        assert_eq!(*_OUTCOME_SALIENCE.get("decision_invalid_tool").unwrap(), 0.6f64);
    }

    #[test]
    fn test_display_name_known() {
        assert_eq!(display_name("bot.invite_to_group"), "invite someone to group");
        assert_eq!(display_name("bot.send_chat"), "send a chat message");
        assert_eq!(display_name("bot.follow"), "follow you");
    }

    #[test]
    fn test_display_name_unknown_falls_back_to_tool_name() {
        assert_eq!(display_name("some.unknown.tool"), "some.unknown.tool");
    }

    #[test]
    fn test_truncate_to_200() {
        let s: String = "x".repeat(300);
        let truncated = truncate_to_200(&s);
        assert_eq!(truncated.len(), 200);
    }
}
