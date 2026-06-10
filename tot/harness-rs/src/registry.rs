//! V1 tool registry.
//!
//! Port of `harness_daemon/registry.py`. Each ToolEntry declares: name,
//! required scope, subject-GUID arg name (for self-binding), and a
//! forwards-to-AC flag.

use std::collections::BTreeMap;

use serde_json::Value;
use thiserror::Error;

// ── types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ToolEntry {
    pub name: String,
    #[allow(dead_code)] // part of registry data mirroring Python registry.py; dispatch matches via scope-glob, not this field
    pub required_scope: String,
    pub subject_guid_arg: Option<String>,
    pub forwards_to_ac: bool,
}

impl ToolEntry {
    fn new(
        name: &str,
        required_scope: &str,
        subject_guid_arg: Option<&str>,
        forwards_to_ac: bool,
    ) -> Self {
        Self {
            name: name.to_string(),
            required_scope: required_scope.to_string(),
            subject_guid_arg: subject_guid_arg.map(str::to_string),
            forwards_to_ac,
        }
    }

    /// Enforce that the request targets the token's bound bot/player.
    ///
    /// Port of `registry.py:ToolEntry.check_self_binding` EXACTLY, including
    /// message strings. Called only when the matched scope pattern is `<ns>.self.*`.
    pub fn check_self_binding(
        &self,
        args: &Value,
        bound_to_guid: i64,
    ) -> Result<(), SelfBindingViolation> {
        let arg_name = match &self.subject_guid_arg {
            None => {
                return Err(SelfBindingViolation(
                    "not_bound: tool has no subject GUID".to_string(),
                ))
            }
            Some(a) => a,
        };
        let subject_val = match args.get(arg_name) {
            None => {
                return Err(SelfBindingViolation(format!(
                    "missing subject arg '{arg_name}'"
                )))
            }
            Some(v) => v,
        };
        // Accept both i64 and u64 representations from JSON.
        let subject = subject_val
            .as_i64()
            .or_else(|| subject_val.as_u64().map(|u| u as i64))
            .ok_or_else(|| {
                SelfBindingViolation(format!(
                    "not_bound: subject {subject_val} != token bound_to_guid {bound_to_guid}"
                ))
            })?;
        if subject != bound_to_guid {
            return Err(SelfBindingViolation(format!(
                "not_bound: subject {subject} != token bound_to_guid {bound_to_guid}"
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
#[error("tool not found: {0}")]
pub struct ToolNotFound(pub String);

#[derive(Debug, Error)]
#[error("{0}")]
pub struct SelfBindingViolation(pub String);

// ── Registry ──────────────────────────────────────────────────────────────────

pub struct Registry {
    by_name: BTreeMap<String, ToolEntry>,
}

impl Registry {
    fn new(entries: Vec<ToolEntry>) -> Self {
        let by_name = entries.into_iter().map(|e| (e.name.clone(), e)).collect();
        Self { by_name }
    }

    pub fn find(&self, name: &str) -> Result<&ToolEntry, ToolNotFound> {
        self.by_name
            .get(name)
            .ok_or_else(|| ToolNotFound(name.to_string()))
    }

    /// Returns tool names in lexicographic order (BTreeMap iterates keys sorted).
    ///
    /// Used in tests and as a public API mirroring Python `registry.py:Registry.names`.
    #[allow(dead_code)]
    pub fn names(&self) -> Vec<String> {
        self.by_name.keys().cloned().collect()
    }
}

// ── build_v1_registry ─────────────────────────────────────────────────────────

/// Build the V1 registry per spec §6.
///
/// Port of `registry.py:build_v1_registry` (54 entries) + M2 slice 2.2 additions (58 total).
pub fn build_v1_registry() -> Registry {
    Registry::new(vec![
        // GM-tier
        ToolEntry::new("gm.additem",             "gm.additem",             Some("target_guid"), true),
        ToolEntry::new("gm.equip_all",            "gm.equip_all",           Some("target_guid"), true),
        ToolEntry::new("gm.teleport",             "gm.teleport",            Some("target_guid"), true),
        ToolEntry::new("gm.set_level",            "gm.set_level",           Some("target_guid"), true),
        ToolEntry::new("gm.run_console",          "gm.run_console",         None,                true),
        ToolEntry::new("gm.read_console_output",  "gm.read_console_output", None,                true),
        ToolEntry::new("gm.strip_gear",           "gm.strip_gear",          Some("target_guid"), true),
        // Bot-tier
        ToolEntry::new("bot.set_goal",            "bot.set_goal",           Some("bot_guid"),    true),
        // Bot-tier (V1.4)
        ToolEntry::new("bot.set_strategy",        "bot.set_strategy",       Some("bot_guid"),    true),
        ToolEntry::new("bot.get_strategies",      "bot.get_strategies",     Some("bot_guid"),    true),
        ToolEntry::new("bot.send_chat",           "bot.send_chat",          Some("bot_guid"),    true),
        ToolEntry::new("bot.follow",              "bot.follow",             Some("bot_guid"),    true),
        ToolEntry::new("bot.stop",                "bot.stop",               Some("bot_guid"),    true),
        // Bot-tier (M1-walk)
        ToolEntry::new("bot.move_path",           "bot.move_path",          Some("bot_guid"),    true),
        // Bot-tier (M1-own)
        ToolEntry::new("bot.set_ai_enabled",      "bot.set_ai_enabled",     Some("bot_guid"),    true),
        // Bot-tier (V1.5 — grouping primitives)
        ToolEntry::new("bot.invite_to_group",     "bot.invite_to_group",    Some("bot_guid"),    true),
        ToolEntry::new("bot.accept_invite",       "bot.accept_invite",      Some("bot_guid"),    true),
        ToolEntry::new("bot.leave_group",         "bot.leave_group",        Some("bot_guid"),    true),
        ToolEntry::new("bot.set_role",            "bot.set_role",           Some("bot_guid"),    true),
        ToolEntry::new("bot.queue_for_dungeon",   "bot.queue_for_dungeon",  Some("bot_guid"),    true),
        ToolEntry::new("bot.enter_instance",      "bot.enter_instance",     Some("bot_guid"),    true),
        // Observation
        ToolEntry::new("obs.ping",                "obs.ping",               None,                true),
        ToolEntry::new("obs.get_state",           "obs.get_state",          Some("target_guid"), true),
        ToolEntry::new("obs.get_auras",           "obs.get_auras",          Some("target_guid"), true),
        ToolEntry::new("obs.get_inventory",       "obs.get_inventory",      Some("target_guid"), true),
        ToolEntry::new("obs.get_combat_log",      "obs.get_combat_log",     Some("target_guid"), true),
        // V1.3 additions
        ToolEntry::new("obs.get_quest_log",       "obs.get_quest_log",      Some("target_guid"), true),
        ToolEntry::new("obs.get_xp",              "obs.get_xp",             Some("target_guid"), true),
        ToolEntry::new("obs.list_players",        "obs.list_players",       None,                true),
        ToolEntry::new("obs.list_bot_population", "obs.list_bot_population",None,                true),
        ToolEntry::new("obs.get_rpg_status",      "obs.get_rpg_status",     Some("target_guid"), true),
        ToolEntry::new("obs.get_money",           "obs.get_money",          Some("target_guid"), true),
        ToolEntry::new("obs.get_position",        "obs.get_position",       Some("target_guid"), true),
        ToolEntry::new("obs.get_group",           "obs.get_group",          Some("target_guid"), true),
        // Observation (V1.4)
        ToolEntry::new("obs.get_talents",         "obs.get_talents",        Some("target_guid"), true),
        // GES Inc-1
        ToolEntry::new("obs.game_events",         "obs.game_events",        None,                true),
        // GES Inc-2
        ToolEntry::new("event.start",             "event.start",            None,                true),
        ToolEntry::new("event.stop",              "event.stop",             None,                true),
        // Daemon-direct (only tool with forwards_to_ac=false)
        ToolEntry::new("obs.query_db",            "obs.query_db",           None,                false),
        // Memory subsystem (Phase 6B)
        ToolEntry::new("memory.write",            "memory.write",           Some("bot_guid"),    true),
        ToolEntry::new("memory.read",             "memory.read",            Some("bot_guid"),    true),
        ToolEntry::new("memory.recall",           "memory.recall",          Some("bot_guid"),    true),
        ToolEntry::new("memory.search",           "memory.search",          Some("bot_guid"),    true),
        ToolEntry::new("memory.list",             "memory.list",            Some("bot_guid"),    true),
        ToolEntry::new("memory.update",           "memory.update",          Some("bot_guid"),    true),
        ToolEntry::new("memory.delete",           "memory.delete",          Some("bot_guid"),    true),
        // nav.* (1) — server-side navmesh primitive
        ToolEntry::new("nav.find_path",           "nav.find_path",          Some("bot_guid"),    true),
        // Bot-tier (M1-combat-loot)
        ToolEntry::new("bot.attack",               "bot.attack",               Some("bot_guid"), true),
        ToolEntry::new("bot.loot",                 "bot.loot",                 Some("bot_guid"), true),
        // Observation (M1-combat-loot)
        ToolEntry::new("obs.get_lootable_corpses", "obs.get_lootable_corpses", Some("bot_guid"), true),
        ToolEntry::new("obs.get_nearby_hostiles",  "obs.get_nearby_hostiles",  Some("bot_guid"), true),
        // Bot-tier (M2 slice 2.2 — cast + economy verb batch)
        ToolEntry::new("bot.cast_spell",  "bot.cast_spell",  Some("bot_guid"), true),
        ToolEntry::new("bot.vendor_sell", "bot.vendor_sell", Some("bot_guid"), true),
        ToolEntry::new("bot.repair",      "bot.repair",      Some("bot_guid"), true),
        ToolEntry::new("bot.mail",        "bot.mail",        Some("bot_guid"), true),
        // LFG force-form primitives (Inc 1)
        ToolEntry::new("lfg.form_group",          "lfg.form_group",         None,                true),
        // LFG cancel drain (Inc 2)
        ToolEntry::new("lfg.cancel",              "lfg.cancel",             None,                true),
        // Stage 3: pull-based intent drain
        ToolEntry::new("obs.lfg_pending",         "obs.lfg_pending",        None,                true),
    ])
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn registry_has_58_tools() {
        let reg = build_v1_registry();
        assert_eq!(reg.names().len(), 58);
    }

    #[test]
    fn names_are_sorted() {
        let reg = build_v1_registry();
        let names = reg.names();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
    }

    #[test]
    fn obs_ping_registered() {
        let reg = build_v1_registry();
        let e = reg.find("obs.ping").unwrap();
        assert_eq!(e.name, "obs.ping");
        assert_eq!(e.required_scope, "obs.ping");
        assert!(e.subject_guid_arg.is_none());
        assert!(e.forwards_to_ac);
    }

    #[test]
    fn gm_additem_has_target_guid_arg() {
        let reg = build_v1_registry();
        let e = reg.find("gm.additem").unwrap();
        assert_eq!(e.subject_guid_arg, Some("target_guid".to_string()));
    }

    #[test]
    fn bot_set_goal_uses_bot_guid() {
        let reg = build_v1_registry();
        let e = reg.find("bot.set_goal").unwrap();
        assert_eq!(e.subject_guid_arg, Some("bot_guid".to_string()));
    }

    #[test]
    fn obs_query_db_is_daemon_direct() {
        let reg = build_v1_registry();
        let e = reg.find("obs.query_db").unwrap();
        assert!(!e.forwards_to_ac);
        assert_eq!(e.required_scope, "obs.query_db");
    }

    #[test]
    fn unknown_tool_raises_not_found() {
        let reg = build_v1_registry();
        assert!(reg.find("gm.nope").is_err());
    }

    #[test]
    fn obs_game_events_registered() {
        let reg = build_v1_registry();
        let e = reg.find("obs.game_events").unwrap();
        assert_eq!(e.name, "obs.game_events");
        assert!(e.subject_guid_arg.is_none());
        assert!(e.forwards_to_ac);
    }

    #[test]
    fn event_start_registered() {
        let reg = build_v1_registry();
        let e = reg.find("event.start").unwrap();
        assert!(e.subject_guid_arg.is_none());
        assert!(e.forwards_to_ac);
    }

    #[test]
    fn event_stop_registered() {
        let reg = build_v1_registry();
        let e = reg.find("event.stop").unwrap();
        assert!(e.subject_guid_arg.is_none());
        assert!(e.forwards_to_ac);
    }

    #[test]
    fn nav_find_path_registered() {
        let reg = build_v1_registry();
        let e = reg.find("nav.find_path").unwrap();
        assert_eq!(e.name, "nav.find_path");
        assert_eq!(e.required_scope, "nav.find_path");
        assert_eq!(e.subject_guid_arg, Some("bot_guid".to_string()));
        assert!(e.forwards_to_ac, "nav.find_path must forward to AC");
    }

    #[test]
    fn bot_move_path_registered() {
        let reg = build_v1_registry();
        let e = reg.find("bot.move_path").unwrap();
        assert_eq!(e.name, "bot.move_path");
        assert_eq!(e.required_scope, "bot.move_path");
        assert_eq!(e.subject_guid_arg, Some("bot_guid".to_string()));
        assert!(e.forwards_to_ac, "bot.move_path must forward to AC");
    }

    #[test]
    fn bot_set_ai_enabled_registered() {
        let reg = build_v1_registry();
        let e = reg.find("bot.set_ai_enabled").unwrap();
        assert_eq!(e.name, "bot.set_ai_enabled");
        assert_eq!(e.required_scope, "bot.set_ai_enabled");
        assert_eq!(e.subject_guid_arg, Some("bot_guid".to_string()));
        assert!(e.forwards_to_ac);
    }

    #[test]
    fn bot_attack_registered() {
        let reg = build_v1_registry();
        let e = reg.find("bot.attack").unwrap();
        assert_eq!(e.name, "bot.attack");
        assert_eq!(e.required_scope, "bot.attack");
        assert_eq!(e.subject_guid_arg, Some("bot_guid".to_string()));
        assert!(e.forwards_to_ac, "bot.attack must forward to AC");
    }

    #[test]
    fn bot_cast_spell_registered() {
        let reg = build_v1_registry();
        let e = reg.find("bot.cast_spell").unwrap();
        assert_eq!(e.required_scope, "bot.cast_spell");
        assert_eq!(e.subject_guid_arg, Some("bot_guid".to_string()));
        assert!(e.forwards_to_ac);
    }

    #[test]
    fn economy_verbs_registered() {
        let reg = build_v1_registry();
        for name in ["bot.vendor_sell", "bot.repair", "bot.mail"] {
            let e = reg.find(name).unwrap_or_else(|_| panic!("{name} missing"));
            assert_eq!(e.required_scope, name);
            assert_eq!(e.subject_guid_arg, Some("bot_guid".to_string()));
            assert!(e.forwards_to_ac, "{name} must forward to AC");
        }
    }

    #[test]
    fn obs_get_nearby_hostiles_registered() {
        let reg = build_v1_registry();
        let e = reg.find("obs.get_nearby_hostiles").unwrap();
        assert_eq!(e.name, "obs.get_nearby_hostiles");
        assert_eq!(e.required_scope, "obs.get_nearby_hostiles");
        assert_eq!(e.subject_guid_arg, Some("bot_guid".to_string()));
        assert!(e.forwards_to_ac, "obs.get_nearby_hostiles must forward to AC");
    }

    #[test]
    fn obs_get_lootable_corpses_registered() {
        let reg = build_v1_registry();
        let e = reg.find("obs.get_lootable_corpses").unwrap();
        assert_eq!(e.name, "obs.get_lootable_corpses");
        assert_eq!(e.required_scope, "obs.get_lootable_corpses");
        assert_eq!(e.subject_guid_arg, Some("bot_guid".to_string()));
        assert!(e.forwards_to_ac, "obs.get_lootable_corpses must forward to AC");
    }

    #[test]
    fn bot_loot_registered() {
        let reg = build_v1_registry();
        let e = reg.find("bot.loot").unwrap();
        assert_eq!(e.name, "bot.loot");
        assert_eq!(e.required_scope, "bot.loot");
        assert_eq!(e.subject_guid_arg, Some("bot_guid".to_string()));
        assert!(e.forwards_to_ac, "bot.loot must forward to AC");
    }

    // ── check_self_binding ─────────────────────────────────────────────────

    #[test]
    fn self_binding_ok_when_match() {
        let reg = build_v1_registry();
        let e = reg.find("gm.additem").unwrap();
        e.check_self_binding(&json!({"target_guid": 12345_i64}), 12345).unwrap();
    }

    #[test]
    fn self_binding_rejects_mismatch() {
        let reg = build_v1_registry();
        let e = reg.find("gm.additem").unwrap();
        let err = e.check_self_binding(&json!({"target_guid": 99_i64}), 12345).unwrap_err();
        assert!(err.0.contains("not_bound"), "expected 'not_bound' in: {}", err.0);
    }

    #[test]
    fn self_binding_rejects_missing_arg() {
        let reg = build_v1_registry();
        let e = reg.find("gm.additem").unwrap();
        let err = e.check_self_binding(&json!({}), 12345).unwrap_err();
        assert!(err.0.contains("missing"), "expected 'missing' in: {}", err.0);
    }

    #[test]
    fn self_binding_rejects_no_subject_guid_arg() {
        let reg = build_v1_registry();
        // obs.ping has subject_guid_arg = None
        let e = reg.find("obs.ping").unwrap();
        let err = e.check_self_binding(&json!({}), 12345).unwrap_err();
        assert!(
            err.0.contains("not_bound: tool has no subject GUID"),
            "unexpected: {}",
            err.0
        );
    }
}
