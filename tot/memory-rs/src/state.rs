//! Shared application state. Full implementation in Task 0.5.
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct EmbedConfig {
    pub url: String,
    pub model: String,
    pub api_key: String,
}

#[derive(Clone, Debug)]
pub struct AppState {
    pub data_dir: PathBuf,
    pub embed: EmbedConfig,
}
