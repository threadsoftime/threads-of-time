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

// ── M3 QuestRunner types ──────────────────────────────────────────────────────

/// Who gives or receives a quest (accept/turnin NPC or GO).
///
/// `entry` is the creature_template / gameobject_template entry id.
/// `pos` is the approach position; the live GUID is resolved at runtime.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuestGiver {
    pub kind: GiverKind,
    /// creature_template or gameobject_template entry (NOT the spawn guid).
    pub entry: u32,
    /// Walk-to destination (approach point).
    pub pos: WorldPos,
}

/// Whether the quest giver is an NPC (creature) or a GameObject.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GiverKind { Npc, GameObject }

/// A single ordered objective step inside a `QuestGoal`.
///
/// Internally tagged on `kind` so JSON round-trips correctly alongside `Goal`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QuestStep {
    /// Walk to a world position (no harness action; arrival = completion).
    GoTo {
        pos: WorldPos,
        /// Human-readable label (debug/observability).
        #[serde(default)]
        label: Option<String>,
    },
    /// Kill mobs of `mob_entry` until `obs.get_quest_log` objective `obj_index`
    /// reaches `required`.  Delegates to the grind scan/fight/loot machinery.
    Kill {
        mob_entry: u32,
        count: u32,
        site: WorldPos,
        /// Index into the quest log's objectives array for progress polling.
        obj_index: u8,
        #[serde(default = "default_search_radius")]
        search_radius: f32,
        #[serde(default)]
        mob_level_min: u32,
        #[serde(default = "default_mob_level_max")]
        mob_level_max: u32,
    },
    /// Collect `count` of `item_entry` by killing mobs of `mob_entry`.
    /// Progress polled via `obs.get_quest_log` objective `obj_index`.
    Collect {
        item_entry: u32,
        count: u32,
        mob_entry: u32,
        site: WorldPos,
        obj_index: u8,
        #[serde(default = "default_search_radius")]
        search_radius: f32,
        #[serde(default)]
        mob_level_min: u32,
        #[serde(default = "default_mob_level_max")]
        mob_level_max: u32,
    },
    /// Use `item_entry` on self (untargeted, v1 only; targeted = phase-2).
    /// Progress polled via `obs.get_quest_log` objective `obj_index`.
    UseItem {
        item_entry: u32,
        obj_index: u8,
    },
    /// Interact with a gameobject identified by `object_entry`.
    /// Exec calls `bot.interact_object` with entry-search mode.
    /// Repeat `count` times; progress polled via objective `obj_index`.
    InteractObject {
        object_entry: u32,
        site: WorldPos,
        #[serde(default = "default_search_range")]
        search_range: f32,
        obj_index: u8,
        #[serde(default = "default_one")]
        count: u32,
    },
}

fn default_search_radius() -> f32 { 35.0 }
fn default_mob_level_max() -> u32 { 80 }
fn default_search_range() -> f32 { 20.0 }
fn default_one() -> u32 { 1 }

/// Run one quest from accept through objectives to turnin.
///
/// `giver` is where the bot accepts the quest; `receiver` is where it turns in
/// (often the same NPC, but not always).  `steps` are the ordered objectives
/// to complete between accept and turnin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuestGoal {
    pub quest_id: u32,
    /// Human-readable title (display/logs only; no runtime semantics).
    pub title: String,
    /// NPC/GO that gives the quest.
    pub giver: QuestGiver,
    /// NPC/GO that receives the turnin (may differ from `giver`).
    pub receiver: QuestGiver,
    /// Ordered objective steps (executed between accept and turnin).
    pub steps: Vec<QuestStep>,
    /// Per-step time budget in seconds.  Default 600 (10 min).
    #[serde(default = "default_step_timeout_s")]
    pub step_timeout_s: u64,
    /// Total quest time budget in seconds.  Default 3600 (1 hr).
    #[serde(default = "default_total_timeout_s")]
    pub total_timeout_s: u64,
}

fn default_step_timeout_s() -> u64 { 600 }
fn default_total_timeout_s() -> u64 { 3_600 }

// ── Goal enum ────────────────────────────────────────────────────────────────

/// The brain→exec intent. Internally tagged on `kind`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Goal {
    Grind(GrindGoal),
    Quest(QuestGoal),
    // Future: AssistPlayer, GoTo, Vendor, Rest
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

#[cfg(test)]
mod quest_goal_tests {
    use super::*;

    fn sample_giver() -> QuestGiver {
        QuestGiver {
            kind: GiverKind::Npc,
            entry: 25816,
            pos: WorldPos { map_id: 571, x: 2223.3, y: 5322.0, z: 10.6 },
        }
    }

