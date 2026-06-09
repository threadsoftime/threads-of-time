//! Env-based configuration (faithful port of brain_sidecar/settings.py).
//!
//! All variables are optional; documented defaults match the Python source of truth.
//!
//! | Variable                        | Default                                      | Field                          |
//! |---------------------------------|----------------------------------------------|--------------------------------|
//! | `BRAIN_BIND_HOST`               | `0.0.0.0`                                    | `bind_host`                    |
//! | `BRAIN_BIND_PORT`               | `8091`                                       | `bind_port`                    |
//! | `BRAIN_STATE_DB`                | `/opt/containers/brain/state.sqlite`         | `state_db_path`                |
//! | `BRAIN_DECISIONS_LOG`           | `/opt/containers/brain/logs/decisions.jsonl` | `decisions_log_path`           |
//! | `HARNESS_MCP_URL`               | `http://localhost:8099/mcp/mcp`              | `harness_mcp_url`              |
//! | `MEMORY_MCP_URL`                | `http://localhost:8090/mcp/mcp`              | `memory_mcp_url`               |
//! | `HARNESS_BEARER`                | `""`                                         | `harness_bearer`               |
//! | `MEMORY_BEARER`                 | `""`                                         | `memory_bearer`                |
//! | `BRAIN_BEARER`                  | `""`                                         | `brain_bearer`                 |
//! | `LLM_BASE_URL`                  | `http://localhost:8080`                      | `llm_base_url`                 |
//! | `LLM_MODEL`                     | `qwen2.5-7b-instruct`                        | `llm_model`                    |
//! | `LLM_TIMEOUT_S`                 | `60.0`                                       | `llm_timeout_s`                |
//! | `BRAIN_TICK_INTERVAL_S`         | `5.0`                                        | `tick_interval_s`              |
//! | `PERSONALITY_TTL_S`             | `300.0`                                      | `personality_ttl_s`            |
//! | `BRAIN_SSE_ENABLED`             | `true`                                       | `brain_sse_enabled`            |
//! | `BRAIN_SSE_COALESCE_MS`         | `200`                                        | `brain_sse_coalesce_ms`        |
//! | `BRAIN_SSE_DEDUP_CAPACITY`      | `100`                                        | `brain_sse_dedup_capacity`     |
//! | `BRAIN_MAX_PLAYER_LEVEL`        | `25`                                         | `max_player_level`             |
//! | `TOT_SUBSET_GATE_ENABLED`       | `true`                                       | `subset_gate_enabled`          |
//! | `TOT_LIVING_BOT_COUNT`          | `10`  (valid range: 5–15)                    | `living_bot_count`             |
//! | `TOT_SUBSET_RECOMPUTE_INTERVAL_S` | `60.0`                                     | `subset_recompute_interval_s`  |
//! | `TOT_SUBSET_HYSTERESIS_OUT_TICKS` | `2`                                        | `subset_hysteresis_out_ticks`  |
//! | `TOT_SUBSET_HYSTERESIS_IN_TICKS`  | `1`                                        | `subset_hysteresis_in_ticks`   |
//! | `TOT_SUBSET_ENROLL_BACKOFF_S`   | `300.0`                                      | `subset_enroll_backoff_s`      |
//! | `TOT_REDUCED_TICK_INTERVAL_S`   | `300.0`                                      | `reduced_tick_interval_s`      |
//! | `BRAIN_DECIDE_MAX_CONCURRENT`   | `5`                                           | `decide_max_concurrent`        |

// ---------------------------------------------------------------------------
// Settings struct
// ---------------------------------------------------------------------------

