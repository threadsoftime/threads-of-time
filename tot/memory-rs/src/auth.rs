//! Bearer-token store for MCP/SSE auth.
//!
//! Mirrors Python `mcp_auth.py::TokenStore`.  The REST routes (`/memory/*`,
//! `/goals/*`) are **not** protected — only the MCP/SSE transport uses this
//! store (matching the Python implementation exactly: the REST routes in
//! `routes_memory.py`, `routes_personality.py`, and `routes_goals.py` have
//! no auth dependency).
//!
//! YAML shape (same as Python):
//! ```yaml
//! tokens:
//!   - token: "some-secret-token"
//!     identity: "brain-sidecar"
//!     scope: ["memory.read", "memory.write"]
//! ```
//!
//! `TokenStore::load` returns `None` when the file is missing or unreadable,
//! matching the Python `create_app` behaviour where MCP is silently disabled
//! when no token file exists:
//! ```python
//! if Path(token_path).exists():
//!     token_store = TokenStore.load_yaml(token_path)
//! else:
//!     print("[mcp] no token store ...")
//! ```

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A single token record loaded from the YAML file.
#[derive(Debug, Clone)]
pub struct TokenRecord {
    pub token: String,
    pub identity: String,
    pub scope: Vec<String>,
}

/// In-memory token store, indexed by token string for O(1) lookup.
pub struct TokenStore {
    by_token: HashMap<String, TokenRecord>,
}

// ---------------------------------------------------------------------------
// YAML deserialization helpers (not pub — only used in load)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct TokenEntry {
    token: String,
    identity: String,
    #[serde(default)]
    scope: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct TokenFile {
    #[serde(default)]
    tokens: Vec<TokenEntry>,
}

// ---------------------------------------------------------------------------
// TokenStore impl
// ---------------------------------------------------------------------------

impl TokenStore {
    /// Load a `TokenStore` from a YAML file at `path`.
    ///
    /// Returns `None` when the file does not exist or cannot be read/parsed —
    /// matches Python's conditional check before calling `TokenStore.load_yaml`.
    ///
    /// Returns `Some(TokenStore)` on success (even if the file has zero tokens).
    pub fn load(path: &Path) -> Option<TokenStore> {
        let content = std::fs::read_to_string(path).ok()?;
        let file: TokenFile = serde_yaml::from_str(&content).ok()?;
        let by_token = file
            .tokens
            .into_iter()
            .map(|e| {
                (
                    e.token.clone(),
                    TokenRecord {
                        token: e.token,
                        identity: e.identity,
                        scope: e.scope,
                    },
                )
            })
            .collect();
        Some(TokenStore { by_token })
    }

    /// Look up a token string.
    ///
    /// Returns `Some(&TokenRecord)` when found, `None` otherwise.
    /// Mirrors Python `TokenStore.find`.
    pub fn verify(&self, token: &str) -> Option<&TokenRecord> {
        self.by_token.get(token)
    }
}

impl tot_mcp_bearer_auth::TokenLookup for TokenStore {
    type Record = TokenRecord;
    fn lookup(&self, token: &str) -> Option<&Self::Record> {
        self.verify(token)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Write a temp YAML file and return its path via a NamedTempFile guard.
    fn write_yaml(content: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().expect("tempfile");
        f.write_all(content.as_bytes()).expect("write yaml");
        f
    }

    // T1: load a valid YAML → verify good token → get expected fields.
    #[test]
    fn load_and_verify_good_token() {
        let yaml = r#"
tokens:
  - token: "secret-abc"
    identity: "brain-sidecar"
    scope: ["memory.read", "memory.write"]
  - token: "secret-xyz"
    identity: "observer"
    scope: []
"#;
        let f = write_yaml(yaml);
        let store = TokenStore::load(f.path()).expect("should load");

        let rec = store.verify("secret-abc").expect("token must be found");
        assert_eq!(rec.token, "secret-abc");
        assert_eq!(rec.identity, "brain-sidecar");
        assert_eq!(rec.scope, vec!["memory.read", "memory.write"]);

        let rec2 = store.verify("secret-xyz").expect("second token must be found");
        assert_eq!(rec2.identity, "observer");
        assert!(rec2.scope.is_empty());
    }

    // T2: verify a token that is not in the store → None.
    #[test]
    fn verify_unknown_token_returns_none() {
        let yaml = "tokens:\n  - token: \"abc\"\n    identity: \"x\"\n";
        let f = write_yaml(yaml);
        let store = TokenStore::load(f.path()).expect("should load");
        assert!(store.verify("not-a-token").is_none());
    }

    // T3: file missing → None.
    #[test]
    fn missing_file_returns_none() {
        let result = TokenStore::load(Path::new("/tmp/this-file-does-not-exist-memory-rs.yaml"));
        assert!(result.is_none());
    }

    // T4: empty tokens list → load succeeds, verify anything → None.
    #[test]
    fn empty_tokens_list_loads_ok() {
        let yaml = "tokens: []\n";
        let f = write_yaml(yaml);
        let store = TokenStore::load(f.path()).expect("should load");
        assert!(store.verify("anything").is_none());
    }

    // T5: scope key absent → defaults to empty vec.
    #[test]
    fn missing_scope_defaults_to_empty() {
        let yaml = "tokens:\n  - token: \"t\"\n    identity: \"id\"\n";
        let f = write_yaml(yaml);
        let store = TokenStore::load(f.path()).expect("should load");
        let rec = store.verify("t").expect("token");
        assert!(rec.scope.is_empty());
    }

    // T6: malformed YAML (invalid syntax) → None (graceful degradation).
    #[test]
    fn malformed_yaml_returns_none() {
        let yaml = "tokens: [{{not valid yaml";
        let f = write_yaml(yaml);
        let result = TokenStore::load(f.path());
        assert!(result.is_none());
    }
}
