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

/// Fallback grind anchor — Elwynn Forest Fargodeep / Jasperlode mine area.
/// Used only when the bot's live position is unavailable in `state_summary`.
const ELWYNN_ANCHOR: WorldPos = WorldPos {
    map_id: 0,
    x: -9384.5,
    y: 63.5,
    z: 54.0,
};

/// Anchor the grind at the bot's CURRENT position so it grinds where it is,
/// not at a fixed point it would walk away to. Reads `location.position` ([x,y,z])
/// from the `obs.get_state` digest; falls back to [`ELWYNN_ANCHOR`] if absent.
///
/// `map_id` is set to 0 (Eastern Kingdoms): the anchor's map is never used in
/// pathing (exec passes only x/y/z to `nav.find_path`, anchored from the bot's
/// live map), so this is harmless for the M1 single-map grind.
fn anchor_from_state(state_summary: &Value) -> WorldPos {
    let pos = state_summary
        .get("location")
        .and_then(|l| l.get("position"))
        .and_then(|v| v.as_array());
    match pos {
        Some(arr) if arr.len() >= 3 => {
            let (x, y, z) = (arr[0].as_f64(), arr[1].as_f64(), arr[2].as_f64());
            match (x, y, z) {
                (Some(x), Some(y), Some(z)) => WorldPos { map_id: 0, x, y, z },
                _ => ELWYNN_ANCHOR,
            }
        }
        _ => ELWYNN_ANCHOR,
    }
}

/// Synthesise a `Grind` goal for `bot_guid` based on the current observation.
///
/// Returns `Some(GoalEnvelope)` when `state_summary["self"]["level"] < max_level`,
/// and `None` when the bot is at or above cap (or when `level` is missing/invalid).
///
/// # Goal fields
/// * `anchor_point`: the bot's current position (`state_summary["location"]["position"]`),
///   falling back to `ELWYNN_ANCHOR` when unavailable
/// * `to_level`: `current_level + 1`
/// * `mob_filter`: `{min_level: level-3, max_level: level+2, creature_type: "humanoid"}`
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

    // Band: level-3 .. level+2. The low end is widened from level-1 so the bot
    // engages nearby slightly-lower mobs (e.g. a L6 bot at the L3 Kobold camp)
    // rather than reporting NoTargetsFound when the camp is below its level.
    let min_level = level.saturating_sub(3);
    let max_level_filter = level + 2;

    Some(GoalEnvelope {
        goal_id: format!("grind-{bot_guid}-{level}"),
        version: GOAL_CONTRACT_VERSION,
        goal: Goal::Grind(GrindGoal {
            anchor_point: anchor_from_state(state_summary),
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
        json!({
            "self": { "level": level, "hp_pct": 100 },
            "location": { "position": [-8641.34, -132.71, 86.93] }
        })
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
        assert_eq!(g.mob_filter.min_level, 3, "min = level-3");
        assert_eq!(g.mob_filter.max_level, 8, "max = level+2");
        assert_eq!(
            g.mob_filter.creature_type.as_deref(),
            Some("humanoid"),
            "creature_type must be humanoid"
        );
    }

    // ── anchor tracks the bot's live position ───────────────────────────────

    #[test]
    fn anchor_uses_bot_position_from_state() {
        let env = synthesize_grind(1173, &state_summary(6), 25).unwrap();
        let Goal::Grind(g) = env.goal else { panic!("not grind") };
        assert_eq!(g.anchor_point.x, -8641.34, "anchor x = bot position");
        assert_eq!(g.anchor_point.y, -132.71, "anchor y = bot position");
        assert_eq!(g.anchor_point.z, 86.93, "anchor z = bot position");
    }

    #[test]
    fn anchor_falls_back_when_position_missing() {
        // state_summary with a level but no location.position → fallback const.
        let s = json!({ "self": { "level": 6 } });
        let env = synthesize_grind(1173, &s, 25).unwrap();
        let Goal::Grind(g) = env.goal else { panic!("not grind") };
        assert_eq!(g.anchor_point, ELWYNN_ANCHOR, "missing position → ELWYNN_ANCHOR");
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
