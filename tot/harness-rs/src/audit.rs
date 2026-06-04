//! Append-only JSONL audit logger.
//!
//! Port of `harness_daemon/audit.py`. Args bodies are NEVER serialized —
//! we log a SHA-256 digest instead (spec §9.1). Each call appends one line.
//!
//! Key wire difference from Python: `ac_latency_ms` is `Option<u64>` and emits
//! `null` when None. Python uses `int = 0`; Rust is stricter for non-forward
//! outcomes.

use std::fs::OpenOptions;
use std::io::Write as IoWrite;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

// ── canonicalize ──────────────────────────────────────────────────────────────

/// Recursively rebuild a `Value` with object keys sorted at every nesting level.
///
/// This is the Rust equivalent of Python's `json.dumps(body, sort_keys=True)`:
/// - Object keys are sorted lexicographically at EVERY depth.
/// - Array element ORDER is preserved (arrays are NOT sorted).
/// - Scalar values (bool, null, number, string) are cloned byte-for-byte.
///
/// This makes `sha256_args` produce the same digest regardless of whether
/// `serde_json` is compiled with `preserve_order` (IndexMap) or without
/// (BTreeMap). With `preserve_order` active, `Value::Object` retains
/// insertion order on serialization, which diverges from Python's
/// `sort_keys=True` output. Canonicalization restores parity.
fn canonicalize(v: &Value) -> Value {
    match v {
        Value::Object(map) => {
            let mut sorted: Map<String, Value> = Map::new();
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for k in keys {
                sorted.insert(k.clone(), canonicalize(&map[k]));
            }
            Value::Object(sorted)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(canonicalize).collect()),
        // Scalars: bool, null, number, string — clone as-is.
        scalar => scalar.clone(),
    }
}

// ── sha256_args ───────────────────────────────────────────────────────────────

/// Canonical JSON hash of the args body.
///
/// MUST equal Python:
/// `hashlib.sha256(json.dumps(body, sort_keys=True, separators=(",",":")).encode()).hexdigest()`
///
/// `canonicalize` sorts object keys recursively before serialization, matching
/// Python's `sort_keys=True` at every nesting depth. This is robust whether
/// `serde_json` is compiled with `preserve_order` (IndexMap) or without (BTreeMap):
/// the canonical form is always sorted, so the digest is always stable.
///
/// NOTE: Python's `json.dumps(default=str)` coerced non-JSON-native types (e.g.
/// `datetime`) to strings before hashing, so callers must serialize such values
/// to strings before constructing `args_body`, or the digest will diverge from
/// Python.
pub fn sha256_args(body: &Value) -> String {
    let canonical = canonicalize(body).to_string();
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    format!("{:x}", hasher.finalize())
}

// ── AuditEvent ────────────────────────────────────────────────────────────────

/// Caller-provided event data.
pub struct AuditEvent {
    pub ts: f64,
    pub request_id: String,
    pub identity: String,
    pub tool: String,
    pub args_body: Value,
    pub outcome: String,
    pub status: u16,
    pub latency_ms: u64,
    pub ac_latency_ms: Option<u64>,
    pub error_detail: String,
    pub transport: String,
}

impl Default for AuditEvent {
    fn default() -> Self {
        Self {
            ts: 0.0,
            request_id: String::new(),
            identity: String::new(),
            tool: String::new(),
            args_body: Value::Object(Default::default()),
            outcome: String::new(),
            status: 200,
            latency_ms: 0,
            ac_latency_ms: None,
            error_detail: String::new(),
            transport: "http".to_string(),
        }
    }
}

// ── AuditRecord — the wire struct ─────────────────────────────────────────────

