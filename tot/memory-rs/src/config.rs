//! Runtime configuration read from environment variables.
//!
//! # Required variables
//!
//! | Variable               | Meaning                                      |
//! |------------------------|----------------------------------------------|
//! | `MEMORY_DATA_DIR`      | Root directory for per-bot SQLite files      |
//! | `BRAIN_EMBEDDINGS_URL` | OpenAI-compatible base URL (without `/embeddings`) |
//! | `BRAIN_EMBEDDINGS_MODEL` | Model string e.g. `nomic-embed-text`       |
//!
//! # Optional variables
//!
//! | Variable                 | Default     |
//! |--------------------------|-------------|
//! | `BRAIN_EMBEDDINGS_API_KEY` | `""`     |
//! | `MEM_BIND_HOST`           | `0.0.0.0` |
//! | `MEM_BIND_PORT`           | `8090`    |

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Settings {
    /// Root directory for per-bot SQLite databases: `<data_dir>/<bot_guid>/memory.sqlite`.
    pub data_dir: PathBuf,
    /// OpenAI-compatible embeddings base URL (the client appends `/embeddings`).
    pub embeddings_url: String,
    /// Model string passed in `{"model": ...}` to the embeddings endpoint.
    pub embeddings_model: String,
    /// Bearer token; empty string means no `Authorization` header is sent.
    pub embeddings_api_key: String,
    /// IP address to bind the HTTP server on.
    pub bind_host: String,
    /// TCP port to bind the HTTP server on.
    pub bind_port: u16,
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
        let req = |k: &str| -> anyhow::Result<String> {
            get(k).ok_or_else(|| anyhow::anyhow!("required env var `{k}` is not set"))
        };

        let bind_port: u16 = get("MEM_BIND_PORT")
            .as_deref()
            .unwrap_or("8090")
            .parse()
            .map_err(|e| anyhow::anyhow!("MEM_BIND_PORT is not a valid u16: {e}"))?;

        Ok(Settings {
            data_dir: PathBuf::from(req("MEMORY_DATA_DIR")?),
            embeddings_url: req("BRAIN_EMBEDDINGS_URL")?,
            embeddings_model: req("BRAIN_EMBEDDINGS_MODEL")?,
            embeddings_api_key: get("BRAIN_EMBEDDINGS_API_KEY").unwrap_or_default(),
            bind_host: get("MEM_BIND_HOST").unwrap_or_else(|| "0.0.0.0".to_string()),
            bind_port,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn getter(map: HashMap<&'static str, &'static str>) -> impl Fn(&str) -> Option<String> {
        move |k| map.get(k).map(|s| s.to_string())
    }

    fn required_map() -> HashMap<&'static str, &'static str> {
        HashMap::from([
            ("MEMORY_DATA_DIR", "/tmp/mem"),
            ("BRAIN_EMBEDDINGS_URL", "http://127.0.0.1:11434"),
            ("BRAIN_EMBEDDINGS_MODEL", "nomic-embed-text"),
        ])
    }

    #[test]
    fn builds_with_defaults_when_optional_vars_absent() {
        let s = Settings::build(getter(required_map())).expect("build");
        assert_eq!(s.data_dir, PathBuf::from("/tmp/mem"));
        assert_eq!(s.embeddings_url, "http://127.0.0.1:11434");
        assert_eq!(s.embeddings_model, "nomic-embed-text");
        assert_eq!(s.embeddings_api_key, "", "api_key defaults to empty string");
        assert_eq!(s.bind_host, "0.0.0.0", "bind_host defaults to 0.0.0.0");
        assert_eq!(s.bind_port, 8090, "bind_port defaults to 8090");
    }

    #[test]
    fn explicit_optional_vars_override_defaults() {
        let mut m = required_map();
        m.extend([
            ("BRAIN_EMBEDDINGS_API_KEY", "secret"),
            ("MEM_BIND_HOST", "127.0.0.1"),
            ("MEM_BIND_PORT", "9000"),
        ]);
        let s = Settings::build(getter(m)).expect("build");
        assert_eq!(s.embeddings_api_key, "secret");
        assert_eq!(s.bind_host, "127.0.0.1");
        assert_eq!(s.bind_port, 9000);
    }

    #[test]
    fn missing_data_dir_returns_error() {
        let mut m = required_map();
        m.remove("MEMORY_DATA_DIR");
        let err = Settings::build(getter(m)).unwrap_err();
        assert!(
            err.to_string().contains("MEMORY_DATA_DIR"),
            "error message must name the missing var; got: {err}"
        );
    }

    #[test]
    fn missing_embeddings_url_returns_error() {
        let mut m = required_map();
        m.remove("BRAIN_EMBEDDINGS_URL");
        let err = Settings::build(getter(m)).unwrap_err();
        assert!(err.to_string().contains("BRAIN_EMBEDDINGS_URL"), "got: {err}");
    }

    #[test]
    fn missing_embeddings_model_returns_error() {
        let mut m = required_map();
        m.remove("BRAIN_EMBEDDINGS_MODEL");
        let err = Settings::build(getter(m)).unwrap_err();
        assert!(err.to_string().contains("BRAIN_EMBEDDINGS_MODEL"), "got: {err}");
    }

    #[test]
    fn invalid_bind_port_returns_error() {
        let mut m = required_map();
        m.insert("MEM_BIND_PORT", "not_a_number");
        let err = Settings::build(getter(m)).unwrap_err();
        assert!(
            err.to_string().contains("MEM_BIND_PORT"),
            "error message must name the bad var; got: {err}"
        );
    }

    #[test]
    fn bind_port_99999_exceeds_u16_max_returns_error() {
        let mut m = required_map();
        m.insert("MEM_BIND_PORT", "99999");
        let err = Settings::build(getter(m)).unwrap_err();
        assert!(err.to_string().contains("MEM_BIND_PORT"), "got: {err}");
    }
}
