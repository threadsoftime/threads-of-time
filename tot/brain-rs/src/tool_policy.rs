// SPDX-License-Identifier: AGPL-3.0
/// Phase B: tier-aware tool whitelist enforcement.
///
/// Faithful Rust port of brain_sidecar/tool_policy.py ToolPolicyEnforcer.
/// Parity: FULL_WHITELIST and REDUCED_WHITELIST must match Python frozensets verbatim.
use std::collections::HashSet;

use once_cell::sync::Lazy;

static FULL_WHITELIST: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    [
        "bot.send_chat",
        "bot.set_strategy",
        "bot.set_goal",
        "bot.set_role",
        "bot.invite_to_group",
        "bot.accept_invite",
        "bot.leave_group",
        "bot.follow",
        "bot.queue_for_dungeon",
        "bot.enter_instance",
        "bot.stop",
        "bot.combat_stop",
    ]
    .iter()
    .copied()
    .collect()
});

static REDUCED_WHITELIST: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    ["bot.set_strategy", "bot.set_goal", "bot.set_role"]
        .iter()
        .copied()
        .collect()
});

/// Zero-sized struct; mirrors Python's frozen dataclass pattern.
#[derive(Debug, Clone, Copy)]
pub struct ToolPolicyEnforcer;

impl ToolPolicyEnforcer {
    /// Return true if `tool_name` is allowed for the given tier.
    /// Mirrors Python: `TIER_TOOL_WHITELIST.get(tier, _FULL_WHITELIST)`.
    /// Unknown tiers fall back to FULL_WHITELIST (Python behaviour).
    pub fn is_allowed(&self, tool_name: &str, tier: &str) -> bool {
        match tier {
            "reduced" => REDUCED_WHITELIST.contains(tool_name),
            _ => FULL_WHITELIST.contains(tool_name),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_full_tier_allows_all_bot_tools() {
        let enforcer = ToolPolicyEnforcer;
        assert!(enforcer.is_allowed("bot.send_chat", "full"));
        assert!(enforcer.is_allowed("bot.invite_to_group", "full"));
        assert!(enforcer.is_allowed("bot.enter_instance", "full"));
        assert!(enforcer.is_allowed("bot.combat_stop", "full"));
    }

    #[test]
    fn test_reduced_tier_allows_only_strategy_goal_role() {
        let enforcer = ToolPolicyEnforcer;
        assert!(enforcer.is_allowed("bot.set_strategy", "reduced"));
        assert!(enforcer.is_allowed("bot.set_goal", "reduced"));
        assert!(enforcer.is_allowed("bot.set_role", "reduced"));
        // Blocked in reduced
        assert!(!enforcer.is_allowed("bot.send_chat", "reduced"));
        assert!(!enforcer.is_allowed("bot.invite_to_group", "reduced"));
        assert!(!enforcer.is_allowed("bot.follow", "reduced"));
        assert!(!enforcer.is_allowed("bot.stop", "reduced"));
        assert!(!enforcer.is_allowed("bot.combat_stop", "reduced"));
    }

    #[test]
    fn test_unknown_tier_falls_back_to_full() {
        let enforcer = ToolPolicyEnforcer;
        // Python: TIER_TOOL_WHITELIST.get(tier, _FULL_WHITELIST) — unknown → full
        assert!(enforcer.is_allowed("bot.send_chat", "unknown_tier"));
        assert!(enforcer.is_allowed("bot.invite_to_group", "unknown_tier"));
    }
}