/// Serialized record written to the JSONL file.
///
/// Field DECLARATION ORDER == wire order (serde emits struct fields in
/// declaration order — do NOT use a Map/Value which would sort keys).
///
/// Key rules:
/// - `ac_latency_ms`: Option<u64> WITHOUT skip_serializing_if → always present
///   (either a number or `null`).
/// - `error_detail`: skip when empty string.
#[derive(Serialize)]
struct AuditRecord<'a> {
    ts: f64,
    request_id: &'a str,
    identity: &'a str,
    tool: &'a str,
    args_sha256: String,
    outcome: &'a str,
    status: u16,
    latency_ms: u64,
    ac_latency_ms: Option<u64>,
    transport: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    error_detail: &'a str,
}

// ── AuditLogger ───────────────────────────────────────────────────────────────

pub struct AuditLogger {
    path: PathBuf,
}

impl AuditLogger {
    /// Create logger; mkdir-parents the containing directory.
    ///
    /// Returns `Err` if the parent directory cannot be created — parity with
    /// Python's `AuditLogger.__init__` which raises on `mkdir` failure rather
    /// than silently swallowing the error (which would cause silent audit loss).
    pub fn new(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Ok(Self { path })
    }

    /// Append ONE compact JSON line for the event.
    pub fn write(&self, ev: &AuditEvent) -> std::io::Result<()> {
        let record = AuditRecord {
            ts: ev.ts,
            request_id: &ev.request_id,
            identity: &ev.identity,
            tool: &ev.tool,
            args_sha256: sha256_args(&ev.args_body),
            outcome: &ev.outcome,
            status: ev.status,
            latency_ms: ev.latency_ms,
            ac_latency_ms: ev.ac_latency_ms,
            transport: &ev.transport,
            error_detail: &ev.error_detail,
        };
        let line = serde_json::to_string(&record)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(file, "{line}")?;
        Ok(())
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── sha256_args fixtures ───────────────────────────────────────────────

    #[test]
    fn sha256_empty_object() {
        // python3 -c 'import hashlib,json; print(hashlib.sha256(json.dumps({}, sort_keys=True, separators=(",",":")).encode()).hexdigest())'
        assert_eq!(
            sha256_args(&json!({})),
            "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
        );
    }

    #[test]
    fn sha256_b1_a2_sorted() {
        // Python sorts keys: json.dumps({"b":1,"a":2}, sort_keys=True, ...) → '{"a":2,"b":1}'
        // python3 -c 'import hashlib,json; print(hashlib.sha256(json.dumps({"b":1,"a":2}, sort_keys=True, separators=(",",":")).encode()).hexdigest())'
        // Also exercises canonicalize() on a Value built from wire JSON (insertion order).
        let wire: Value = serde_json::from_str(r#"{"b":1,"a":2}"#).unwrap();
        assert_eq!(
            sha256_args(&wire),
            "d3626ac30a87e6f7a6428233b3c68299976865fa5508e4267c5415c76af7a772"
        );
    }

    #[test]
    fn sha256_float_1_0() {
        // Python: json.dumps({"x":1.0}, ...) → '{"x":1.0}'
        // serde_json: json!({"x": 1.0_f64}) → '{"x":1.0}' (preserves trailing .0)
        // python3 -c 'import hashlib,json; print(hashlib.sha256(json.dumps({"x":1.0}, sort_keys=True, separators=(",",":")).encode()).hexdigest())'
        assert_eq!(
            sha256_args(&json!({"x": 1.0_f64})),
            "bf32f56236899e13ef54db875d136c6cbcc65244464829c54911aa9069b0ae25"
        );
    }

    #[test]
    fn sha256_nested_objects_sorted_recursively() {
        // Regression: canonicalize must sort keys at EVERY nesting level, not
        // just the top level. Python's sort_keys=True is recursive.
        //
        // Canonical string: '{"a":{"c":3,"d":4},"b":1}'
        // python3 -c 'import hashlib,json; print(hashlib.sha256(json.dumps({"b":1,"a":{"d":4,"c":3}}, sort_keys=True, separators=(",",":")).encode()).hexdigest())'
        let wire: Value = serde_json::from_str(r#"{"b":1,"a":{"d":4,"c":3}}"#).unwrap();
        assert_eq!(
            sha256_args(&wire),
            "943d56ce0b02b80a8afcd12d849426226b68f2d8cd2840af8f6f93067f14c360"
        );
    }

    #[test]
    fn sha256_array_preserves_order_but_object_keys_sorted() {
        // Arrays keep their element order (Python does NOT sort array contents);
        // object keys nested inside array elements are still sorted.
        //
        // Canonical string: '{"arr":[{"x":1,"y":2}]}'
        // python3 -c 'import hashlib,json; print(hashlib.sha256(json.dumps({"arr":[{"y":2,"x":1}]}, sort_keys=True, separators=(",",":")).encode()).hexdigest())'
        let wire: Value = serde_json::from_str(r#"{"arr":[{"y":2,"x":1}]}"#).unwrap();
        assert_eq!(
            sha256_args(&wire),
            "e3ca0879bddffc9b8f5e08dbb44d9c90238d375daf4b798a0cc2dfbb4e6ff3a2"
        );
    }

    // ── canonical output format ────────────────────────────────────────────

    #[test]
    fn written_line_starts_with_ts_field() {
        let dir = std::env::temp_dir();
        let p = dir.join("harness_rs_audit_test_ts.jsonl");
        let _ = std::fs::remove_file(&p);
        let logger = AuditLogger::new(&p).unwrap();
        logger.write(&AuditEvent {
            ts: 1.0,
            request_id: "req_ts".to_string(),
            identity: "alice".to_string(),
            tool: "obs.ping".to_string(),
            args_body: json!({}),
            outcome: "ok".to_string(),
            status: 200,
            latency_ms: 5,
            ac_latency_ms: None,
            error_detail: String::new(),
            transport: "http".to_string(),
        }).unwrap();
        let line = std::fs::read_to_string(&p).unwrap();
        let line = line.trim();
        assert!(line.starts_with(r#"{"ts":1.0,"request_id":"#),
            "line should start with ts field: {line}");
    }

    #[test]
    fn ac_latency_ms_null_when_none() {
        let dir = std::env::temp_dir();
        let p = dir.join("harness_rs_audit_test_null.jsonl");
        let _ = std::fs::remove_file(&p);
        let logger = AuditLogger::new(&p).unwrap();
        logger.write(&AuditEvent {
            ts: 1.0,
            request_id: "req_null".to_string(),
            identity: "alice".to_string(),
            tool: "obs.ping".to_string(),
            args_body: json!({}),
            outcome: "ok".to_string(),
            status: 200,
            latency_ms: 5,
            ac_latency_ms: None,
            error_detail: String::new(),
            transport: "http".to_string(),
        }).unwrap();
        let line = std::fs::read_to_string(&p).unwrap();
        assert!(line.contains(r#""ac_latency_ms":null"#),
            "expected ac_latency_ms:null in: {line}");
    }

    #[test]
    fn ac_latency_ms_value_when_some() {
        let dir = std::env::temp_dir();
        let p = dir.join("harness_rs_audit_test_ac.jsonl");
        let _ = std::fs::remove_file(&p);
        let logger = AuditLogger::new(&p).unwrap();
        logger.write(&AuditEvent {
            ts: 1.0,
            request_id: "req_ac".to_string(),
            identity: "alice".to_string(),
            tool: "obs.ping".to_string(),
            args_body: json!({}),
            outcome: "ok".to_string(),
            status: 200,
            latency_ms: 5,
            ac_latency_ms: Some(5),
            error_detail: String::new(),
            transport: "http".to_string(),
        }).unwrap();
        let line = std::fs::read_to_string(&p).unwrap();
        assert!(line.contains(r#""ac_latency_ms":5"#),
            "expected ac_latency_ms:5 in: {line}");
    }

    #[test]
    fn error_detail_omitted_when_empty() {
        let dir = std::env::temp_dir();
        let p = dir.join("harness_rs_audit_test_noerr.jsonl");
        let _ = std::fs::remove_file(&p);
        let logger = AuditLogger::new(&p).unwrap();
        logger.write(&AuditEvent {
            ts: 1.0,
            request_id: "req_noerr".to_string(),
            identity: "alice".to_string(),
            tool: "obs.ping".to_string(),
            args_body: json!({}),
            outcome: "ok".to_string(),
            status: 200,
            latency_ms: 5,
            ac_latency_ms: None,
            error_detail: String::new(),
            transport: "http".to_string(),
        }).unwrap();
        let line = std::fs::read_to_string(&p).unwrap();
        assert!(!line.contains("error_detail"),
            "error_detail should be omitted when empty: {line}");
    }

    #[test]
    fn error_detail_present_when_non_empty() {
        let dir = std::env::temp_dir();
        let p = dir.join("harness_rs_audit_test_err.jsonl");
        let _ = std::fs::remove_file(&p);
        let logger = AuditLogger::new(&p).unwrap();
        logger.write(&AuditEvent {
            ts: 1.0,
            request_id: "req_err".to_string(),
            identity: "alice".to_string(),
            tool: "obs.ping".to_string(),
            args_body: json!({}),
            outcome: "error".to_string(),
            status: 500,
            latency_ms: 5,
            ac_latency_ms: None,
            error_detail: "something failed".to_string(),
            transport: "http".to_string(),
        }).unwrap();
        let line = std::fs::read_to_string(&p).unwrap();
        assert!(line.contains(r#""error_detail":"something failed""#),
            "error_detail should be present: {line}");
    }

    #[test]
    fn written_line_is_valid_json_with_expected_fields() {
        let dir = std::env::temp_dir();
        let p = dir.join("harness_rs_audit_test_fields.jsonl");
        let _ = std::fs::remove_file(&p);
        let logger = AuditLogger::new(&p).unwrap();
        logger.write(&AuditEvent {
            ts: 1779000000.123,
            request_id: "req_1".to_string(),
            identity: "alice".to_string(),
            tool: "obs.ping".to_string(),
            args_body: json!({"x": 1}),
            outcome: "ok".to_string(),
            status: 200,
            latency_ms: 12,
            ac_latency_ms: Some(5),
            error_detail: String::new(),
            transport: "http".to_string(),
        }).unwrap();
        let line = std::fs::read_to_string(&p).unwrap();
        let rec: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(rec["request_id"], "req_1");
        assert_eq!(rec["identity"], "alice");
        assert_eq!(rec["tool"], "obs.ping");
        assert_eq!(rec["outcome"], "ok");
        assert_eq!(rec["status"], 200);
        assert!(rec["args_sha256"].as_str().unwrap().len() == 64);
    }

    #[test]
    fn args_not_logged_only_hash() {
        let dir = std::env::temp_dir();
        let p = dir.join("harness_rs_audit_test_hash.jsonl");
        let _ = std::fs::remove_file(&p);
        let logger = AuditLogger::new(&p).unwrap();
        logger.write(&AuditEvent {
            ts: 1.0,
            request_id: "req_2".to_string(),
            identity: "alice".to_string(),
            tool: "gm.run_console".to_string(),
            args_body: json!({"password": "do-not-log-me"}),
            outcome: "ok".to_string(),
            status: 200,
            latency_ms: 10,
            ac_latency_ms: Some(2),
            error_detail: String::new(),
            transport: "http".to_string(),
        }).unwrap();
        let raw = std::fs::read_to_string(&p).unwrap();
        assert!(!raw.contains("do-not-log-me"), "secret must not appear in audit line");
        let rec: serde_json::Value = serde_json::from_str(raw.trim()).unwrap();
        assert!(rec["args_sha256"].as_str().unwrap().len() == 64);
    }
}
