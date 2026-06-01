//! YAML config loader and validation for harness-daemon.
//!
//! Port of `harness_daemon/config.py`. Invariants:
//! - `augmented` tokens are capped at `max_augmented_bots` (default 1).
//! - An `augmented` token MUST have `bound_to_guid` set.
//! - Token strings must be unique.

use std::collections::HashSet;
use std::path::Path;

use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Clone, Deserialize)]
pub struct TokenRecord {
    pub token: String,
    pub identity: String,
    pub scope: Vec<String>,
    #[serde(default)]
    pub augmented: bool,
    pub bound_to_guid: Option<i64>,
    #[allow(dead_code)] // parsed from tokens.yaml for operator docs; not used at runtime (matches Python)
    pub note: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DaemonConfig {
    #[serde(default = "default_max_augmented_bots")]
    pub max_augmented_bots: u32,
    pub ac_bridge_url: String,
    pub audit_path: String,
    pub listen_address: String,
    #[serde(default)]
    pub tokens: Vec<TokenRecord>,
}

fn default_max_augmented_bots() -> u32 {
    1
}

#[derive(Debug, Error)]
#[error("{0}")]
pub struct ConfigError(pub String);

/// Parse from an already-loaded YAML string and run invariants.
pub fn load_config_from_str(s: &str) -> Result<DaemonConfig, ConfigError> {
    let cfg: DaemonConfig = serde_yaml::from_str(s)
        .map_err(|e| ConfigError(e.to_string()))?;
    validate(cfg)
}

/// Read YAML from disk and parse.
pub fn load_config_from_path(path: impl AsRef<Path>) -> Result<DaemonConfig, ConfigError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| ConfigError(e.to_string()))?;
    load_config_from_str(&text)
}

