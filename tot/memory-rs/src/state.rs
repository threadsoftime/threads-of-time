//! Shared application state injected into axum handlers via [`axum::extract::State`].
//!
//! Both types are `Clone` so axum can clone the state for each handler call.
//! `PathBuf` and `String` clones are cheap for the short values we carry here;
//! no `Arc` wrapping is needed.

use std::path::PathBuf;

/// Configuration for the external embeddings HTTP endpoint.
#[derive(Clone, Debug)]
pub struct EmbedConfig {
    /// OpenAI-compatible base URL (without trailing `/embeddings`).
    pub url: String,
    /// Model string passed in `{"model": ...}`.
    pub model: String,
    /// Bearer token; empty string → no `Authorization` header sent.
    pub api_key: String,
}

/// Axum application state — cloned into every handler invocation.
#[derive(Clone, Debug)]
pub struct AppState {
    /// Root directory for per-bot SQLite files: `<data_dir>/<bot_guid>/memory.sqlite`.
    pub data_dir: PathBuf,
    /// Embeddings endpoint configuration.
    pub embed: EmbedConfig,
}

impl AppState {
    /// Construct from a validated [`crate::config::Settings`].
    ///
    /// Field mapping (v0.2.1 re-target):
    /// - `Settings::db_path` → `AppState::data_dir` (parent dir; kept for future per-bot routing)
    /// - `Settings::embed_endpoint` → `EmbedConfig::url`
    /// - `EmbedConfig::model` stubbed to empty string (v0.2.1 embed service is model-implicit)
    /// - `EmbedConfig::api_key` stubbed to empty string (no key required by the local stub)
    pub fn from_settings(s: &crate::config::Settings) -> Self {
        AppState {
            // db_path is the single-file path; expose its parent as data_dir for
            // backward compat with any route code that constructs per-bot sub-paths.
            data_dir: s
                .db_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| s.db_path.clone()),
            embed: EmbedConfig {
                url: s.embed_endpoint.clone(),
                model: String::new(),
                api_key: String::new(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_state() -> AppState {
        AppState {
            data_dir: PathBuf::from("/tmp/mem"),
            embed: EmbedConfig {
                url: "http://127.0.0.1:11434".to_string(),
                model: "nomic-embed-text".to_string(),
                api_key: String::new(),
            },
        }
    }

    /// AppState must be Clone (axum's State extractor requires it).
    #[test]
    fn app_state_is_clone() {
        let s = make_state();
        let s2 = s.clone();
        assert_eq!(s.data_dir, s2.data_dir);
        assert_eq!(s.embed.url, s2.embed.url);
        assert_eq!(s.embed.model, s2.embed.model);
        assert_eq!(s.embed.api_key, s2.embed.api_key);
    }

    /// from_settings must propagate all fields from Settings (v0.2.1 env names).
    #[test]
    fn from_settings_copies_all_fields() {
        use crate::config::Settings;
        use std::collections::HashMap;

        let settings = Settings::build(|k| {
            HashMap::from([
                ("MEM_DB_PATH", "/var/memory/db.sqlite"),
                ("MEM_EMBED_ENDPOINT", "http://embed.internal"),
            ])
            .get(k)
            .map(|s| s.to_string())
        })
        .expect("build settings");

        let state = AppState::from_settings(&settings);
        // db_path is /var/memory/db.sqlite → parent is /var/memory
        assert_eq!(state.data_dir, PathBuf::from("/var/memory"));
        assert_eq!(state.embed.url, "http://embed.internal");
    }
}
