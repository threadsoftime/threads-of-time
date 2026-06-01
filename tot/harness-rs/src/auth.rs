//! Bearer auth + scope-glob + self-binding checks.
//!
//! Port of `harness_daemon/auth.py`. Spec §7.3 + §7.4.

use std::collections::HashMap;

use thiserror::Error;

use crate::config::TokenRecord;

// ── types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct AuthResult {
    pub identity: String,
    pub scope: Vec<String>,
    pub bound_to_guid: Option<i64>,
    pub augmented: bool,
}

/// O(1) lookup over a list of TokenRecord.
pub struct TokenStore {
    by_token: HashMap<String, TokenRecord>,
}

impl TokenStore {
    pub fn new(tokens: Vec<TokenRecord>) -> Self {
        let by_token = tokens.into_iter().map(|t| (t.token.clone(), t)).collect();
        Self { by_token }
    }

    pub fn find(&self, token: &str) -> Option<&TokenRecord> {
        self.by_token.get(token)
    }
}

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("unauthorized")]
    Unauthorized,
}

// ── authenticate_bearer ───────────────────────────────────────────────────────

/// Parse `Authorization: Bearer <token>` and resolve the TokenRecord.
///
/// Raises `AuthError::Unauthorized` for any failure — never leaks
/// whether the token was unknown vs. malformed vs. missing.
pub fn authenticate_bearer(
    store: &TokenStore,
    header_value: Option<&str>,
) -> Result<AuthResult, AuthError> {
    let header = match header_value {
        None | Some("") => return Err(AuthError::Unauthorized),
        Some(h) => h,
    };
    let prefix = "Bearer ";
    if !header.starts_with(prefix) {
        return Err(AuthError::Unauthorized);
    }
    let token = header[prefix.len()..].trim();
    if token.is_empty() {
        return Err(AuthError::Unauthorized);
    }
    let record = store.find(token).ok_or(AuthError::Unauthorized)?;
    Ok(AuthResult {
        identity: record.identity.clone(),
        scope: record.scope.clone(),
        bound_to_guid: record.bound_to_guid,
        augmented: record.augmented,
    })
}

// ── scope-glob ────────────────────────────────────────────────────────────────

/// True iff this scope pattern requires subject-GUID self-binding.
///
/// Patterns shaped like `<ns>.self.*` are structural.
pub fn is_self_scope(pattern: &str) -> bool {
    let parts: Vec<&str> = pattern.splitn(3, '.').collect();
    parts.len() >= 2 && parts[1] == "self"
}

/// fnmatch-style glob constrained to single-segment `*` semantics.
///
/// Port of `auth.py:_pattern_matches` EXACTLY.
pub fn pattern_matches(pattern: &str, tool: &str) -> bool {
    if !pattern.contains('*') {
        return pattern == tool;
    }

    // Collapse `<ns>.self.*` to `<ns>.*` for matching.
    let normalized: String = if is_self_scope(pattern) {
        let mut parts = pattern.splitn(3, '.');
        let ns = parts.next().unwrap_or("");
        let _self_seg = parts.next();
        let rest = parts.next().unwrap_or("*");
        format!("{ns}.{rest}")
    } else {
        pattern.to_owned()
    };

    // Enforce single-segment `*`:
    // `gm.*` → prefix `gm.`, remainder must be non-empty + no `.`
    if normalized.ends_with(".*") {
        let prefix = &normalized[..normalized.len() - 1]; // "gm."
        if !tool.starts_with(prefix) {
            return false;
        }
        let suffix = &tool[prefix.len()..];
        return !suffix.is_empty() && !suffix.contains('.');
    }

    // Other glob forms not used in V1 — fall through to exact match.
    normalized == tool
}

/// True iff at least one pattern in `scope` matches `tool`.
pub fn scope_allows<'a>(scope: impl IntoIterator<Item = &'a str>, tool: &str) -> bool {
    scope.into_iter().any(|p| pattern_matches(p, tool))
}

