// SPDX-License-Identifier: AGPL-3.0
//! Compile-time build identity (git SHA, build time, crate version).
//!
//! `BUILD_SHA` is embedded by `build.rs` (priority: BRAIN_BUILD_SHA env-arg →
//! `git rev-parse --short HEAD` → "unknown"). Surfaced in the startup banner and
//! stamped on every decision record so a deploy is provable from the binary's
//! own output.

pub const BUILD_SHA: &str = match option_env!("BRAIN_BUILD_SHA") {
    Some(s) => s,
    None => "unknown",
};
pub const BUILD_TIME: &str = match option_env!("BRAIN_BUILD_TIME") {
    Some(s) => s,
    None => "unknown",
};
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// One-line human-readable identity for the startup banner.
pub fn banner() -> String {
    format!("brain_build sha={BUILD_SHA} version={VERSION} built_at={BUILD_TIME}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_sha_is_non_empty() {
        assert!(!BUILD_SHA.is_empty(), "BUILD_SHA must never be empty (>= \"unknown\")");
    }

    #[test]
    fn version_matches_cargo_pkg_version() {
        assert_eq!(VERSION, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn banner_contains_all_fields() {
        let b = banner();
        assert!(b.contains("brain_build sha="));
        assert!(b.contains("version="));
        assert!(b.contains("built_at="));
    }
}
