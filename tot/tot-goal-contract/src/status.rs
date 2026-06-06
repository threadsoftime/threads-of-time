//! The result the executor reports back to the brain.

use serde::{Deserialize, Serialize};

use crate::goal::WorldPos;

/// Progress hint carried by `GoalStatus::Running` (for logging/observability).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoalProgress {
    pub kills_this_goal: u32,
    pub level_at_start: u32,
    pub current_level: u32,
}

/// Why the executor cannot make progress (but may retry later).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockedReason {
    NoTargetsFound,
    RepathExhausted,
    UnknownGoalVariant,
    VersionTooNew,
    Other,
}

/// An out-of-cadence interrupt that requires a fresh brain decision.
/// M1 raises `BotDied`/`PathStuck`; `PlayerSpoke` is the M2 hook.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum EscalationEvent {
    BotDied { position: Option<WorldPos> },
    PathStuck { repath_attempts: u32, last_position: Option<WorldPos> },
    PlayerSpoke { player_guid: u64, message: String },
}

/// The executor → brain status, emitted on each phase transition or escalation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum GoalStatus {
    Running { progress: Option<GoalProgress> },
    Completed { summary: String },
    Blocked { reason: BlockedReason, detail: Option<String> },
    NeedsDecision { event: EscalationEvent },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goal::WorldPos;

    #[test]
    fn status_variants_round_trip() {
        let cases = vec![
            GoalStatus::Running { progress: Some(GoalProgress {
                kills_this_goal: 3, level_at_start: 5, current_level: 5 }) },
            GoalStatus::Completed { summary: "reached level 6".into() },
            GoalStatus::Blocked { reason: BlockedReason::NoTargetsFound, detail: None },
            GoalStatus::NeedsDecision { event: EscalationEvent::BotDied {
                position: Some(WorldPos { map_id: 0, x: 1.0, y: 2.0, z: 3.0 }) } },
        ];
        for s in cases {
            let json = serde_json::to_string(&s).unwrap();
            let back: GoalStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(s, back, "round-trip mismatch for {json}");
        }
    }

    #[test]
    fn status_is_internally_tagged_on_status() {
        let json = serde_json::to_string(&GoalStatus::Completed { summary: "x".into() }).unwrap();
        assert!(json.contains(r#""status":"completed""#), "tag missing: {json}");
    }

    #[test]
    fn escalation_is_internally_tagged_on_kind() {
        let json = serde_json::to_string(&EscalationEvent::PathStuck {
            repath_attempts: 5, last_position: None }).unwrap();
        assert!(json.contains(r#""kind":"path_stuck""#), "tag missing: {json}");
    }
}
