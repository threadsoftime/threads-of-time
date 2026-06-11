//! The `Goal` intent the brain emits to the executor.

use serde::{Deserialize, Serialize};

/// A point in WoW world-space (with the map it belongs to).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WorldPos {
    pub map_id: u32,
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// Which creatures a grind targets: a level band + an optional creature-type hint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MobFilter {
    pub min_level: u32,
    pub max_level: u32,
    /// Lower-case creature-type hint (e.g. "humanoid"); `None` = any.
    pub creature_type: Option<String>,
}

/// Camp vendor for in-grind economy trips (M2 slice 2.4). Mined per camp from the
/// world-DB dumps (spec §6); absent = no economy checks for this goal.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct VendorInfo {
    /// DB creature spawn id (`creature.guid`) — what bot.vendor_sell/bot.repair take.
    pub spawn_id: u64,
    /// Vendor spawn position (nav destination). Same map as the goal anchor.
    pub pos: WorldPos,
    /// Mined npcflag & 0x1000 — when true the trip also calls bot.repair.
    pub can_repair: bool,
}

/// Grind a level band of mobs around an anchor until a level or kill-count target.
///
/// `to_level` and `kill_count` are OR stop-conditions (whichever is met first).
/// `max_search_radius` MUST be < `wander_radius` (the executor enforces the bound).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GrindGoal {
    pub anchor_point: WorldPos,
    pub wander_radius: f32,
    pub max_search_radius: f32,
    pub mob_filter: MobFilter,
    pub to_level: u32,
    pub kill_count: Option<u32>,
    /// Health fraction [0,1] at which the bot stops to regen.
    pub rest_threshold: f32,
    /// Rotation plugin id (profile-driven, M2 slice 2.2). `None` → auto_attack (M1 back-compat).
    #[serde(default)]
    pub rotation_id: Option<String>,
    /// Camp vendor for economy trips (M2 slice 2.4). `None` → no economy checks.
    #[serde(default)]
    pub vendor: Option<VendorInfo>,
}

/// The brain→exec intent. M1 ships exactly one variant. Internally tagged on `kind`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Goal {
    Grind(GrindGoal),
    // M2: AssistPlayer, GoTo, Vendor, Rest
}

/// Identity + version wrapper around a `Goal`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoalEnvelope {
    /// Monotonic per (brain, bot); the executor ignores a duplicate id.
    pub goal_id: String,
    /// Contract version; the executor rejects `version` > `GOAL_CONTRACT_VERSION`.
    pub version: u32,
    pub goal: Goal,
}

/// Why a received `GoalEnvelope` is unusable.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum EnvelopeError {
    #[error("goal contract version {got} exceeds supported {supported}")]
    VersionTooNew { got: u32, supported: u32 },
}

