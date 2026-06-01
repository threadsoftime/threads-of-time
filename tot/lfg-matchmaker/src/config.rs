#[derive(Clone, Debug, PartialEq)]
pub struct Dungeon {
    pub id: u32,
    pub map_id: u32,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub o: f64,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub listen_addr: String,
    pub harness_base_url: String,
    pub harness_bearer: String,
    pub tick_secs: u64,
    pub dungeon: Dungeon,
    /// When `true`, the tick loop performs mutating matchmaking actions
    /// (`lfg.form_group`, `bot.invite_to_group`, `bot.enter_instance`, etc.).
    ///
    /// **Default: `false` (inert/shadow mode).**  The slice is always spawned so
    /// the `/lfg` HTTP routes are available and queue state can be observed, but
    /// no live game-object mutations occur unless `LFG_ENABLED=true` is set.
    ///
    /// Mirror of `GES_DRIVE` in the game-event-scheduler.
    pub enabled: bool,
}

impl Config {
    pub fn from_env() -> Result<Config, String> {
        Self::build(|k| std::env::var(k).ok())
    }

    fn build(get: impl Fn(&str) -> Option<String>) -> Result<Config, String> {
        let req = |k: &str| get(k).ok_or_else(|| format!("missing env {k}"));

        // LFG_DUNGEON_ID is the LFGDungeons.dbc id (4 = Ragefire Chasm).
        // LFG_DUNGEON_MAP_ID is the map_id used by bot.enter_instance (389 for RFC).
        // These are two distinct identifiers — do not conflate them.
        let dungeon = Dungeon {
            id: get("LFG_DUNGEON_ID").and_then(|s| s.parse().ok()).unwrap_or(4),
            map_id: get("LFG_DUNGEON_MAP_ID").and_then(|s| s.parse().ok()).unwrap_or(389),
            x: get("LFG_DUNGEON_X").and_then(|s| s.parse().ok()).unwrap_or(3.81),
            y: get("LFG_DUNGEON_Y").and_then(|s| s.parse().ok()).unwrap_or(-14.82),
            z: get("LFG_DUNGEON_Z").and_then(|s| s.parse().ok()).unwrap_or(-17.84),
            o: get("LFG_DUNGEON_O").and_then(|s| s.parse().ok()).unwrap_or(4.39),
        };
        let enabled = get("LFG_ENABLED")
            .map(|s| matches!(s.to_lowercase().as_str(), "true" | "1"))
            .unwrap_or(false);

        Ok(Config {
            listen_addr: get("LFG_LISTEN_ADDR").unwrap_or_else(|| "0.0.0.0:8095".into()),
            harness_base_url: req("HARNESS_BASE_URL")?,
            harness_bearer: req("HARNESS_BEARER")?,
            tick_secs: get("LFG_TICK_SECS").and_then(|s| s.parse().ok()).unwrap_or(2),
            dungeon,
            enabled,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn getter(map: HashMap<&'static str, &'static str>) -> impl Fn(&str) -> Option<String> {
        move |k| map.get(k).map(|s| s.to_string())
    }

    fn required_map() -> HashMap<&'static str, &'static str> {
        HashMap::from([("HARNESS_BASE_URL", "http://127.0.0.1:8099"), ("HARNESS_BEARER", "tok")])
    }

    fn full_map() -> HashMap<&'static str, &'static str> {
        let mut m = required_map();
        m.extend([
            ("LFG_DUNGEON_ID", "4"),
            ("LFG_DUNGEON_MAP_ID", "389"),
            ("LFG_DUNGEON_X", "3.81"),
            ("LFG_DUNGEON_Y", "-14.82"),
            ("LFG_DUNGEON_Z", "-17.84"),
            ("LFG_DUNGEON_O", "4.39"),
        ]);
        m
    }

    // Dungeon coords default to RFC (LFGDungeons id 4, map 389) when omitted.
    #[test]
    fn build_succeeds_with_rfc_defaults() {
        let cfg = Config::build(getter(required_map())).unwrap();
        assert_eq!(cfg.listen_addr, "0.0.0.0:8095");
        assert_eq!(cfg.tick_secs, 2);
        assert_eq!(cfg.dungeon.id, 4, "LFGDungeons.dbc id for RFC is 4");
        assert_eq!(cfg.dungeon.map_id, 389);
        assert!((cfg.dungeon.x - 3.81).abs() < 1e-6);
        assert!((cfg.dungeon.y - -14.82).abs() < 1e-6);
        assert!((cfg.dungeon.z - -17.84).abs() < 1e-6);
        assert!((cfg.dungeon.o - 4.39).abs() < 1e-6);
        // LFG_ENABLED defaults to false (inert mode) when not set.
        assert!(!cfg.enabled, "enabled must default to false when LFG_ENABLED is unset");
    }

    // LFG_ENABLED gates mutating matchmaking; must default false and parse true/1.
    #[test]
    fn lfg_enabled_defaults_false_and_parses_true() {
        // Absent → false.
        let cfg = Config::build(getter(required_map())).unwrap();
        assert!(!cfg.enabled, "LFG_ENABLED absent → false");

        // "true" → true.
        let mut m = required_map();
        m.insert("LFG_ENABLED", "true");
        let cfg = Config::build(getter(m)).unwrap();
        assert!(cfg.enabled, "LFG_ENABLED=true → true");

        // "1" → true.
        let mut m = required_map();
        m.insert("LFG_ENABLED", "1");
        let cfg = Config::build(getter(m)).unwrap();
        assert!(cfg.enabled, "LFG_ENABLED=1 → true");

        // "false" → false.
        let mut m = required_map();
        m.insert("LFG_ENABLED", "false");
        let cfg = Config::build(getter(m)).unwrap();
        assert!(!cfg.enabled, "LFG_ENABLED=false → false");
    }

    // When all env vars are explicitly set they override the defaults.
    #[test]
    fn build_uses_explicit_env_values() {
        let cfg = Config::build(getter(full_map())).unwrap();
        assert_eq!(cfg.dungeon.id, 4);
        assert_eq!(cfg.dungeon.map_id, 389);
        assert!((cfg.dungeon.o - 4.39).abs() < 1e-6);
    }

    #[test]
    fn build_rejects_missing_required_var() {
        let mut m = required_map();
        m.remove("HARNESS_BASE_URL");
        let err = Config::build(getter(m)).unwrap_err();
        assert!(err.contains("HARNESS_BASE_URL"), "got: {err}");
    }

    #[test]
    fn build_rejects_bad_integer() {
        let mut m = full_map();
        m.insert("LFG_DUNGEON_MAP_ID", "not_a_number");
        // bad parse is silently ignored (falls back to default); only truly required
        // vars (HARNESS_BASE_URL, HARNESS_BEARER) cause hard failures.
        let cfg = Config::build(getter(m)).unwrap();
        assert_eq!(cfg.dungeon.map_id, 389, "falls back to default on bad parse");
    }
}