/// Returns the FIRST pattern in `scope` that matches `tool`.
/// Used by dispatch to distinguish self-binding patterns.
pub fn find_matching_pattern<'a>(scope: &'a [String], tool: &str) -> Option<&'a str> {
    scope.iter().find(|p| pattern_matches(p.as_str(), tool)).map(|s| s.as_str())
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TokenRecord;

    fn make_store() -> TokenStore {
        TokenStore::new(vec![
            TokenRecord {
                token: "gm-tok".to_string(),
                identity: "gm.tbrack".to_string(),
                scope: vec!["gm.*".to_string(), "obs.*".to_string()],
                augmented: false,
                bound_to_guid: None,
                note: None,
            },
            TokenRecord {
                token: "bot-tok".to_string(),
                identity: "bot.Krak".to_string(),
                scope: vec!["bot.self.*".to_string(), "obs.self.*".to_string()],
                augmented: true,
                bound_to_guid: Some(12345),
                note: None,
            },
        ])
    }

    // ── authenticate_bearer ────────────────────────────────────────────────

    #[test]
    fn bearer_happy_path() {
        let store = make_store();
        let r = authenticate_bearer(&store, Some("Bearer gm-tok")).unwrap();
        assert_eq!(r.identity, "gm.tbrack");
        assert_eq!(r.scope, vec!["gm.*", "obs.*"]);
    }

    #[test]
    fn bearer_unknown_token() {
        let store = make_store();
        assert!(matches!(
            authenticate_bearer(&store, Some("Bearer nope")),
            Err(AuthError::Unauthorized)
        ));
    }

    #[test]
    fn bearer_missing_header() {
        let store = make_store();
        assert!(matches!(
            authenticate_bearer(&store, None),
            Err(AuthError::Unauthorized)
        ));
    }

    #[test]
    fn bearer_malformed_no_prefix() {
        let store = make_store();
        assert!(matches!(
            authenticate_bearer(&store, Some("gm-tok")),
            Err(AuthError::Unauthorized)
        ));
    }

    #[test]
    fn bearer_malformed_empty_token() {
        let store = make_store();
        // "Bearer   " — token after trim is empty
        assert!(matches!(
            authenticate_bearer(&store, Some("Bearer   ")),
            Err(AuthError::Unauthorized)
        ));
    }

    // ── scope-glob truth table ─────────────────────────────────────────────

    #[test]
    fn scope_allows_exact_match() {
        assert!(scope_allows(["gm.additem"].iter().copied(), "gm.additem"));
    }

    #[test]
    fn scope_allows_single_segment_glob() {
        assert!(scope_allows(["gm.*"].iter().copied(), "gm.additem"));
        assert!(scope_allows(["gm.*"].iter().copied(), "gm.teleport"));
    }

    #[test]
    fn scope_glob_does_not_cross_dots() {
        // gm.* must NOT match gm.sub.foo (two-segment suffix)
        assert!(!scope_allows(["gm.*"].iter().copied(), "gm.sub.foo"));
    }

    #[test]
    fn scope_glob_does_not_match_empty_segment() {
        // "gm." — trailing dot, empty segment — must not match
        assert!(!scope_allows(["gm.*"].iter().copied(), "gm."));
    }

    #[test]
    fn scope_denies_other_namespace() {
        assert!(!scope_allows(["gm.*"].iter().copied(), "bot.set_goal"));
    }

    #[test]
    fn scope_self_glob_matches_namespace() {
        // `bot.self.*` collapses to `bot.*` for glob matching
        assert!(scope_allows(["bot.self.*"].iter().copied(), "bot.set_goal"));
    }

    #[test]
    fn is_self_scope_detection() {
        assert!(is_self_scope("bot.self.*"));
        assert!(is_self_scope("obs.self.*"));
        assert!(!is_self_scope("gm.*"));
        assert!(!is_self_scope("bot.*"));
    }

    #[test]
    fn find_matching_pattern_returns_first() {
        let scope = vec!["gm.*".to_string(), "obs.*".to_string()];
        assert_eq!(find_matching_pattern(&scope, "gm.additem"), Some("gm.*"));
        assert_eq!(find_matching_pattern(&scope, "obs.ping"), Some("obs.*"));
        assert_eq!(find_matching_pattern(&scope, "bot.set_goal"), None);
    }

    #[test]
    fn self_scope_find_matching_pattern() {
        let scope = vec!["bot.self.*".to_string()];
        assert_eq!(find_matching_pattern(&scope, "bot.set_goal"), Some("bot.self.*"));
        assert_eq!(find_matching_pattern(&scope, "gm.additem"), None);
    }
}