    fn sample_kill_step() -> QuestStep {
        QuestStep::Kill {
            mob_entry: 27736,
            count: 6,
            site: WorldPos { map_id: 571, x: 2300.0, y: 5400.0, z: 15.0 },
            obj_index: 0,
            search_radius: 35.0,
            mob_level_min: 68,
            mob_level_max: 70,
        }
    }

    fn sample_quest_goal() -> QuestGoal {
        QuestGoal {
            quest_id: 11797,
            title: "The Siege".into(),
            giver: sample_giver(),
            receiver: sample_giver(),
            steps: vec![sample_kill_step()],
            step_timeout_s: 600,
            total_timeout_s: 3600,
        }
    }

    #[test]
    fn quest_goal_round_trips_through_json() {
        let g = sample_quest_goal();
        let json = serde_json::to_string(&g).unwrap();
        let back: QuestGoal = serde_json::from_str(&json).unwrap();
        assert_eq!(g, back);
    }

    #[test]
    fn quest_goal_envelope_has_kind_quest_tag() {
        let env = GoalEnvelope {
            goal_id: "q-1".into(),
            version: 1,
            goal: Goal::Quest(sample_quest_goal()),
        };
        let json = serde_json::to_string(&env).unwrap();
        assert!(json.contains(r#""kind":"quest""#), "quest tag missing: {json}");
    }

    #[test]
    fn quest_goal_defaults_apply_when_optional_fields_absent() {
        // A JSON without step_timeout_s / total_timeout_s should deserialize using defaults.
        let json = serde_json::json!({
            "quest_id": 1,
            "title": "Test Quest",
            "giver":    {"kind":"npc","entry":1,"pos":{"map_id":0,"x":0.0,"y":0.0,"z":0.0}},
            "receiver": {"kind":"npc","entry":1,"pos":{"map_id":0,"x":0.0,"y":0.0,"z":0.0}},
            "steps": []
        });
        let g: QuestGoal = serde_json::from_value(json).unwrap();
        assert_eq!(g.step_timeout_s, 600);
        assert_eq!(g.total_timeout_s, 3600);
    }

    #[test]
    fn quest_step_kill_round_trips() {
        let step = sample_kill_step();
        let json = serde_json::to_string(&step).unwrap();
        assert!(json.contains(r#""kind":"kill""#), "kill tag missing: {json}");
        let back: QuestStep = serde_json::from_str(&json).unwrap();
        assert_eq!(step, back);
    }

    #[test]
    fn quest_step_collect_round_trips() {
        let step = QuestStep::Collect {
            item_entry: 38572,
            count: 4,
            mob_entry: 27736,
            site: WorldPos { map_id: 571, x: 2300.0, y: 5400.0, z: 15.0 },
            obj_index: 0,
            search_radius: 40.0,
            mob_level_min: 68,
            mob_level_max: 70,
        };
        let json = serde_json::to_string(&step).unwrap();
        assert!(json.contains(r#""kind":"collect""#), "collect tag missing: {json}");
        let back: QuestStep = serde_json::from_str(&json).unwrap();
        assert_eq!(step, back);
    }

    #[test]
    fn quest_step_use_item_round_trips() {
        let step = QuestStep::UseItem { item_entry: 44012, obj_index: 0 };
        let json = serde_json::to_string(&step).unwrap();
        assert!(json.contains(r#""kind":"use_item""#), "use_item tag missing: {json}");
        let back: QuestStep = serde_json::from_str(&json).unwrap();
        assert_eq!(step, back);
    }

    #[test]
    fn quest_step_interact_object_round_trips() {
        let step = QuestStep::InteractObject {
            object_entry: 189188,
            site: WorldPos { map_id: 571, x: 2250.0, y: 5350.0, z: 12.0 },
            search_range: 20.0,
            obj_index: 0,
            count: 1,
        };
        let json = serde_json::to_string(&step).unwrap();
        assert!(json.contains(r#""kind":"interact_object""#), "interact_object tag missing: {json}");
        let back: QuestStep = serde_json::from_str(&json).unwrap();
        assert_eq!(step, back);
    }

    #[test]
    fn quest_step_goto_round_trips() {
        let step = QuestStep::GoTo {
            pos: WorldPos { map_id: 571, x: 2200.0, y: 5300.0, z: 10.0 },
            label: Some("approach_giver".into()),
        };
        let json = serde_json::to_string(&step).unwrap();
        assert!(json.contains(r#""kind":"go_to""#), "go_to tag missing: {json}");
        let back: QuestStep = serde_json::from_str(&json).unwrap();
        assert_eq!(step, back);
    }

    #[test]
    fn giver_kind_npc_and_gameobject_serialize() {
        let npc = serde_json::to_string(&GiverKind::Npc).unwrap();
        let go  = serde_json::to_string(&GiverKind::GameObject).unwrap();
        assert!(npc.contains("npc"), "npc: {npc}");
        assert!(go.contains("game_object"), "game_object: {go}");
    }
}
