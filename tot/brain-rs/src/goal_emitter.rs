// SPDX-License-Identifier: AGPL-3.0
//! Deterministic goal synthesis for brain-rs.
//!
//! A pure function that maps the current `state_summary` JSON (from the harness
//! `obs.get_state` response) and a `GrindProfile` to a `GoalEnvelope`.
//! No I/O, no side effects — trivially testable and safe to call from anywhere
//! in the tick path.

use serde_json::Value;
use tot_goal_contract::{
    Goal, GoalEnvelope, GrindGoal, MobFilter, WorldPos, GOAL_CONTRACT_VERSION,
};

use crate::profile::GrindProfile;

/// Synthesise a `Grind` goal for `bot_guid` from its profile + current observation.
/// Returns `None` at/above `max_level` (or when level is missing/invalid).
///
/// # Goal fields
/// * `anchor_point`: taken directly from `profile.anchor` (explicit, stable camp position)
/// * `to_level`: `current_level + 1`
/// * `mob_filter`: `{min_level: level - profile.level_band.below, max_level: level + profile.level_band.above, creature_type: profile.creature_type}`
/// * `wander_radius`: `profile.wander_radius`
/// * `max_search_radius`: `profile.max_search_radius`
/// * `rest_threshold`: `profile.rest_threshold`
/// * `kill_count`: `None`
/// * `goal_id`: `format!("grind-{bot_guid}-{level}")` — stable per (bot, level)
pub fn synthesize_grind(
    bot_guid: i64,
    state_summary: &Value,
    max_level: u32,
    profile: &GrindProfile,
) -> Option<GoalEnvelope> {
    let level = state_summary
        .get("self")
        .and_then(|s| s.get("level"))
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)?;

    if level >= max_level {
        return None;
    }

    let min_level = level.saturating_sub(profile.level_band.below);
    let max_level_filter = level + profile.level_band.above;

    Some(GoalEnvelope {
        goal_id: format!("grind-{bot_guid}-{level}"),
        version: GOAL_CONTRACT_VERSION,
        goal: Goal::Grind(GrindGoal {
            anchor_point: WorldPos {
                map_id: profile.anchor.map_id,
                x: profile.anchor.x,
                y: profile.anchor.y,
                z: profile.anchor.z,
            },
            wander_radius: profile.wander_radius,
            max_search_radius: profile.max_search_radius,
            mob_filter: MobFilter {
                min_level,
                max_level: max_level_filter,
                creature_type: Some(profile.creature_type.clone()),
            },
            to_level: level + 1,
            kill_count: None,
            rest_threshold: profile.rest_threshold,
        }),
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{GrindProfile, LevelBand, ProfileAnchor};
    use serde_json::json;

    fn state_summary(level: u64) -> Value {
        json!({
            "self": { "level": level, "hp_pct": 100 },
            "location": { "position": [-8641.34, -132.71, 86.93] }
        })
    }

    fn test_profile() -> GrindProfile {
        GrindProfile {
            anchor: ProfileAnchor { map_id: 0, x: -9690.0, y: 100.0, z: 55.0 },
            level_band: LevelBand { below: 3, above: 2 },
            creature_type: "humanoid".into(),
            wander_radius: 90.0,
            max_search_radius: 35.0,
            rest_threshold: 0.35,
            rotation_id: "auto_attack".into(),
            custom_behavior: None,
        }
    }

    // ── below cap → Some ────────────────────────────────────────────────────

    #[test]
    fn below_cap_returns_some() {
        let env = synthesize_grind(1003, &state_summary(6), 25, &test_profile());
        assert!(env.is_some(), "expected Some for level 6 with cap 25");
    }

    #[test]
    fn below_cap_goal_is_grind_variant() {
        let env = synthesize_grind(1003, &state_summary(6), 25, &test_profile()).unwrap();
        assert!(matches!(env.goal, Goal::Grind(_)));
    }

    #[test]
    fn below_cap_to_level_is_level_plus_one() {
        let env = synthesize_grind(1003, &state_summary(6), 25, &test_profile()).unwrap();
        let Goal::Grind(g) = env.goal else { panic!("not grind") };
        assert_eq!(g.to_level, 7, "to_level must be current+1");
    }

    // ── anchor comes from profile ───────────────────────────────────────────

    #[test]
    fn anchor_comes_from_profile() {
        let p = test_profile();
        let env = synthesize_grind(1173, &state_summary(6), 25, &p).unwrap();
        let Goal::Grind(g) = env.goal else { panic!("not grind") };
        assert_eq!(g.anchor_point.x, -9690.0);
        assert_eq!(g.anchor_point.y, 100.0);
        assert_eq!(g.anchor_point.z, 55.0);
        assert_eq!(g.anchor_point.map_id, 0);
    }

    #[test]
    fn mob_filter_band_from_profile() {
        let p = test_profile();
        let env = synthesize_grind(1003, &state_summary(6), 25, &p).unwrap();
        let Goal::Grind(g) = env.goal else { panic!("not grind") };
        assert_eq!(g.mob_filter.min_level, 3, "level(6)-below(3)");
        assert_eq!(g.mob_filter.max_level, 8, "level(6)+above(2)");
        assert_eq!(g.mob_filter.creature_type.as_deref(), Some("humanoid"));
    }

    #[test]
    fn radii_and_rest_from_profile() {
        let p = test_profile();
        let env = synthesize_grind(1003, &state_summary(6), 25, &p).unwrap();
        let Goal::Grind(g) = env.goal else { panic!("not grind") };
        assert_eq!(g.wander_radius, 90.0);
        assert_eq!(g.max_search_radius, 35.0);
        assert!((g.rest_threshold - 0.35).abs() < f32::EPSILON);
    }

    #[test]
    fn below_cap_wander_and_search_radius() {
        let env = synthesize_grind(1003, &state_summary(6), 25, &test_profile()).unwrap();
        let Goal::Grind(g) = env.goal else { panic!("not grind") };
        assert_eq!(g.wander_radius, 90.0);
        assert_eq!(g.max_search_radius, 35.0);
    }

    #[test]
    fn below_cap_rest_threshold_and_kill_count() {
        let env = synthesize_grind(1003, &state_summary(6), 25, &test_profile()).unwrap();
        let Goal::Grind(g) = env.goal else { panic!("not grind") };
        assert!((g.rest_threshold - 0.35).abs() < f32::EPSILON);
        assert!(g.kill_count.is_none());
    }

    // ── at cap → None ───────────────────────────────────────────────────────

    #[test]
    fn at_cap_returns_none() {
        let env = synthesize_grind(1003, &state_summary(25), 25, &test_profile());
        assert!(env.is_none(), "expected None when level == max_level");
    }

    #[test]
    fn above_cap_returns_none() {
        let env = synthesize_grind(1003, &state_summary(26), 25, &test_profile());
        assert!(env.is_none(), "expected None when level > max_level");
    }

    // ── goal_id stability ───────────────────────────────────────────────────

    #[test]
    fn goal_id_stable_per_bot_and_level() {
        let e1 = synthesize_grind(1003, &state_summary(6), 25, &test_profile()).unwrap();
        let e2 = synthesize_grind(1003, &state_summary(6), 25, &test_profile()).unwrap();
        assert_eq!(e1.goal_id, e2.goal_id, "same (bot,level) → same goal_id");
    }

    #[test]
    fn goal_id_different_for_different_levels() {
        let e6 = synthesize_grind(1003, &state_summary(6), 25, &test_profile()).unwrap();
        let e7 = synthesize_grind(1003, &state_summary(7), 25, &test_profile()).unwrap();
        assert_ne!(e6.goal_id, e7.goal_id);
    }

    #[test]
    fn goal_id_different_for_different_bots() {
        let e_a = synthesize_grind(1003, &state_summary(6), 25, &test_profile()).unwrap();
        let e_b = synthesize_grind(1004, &state_summary(6), 25, &test_profile()).unwrap();
        assert_ne!(e_a.goal_id, e_b.goal_id);
    }

    #[test]
    fn goal_id_format_contains_bot_and_level() {
        let env = synthesize_grind(1003, &state_summary(8), 25, &test_profile()).unwrap();
        assert!(
            env.goal_id.contains("1003") && env.goal_id.contains("8"),
            "goal_id '{}' should encode bot_guid and level",
            env.goal_id
        );
    }

    // ── level 1 (saturating_sub safety) ────────────────────────────────────

    #[test]
    fn level_one_does_not_underflow_min_level() {
        let env = synthesize_grind(1003, &state_summary(1), 25, &test_profile()).unwrap();
        let Goal::Grind(g) = env.goal else { panic!("not grind") };
        // saturating_sub(3) on u32 level=1 → 0
        assert_eq!(g.mob_filter.min_level, 0, "saturating_sub must not underflow");
    }

    // ── missing / invalid level ─────────────────────────────────────────────

    #[test]
    fn missing_level_returns_none() {
        let env = synthesize_grind(1003, &json!({ "self": {} }), 25, &test_profile());
        assert!(env.is_none(), "missing level must yield None");
    }

    #[test]
    fn missing_self_key_returns_none() {
        let env = synthesize_grind(1003, &json!({}), 25, &test_profile());
        assert!(env.is_none(), "missing self key must yield None");
    }

    // ── version ─────────────────────────────────────────────────────────────

    #[test]
    fn envelope_version_is_current_contract() {
        let env = synthesize_grind(1003, &state_summary(6), 25, &test_profile()).unwrap();
        assert_eq!(env.version, GOAL_CONTRACT_VERSION);
    }
}
