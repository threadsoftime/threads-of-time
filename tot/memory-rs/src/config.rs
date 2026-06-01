//! Settings from environment variables. Full implementation in Task 0.3.
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Settings {
    pub data_dir: PathBuf,
    pub embeddings_url: String,
    pub embeddings_model: String,
    pub embeddings_api_key: String,
    pub bind_host: String,
    pub bind_port: u16,
}

impl Settings {
    pub fn from_env() -> anyhow::Result<Settings> {
        todo!("Task 0.3")
    }
}
