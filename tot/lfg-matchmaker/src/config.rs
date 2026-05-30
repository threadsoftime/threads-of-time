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
}

impl Config {
    pub fn from_env() -> Result<Config, String> {
        Self::build(|k| std::env::var(k).ok())
    }

    fn build(get: impl Fn(&str) -> Option<String>) -> Result<Config, String> {
        let req = |k: &str| get(k).ok_or_else(|| format!("missing env {k}"));

        let dungeon = Dungeon {
            id: req("LFG_DUNGEON_ID")?.parse().map_err(|_| "LFG_DUNGEON_ID not u32".to_string())?,
            map_id: req("LFG_DUNGEON_MAP_ID")?.parse().map_err(|_| "LFG_DUNGEON_MAP_ID not u32".to_string())?,
            x: req("LFG_DUNGEON_X")?.parse().map_err(|_| "LFG_DUNGEON_X not f64".to_string())?,
            y: req("LFG_DUNGEON_Y")?.parse().map_err(|_| "LFG_DUNGEON_Y not f64".to_string())?,
            z: req("LFG_DUNGEON_Z")?.parse().map_err(|_| "LFG_DUNGEON_Z not f64".to_string())?,
            o: get("LFG_DUNGEON_O").and_then(|s| s.parse().ok()).unwrap_or(0.0),
        };
        Ok(Config {
            listen_addr: get("LFG_LISTEN_ADDR").unwrap_or_else(|| "0.0.0.0:8095".into()),
            harness_base_url: req("HARNESS_BASE_URL")?,
            harness_bearer: req("HARNESS_BEARER")?,
            tick_secs: get("LFG_TICK_SECS").and_then(|s| s.parse().ok()).unwrap_or(2),
            dungeon,
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

    fn full_map() -> HashMap<&'static str, &'static str> {
        HashMap::from([
            ("HARNESS_BASE_URL", "http://127.0.0.1:8099"),
            ("HARNESS_BEARER", "tok"),
            ("LFG_DUNGEON_ID", "36"),
            ("LFG_DUNGEON_MAP_ID", "389"),
            ("LFG_DUNGEON_X", "1.0"),
            ("LFG_DUNGEON_Y", "2.0"),
            ("LFG_DUNGEON_Z", "3.0"),
        ])
    }

    #[test]
    fn build_succeeds_with_defaults() {
        let cfg = Config::build(getter(full_map())).unwrap();
        assert_eq!(cfg.listen_addr, "0.0.0.0:8095");
        assert_eq!(cfg.tick_secs, 2);
        assert_eq!(cfg.dungeon.map_id, 389);
        assert_eq!(cfg.dungeon.o, 0.0);
    }

    #[test]
    fn build_rejects_missing_required_var() {
        let mut m = full_map();
        m.remove("LFG_DUNGEON_ID");
        let err = Config::build(getter(m)).unwrap_err();
        assert!(err.contains("LFG_DUNGEON_ID"), "got: {err}");
    }

    #[test]
    fn build_rejects_bad_integer() {
        let mut m = full_map();
        m.insert("LFG_DUNGEON_MAP_ID", "not_a_number");
        assert!(Config::build(getter(m)).is_err());
    }
}