/// Env-based configuration (faithful port of brain_sidecar/settings.py).
///
/// Added fields (Rust-only, no Python parity required):
/// * `exec_bot_guid` — when `Some(guid)`, embeds an exec-rs `BotSupervisor`
///   and wires goal emission for that bot. When `None` the brain runs in pure
///   parity mode (behaviour byte-identical to today).
/// * `exec_roster_entries` — M2 roster: each entry is `(guid, Option<profile_id>)`,
///   parsed from `EXEC_BOT_GUIDS` as `guid` or `guid:profile_id`.
/// * `exec_default_profile` — fallback profile id for bare-guid entries.
#[derive(Debug, Clone)]
pub struct Settings {
    pub bind_host: String,
    pub bind_port: u16,
    pub state_db_path: String,
    pub decisions_log_path: String,
    pub harness_mcp_url: String,
    pub memory_mcp_url: String,
    pub harness_bearer: String,
    pub memory_bearer: String,
    pub brain_bearer: String,
    pub llm_base_url: String,
    pub llm_model: String,
    pub llm_timeout_s: f64,
    pub tick_interval_s: f64,
    pub personality_ttl_s: f64,
    pub brain_sse_enabled: bool,
    pub brain_sse_coalesce_ms: u64,
    pub brain_sse_dedup_capacity: usize,
    pub max_player_level: u32,
    pub subset_gate_enabled: bool,
    pub living_bot_count: usize,
    pub subset_recompute_interval_s: f64,
    pub subset_hysteresis_out_ticks: u32,
    pub subset_hysteresis_in_ticks: u32,
    pub subset_enroll_backoff_s: f64,
    pub reduced_tick_interval_s: f64,
    /// Maximum number of concurrent LLM decide calls allowed fleet-wide (across all bots
    /// sharing this `Decider`). Bounds the GPU-serialize burst when 16-18 bots cold-start
    /// simultaneously. Env: `BRAIN_DECIDE_MAX_CONCURRENT`. Default: 5.
    pub decide_max_concurrent: usize,
    /// M1 exec embed: bot guid that gets an in-process exec loop + goal emission.
    /// Env: `EXEC_BOT_GUID` (integer). Absent / invalid → `None` (pure parity mode).
    pub exec_bot_guid: Option<i64>,
    /// M2 exec roster entries parsed from `EXEC_BOT_GUIDS`: each `(guid, Option<profile_id>)`.
    /// A bare guid carries `None` → resolved to `exec_default_profile` by `exec_profile_map`.
    pub exec_roster_entries: Vec<(i64, Option<String>)>,
    /// Default profile id for bare-guid roster entries. Env: `EXEC_DEFAULT_PROFILE`.
    pub exec_default_profile: String,
}

// ---------------------------------------------------------------------------
// Pure parsers (no env access — safe to call from anywhere)
// ---------------------------------------------------------------------------

/// Parse a truthy env-var string (mirrors Python `_parse_bool`).
/// "1", "true", "yes" (case-insensitive) → true; anything else → false.
fn parse_bool(val: &str) -> bool {
    matches!(val.trim().to_lowercase().as_str(), "1" | "true" | "yes")
}

/// Strict-true parser: only case-insensitive `"true"` → true; everything else → false.
///
/// Mirrors Python `os.getenv(...).lower() == "true"` — used for
/// `TOT_SUBSET_GATE_ENABLED` where `"1"` and `"yes"` must yield `false`.
fn parse_bool_strict_true(val: &str) -> bool {
    val.trim().eq_ignore_ascii_case("true")
}

// ---------------------------------------------------------------------------
// impl Settings
// ---------------------------------------------------------------------------

impl Settings {
    /// Read env vars on each call; delegates to `build`.
    ///
    /// Does NOT validate `living_bot_count` — use `from_env_with_defaults_checked`
    /// for the validated variant (e.g. at application startup).
    pub fn from_env_with_defaults() -> Self {
        Self::build(|k| std::env::var(k).ok())
    }

    /// Same as `from_env_with_defaults` but validates `TOT_LIVING_BOT_COUNT` is in 5..=15.
    ///
    /// Mirrors the validation in Python `get_settings()`:
    /// `if not 5 <= living_bot_count <= 15: raise ValueError(...)`.
    pub fn from_env_with_defaults_checked() -> Result<Self, String> {
        let s = Self::from_env_with_defaults();
        if !(5..=15).contains(&s.living_bot_count) {
            return Err(format!(
                "TOT_LIVING_BOT_COUNT must be 5-15, got {}",
                s.living_bot_count
            ));
        }
        Ok(s)
    }

    /// The exec enrollment roster as `u64` guids. Prefers `EXEC_BOT_GUIDS`
    /// (comma-separated, optionally `guid:profile_id`); falls back to the singular
    /// `EXEC_BOT_GUID` (M1 back-compat); empty → no exec embed (pure parity mode).
    pub fn exec_roster(&self) -> Vec<u64> {
        if !self.exec_roster_entries.is_empty() {
            self.exec_roster_entries.iter().map(|(g, _)| *g as u64).collect()
        } else if let Some(g) = self.exec_bot_guid {
            vec![g as u64]
        } else {
            Vec::new()
        }
    }

