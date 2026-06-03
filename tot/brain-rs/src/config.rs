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

// ---------------------------------------------------------------------------
// Settings struct
// ---------------------------------------------------------------------------

/// Env-based configuration (faithful port of brain_sidecar/settings.py).
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
}

// ---------------------------------------------------------------------------
// Env helpers
// ---------------------------------------------------------------------------

fn env_str(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn env_f64(key: &str, default: f64) -> f64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_u32(key: &str, default: u32) -> u32 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Parse a truthy env-var string (mirrors Python `_parse_bool`).
/// "1", "true", "yes" (case-insensitive) → true; anything else → false.
fn parse_bool(val: &str) -> bool {
    matches!(val.trim().to_lowercase().as_str(), "1" | "true" | "yes")
}

fn env_bool(key: &str, default: bool) -> bool {
    std::env::var(key)
        .map(|v| parse_bool(&v))
        .unwrap_or(default)
}

// ---------------------------------------------------------------------------
// impl Settings
// ---------------------------------------------------------------------------

impl Settings {
    /// Read env vars on each call. Tests can set env vars before calling.
    ///
    /// Does NOT validate `living_bot_count` — use `from_env_with_defaults_checked`
    /// for the validated variant (e.g. at application startup).
    pub fn from_env_with_defaults() -> Self {
        let living_bot_count = env_usize("TOT_LIVING_BOT_COUNT", 10);
        Settings {
            bind_host:                      env_str("BRAIN_BIND_HOST", "0.0.0.0"),
            bind_port:                      env_u32("BRAIN_BIND_PORT", 8091) as u16,
            state_db_path:                  env_str("BRAIN_STATE_DB", "/opt/containers/brain/state.sqlite"),
            decisions_log_path:             env_str("BRAIN_DECISIONS_LOG", "/opt/containers/brain/logs/decisions.jsonl"),
            harness_mcp_url:                env_str("HARNESS_MCP_URL", "http://localhost:8099/mcp/mcp"),
            memory_mcp_url:                 env_str("MEMORY_MCP_URL", "http://localhost:8090/mcp/mcp"),
            harness_bearer:                 env_str("HARNESS_BEARER", ""),
            memory_bearer:                  env_str("MEMORY_BEARER", ""),
            brain_bearer:                   env_str("BRAIN_BEARER", ""),
            llm_base_url:                   env_str("LLM_BASE_URL", "http://localhost:8080"),
            llm_model:                      env_str("LLM_MODEL", "qwen2.5-7b-instruct"),
            llm_timeout_s:                  env_f64("LLM_TIMEOUT_S", 60.0),
            tick_interval_s:                env_f64("BRAIN_TICK_INTERVAL_S", 5.0),
            personality_ttl_s:              env_f64("PERSONALITY_TTL_S", 300.0),
            brain_sse_enabled:              env_bool("BRAIN_SSE_ENABLED", true),
            brain_sse_coalesce_ms:          env_usize("BRAIN_SSE_COALESCE_MS", 200) as u64,
            brain_sse_dedup_capacity:       env_usize("BRAIN_SSE_DEDUP_CAPACITY", 100),
            max_player_level:               env_u32("BRAIN_MAX_PLAYER_LEVEL", 25),
            subset_gate_enabled:            env_bool("TOT_SUBSET_GATE_ENABLED", true),
            living_bot_count,
            subset_recompute_interval_s:    env_f64("TOT_SUBSET_RECOMPUTE_INTERVAL_S", 60.0),
            subset_hysteresis_out_ticks:    env_u32("TOT_SUBSET_HYSTERESIS_OUT_TICKS", 2),
            subset_hysteresis_in_ticks:     env_u32("TOT_SUBSET_HYSTERESIS_IN_TICKS", 1),
            subset_enroll_backoff_s:        env_f64("TOT_SUBSET_ENROLL_BACKOFF_S", 300.0),
            reduced_tick_interval_s:        env_f64("TOT_REDUCED_TICK_INTERVAL_S", 300.0),
        }
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
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defaults() {
        // Clear relevant env vars to test defaults
        let s = Settings::from_env_with_defaults();
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
    }

    #[test]
    fn test_living_bot_count_validation_rejects_out_of_range() {
        // TOT_LIVING_BOT_COUNT=4 → error; 5 → ok; 15 → ok; 16 → error
        std::env::set_var("TOT_LIVING_BOT_COUNT", "4");
        let err = Settings::from_env_with_defaults_checked();
        std::env::remove_var("TOT_LIVING_BOT_COUNT");
        assert!(err.is_err(), "should reject living_bot_count=4");

        std::env::set_var("TOT_LIVING_BOT_COUNT", "16");
        let err = Settings::from_env_with_defaults_checked();
        std::env::remove_var("TOT_LIVING_BOT_COUNT");
        assert!(err.is_err(), "should reject living_bot_count=16");
    }

    #[test]
    fn test_living_bot_count_validation_accepts_boundary() {
        std::env::set_var("TOT_LIVING_BOT_COUNT", "5");
        let s = Settings::from_env_with_defaults_checked().unwrap();
        std::env::remove_var("TOT_LIVING_BOT_COUNT");
        assert_eq!(s.living_bot_count, 5);

        std::env::set_var("TOT_LIVING_BOT_COUNT", "15");
        let s = Settings::from_env_with_defaults_checked().unwrap();
        std::env::remove_var("TOT_LIVING_BOT_COUNT");
        assert_eq!(s.living_bot_count, 15);
    }

    #[test]
    fn test_brain_bearer_empty_disables_auth() {
        std::env::set_var("BRAIN_BEARER", "");
        let s = Settings::from_env_with_defaults();
        std::env::remove_var("BRAIN_BEARER");
        assert_eq!(s.brain_bearer, "");
    }

    #[test]
    fn test_parse_bool_variants() {
        for v in &["1", "true", "True", "yes", "YES"] {
            std::env::set_var("BRAIN_SSE_ENABLED", v);
            let s = Settings::from_env_with_defaults();
            assert!(s.brain_sse_enabled, "expected true for {v}");
            std::env::remove_var("BRAIN_SSE_ENABLED");
        }
        for v in &["0", "false", "False", "no"] {
            std::env::set_var("BRAIN_SSE_ENABLED", v);
            let s = Settings::from_env_with_defaults();
            assert!(!s.brain_sse_enabled, "expected false for {v}");
            std::env::remove_var("BRAIN_SSE_ENABLED");
        }
    }
}
