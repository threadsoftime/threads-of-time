//! EmissionLedger — shared goal-emission dedup + per-status re-emission cooldowns.
//!
//! Replaces the old once-per-(bot,level)-forever dedup (`emitted_goals` map in
//! LoopSupervisor) that permanently stalled bots after any terminal exec status
//! (kb_87a7eade gap #1 — the 6 h overnight stall). Shared between
//! `LoopSupervisor::maybe_emit_goal` (emit side) and the exec status drain task
//! in `app.rs` (terminal side). Design: docs spec 2026-06-10 §1 (heimdal repo).

use std::collections::HashMap;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

use tot_goal_contract::GoalStatus;

/// Per-terminal-status re-emission cooldowns (flat, user-selected in design).
#[derive(Debug, Clone)]
pub struct CooldownConfig {
    /// After `Blocked` (respawn-wait backstop, transient harness errors, bad camps).
    pub blocked: Duration,
    /// After `NeedsDecision` (post-recovery-failure pacing).
    pub needs_decision: Duration,
}

impl Default for CooldownConfig {
    fn default() -> Self {
        Self {
            blocked: Duration::from_secs(300),
            needs_decision: Duration::from_secs(600),
        }
    }
}

struct EmissionEntry {
    goal_id: String,
    /// `None` = goal in flight (emitted, no terminal status yet).
    /// `Some(t)` = terminal status received; re-emission allowed once `now >= t`.
    cooldown_until: Option<Instant>,
}

/// StdMutex discipline matches LoopSupervisor: never held across an `.await`.
pub struct EmissionLedger {
    cooldowns: CooldownConfig,
    entries: StdMutex<HashMap<i64, EmissionEntry>>,
}

impl EmissionLedger {
    pub fn new(cooldowns: CooldownConfig) -> Self {
        Self { cooldowns, entries: StdMutex::new(HashMap::new()) }
    }

    /// May `goal_id` be emitted for `bot_guid` now?
    /// A different goal_id than the recorded one (bot leveled) is ALWAYS allowed —
    /// cooldowns are per-(bot, goal_id), so a level-up bypasses a stale cooldown.
    pub fn may_emit(&self, bot_guid: i64, goal_id: &str) -> bool {
        let entries = self.entries.lock().unwrap();
        match entries.get(&bot_guid) {
            None => true,
            Some(e) if e.goal_id != goal_id => true,
            Some(e) => match e.cooldown_until {
                None => false,                  // in flight
                Some(t) => Instant::now() >= t, // cooled down?
            },
        }
    }

    /// Record an emission (goal now in flight — no re-emit until a terminal status).
    pub fn record_emit(&self, bot_guid: i64, goal_id: &str) {
        self.entries.lock().unwrap().insert(
            bot_guid,
            EmissionEntry { goal_id: goal_id.to_string(), cooldown_until: None },
        );
    }

    /// Feed a status from the exec drain. Terminal statuses stamp the per-status
    /// cooldown; `Running` is ignored. A status for an unknown bot is a no-op
    /// (e.g. a goal emitted by a previous brain lifetime).
    pub fn on_terminal(&self, bot_guid: i64, status: &GoalStatus) {
        let cooldown = match status {
            GoalStatus::Running { .. } => return,
            GoalStatus::Completed { .. } => Duration::ZERO,
            GoalStatus::Blocked { .. } => self.cooldowns.blocked,
            GoalStatus::NeedsDecision { .. } => self.cooldowns.needs_decision,
        };
        if let Some(e) = self.entries.lock().unwrap().get_mut(&bot_guid) {
            e.cooldown_until = Some(Instant::now() + cooldown);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tot_goal_contract::{BlockedReason, EscalationEvent};

    fn ledger() -> EmissionLedger {
        EmissionLedger::new(CooldownConfig::default())
    }

    fn blocked() -> GoalStatus {
        GoalStatus::Blocked { reason: BlockedReason::NoTargetsFound, detail: None }
    }

    #[test]
    fn fresh_bot_may_emit() {
        assert!(ledger().may_emit(1, "grind-1-6"));
    }

    #[test]
    fn in_flight_goal_blocks_same_id() {
        let l = ledger();
        l.record_emit(1, "grind-1-6");
        assert!(!l.may_emit(1, "grind-1-6"));
    }

    #[test]
    fn different_goal_id_bypasses_in_flight_and_cooldown() {
        let l = EmissionLedger::new(CooldownConfig {
            blocked: Duration::from_secs(3600),
            ..Default::default()
        });
        l.record_emit(1, "grind-1-6");
        assert!(l.may_emit(1, "grind-1-7"), "level-up goal_id always allowed");
        l.on_terminal(1, &blocked());
        assert!(l.may_emit(1, "grind-1-7"), "cooldown is per goal_id");
    }

    #[test]
    fn completed_allows_immediate_reemit() {
        let l = ledger();
        l.record_emit(1, "grind-1-6");
        l.on_terminal(1, &GoalStatus::Completed { summary: "x".into() });
        assert!(l.may_emit(1, "grind-1-6"));
    }

    #[test]
    fn blocked_cooldown_blocks_then_expires() {
        let l = EmissionLedger::new(CooldownConfig {
            blocked: Duration::from_millis(50),
            needs_decision: Duration::from_millis(50),
        });
        l.record_emit(1, "grind-1-6");
        l.on_terminal(1, &blocked());
        assert!(!l.may_emit(1, "grind-1-6"), "cooling down");
        std::thread::sleep(Duration::from_millis(110));
        assert!(l.may_emit(1, "grind-1-6"), "cooldown expired");
    }

    #[test]
    fn running_status_ignored() {
        let l = ledger();
        l.record_emit(1, "grind-1-6");
        l.on_terminal(1, &GoalStatus::Running { progress: None });
        assert!(!l.may_emit(1, "grind-1-6"), "Running must not unlock re-emission");
    }

    #[test]
    fn needs_decision_uses_its_own_cooldown() {
        let l = EmissionLedger::new(CooldownConfig {
            blocked: Duration::ZERO,
            needs_decision: Duration::from_secs(3600),
        });
        l.record_emit(1, "grind-1-6");
        l.on_terminal(1, &GoalStatus::NeedsDecision {
            event: EscalationEvent::BotDied { position: None },
        });
        assert!(!l.may_emit(1, "grind-1-6"));
    }

    #[test]
    fn unknown_bot_terminal_is_noop() {
        let l = ledger();
        l.on_terminal(99, &GoalStatus::Completed { summary: "x".into() });
        assert!(l.may_emit(99, "grind-99-6"));
    }
}
