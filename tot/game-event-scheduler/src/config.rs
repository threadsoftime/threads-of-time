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
    /// When `true`, the tick loop reconciles live world-event state against Rust's
    /// schedule computation by calling `event.start` / `event.stop` on divergences.
    ///
    /// **Default: `false` (shadow-only mode).**  Set `GES_DRIVE=true` to enable.
    /// Drive mode NEVER acts on excluded events (Internal / ManualStart / non-Normal).
    pub drive: bool,

    /// When `true`, the live shadow report includes CHECK 1 (DateMath) and CHECK 2
    /// (Resolution) diff results, and their mismatches contribute to `report.mismatches`.
    ///
    /// **Default: `false`.**  The native C++ date resolver (`SetHolidayEventTime`,
    /// `LoadHolidayDates`, `HolidayDateCalculator`) was deleted in GES Inc-3, so the
    /// C++ ground truth for holiday dates and resolved event Start/End is now raw/unresolved.
    /// Diffing Rust's correct resolved values against C++ raw values produces ~126 spurious
    /// mismatches on every tick, making the report misleading.  The active_set check still
    /// has valid C++ ground truth (the live active set) and is the sole live invariant.
    ///
    /// The date-math and resolution correctness are validated by deterministic unit tests
    /// (the ported `HolidayDateCalculatorTest` vectors + forward-date probes), which do not
    /// depend on live C++ state and are unaffected by this flag.
    ///
    /// Set `GES_LIVE_SHADOW_RESOLUTION=true` to re-enable if/when a future C++ resolver
    /// is added and live ground truth becomes valid again.
    pub live_shadow_resolution: bool,
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
    /// | `GES_DRIVE` | no | `false` |
    /// | `GES_LIVE_SHADOW_RESOLUTION` | no | `false` |
    ///
    /// `GES_DRIVE` and `GES_LIVE_SHADOW_RESOLUTION` accept `"true"` or `"1"`
    /// (case-insensitive) to enable.  Any other value is treated as `false`.
    ///
    /// `GES_LIVE_SHADOW_RESOLUTION` defaults to `false` because the C++ date resolver
    /// was deleted in GES Inc-3 — the C++ ground truth for holiday dates and resolved
    /// event Start/End is now raw/unresolved.  See `Config::live_shadow_resolution` for
    /// the full rationale.
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

        let drive = match std::env::var("GES_DRIVE") {
            Ok(s) => matches!(s.to_lowercase().as_str(), "true" | "1"),
            Err(_) => false,
        };

        // Default false: native C++ resolver deleted in GES Inc-3; live date/resolution
        // ground truth is no longer valid — active_set is the sole live invariant.
        let live_shadow_resolution = match std::env::var("GES_LIVE_SHADOW_RESOLUTION") {
            Ok(s) => matches!(s.to_lowercase().as_str(), "true" | "1"),
            Err(_) => false,
        };

        Ok(Config {
            listen_addr,
            harness_base_url,
            harness_bearer,
            tick_secs,
            drive,
            live_shadow_resolution,
        })
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

    #[test]
    fn drive_defaults_to_false_when_ges_drive_unset() {
        with_env(
            &[
                ("HARNESS_BASE_URL", Some("http://localhost:8099")),
                ("HARNESS_BEARER", Some("tok")),
                ("GES_LISTEN_ADDR", None),
                ("GES_TICK_SECS", None),
                ("GES_DRIVE", None),
            ],
            || {
                let cfg = Config::from_env().expect("should succeed");
                assert!(!cfg.drive, "drive must default to false when GES_DRIVE is unset");
            },
        );
    }

    #[test]
    fn drive_true_when_ges_drive_is_true_string() {
        with_env(
            &[
                ("HARNESS_BASE_URL", Some("http://localhost:8099")),
                ("HARNESS_BEARER", Some("tok")),
                ("GES_LISTEN_ADDR", None),
                ("GES_TICK_SECS", None),
                ("GES_DRIVE", Some("true")),
            ],
            || {
                let cfg = Config::from_env().expect("should succeed");
                assert!(cfg.drive, "drive must be true when GES_DRIVE=true");
            },
        );
    }

    #[test]
    fn drive_true_when_ges_drive_is_one() {
        with_env(
            &[
                ("HARNESS_BASE_URL", Some("http://localhost:8099")),
                ("HARNESS_BEARER", Some("tok")),
                ("GES_LISTEN_ADDR", None),
                ("GES_TICK_SECS", None),
                ("GES_DRIVE", Some("1")),
            ],
            || {
                let cfg = Config::from_env().expect("should succeed");
                assert!(cfg.drive, "drive must be true when GES_DRIVE=1");
            },
        );
    }

    #[test]
    fn drive_false_when_ges_drive_is_false_string() {
        with_env(
            &[
                ("HARNESS_BASE_URL", Some("http://localhost:8099")),
                ("HARNESS_BEARER", Some("tok")),
                ("GES_LISTEN_ADDR", None),
                ("GES_TICK_SECS", None),
                ("GES_DRIVE", Some("false")),
            ],
            || {
                let cfg = Config::from_env().expect("should succeed");
                assert!(!cfg.drive, "drive must be false when GES_DRIVE=false");
            },
        );
    }

    // ── GES_LIVE_SHADOW_RESOLUTION ────────────────────────────────────────────
    //
    // These tests mirror the GES_DRIVE tests above.  The default is false because
    // the native C++ date resolver was deleted in GES Inc-3; re-enable only if a
    // future resolver is added and live ground truth becomes valid again.

    #[test]
    fn live_shadow_resolution_defaults_to_false_when_unset() {
        with_env(
            &[
                ("HARNESS_BASE_URL", Some("http://localhost:8099")),
                ("HARNESS_BEARER", Some("tok")),
                ("GES_LISTEN_ADDR", None),
                ("GES_TICK_SECS", None),
                ("GES_DRIVE", None),
                ("GES_LIVE_SHADOW_RESOLUTION", None),
            ],
            || {
                let cfg = Config::from_env().expect("should succeed");
                assert!(
                    !cfg.live_shadow_resolution,
                    "live_shadow_resolution must default to false when GES_LIVE_SHADOW_RESOLUTION is unset"
                );
            },
        );
    }

    #[test]
    fn live_shadow_resolution_true_when_set_to_true_string() {
        with_env(
            &[
                ("HARNESS_BASE_URL", Some("http://localhost:8099")),
                ("HARNESS_BEARER", Some("tok")),
                ("GES_LISTEN_ADDR", None),
                ("GES_TICK_SECS", None),
                ("GES_LIVE_SHADOW_RESOLUTION", Some("true")),
            ],
            || {
                let cfg = Config::from_env().expect("should succeed");
                assert!(
                    cfg.live_shadow_resolution,
                    "live_shadow_resolution must be true when GES_LIVE_SHADOW_RESOLUTION=true"
                );
            },
        );
    }

    #[test]
    fn live_shadow_resolution_true_when_set_to_one() {
        with_env(
            &[
                ("HARNESS_BASE_URL", Some("http://localhost:8099")),
                ("HARNESS_BEARER", Some("tok")),
                ("GES_LISTEN_ADDR", None),
                ("GES_TICK_SECS", None),
                ("GES_LIVE_SHADOW_RESOLUTION", Some("1")),
            ],
            || {
                let cfg = Config::from_env().expect("should succeed");
                assert!(
                    cfg.live_shadow_resolution,
                    "live_shadow_resolution must be true when GES_LIVE_SHADOW_RESOLUTION=1"
                );
            },
        );
    }

    #[test]
    fn live_shadow_resolution_false_when_set_to_false_string() {
        with_env(
            &[
                ("HARNESS_BASE_URL", Some("http://localhost:8099")),
                ("HARNESS_BEARER", Some("tok")),
                ("GES_LISTEN_ADDR", None),
                ("GES_TICK_SECS", None),
                ("GES_LIVE_SHADOW_RESOLUTION", Some("false")),
            ],
            || {
                let cfg = Config::from_env().expect("should succeed");
                assert!(
                    !cfg.live_shadow_resolution,
                    "live_shadow_resolution must be false when GES_LIVE_SHADOW_RESOLUTION=false"
                );
            },
        );
    }
}