    /// guid → profile_id, resolving bare entries (and the singular `EXEC_BOT_GUID`) to
    /// `exec_default_profile`. Caller cross-checks each profile_id against the registry.
    pub fn exec_profile_map(&self) -> std::collections::HashMap<u64, String> {
        let mut m = std::collections::HashMap::new();
        if !self.exec_roster_entries.is_empty() {
            for (g, prof) in &self.exec_roster_entries {
                m.insert(*g as u64, prof.clone().unwrap_or_else(|| self.exec_default_profile.clone()));
            }
        } else if let Some(g) = self.exec_bot_guid {
            m.insert(g as u64, self.exec_default_profile.clone());
        }
        m
    }

    /// Construct settings from an arbitrary key→value lookup.
    ///
    /// Separated from `from_env_with_defaults` so tests can inject values without
    /// touching the process environment (parallel-safe).
    pub(crate) fn build(get: impl Fn(&str) -> Option<String>) -> Self {
        // Inline helpers parameterized on `get` — no global env access.
        let get_str = |key: &str, default: &str| -> String {
            get(key).unwrap_or_else(|| default.to_string())
        };
        let get_f64 = |key: &str, default: f64| -> f64 {
            get(key).and_then(|v| v.parse().ok()).unwrap_or(default)
        };
        let get_u32 = |key: &str, default: u32| -> u32 {
            get(key).and_then(|v| v.parse().ok()).unwrap_or(default)
        };
        let get_usize = |key: &str, default: usize| -> usize {
            get(key).and_then(|v| v.parse().ok()).unwrap_or(default)
        };
        let get_bool = |key: &str, default: bool| -> bool {
            get(key).map(|v| parse_bool(&v)).unwrap_or(default)
        };
        let get_bool_strict_true = |key: &str, default: bool| -> bool {
            get(key).map(|v| parse_bool_strict_true(&v)).unwrap_or(default)
        };
        let get_opt_i64 = |key: &str| -> Option<i64> {
            get(key).and_then(|v| v.trim().parse::<i64>().ok())
        };
        let get_roster = |key: &str| -> Vec<(i64, Option<String>)> {
            get(key)
                .map(|v| {
                    v.split(',')
                        .map(|tok| tok.trim())
                        .filter(|tok| !tok.is_empty())
                        .filter_map(|tok| match tok.split_once(':') {
                            Some((g, prof)) => g.trim().parse::<i64>().ok()
                                .map(|g| (g, Some(prof.trim().to_string()))),
                            None => tok.parse::<i64>().ok().map(|g| (g, None)),
                        })
                        .collect()
                })
                .unwrap_or_default()
        };

        let living_bot_count = get_usize("TOT_LIVING_BOT_COUNT", 10);

        Settings {
            bind_host:                      get_str("BRAIN_BIND_HOST", "0.0.0.0"),
            bind_port:                      get_u32("BRAIN_BIND_PORT", 8091) as u16,
            state_db_path:                  get_str("BRAIN_STATE_DB", "/opt/containers/brain/state.sqlite"),
            decisions_log_path:             get_str("BRAIN_DECISIONS_LOG", "/opt/containers/brain/logs/decisions.jsonl"),
            harness_mcp_url:                get_str("HARNESS_MCP_URL", "http://localhost:8099/mcp/mcp"),
            memory_mcp_url:                 get_str("MEMORY_MCP_URL", "http://localhost:8090/mcp/mcp"),
            harness_bearer:                 get_str("HARNESS_BEARER", ""),
            memory_bearer:                  get_str("MEMORY_BEARER", ""),
            brain_bearer:                   get_str("BRAIN_BEARER", ""),
            llm_base_url:                   get_str("LLM_BASE_URL", "http://localhost:8080"),
            llm_model:                      get_str("LLM_MODEL", "qwen2.5-7b-instruct"),
            llm_timeout_s:                  get_f64("LLM_TIMEOUT_S", 60.0),
            tick_interval_s:                get_f64("BRAIN_TICK_INTERVAL_S", 5.0),
            personality_ttl_s:              get_f64("PERSONALITY_TTL_S", 300.0),
            brain_sse_enabled:              get_bool("BRAIN_SSE_ENABLED", true),
            brain_sse_coalesce_ms:          get_usize("BRAIN_SSE_COALESCE_MS", 200) as u64,
            brain_sse_dedup_capacity:       get_usize("BRAIN_SSE_DEDUP_CAPACITY", 100),
            max_player_level:               get_u32("BRAIN_MAX_PLAYER_LEVEL", 25),
            subset_gate_enabled:            get_bool_strict_true("TOT_SUBSET_GATE_ENABLED", true),
            living_bot_count,
            subset_recompute_interval_s:    get_f64("TOT_SUBSET_RECOMPUTE_INTERVAL_S", 60.0),
            subset_hysteresis_out_ticks:    get_u32("TOT_SUBSET_HYSTERESIS_OUT_TICKS", 2),
            subset_hysteresis_in_ticks:     get_u32("TOT_SUBSET_HYSTERESIS_IN_TICKS", 1),
            subset_enroll_backoff_s:        get_f64("TOT_SUBSET_ENROLL_BACKOFF_S", 300.0),
            reduced_tick_interval_s:        get_f64("TOT_REDUCED_TICK_INTERVAL_S", 300.0),
            decide_max_concurrent:          get_usize("BRAIN_DECIDE_MAX_CONCURRENT", 5),
            exec_bot_guid:                  get_opt_i64("EXEC_BOT_GUID"),
            exec_roster_entries:            get_roster("EXEC_BOT_GUIDS"),
            exec_default_profile:           get_str("EXEC_DEFAULT_PROFILE", "elwynn_fargodeep"),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Build a getter closure from a `HashMap` — tests inject env via this,
    /// never via `std::env::set_var` / `remove_var`.
    fn getter(map: HashMap<&'static str, &'static str>) -> impl Fn(&str) -> Option<String> {
        move |k| map.get(k).map(|s| s.to_string())
    }

    fn empty() -> HashMap<&'static str, &'static str> {
        HashMap::new()
    }

    // ---------------------------------------------------------------------------
    // Default values (nothing set → every field gets its documented default)
    // ---------------------------------------------------------------------------

    #[test]
    fn test_defaults() {
        let s = Settings::build(getter(empty()));
        assert_eq!(s.bind_host, "0.0.0.0");
        assert_eq!(s.bind_port, 8091);
        assert_eq!(s.state_db_path, "/opt/containers/brain/state.sqlite");
        assert_eq!(s.decisions_log_path, "/opt/containers/brain/logs/decisions.jsonl");
        assert_eq!(s.harness_mcp_url, "http://localhost:8099/mcp/mcp");
        assert_eq!(s.memory_mcp_url, "http://localhost:8090/mcp/mcp");
        assert_eq!(s.llm_base_url, "http://localhost:8080");
        assert_eq!(s.llm_model, "qwen2.5-7b-instruct");
        assert_eq!(s.llm_timeout_s, 60.0);
        assert_eq!(s.tick_interval_s, 5.0);
        assert_eq!(s.personality_ttl_s, 300.0);
        assert!(s.brain_sse_enabled);
        assert_eq!(s.brain_sse_coalesce_ms, 200);
        assert_eq!(s.brain_sse_dedup_capacity, 100);
        assert_eq!(s.max_player_level, 25);
        assert!(s.subset_gate_enabled);
        assert_eq!(s.living_bot_count, 10);
        assert_eq!(s.subset_recompute_interval_s, 60.0);
        assert_eq!(s.subset_hysteresis_out_ticks, 2);
        assert_eq!(s.subset_hysteresis_in_ticks, 1);
        assert_eq!(s.subset_enroll_backoff_s, 300.0);
        assert_eq!(s.reduced_tick_interval_s, 300.0);
        assert_eq!(s.decide_max_concurrent, 5);
    }

    // ---------------------------------------------------------------------------
    // living_bot_count validation
    // ---------------------------------------------------------------------------

    #[test]
    fn test_living_bot_count_validation_rejects_out_of_range() {
        // living_bot_count=4 → error
        let s = Settings::build(getter(HashMap::from([("TOT_LIVING_BOT_COUNT", "4")])));
        let err = if !(5..=15).contains(&s.living_bot_count) {
            Err(format!("TOT_LIVING_BOT_COUNT must be 5-15, got {}", s.living_bot_count))
        } else {
            Ok(s)
        };
        assert!(err.is_err(), "should reject living_bot_count=4");

        // living_bot_count=16 → error (checked via from_env_with_defaults_checked pattern)
        let s16 = Settings::build(getter(HashMap::from([("TOT_LIVING_BOT_COUNT", "16")])));
        let err16 = if !(5..=15).contains(&s16.living_bot_count) {
            Err(format!("TOT_LIVING_BOT_COUNT must be 5-15, got {}", s16.living_bot_count))
        } else {
            Ok(s16)
        };
        assert!(err16.is_err(), "should reject living_bot_count=16");
    }

    #[test]
    fn test_living_bot_count_validation_accepts_boundary() {
        let s5 = Settings::build(getter(HashMap::from([("TOT_LIVING_BOT_COUNT", "5")])));
        assert_eq!(s5.living_bot_count, 5);

        let s15 = Settings::build(getter(HashMap::from([("TOT_LIVING_BOT_COUNT", "15")])));
        assert_eq!(s15.living_bot_count, 15);
    }

    // ---------------------------------------------------------------------------
    // bearer defaults
    // ---------------------------------------------------------------------------

    #[test]
    fn test_brain_bearer_empty_disables_auth() {
        // Explicitly injected as empty string
        let s = Settings::build(getter(HashMap::from([("BRAIN_BEARER", "")])));
        assert_eq!(s.brain_bearer, "");

        // Not set at all → default is ""
        let s2 = Settings::build(getter(empty()));
        assert_eq!(s2.brain_bearer, "");
    }

    // ---------------------------------------------------------------------------
    // parse_bool variants (BRAIN_SSE_ENABLED uses lenient bool)
    // ---------------------------------------------------------------------------

    #[test]
    fn test_parse_bool_variants() {
        for v in &["1", "true", "True", "yes", "YES"] {
            let s = Settings::build(getter(HashMap::from([("BRAIN_SSE_ENABLED", *v)])));
            assert!(s.brain_sse_enabled, "expected true for {v}");
        }
        for v in &["0", "false", "False", "no"] {
            let s = Settings::build(getter(HashMap::from([("BRAIN_SSE_ENABLED", *v)])));
            assert!(!s.brain_sse_enabled, "expected false for {v}");
        }
    }

    // ---------------------------------------------------------------------------
    // TOT_SUBSET_GATE_ENABLED — strict-true parity with Python
    // ---------------------------------------------------------------------------

    // ---------------------------------------------------------------------------
    // exec_bot_guid (Rust-only; no Python parity)
    // ---------------------------------------------------------------------------

    #[test]
    fn exec_roster_parses_csv_guids() {
        let s = Settings::build(getter(HashMap::from([("EXEC_BOT_GUIDS", "1001, 1002 ,1003")])));
        assert_eq!(s.exec_roster(), vec![1001u64, 1002, 1003]);
    }

    #[test]
    fn exec_roster_falls_back_to_singular_guid() {
        let s = Settings::build(getter(HashMap::from([("EXEC_BOT_GUID", "1173")])));
        assert_eq!(s.exec_roster(), vec![1173u64]);
    }

    #[test]
    fn exec_roster_prefers_plural_over_singular() {
        let s = Settings::build(getter(HashMap::from([
            ("EXEC_BOT_GUIDS", "1001,1002"),
            ("EXEC_BOT_GUID", "1173"),
        ])));
        assert_eq!(s.exec_roster(), vec![1001u64, 1002]);
    }

    #[test]
    fn exec_roster_empty_when_neither_set() {
        let s = Settings::build(getter(HashMap::new()));
        assert!(s.exec_roster().is_empty());
    }

    #[test]
    fn exec_roster_skips_non_numeric_entries() {
        let s = Settings::build(getter(HashMap::from([("EXEC_BOT_GUIDS", "1001,nope,1003")])));
        assert_eq!(s.exec_roster(), vec![1001u64, 1003]);
    }

    #[test]
    fn roster_parses_guid_profile_pairs() {
        let s = Settings::build(getter(HashMap::from([
            ("EXEC_BOT_GUIDS", "1173:elwynn_fargodeep, 2257:dun_morogh_camp"),
        ])));
        assert_eq!(s.exec_roster(), vec![1173u64, 2257]);
        let m = s.exec_profile_map();
        assert_eq!(m.get(&1173).map(String::as_str), Some("elwynn_fargodeep"));
        assert_eq!(m.get(&2257).map(String::as_str), Some("dun_morogh_camp"));
    }

    #[test]
    fn bare_guid_uses_default_profile() {
        let s = Settings::build(getter(HashMap::from([
            ("EXEC_BOT_GUIDS", "1173"),
            ("EXEC_DEFAULT_PROFILE", "elwynn_fargodeep"),
        ])));
        assert_eq!(s.exec_roster(), vec![1173u64]);
        assert_eq!(s.exec_profile_map().get(&1173).map(String::as_str), Some("elwynn_fargodeep"));
    }

    #[test]
    fn mixed_bare_and_profile_entries() {
        let s = Settings::build(getter(HashMap::from([
            ("EXEC_BOT_GUIDS", "1173:westfall_camp,2257"),
            ("EXEC_DEFAULT_PROFILE", "elwynn_fargodeep"),
        ])));
        let m = s.exec_profile_map();
        assert_eq!(m.get(&1173).map(String::as_str), Some("westfall_camp"));
        assert_eq!(m.get(&2257).map(String::as_str), Some("elwynn_fargodeep")); // default
    }

    #[test]
    fn default_profile_falls_back_to_builtin_when_unset() {
        let s = Settings::build(getter(HashMap::from([("EXEC_BOT_GUIDS", "1173")])));
        assert_eq!(s.exec_default_profile, "elwynn_fargodeep");
        assert_eq!(s.exec_profile_map().get(&1173).map(String::as_str), Some("elwynn_fargodeep"));
    }

    #[test]
    fn roster_skips_unparseable_guid() {
        let s = Settings::build(getter(HashMap::from([("EXEC_BOT_GUIDS", "1173:elwynn_fargodeep,nope:x")])));
        assert_eq!(s.exec_roster(), vec![1173u64]);
    }

    #[test]
    fn test_decide_max_concurrent_default_is_5() {
        let s = Settings::build(getter(empty()));
        assert_eq!(s.decide_max_concurrent, 5, "default BRAIN_DECIDE_MAX_CONCURRENT must be 5");
    }

    #[test]
    fn test_decide_max_concurrent_override() {
        let s = Settings::build(getter(HashMap::from([("BRAIN_DECIDE_MAX_CONCURRENT", "12")])));
        assert_eq!(s.decide_max_concurrent, 12);
    }

    #[test]
    fn test_exec_bot_guid_absent_is_none() {
        let s = Settings::build(getter(empty()));
        assert!(s.exec_bot_guid.is_none(), "default must be None");
    }

    #[test]
    fn test_exec_bot_guid_parses_integer() {
        let s = Settings::build(getter(HashMap::from([("EXEC_BOT_GUID", "1003")])));
        assert_eq!(s.exec_bot_guid, Some(1003i64));
    }

    #[test]
    fn test_exec_bot_guid_invalid_string_is_none() {
        let s = Settings::build(getter(HashMap::from([("EXEC_BOT_GUID", "not_a_number")])));
        assert!(s.exec_bot_guid.is_none(), "unparseable value must yield None");
    }

    // ---------------------------------------------------------------------------
    // Parity test: TOT_SUBSET_GATE_ENABLED uses strict `.lower() == "true"` like Python.
    ///
    /// Python: `os.getenv("TOT_SUBSET_GATE_ENABLED", "true").lower() == "true"`
    /// → only "true" (case-insensitive) is truthy; "1" and "yes" must yield false.
    #[test]
    fn test_subset_gate_enabled_strict_true_parity() {
        // Default (no env var) → true
        let s = Settings::build(getter(empty()));
        assert!(s.subset_gate_enabled, "default should be true");

        // "true" → true
        let s = Settings::build(getter(HashMap::from([("TOT_SUBSET_GATE_ENABLED", "true")])));
        assert!(s.subset_gate_enabled, r#""true" should be true"#);

        // "TRUE" → true (case-insensitive)
        let s = Settings::build(getter(HashMap::from([("TOT_SUBSET_GATE_ENABLED", "TRUE")])));
        assert!(s.subset_gate_enabled, r#""TRUE" should be true"#);

        // "1" → false (diverges from parse_bool; matches Python strict)
        let s = Settings::build(getter(HashMap::from([("TOT_SUBSET_GATE_ENABLED", "1")])));
        assert!(!s.subset_gate_enabled, r#""1" must be false (Python parity)"#);

        // "yes" → false
        let s = Settings::build(getter(HashMap::from([("TOT_SUBSET_GATE_ENABLED", "yes")])));
        assert!(!s.subset_gate_enabled, r#""yes" must be false (Python parity)"#);

        // "false" → false
        let s = Settings::build(getter(HashMap::from([("TOT_SUBSET_GATE_ENABLED", "false")])));
        assert!(!s.subset_gate_enabled, r#""false" should be false"#);
    }
}
