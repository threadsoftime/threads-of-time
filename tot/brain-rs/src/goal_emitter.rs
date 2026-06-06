// SPDX-License-Identifier: AGPL-3.0
//! Deterministic goal synthesis for brain-rs.
//!
//! A pure function that maps the current `state_summary` JSON (from the harness
//! `obs.get_state` response) to a `GoalEnvelope`. No I/O, no side effects —
//! trivially testable and safe to call from anywhere in the tick path.
//!
//! # Elwynn Grind anchor
//!
//! The Fargodeep / Jasperlode mine area in Elwynn Forest (map 0), a good
//! Alliance Warrior starting grind zone for levels 4–12.

use serde_json::Value;
use tot_goal_contract::{
    Goal, GoalEnvelope, GrindGoal, MobFilter, WorldPos, GOAL_CONTRACT_VERSION,
};

/// Elwynn Forest grind anchor — Fargodeep / Jasperlode mine area.
/// These coordinates place the anchor near the Kobold camps at the east side
/// of Elwynn (map 0, Eastern Kingdoms).
const ELWYNN_ANCHOR: WorldPos = WorldPos {
    map_id: 0,
    x: -9384.5,
    y: 63.5,
    z: 54.0,
};

/// Synthesise a `Grind` goal for `bot_guid` based on the current observation.
///
/// Returns `Some(GoalEnvelope)` when `state_summary["self"]["level"] < max_level`,
/// and `None` when the bot is at or above cap (or when `level` is missing/invalid).
///
/// # Goal fields
/// * `anchor_point`: `ELWYNN_ANCHOR` (Fargodeep / Jasperlode mine area)
/// * `to_level`: `current_level + 1`
/// * `mob_filter`: `{min_level: level-1, max_level: level+2, creature_type: "humanoid"}`
/// * `wander_radius`: 90.0
/// * `max_search_radius`: 35.0
/// * `rest_threshold`: 0.35
/// * `kill_count`: `None`
/// * `goal_id`: `format!("grind-{bot_guid}-{level}")` — stable per (bot, level)
pub fn synthesize_grind(
    bot_guid: i64,
    state_summary: &Value,
    max_level: u32,
) -> Option<GoalEnvelope> {
    // Navigate state_summary["self"]["level"] — same path decide()'s at_cap uses.
    let level = state_summary
        .get("self")
        .and_then(|s| s.get("level"))
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)?;

    if level >= max_level {
        return None;
    }

    let min_level = level.saturating_sub(1);
    let max_level_filter = level + 2;

    Some(GoalEnvelope {
        goal_id: format!("grind-{bot_guid}-{level}"),
        version: GOAL_CONTRACT_VERSION,
        goal: Goal::Grind(GrindGoal {
            anchor_point: ELWYNN_ANCHOR,
            wander_radius: 90.0,
            max_search_radius: 35.0,
            mob_filter: MobFilter {
                min_level,
                max_level: max_level_filter,
                creature_type: Some("humanoid".into()),
            },
            to_level: level + 1,
            kill_count: None,
            rest_threshold: 0.35,
        }),
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn state_summary(level: u64) -> Value {
        json!({ "self": { "level": level, "hp_pct": 100 } })
    }

    // ── below cap → Some ────────────────────────────────────────────────────

    #[test]
    fn below_cap_returns_some() {
        let env = synthesize_grind(1003, &state_summary(6), 25);
        assert!(env.is_some(), "expected Some for level 6 with cap 25");
    }

    #[test]
    fn below_cap_goal_is_grind_variant() {
        let env = synthesize_grind(1003, &state_summary(6), 25).unwrap();
        assert!(matches!(env.goal, Goal::Grind(_)));
    }

    #[test]
    fn below_cap_to_level_is_level_plus_one() {
        let env = synthesize_grind(1003, &state_summary(6), 25).unwrap();
        let Goal::Grind(g) = env.goal else { panic!("not grind") };
        assert_eq!(g.to_level, 7, "to_level must be current+1");
    }

    #[test]
    fn below_cap_mob_filter_correct() {
        let env = synthesize_grind(1003, &state_summary(6), 25).unwrap();
        let Goal::Grind(g) = env.goal else { panic!("not grind") };
        assert_eq!(g.mob_filter.min_level, 5, "min = level-1");
        assert_eq!(g.mob_filter.max_level, 8, "max = level+2");
        assert_eq!(
            g.mob_filter.creature_type.as_deref(),
            Some("humanoid"),
            "creature_type must be humanoid"
        );
    }

    #[test]
    fn below_cap_wander_and_search_radius() {
        let env = synthesize_grind(1003, &state_summary(6), 25).unwrap();
        let Goal::Grind(g) = env.goal else { panic!("not grind") };
        assert_eq!(g.wander_radius, 90.0);
        assert_eq!(g.max_search_radius, 35.0);
    }

    #[test]
    fn below_cap_rest_threshold_and_kill_count() {
        let env = synthesize_grind(1003, &state_summary(6), 25).unwrap();
        let Goal::Grind(g) = env.goal else { panic!("not grind") };
        assert!((g.rest_threshold - 0.35).abs() < f32::EPSILON);
        assert!(g.kill_count.is_none());
    }

    // ── at cap → None ───────────────────────────────────────────────────────

    #[test]
    fn at_cap_returns_none() {
        let env = synthesize_grind(1003, &state_summary(25), 25);
        assert!(env.is_none(), "expected None when level == max_level");
    }

    #[test]
    fn above_cap_returns_none() {
        let env = synthesize_grind(1003, &state_summary(26), 25);
        assert!(env.is_none(), "expected None when level > max_level");
    }

    // ── goal_id stability ───────────────────────────────────────────────────

    #[test]
    fn goal_id_stable_per_bot_and_level() {
        let e1 = synthesize_grind(1003, &state_summary(6), 25).unwrap();
        let e2 = synthesize_grind(1003, &state_summary(6), 25).unwrap();
        assert_eq!(e1.goal_id, e2.goal_id, "same (bot,level) → same goal_id");
    }

    #[test]
    fn goal_id_different_for_different_levels() {
        let e6 = synthesize_grind(1003, &state_summary(6), 25).unwrap();
        let e7 = synthesize_grind(1003, &state_summary(7), 25).unwrap();
        assert_ne!(e6.goal_id, e7.goal_id);
    }

    #[test]
    fn goal_id_different_for_different_bots() {
        let e_a = synthesize_grind(1003, &state_summary(6), 25).unwrap();
        let e_b = synthesize_grind(1004, &state_summary(6), 25).unwrap();
        assert_ne!(e_a.goal_id, e_b.goal_id);
    }

    #[test]
    fn goal_id_format_contains_bot_and_level() {
        let env = synthesize_grind(1003, &state_summary(8), 25).unwrap();
        assert!(
            env.goal_id.contains("1003") && env.goal_id.contains("8"),
            "goal_id '{}' should encode bot_guid and level",
            env.goal_id
        );
    }

    // ── level 1 (saturating_sub safety) ────────────────────────────────────

    #[test]
    fn level_one_does_not_underflow_min_level() {
        let env = synthesize_grind(1003, &state_summary(1), 25).unwrap();
        let Goal::Grind(g) = env.goal else { panic!("not grind") };
        // saturating_sub(1) on u32 level=1 → 0
        assert_eq!(g.mob_filter.min_level, 0, "saturating_sub must not underflow");
    }

    // ── missing / invalid level ─────────────────────────────────────────────

    #[test]
    fn missing_level_returns_none() {
        let env = synthesize_grind(1003, &json!({ "self": {} }), 25);
        assert!(env.is_none(), "missing level must yield None");
    }

    #[test]
    fn missing_self_key_returns_none() {
        let env = synthesize_grind(1003, &json!({}), 25);
        assert!(env.is_none(), "missing self key must yield None");
    }

    // ── version ─────────────────────────────────────────────────────────────

    #[test]
    fn envelope_version_is_current_contract() {
        let env = synthesize_grind(1003, &state_summary(6), 25).unwrap();
        assert_eq!(env.version, GOAL_CONTRACT_VERSION);
    }
}
