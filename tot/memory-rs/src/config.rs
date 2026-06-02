//! Runtime configuration read from environment variables (v0.2.1 contract).
//!
//! All variables are optional; documented defaults are parity-critical (the live
//! Python memory-sidecar v0.2.1 uses the same defaults).
//!
//! | Variable              | Default                    | Field                    |
//! |-----------------------|----------------------------|--------------------------|
//! | `MEM_DB_PATH`         | `/var/memory/db.sqlite`    | `db_path`                |
//! | `MEM_EMBED_ENDPOINT`  | `http://127.0.0.1:8081`    | `embed_endpoint`         |
//! | `MEM_CAP_PER_BOT`     | `2000`                     | `cap_per_bot`            |
//! | `MEM_W_REL`           | `0.5`                      | `weights.w_rel`          |
//! | `MEM_W_REC`           | `0.2`                      | `weights.w_rec`          |
//! | `MEM_W_IMP`           | `0.3`                      | `weights.w_imp`          |
//! | `MEM_TAU_SECONDS`     | `604800`                   | `weights.tau_seconds`    |
//! | `MEM_W_REC_TIMESTAMP` | `created`                  | `recency_basis`          |
//! | `MEM_MMR_LAMBDA`      | `0.7`                      | `mmr_lambda`             |
//! | `MEM_TOKEN_STORE`     | `/etc/memory/tokens.yaml`  | `token_store`            |
//! | `MEM_BIND_HOST`       | `0.0.0.0`                  | `bind_host`              |
//! | `MEM_BIND_PORT`       | `8090`                     | `bind_port`              |
//! | `MEM_MIGRATIONS_DIR`  | `<crate>/migrations`       | `migrations_dir`         |

use std::path::PathBuf;
use std::str::FromStr;

// ---------------------------------------------------------------------------
// RecencyBasis
// ---------------------------------------------------------------------------

/// Which timestamp is used for the time-decay (recency) scoring component.
#[derive(Debug, Clone, PartialEq)]
pub enum RecencyBasis {
    /// Use the episode creation timestamp.
    Created,
    /// Use the timestamp of the most recent recall of the episode.
    LastRecalled,
}