impl GoalEnvelope {
    /// Validate the envelope against this build's contract version.
    /// (Unknown `Goal` variants are rejected earlier, at deserialization time.)
    pub fn validate(&self) -> Result<(), EnvelopeError> {
        if self.version > crate::GOAL_CONTRACT_VERSION {
            return Err(EnvelopeError::VersionTooNew {
                got: self.version,
                supported: crate::GOAL_CONTRACT_VERSION,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_envelope() -> GoalEnvelope {
        GoalEnvelope {
            goal_id: "g-1".into(),
            version: 1,
            goal: Goal::Grind(GrindGoal {
                anchor_point: WorldPos { map_id: 0, x: -9450.0, y: 50.0, z: 60.0 },
                wander_radius: 90.0,
                max_search_radius: 35.0,
                mob_filter: MobFilter { min_level: 4, max_level: 7, creature_type: Some("humanoid".into()) },
                to_level: 6,
                kill_count: None,
                rest_threshold: 0.35,
                rotation_id: None,
                vendor: None,
            }),
        }
    }

    #[test]
    fn grind_envelope_round_trips_through_json() {
        let env = sample_envelope();
        let json = serde_json::to_string(&env).unwrap();
        // Internally-tagged enum: the goal carries a "kind":"grind" discriminant.
        assert!(json.contains(r#""kind":"grind""#), "tagged discriminant missing: {json}");
        let back: GoalEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(env, back);
    }

    #[test]
    fn grind_goal_fields_present_in_json() {
        let json = serde_json::to_string(&sample_envelope()).unwrap();
        for field in ["anchor_point", "wander_radius", "max_search_radius",
                      "mob_filter", "to_level", "rest_threshold"] {
            assert!(json.contains(field), "missing field {field} in {json}");
        }
    }

    #[test]
    fn validate_accepts_current_version() {
        let env = sample_envelope();
        assert!(env.validate().is_ok());
    }

    #[test]
    fn validate_rejects_future_version() {
        let mut env = sample_envelope();
        env.version = crate::GOAL_CONTRACT_VERSION + 1;
        let err = env.validate().unwrap_err();
        assert!(matches!(err, EnvelopeError::VersionTooNew { .. }), "got {err:?}");
    }

    #[test]
    fn unknown_goal_kind_fails_to_deserialize() {
        // An unrecognised "kind" must fail deserialization (caller maps to Blocked).
        let json = r#"{"goal_id":"g","version":1,"goal":{"kind":"teleport_to_moon"}}"#;
        let res: Result<GoalEnvelope, _> = serde_json::from_str(json);
        assert!(res.is_err(), "unknown variant must not deserialize");
    }
}

#[cfg(test)]
mod vendor_tests {
    use super::*;

    /// A pre-2.4 GrindGoal JSON (no vendor) must still deserialize → None.
    #[test]
    fn grind_goal_without_vendor_deserializes() {
        let json = serde_json::json!({
            "anchor_point": {"map_id": 0, "x": 1.0, "y": 2.0, "z": 3.0},
            "wander_radius": 90.0, "max_search_radius": 35.0,
            "mob_filter": {"min_level": 1, "max_level": 5, "creature_type": null},
            "to_level": 6, "kill_count": null, "rest_threshold": 0.35
        });
        let g: GrindGoal = serde_json::from_value(json).unwrap();
        assert!(g.vendor.is_none());
    }

    #[test]
    fn grind_goal_vendor_round_trips() {
        let mut g: GrindGoal = serde_json::from_value(serde_json::json!({
            "anchor_point": {"map_id": 1, "x": 1.0, "y": 2.0, "z": 3.0},
            "wander_radius": 90.0, "max_search_radius": 35.0,
            "mob_filter": {"min_level": 1, "max_level": 5, "creature_type": null},
            "to_level": 6, "kill_count": null, "rest_threshold": 0.35
        })).unwrap();
        g.vendor = Some(VendorInfo {
            spawn_id: 40_001,
            pos: WorldPos { map_id: 1, x: 2200.0, y: -300.0, z: 95.0 },
            can_repair: true,
        });
        let back: GrindGoal = serde_json::from_value(serde_json::to_value(&g).unwrap()).unwrap();
        let v = back.vendor.expect("vendor survives round-trip");
        assert_eq!(v.spawn_id, 40_001);
        assert_eq!(v.pos.map_id, 1);
        assert!(v.can_repair);
    }
}

#[cfg(test)]
mod rotation_id_tests {
    use super::*;

    /// An M1-era GrindGoal JSON (no rotation_id) must still deserialize → None.
    #[test]
    fn grind_goal_without_rotation_id_deserializes() {
        let json = serde_json::json!({
            "anchor_point": {"map_id": 0, "x": 1.0, "y": 2.0, "z": 3.0},
            "wander_radius": 90.0,
            "max_search_radius": 35.0,
            "mob_filter": {"min_level": 1, "max_level": 5, "creature_type": "humanoid"},
            "to_level": 6,
            "kill_count": null,
            "rest_threshold": 0.35
        });
        let g: GrindGoal = serde_json::from_value(json).unwrap();
        assert_eq!(g.rotation_id, None);
    }

    #[test]
    fn grind_goal_rotation_id_round_trips() {
        let mut g: GrindGoal = serde_json::from_value(serde_json::json!({
            "anchor_point": {"map_id": 0, "x": 1.0, "y": 2.0, "z": 3.0},
            "wander_radius": 90.0, "max_search_radius": 35.0,
            "mob_filter": {"min_level": 1, "max_level": 5, "creature_type": null},
            "to_level": 6, "kill_count": null, "rest_threshold": 0.35
        })).unwrap();
        g.rotation_id = Some("mage_frost_b1".into());
        let back: GrindGoal = serde_json::from_value(serde_json::to_value(&g).unwrap()).unwrap();
        assert_eq!(back.rotation_id.as_deref(), Some("mage_frost_b1"));
    }
}
