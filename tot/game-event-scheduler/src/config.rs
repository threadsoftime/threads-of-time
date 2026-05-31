//! Configuration for the game-event-scheduler process.
//!
//! All configuration is read from environment variables. Unset required variables
//! produce a descriptive `Err(String)` — call `Config::from_env()` early in `main`
//! and exit with code 2 on failure.

/// Runtime configuration for the game-event-scheduler process.
#[derive(Debug, Clone)]
pub struct Config {
    /// Bind address for the HTTP report API (default: `127.0.0.1:8091`).
    pub listen_addr: String,
    /// Base URL of the harness (no trailing slash, e.g. `http://192.168.1.3:8099`).
    pub harness_base_url: String,
    /// Raw bearer token (without the `Bearer ` prefix).
    pub harness_bearer: String,
    /// Tick interval in seconds (default: 15; clamped to minimum 1 at use-site).
    pub tick_secs: u64,
}

impl Config {
    /// Read configuration from environment variables.
    ///
    /// | Variable | Required | Default |
    /// |---|---|---|
    /// | `GES_LISTEN_ADDR` | no | `127.0.0.1:8091` |
    /// | `HARNESS_BASE_URL` | yes | — |
    /// | `HARNESS_BEARER` | yes | — |
    /// | `GES_TICK_SECS` | no | `15` |
    ///
    /// Returns `Err(String)` with a human-readable message for any missing required
    /// variable or parse failure.
    pub fn from_env() -> Result<Config, String> {
        let listen_addr =
            std::env::var("GES_LISTEN_ADDR").unwrap_or_else(|_| "127.0.0.1:8091".to_string());

        let harness_base_url = std::env::var("HARNESS_BASE_URL")
            .map_err(|_| "HARNESS_BASE_URL is required but not set".to_string())?;

        let harness_bearer = std::env::var("HARNESS_BEARER")
            .map_err(|_| "HARNESS_BEARER is required but not set".to_string())?;

        let tick_secs = match std::env::var("GES_TICK_SECS") {
            Ok(s) => s
                .parse::<u64>()
                .map_err(|e| format!("GES_TICK_SECS is not a valid u64: {e}"))?,
            Err(_) => 15,
        };

        Ok(Config { listen_addr, harness_base_url, harness_bearer, tick_secs })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    // `std::env::set_var` / `remove_var` are process-global and data-race-prone
    // when tests run in parallel.  We serialize all env-touching tests behind a
    // module-level mutex so they are safe even under `RUST_TEST_THREADS` > 1.
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn env_lock() -> &'static Mutex<()> {
        ENV_LOCK.get_or_init(|| Mutex::new(()))
    }

    // Helper that temporarily sets env vars for a test closure, restoring them
    // afterward.  Serializes via `ENV_LOCK` to avoid inter-test interference.
    fn with_env<F: FnOnce()>(vars: &[(&str, Option<&str>)], f: F) {
        let _guard = env_lock().lock().unwrap();

        // Save originals.
        let saved: Vec<(&str, Option<String>)> =
            vars.iter().map(|(k, _)| (*k, std::env::var(k).ok())).collect();

        // Set / unset.
        for (k, v) in vars {
            match v {
                Some(val) => std::env::set_var(k, val),
                None => std::env::remove_var(k),
            }
        }

        f();

        // Restore.
        for (k, orig) in &saved {
            match orig {
                Some(val) => std::env::set_var(k, val),
                None => std::env::remove_var(k),
            }
        }
    }

    #[test]
    fn required_vars_missing_returns_err() {
        with_env(
            &[
                ("HARNESS_BASE_URL", None),
                ("HARNESS_BEARER", None),
                ("GES_LISTEN_ADDR", None),
                ("GES_TICK_SECS", None),
            ],
            || {
                let result = Config::from_env();
                assert!(result.is_err(), "should fail when required vars are missing");
                let msg = result.unwrap_err();
                assert!(
                    msg.contains("HARNESS_BASE_URL"),
                    "error message should mention the missing var: {msg}"
                );
            },
        );
    }

    #[test]
    fn bearer_missing_returns_err() {
        with_env(
            &[
                ("HARNESS_BASE_URL", Some("http://localhost:8099")),
                ("HARNESS_BEARER", None),
                ("GES_LISTEN_ADDR", None),
                ("GES_TICK_SECS", None),
            ],
            || {
                let result = Config::from_env();
                assert!(result.is_err());
                assert!(result.unwrap_err().contains("HARNESS_BEARER"));
            },
        );
    }

    #[test]
    fn defaults_applied_when_optional_vars_unset() {
        with_env(
            &[
                ("HARNESS_BASE_URL", Some("http://localhost:8099")),
                ("HARNESS_BEARER", Some("tok")),
                ("GES_LISTEN_ADDR", None),
                ("GES_TICK_SECS", None),
            ],
            || {
                let cfg = Config::from_env().expect("should succeed with required vars");
                assert_eq!(cfg.listen_addr, "127.0.0.1:8091");
                assert_eq!(cfg.tick_secs, 15);
            },
        );
    }

    #[test]
    fn all_vars_set_correctly_parsed() {
        with_env(
            &[
                ("HARNESS_BASE_URL", Some("http://192.168.1.3:8099")),
                ("HARNESS_BEARER", Some("mytoken")),
                ("GES_LISTEN_ADDR", Some("0.0.0.0:9000")),
                ("GES_TICK_SECS", Some("30")),
            ],
            || {
                let cfg = Config::from_env().expect("should succeed");
                assert_eq!(cfg.harness_base_url, "http://192.168.1.3:8099");
                assert_eq!(cfg.harness_bearer, "mytoken");
                assert_eq!(cfg.listen_addr, "0.0.0.0:9000");
                assert_eq!(cfg.tick_secs, 30);
            },
        );
    }

    #[test]
    fn invalid_tick_secs_returns_err() {
        with_env(
            &[
                ("HARNESS_BASE_URL", Some("http://localhost:8099")),
                ("HARNESS_BEARER", Some("tok")),
                ("GES_LISTEN_ADDR", None),
                ("GES_TICK_SECS", Some("notanumber")),
            ],
            || {
                let result = Config::from_env();
                assert!(result.is_err());
                assert!(result.unwrap_err().contains("GES_TICK_SECS"));
            },
        );
    }
}