impl FromStr for RecencyBasis {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "created" => Ok(RecencyBasis::Created),
            "last_recalled" => Ok(RecencyBasis::LastRecalled),
            other => Err(anyhow::anyhow!(
                "MEM_W_REC_TIMESTAMP must be `created` or `last_recalled`, got `{other}`"
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// ScoringWeights
// ---------------------------------------------------------------------------

/// Weights and decay constant for the hybrid scoring formula.
///
/// Score = w_rel·relevance + w_rec·recency_decay + w_imp·importance
/// where recency_decay = exp(−Δt / tau_seconds).
#[derive(Debug, Clone, PartialEq)]
pub struct ScoringWeights {
    /// Weight for the relevance (semantic similarity) component. Default 0.5.
    pub w_rel: f64,
    /// Weight for the recency-decay component. Default 0.2.
    pub w_rec: f64,
    /// Weight for the importance component. Default 0.3.
    pub w_imp: f64,
    /// Time-decay constant in seconds. Default 604800 (7 days).
    pub tau_seconds: u64,
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/// Full runtime configuration for `memory-rs`.
#[derive(Debug, Clone)]
pub struct Settings {
    /// Path to the single SQLite database file.
    pub db_path: PathBuf,
    /// Base URL of the embeddings service (no trailing path).
    pub embed_endpoint: String,
    /// Maximum number of episodes kept per bot before oldest are evicted.
    pub cap_per_bot: usize,
    /// Scoring weights + time-decay constant.
    pub weights: ScoringWeights,
    /// Which timestamp drives the recency component.
    pub recency_basis: RecencyBasis,
    /// MMR diversity trade-off: 1.0 = pure relevance, 0.0 = pure diversity.
    pub mmr_lambda: f64,
    /// Path to the token YAML file; `None` if the var was explicitly unset
    /// (empty string). Defaults to `/etc/memory/tokens.yaml`.
    pub token_store: Option<PathBuf>,
    /// IP address to bind the HTTP server on.
    pub bind_host: String,
    /// TCP port to bind the HTTP server on.
    pub bind_port: u16,
    /// Directory that contains the SQLite migration `.sql` files.
    pub migrations_dir: PathBuf,
    /// Additional hosts (beyond `127.0.0.1`, `localhost`, and `bind_host`) that
    /// the MCP DNS-rebind protection will accept in the HTTP `Host` header.
    ///
    /// Read from `MEM_EXTRA_ALLOWED_HOSTS` (comma-separated; default empty).
    /// At deploy the Quadlet sets:
    ///   `MEM_EXTRA_ALLOWED_HOSTS=192.168.1.3:8090,192.168.1.3`
    /// so the brain connecting via the LAN IP is not 403-rejected.
    pub extra_allowed_hosts: Vec<String>,
}

impl Settings {
    /// Read settings from the process environment.
    pub fn from_env() -> anyhow::Result<Settings> {
        Self::build(|k| std::env::var(k).ok())
    }

    /// Construct settings from an arbitrary key→value lookup.
    ///
    /// Separated from `from_env` so tests can inject values without touching
    /// the process environment (parallel-safe).
    pub(crate) fn build(get: impl Fn(&str) -> Option<String>) -> anyhow::Result<Settings> {
        // Helper: parse a var as a concrete type, applying a default when absent.
        let parse_or = |key: &str, default: &str| -> anyhow::Result<String> {
            Ok(get(key).unwrap_or_else(|| default.to_string()))
        };

        let parse_u64 = |key: &str, default: u64| -> anyhow::Result<u64> {
            match get(key) {
                None => Ok(default),
                Some(v) => v.parse::<u64>().map_err(|e| {
                    anyhow::anyhow!("{key} is not a valid u64: {e}")
                }),
            }
        };

        let parse_usize = |key: &str, default: usize| -> anyhow::Result<usize> {
            match get(key) {
                None => Ok(default),
                Some(v) => v.parse::<usize>().map_err(|e| {
                    anyhow::anyhow!("{key} is not a valid usize: {e}")
                }),
            }
        };

        let parse_f64 = |key: &str, default: f64| -> anyhow::Result<f64> {
            match get(key) {
                None => Ok(default),
                Some(v) => v.parse::<f64>().map_err(|e| {
                    anyhow::anyhow!("{key} is not a valid f64: {e}")
                }),
            }
        };

        let parse_u16 = |key: &str, default: u16| -> anyhow::Result<u16> {
            match get(key) {
                None => Ok(default),
                Some(v) => v.parse::<u16>().map_err(|e| {
                    anyhow::anyhow!("{key} is not a valid u16: {e}")
                }),
            }
        };

        // token_store: Some(path) if set to a non-empty string; None if set to
        // empty string; defaults to Some("/etc/memory/tokens.yaml").
        let token_store = match get("MEM_TOKEN_STORE").as_deref() {
            None => Some(PathBuf::from("/etc/memory/tokens.yaml")),
            Some("") => None,
            Some(p) => Some(PathBuf::from(p)),
        };

        // MEM_MIGRATIONS_DIR defaults to the compile-time crate root + "migrations".
        let migrations_dir = PathBuf::from(
            get("MEM_MIGRATIONS_DIR")
                .unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/migrations").to_string()),
        );

        let recency_basis_str = parse_or("MEM_W_REC_TIMESTAMP", "created")?;
        let recency_basis = recency_basis_str.parse::<RecencyBasis>()?;

        // MEM_EXTRA_ALLOWED_HOSTS: comma-separated extra hosts for MCP DNS-rebind protection.
        // e.g. "192.168.1.3:8090,192.168.1.3"  → allows the brain connecting via LAN IP.
        let extra_allowed_hosts: Vec<String> = get("MEM_EXTRA_ALLOWED_HOSTS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        Ok(Settings {
            db_path: PathBuf::from(parse_or("MEM_DB_PATH", "/var/memory/db.sqlite")?),
            embed_endpoint: parse_or("MEM_EMBED_ENDPOINT", "http://127.0.0.1:8081")?,
            cap_per_bot: parse_usize("MEM_CAP_PER_BOT", 2000)?,
            weights: ScoringWeights {
                w_rel: parse_f64("MEM_W_REL", 0.5)?,
                w_rec: parse_f64("MEM_W_REC", 0.2)?,
                w_imp: parse_f64("MEM_W_IMP", 0.3)?,
                tau_seconds: parse_u64("MEM_TAU_SECONDS", 604800)?,
            },
            recency_basis,
            mmr_lambda: parse_f64("MEM_MMR_LAMBDA", 0.7)?,
            token_store,
            bind_host: parse_or("MEM_BIND_HOST", "0.0.0.0")?,
            bind_port: parse_u16("MEM_BIND_PORT", 8090)?,
            migrations_dir,
            extra_allowed_hosts,
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn getter(map: HashMap<&'static str, &'static str>) -> impl Fn(&str) -> Option<String> {
        move |k| map.get(k).map(|s| s.to_string())
    }

    // An empty map exercises "all-defaults" mode (no env vars set).
    fn empty() -> HashMap<&'static str, &'static str> {
        HashMap::new()
    }

    // ---------------------------------------------------------------------------
    // Default values (nothing set → every field gets its documented default)
    // ---------------------------------------------------------------------------

    #[test]
    fn defaults_when_no_vars_set() {
        let s = Settings::build(getter(empty())).expect("build");
        assert_eq!(s.db_path, PathBuf::from("/var/memory/db.sqlite"));
        assert_eq!(s.embed_endpoint, "http://127.0.0.1:8081");
        assert_eq!(s.cap_per_bot, 2000);
        assert!((s.weights.w_rel - 0.5).abs() < f64::EPSILON, "w_rel");
        assert!((s.weights.w_rec - 0.2).abs() < f64::EPSILON, "w_rec");
        assert!((s.weights.w_imp - 0.3).abs() < f64::EPSILON, "w_imp");
        assert_eq!(s.weights.tau_seconds, 604800);
        assert_eq!(s.recency_basis, RecencyBasis::Created);
        assert!((s.mmr_lambda - 0.7).abs() < f64::EPSILON, "mmr_lambda");
        assert_eq!(
            s.token_store,
            Some(PathBuf::from("/etc/memory/tokens.yaml")),
            "token_store defaults to Some(/etc/memory/tokens.yaml)"
        );
        assert_eq!(s.bind_host, "0.0.0.0");
        assert_eq!(s.bind_port, 8090);
        // migrations_dir default is CARGO_MANIFEST_DIR/migrations — non-empty path.
        assert!(
            s.migrations_dir.to_string_lossy().ends_with("migrations"),
            "migrations_dir should end with 'migrations', got: {:?}",
            s.migrations_dir
        );
    }

    // ---------------------------------------------------------------------------
    // Non-default values (all MEM_* set to override values)
    // ---------------------------------------------------------------------------

    #[test]
    fn all_vars_override_defaults() {
        let m = HashMap::from([
            ("MEM_DB_PATH", "/custom/db.sqlite"),
            ("MEM_EMBED_ENDPOINT", "http://embed.internal:8888"),
            ("MEM_CAP_PER_BOT", "500"),
            ("MEM_W_REL", "0.6"),
            ("MEM_W_REC", "0.1"),
            ("MEM_W_IMP", "0.3"),
            ("MEM_TAU_SECONDS", "86400"),
            ("MEM_W_REC_TIMESTAMP", "last_recalled"),
            ("MEM_MMR_LAMBDA", "0.9"),
            ("MEM_TOKEN_STORE", "/run/secrets/tokens.yaml"),
            ("MEM_BIND_HOST", "127.0.0.1"),
            ("MEM_BIND_PORT", "9090"),
            ("MEM_MIGRATIONS_DIR", "/opt/migrations"),
        ]);

        let s = Settings::build(getter(m)).expect("build with all overrides");

        assert_eq!(s.db_path, PathBuf::from("/custom/db.sqlite"));
        assert_eq!(s.embed_endpoint, "http://embed.internal:8888");
        assert_eq!(s.cap_per_bot, 500);
        assert!((s.weights.w_rel - 0.6).abs() < f64::EPSILON, "w_rel");
        assert!((s.weights.w_rec - 0.1).abs() < f64::EPSILON, "w_rec");
        assert!((s.weights.w_imp - 0.3).abs() < f64::EPSILON, "w_imp");
        assert_eq!(s.weights.tau_seconds, 86400);
        assert_eq!(s.recency_basis, RecencyBasis::LastRecalled);
        assert!((s.mmr_lambda - 0.9).abs() < f64::EPSILON, "mmr_lambda");
        assert_eq!(
            s.token_store,
            Some(PathBuf::from("/run/secrets/tokens.yaml"))
        );
        assert_eq!(s.bind_host, "127.0.0.1");
        assert_eq!(s.bind_port, 9090);
        assert_eq!(s.migrations_dir, PathBuf::from("/opt/migrations"));
    }

    // ---------------------------------------------------------------------------
    // RecencyBasis parsing
    // ---------------------------------------------------------------------------

    #[test]
    fn recency_basis_created_parses() {
        let s = Settings::build(getter(HashMap::from([
            ("MEM_W_REC_TIMESTAMP", "created"),
        ])))
        .expect("build");
        assert_eq!(s.recency_basis, RecencyBasis::Created);
    }

    #[test]
    fn recency_basis_last_recalled_parses() {
        let s = Settings::build(getter(HashMap::from([
            ("MEM_W_REC_TIMESTAMP", "last_recalled"),
        ])))
        .expect("build");
        assert_eq!(s.recency_basis, RecencyBasis::LastRecalled);
    }

    #[test]
    fn recency_basis_invalid_returns_error() {
        let err = Settings::build(getter(HashMap::from([
            ("MEM_W_REC_TIMESTAMP", "wall_clock"),
        ])))
        .unwrap_err();
        assert!(
            err.to_string().contains("MEM_W_REC_TIMESTAMP"),
            "error must name the bad var; got: {err}"
        );
    }

    // ---------------------------------------------------------------------------
    // token_store: None when set to empty string
    // ---------------------------------------------------------------------------

    #[test]
    fn token_store_none_when_empty_string() {
        let s = Settings::build(getter(HashMap::from([
            ("MEM_TOKEN_STORE", ""),
        ])))
        .expect("build");
        assert_eq!(s.token_store, None, "empty string → None");
    }

    // ---------------------------------------------------------------------------
    // extra_allowed_hosts parsing
    // ---------------------------------------------------------------------------

    #[test]
    fn extra_allowed_hosts_empty_by_default() {
        let s = Settings::build(getter(empty())).expect("build");
        assert!(
            s.extra_allowed_hosts.is_empty(),
            "extra_allowed_hosts must be empty when MEM_EXTRA_ALLOWED_HOSTS is absent"
        );
    }

    #[test]
    fn extra_allowed_hosts_parses_comma_separated() {
        let s = Settings::build(getter(HashMap::from([
            ("MEM_EXTRA_ALLOWED_HOSTS", "192.168.1.3:8090,192.168.1.3"),
        ])))
        .expect("build");
        assert_eq!(
            s.extra_allowed_hosts,
            vec!["192.168.1.3:8090".to_string(), "192.168.1.3".to_string()],
        );
    }

    #[test]
    fn extra_allowed_hosts_trims_whitespace() {
        let s = Settings::build(getter(HashMap::from([
            ("MEM_EXTRA_ALLOWED_HOSTS", " 192.168.1.3:8090 , 192.168.1.3 "),
        ])))
        .expect("build");
        assert_eq!(
            s.extra_allowed_hosts,
            vec!["192.168.1.3:8090".to_string(), "192.168.1.3".to_string()],
        );
    }

    // ---------------------------------------------------------------------------
    // Parse error: bad bind_port
    // ---------------------------------------------------------------------------

    #[test]
    fn invalid_bind_port_returns_error() {
        let err = Settings::build(getter(HashMap::from([
            ("MEM_BIND_PORT", "not_a_number"),
        ])))
        .unwrap_err();
        assert!(
            err.to_string().contains("MEM_BIND_PORT"),
            "error must name the bad var; got: {err}"
        );
    }

    #[test]
    fn bind_port_out_of_u16_range_returns_error() {
        let err = Settings::build(getter(HashMap::from([
            ("MEM_BIND_PORT", "99999"),
        ])))
        .unwrap_err();
        assert!(
            err.to_string().contains("MEM_BIND_PORT"),
            "error must name the bad var; got: {err}"
        );
    }

    // ---------------------------------------------------------------------------
    // Parse error: bad numeric fields
    // ---------------------------------------------------------------------------

    #[test]
    fn invalid_cap_per_bot_returns_error() {
        let err = Settings::build(getter(HashMap::from([
            ("MEM_CAP_PER_BOT", "abc"),
        ])))
        .unwrap_err();
        assert!(err.to_string().contains("MEM_CAP_PER_BOT"), "got: {err}");
    }

    #[test]
    fn invalid_w_rel_returns_error() {
        let err = Settings::build(getter(HashMap::from([
            ("MEM_W_REL", "not_a_float"),
        ])))
        .unwrap_err();
        assert!(err.to_string().contains("MEM_W_REL"), "got: {err}");
    }
}