fn validate(cfg: DaemonConfig) -> Result<DaemonConfig, ConfigError> {
    // Augmented tokens require bound_to_guid
    for (i, t) in cfg.tokens.iter().enumerate() {
        if t.augmented && t.bound_to_guid.is_none() {
            return Err(ConfigError(format!(
                "tokens[{i}]: augmented=true requires bound_to_guid"
            )));
        }
    }

    // Uniqueness check
    let mut seen: HashSet<&str> = HashSet::new();
    for t in &cfg.tokens {
        if !seen.insert(t.token.as_str()) {
            return Err(ConfigError(format!("duplicate token: '{}'", t.token)));
        }
    }

    // Augmented-bot cap
    let aug_count = cfg.tokens.iter().filter(|t| t.augmented).count() as u32;
    if aug_count > cfg.max_augmented_bots {
        let offenders: Vec<&str> = cfg.tokens.iter()
            .filter(|t| t.augmented)
            .map(|t| t.identity.as_str())
            .collect();
        return Err(ConfigError(format!(
            "augmented-bot cap exceeded: {aug_count} > max_augmented_bots={}; \
             offending identities: {offenders:?}",
            cfg.max_augmented_bots
        )));
    }

    Ok(cfg)
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_yaml() -> &'static str {
        r#"
max_augmented_bots: 1
ac_bridge_url: "http://127.0.0.1:8091"
audit_path: "/tmp/audit.jsonl"
listen_address: "0.0.0.0:8090"
tokens:
  - token: "tok-test-123"
    identity: "test.runner"
    scope:
      - "gm.*"
"#
    }

    // TDD test 1: minimal valid config
    #[test]
    fn loads_minimal_valid_config() {
        let cfg = load_config_from_str(minimal_yaml()).unwrap();
        assert_eq!(cfg.max_augmented_bots, 1);
        assert_eq!(cfg.ac_bridge_url, "http://127.0.0.1:8091");
        assert_eq!(cfg.listen_address, "0.0.0.0:8090");
        assert_eq!(cfg.tokens.len(), 1);
        assert_eq!(cfg.tokens[0].identity, "test.runner");
        assert_eq!(cfg.tokens[0].scope, vec!["gm.*"]);
        assert!(!cfg.tokens[0].augmented);
        assert!(cfg.tokens[0].bound_to_guid.is_none());
    }

    // Match test_config.py: scope ["gm.*", "obs.*"]
    #[test]
    fn loads_multi_scope_token() {
        let yaml = r#"
ac_bridge_url: "http://127.0.0.1:8091"
audit_path: "/tmp/audit.jsonl"
listen_address: "0.0.0.0:8090"
tokens:
  - token: "tok-test-123"
    identity: "test.runner"
    scope:
      - "gm.*"
      - "obs.*"
"#;
        let cfg = load_config_from_str(yaml).unwrap();
        assert_eq!(cfg.tokens[0].scope, vec!["gm.*", "obs.*"]);
        assert!(!cfg.tokens[0].augmented);
        assert!(cfg.tokens[0].bound_to_guid.is_none());
    }

    // Invariant: duplicate token strings → Err containing "duplicate"
    #[test]
    fn duplicate_tokens_rejected() {
        let yaml = r#"
ac_bridge_url: "http://127.0.0.1:8091"
audit_path: "/tmp/audit.jsonl"
listen_address: "0.0.0.0:8090"
tokens:
  - token: "same"
    identity: "a"
    scope: ["gm.*"]
  - token: "same"
    identity: "b"
    scope: ["gm.*"]
"#;
        let err = load_config_from_str(yaml).unwrap_err();
        assert!(err.0.contains("duplicate"), "expected 'duplicate' in: {}", err.0);
    }

    // Invariant: augmented=true without bound_to_guid → Err containing "bound_to_guid"
    #[test]
    fn augmented_requires_bound_to_guid() {
        let yaml = r#"
ac_bridge_url: "http://127.0.0.1:8091"
audit_path: "/tmp/audit.jsonl"
listen_address: "0.0.0.0:8090"
tokens:
  - token: "t1"
    identity: "bot.A"
    scope: ["bot.self.*"]
    augmented: true
"#;
        let err = load_config_from_str(yaml).unwrap_err();
        assert!(err.0.contains("bound_to_guid"), "expected 'bound_to_guid' in: {}", err.0);
    }

    // Invariant: count(augmented) > max_augmented_bots → Err containing "augmented"
    #[test]
    fn augmented_cap_rejects_excess() {
        let yaml = r#"
max_augmented_bots: 1
ac_bridge_url: "http://127.0.0.1:8091"
audit_path: "/tmp/audit.jsonl"
listen_address: "0.0.0.0:8090"
tokens:
  - token: "t1"
    identity: "bot.A"
    scope: ["bot.self.*"]
    augmented: true
    bound_to_guid: 1
  - token: "t2"
    identity: "bot.B"
    scope: ["bot.self.*"]
    augmented: true
    bound_to_guid: 2
"#;
        let err = load_config_from_str(yaml).unwrap_err();
        assert!(err.0.contains("augmented"), "expected 'augmented' in: {}", err.0);
    }

    // Allows augmented count == max_augmented_bots
    #[test]
    fn augmented_cap_allows_at_limit() {
        let yaml = r#"
max_augmented_bots: 2
ac_bridge_url: "http://127.0.0.1:8091"
audit_path: "/tmp/audit.jsonl"
listen_address: "0.0.0.0:8090"
tokens:
  - token: "t1"
    identity: "bot.A"
    scope: ["bot.self.*"]
    augmented: true
    bound_to_guid: 1
"#;
        let cfg = load_config_from_str(yaml).unwrap();
        assert_eq!(cfg.tokens.iter().filter(|t| t.augmented).count(), 1);
    }

    // Invariant: missing required key (no ac_bridge_url) → Err
    #[test]
    fn missing_required_field_ac_bridge_url() {
        let yaml = r#"
audit_path: "/tmp/audit.jsonl"
listen_address: "0.0.0.0:8090"
tokens: []
"#;
        let err = load_config_from_str(yaml);
        assert!(err.is_err());
    }

    // Invariant: token missing "token" field → Err containing "token"
    #[test]
    fn token_missing_token_field() {
        let yaml = r#"
ac_bridge_url: "http://127.0.0.1:8091"
audit_path: "/tmp/audit.jsonl"
listen_address: "0.0.0.0:8090"
tokens:
  - identity: "x"
    scope: ["gm.*"]
"#;
        let err = load_config_from_str(yaml);
        assert!(err.is_err());
        // The error must mention "token" (matching test_config.py's match="token")
        let msg = err.unwrap_err().0;
        assert!(msg.contains("token"), "expected 'token' in: {msg}");
    }

    // load_config_from_path round-trip
    #[test]
    fn load_from_path_round_trip() {
        let dir = std::env::temp_dir();
        let p = dir.join("harness_rs_test_config.yaml");
        std::fs::write(&p, minimal_yaml()).unwrap();
        let cfg = load_config_from_path(&p).unwrap();
        assert_eq!(cfg.tokens[0].token, "tok-test-123");
    }
}
