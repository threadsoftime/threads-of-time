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

fn req(k: &str) -> Result<String, String> {
    std::env::var(k).map_err(|_| format!("missing env {k}"))
}

impl Config {
    pub fn from_env() -> Result<Config, String> {
        let dungeon = Dungeon {
            id: req("LFG_DUNGEON_ID")?.parse().map_err(|_| "LFG_DUNGEON_ID not u32".to_string())?,
            map_id: req("LFG_DUNGEON_MAP_ID")?.parse().map_err(|_| "LFG_DUNGEON_MAP_ID not u32".to_string())?,
            x: req("LFG_DUNGEON_X")?.parse().map_err(|_| "LFG_DUNGEON_X not f64".to_string())?,
            y: req("LFG_DUNGEON_Y")?.parse().map_err(|_| "LFG_DUNGEON_Y not f64".to_string())?,
            z: req("LFG_DUNGEON_Z")?.parse().map_err(|_| "LFG_DUNGEON_Z not f64".to_string())?,
            o: std::env::var("LFG_DUNGEON_O").ok().and_then(|s| s.parse().ok()).unwrap_or(0.0),
        };
        Ok(Config {
            listen_addr: std::env::var("LFG_LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:8095".into()),
            harness_base_url: req("HARNESS_BASE_URL")?,
            harness_bearer: req("HARNESS_BEARER")?,
            tick_secs: std::env::var("LFG_TICK_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(2),
            dungeon,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dungeon_parses_with_default_orientation() {
        let d = Dungeon { id: 36, map_id: 389, x: 1.0, y: 2.0, z: 3.0, o: 0.0 };
        assert_eq!(d.map_id, 389);
        assert_eq!(d.o, 0.0);
    }
}
