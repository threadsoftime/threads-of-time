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
    pub fn from_settings(s: &crate::config::Settings) -> Self {
        AppState {
            data_dir: s.data_dir.clone(),
            embed: EmbedConfig {
                url: s.embeddings_url.clone(),
                model: s.embeddings_model.clone(),
                api_key: s.embeddings_api_key.clone(),
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

    /// from_settings must propagate all fields from Settings.
    #[test]
    fn from_settings_copies_all_fields() {
        use crate::config::Settings;
        use std::collections::HashMap;

        let settings = Settings::build(|k| {
            HashMap::from([
                ("MEMORY_DATA_DIR", "/var/memory"),
                ("BRAIN_EMBEDDINGS_URL", "http://embed.internal"),
                ("BRAIN_EMBEDDINGS_MODEL", "nomic-embed-text"),
                ("BRAIN_EMBEDDINGS_API_KEY", "tok123"),
            ])
            .get(k)
            .map(|s| s.to_string())
        })
        .expect("build settings");

        let state = AppState::from_settings(&settings);
        assert_eq!(state.data_dir, PathBuf::from("/var/memory"));
        assert_eq!(state.embed.url, "http://embed.internal");
        assert_eq!(state.embed.model, "nomic-embed-text");
        assert_eq!(state.embed.api_key, "tok123");
    }
}
