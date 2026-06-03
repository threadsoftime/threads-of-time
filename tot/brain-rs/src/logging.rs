// SPDX-License-Identifier: AGPL-3.0
//! JSONL decision-log writer + tracing setup.
//!
//! Faithful Rust port of `brain_sidecar/logging_setup.py`.
//!
//! # Thread-safety
//! `JsonlDecisionLogWriter` uses a `std::sync::Mutex` over the log path (re-opens
//! the file per write, matching the Python `with open(..., "a")` pattern).
//! This is intentionally over-protective for the async path — acceptable for MVP
//! (per Task 10 domain-knowledge note #1 in the Python source).

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

use serde_json::Value;
use tracing_subscriber::{fmt, EnvFilter};

use crate::loop_supervisor::DecisionLogWriter;

// ---------------------------------------------------------------------------
// JsonlDecisionLogWriter
// ---------------------------------------------------------------------------

/// Append one JSON object per line. Thread-safe via a single Mutex.
///
/// Mirrors Python `JsonlDecisionLogWriter`.
pub struct JsonlDecisionLogWriter {
    path: String,
    /// Lock guards the file write. The mutex wraps a `()` (we re-open the file
    /// per write, matching Python's `with open(..., "a")` pattern).
    lock: Mutex<()>,
}

impl JsonlDecisionLogWriter {
    /// Create the writer, ensuring the parent directory exists.
    pub fn new(path: impl Into<String>) -> Self {
        let path = path.into();
        if let Some(parent) = Path::new(&path).parent() {
            // Ignore error — open will surface any real I/O problem.
            let _ = fs::create_dir_all(parent);
        }
        Self {
            path,
            lock: Mutex::new(()),
        }
    }
}

impl DecisionLogWriter for JsonlDecisionLogWriter {
    /// Serialize `record` to a single JSON line and append to the log file.
    ///
    /// Mirrors Python:
    /// ```python
    /// line = json.dumps(record, default=str)
    /// with self._lock, open(self._path, "a", encoding="utf-8") as f:
    ///     f.write(line + "\n")
    /// ```
    fn write(&self, record: &Value) {
        let line = serde_json::to_string(record).unwrap_or_else(|e| {
            format!("{{\"error\":\"serialize_failed\",\"msg\":{e:?}}}")
        });
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        if let Ok(mut f) = OpenOptions::new().append(true).create(true).open(&self.path) {
            let _ = writeln!(f, "{line}");
        }
    }
}

// ---------------------------------------------------------------------------
// setup_tracing
// ---------------------------------------------------------------------------

/// Initialize `tracing_subscriber` with the given level filter.
///
/// Mirrors Python `setup_stdlib_logging(level)`:
/// - Writes to stdout.
/// - Level can be overridden via `RUST_LOG` env var (standard tracing-subscriber
///   behaviour).
///
/// # Panics
/// Panics if called more than once in a process (tracing global subscriber can
/// only be set once). In tests, call this at most once, or use the subscriber
/// in a test-scoped span instead.
pub fn setup_tracing(level: &str) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(level));
    let _ = fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Read;
    use tempfile::NamedTempFile;

    #[test]
    fn test_jsonl_writer_appends_valid_json_line() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        let writer = JsonlDecisionLogWriter::new(path.clone());

        let record = json!({"event": "tick", "bot_guid": 42, "ts_ms": 1234567890});
        writer.write(&record);

        let mut content = String::new();
        std::fs::File::open(&path).unwrap().read_to_string(&mut content).unwrap();
        let trimmed = content.trim();
        assert!(!trimmed.is_empty());
        // Must be valid JSON
        let parsed: Value = serde_json::from_str(trimmed).expect("should be valid JSON");
        assert_eq!(parsed["event"], "tick");
        assert_eq!(parsed["bot_guid"], 42);
    }

    #[test]
    fn test_jsonl_writer_appends_multiple_lines() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        let writer = JsonlDecisionLogWriter::new(path.clone());

        for i in 0..3 {
            writer.write(&json!({"seq": i}));
        }

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 3);
        // Each line is valid JSON
        for (i, line) in lines.iter().enumerate() {
            let parsed: Value = serde_json::from_str(line).expect("valid JSON line");
            assert_eq!(parsed["seq"], i as i64);
        }
    }

    #[test]
    fn test_jsonl_writer_creates_parent_dir() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let path = tmp_dir.path().join("subdir/deep/decisions.jsonl");
        let writer = JsonlDecisionLogWriter::new(path.to_str().unwrap());
        writer.write(&json!({"event": "test"}));
        assert!(path.exists());
    }
}
