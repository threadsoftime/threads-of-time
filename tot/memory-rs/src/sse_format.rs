//! SSE frame encoders — Rust port of Python `sse_format.py`.
//!
//! Each function returns an exact SSE frame string matching the Python output
//! byte-for-byte.  These strings are used in unit tests for parity validation.
//! The actual SSE route uses `axum::response::sse::Event` which produces
//! identical wire bytes via its builder API.
//!
//! Frame shapes:
//!
//! **memory event** (has `id:` line — advances `Last-Event-ID`):
//! ```text
//! event: memory
//! id: <row_id>
//! data: {"row_id":...,"memory_id":...,"bot_id":...,"text":...,"ts":...,"salience":...}
//!
//! ```
//!
//! **heartbeat event** (NO `id:` line — does NOT advance `Last-Event-ID`):
//! ```text
//! event: heartbeat
//! data: {"ts":<unix_ts>}
//!
//! ```
//!
//! **error event**:
//! ```text
//! event: error
//! data: {"code":"...","message":"..."}
//!
//! ```

use crate::pubsub::MemoryRow;

/// Encode a memory row as an SSE `memory` event frame.
///
/// Matches Python `format_sse_memory` byte-for-byte.
/// JSON uses compact separators (`","` / `":"`), same as Python `json.dumps(separators=(',',':'))`.
pub fn format_sse_memory(row: &MemoryRow) -> String {
    // Build compact JSON manually to match Python's `json.dumps(separators=(',',':'))`.
    // This avoids a serde_json pretty-print and ensures field order matches Python.
    let json = format!(
        r#"{{"row_id":{},"memory_id":{},"bot_id":{},"text":{},"ts":{},"salience":{}}}"#,
        row.row_id,
        serde_json::to_string(&row.memory_id).unwrap_or_default(),
        serde_json::to_string(&row.bot_id).unwrap_or_default(),
        serde_json::to_string(&row.text).unwrap_or_default(),
        row.created_ts,
        row.salience,
    );
    format!("event: memory\nid: {}\ndata: {}\n\n", row.row_id, json)
}

/// Encode a heartbeat frame.  No `id:` line — does NOT advance `Last-Event-ID`.
///
/// Matches Python `format_sse_heartbeat` byte-for-byte.
pub fn format_sse_heartbeat(now_ts: i64) -> String {
    format!("event: heartbeat\ndata: {{\"ts\":{now_ts}}}\n\n")
}

/// Encode an error frame.
///
/// Matches Python `format_sse_error` byte-for-byte.
pub fn format_sse_error(code: &str, message: &str) -> String {
    let json = serde_json::json!({"code": code, "message": message});
    format!("event: error\ndata: {}\n\n", json)
}

// ---------------------------------------------------------------------------
// Tests — exact frame string assertions
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_row() -> MemoryRow {
        MemoryRow {
            row_id: 42,
            memory_id: "m_abc123".to_owned(),
            bot_id: "bot_1".to_owned(),
            text: "hello world".to_owned(),
            created_ts: 1_700_000_000,
            salience: 0.75,
        }
    }

    /// Memory frame has `id:` line (advances Last-Event-ID).
    #[test]
    fn memory_frame_has_id_line() {
        let frame = format_sse_memory(&make_row());
        assert!(
            frame.starts_with("event: memory\n"),
            "must start with event line"
        );
        assert!(
            frame.contains("id: 42\n"),
            "must contain id line: {frame:?}"
        );
        assert!(frame.contains("data: "), "must contain data line");
        assert!(
            frame.ends_with("\n\n"),
            "must end with double newline: {frame:?}"
        );
    }

    /// Heartbeat frame has NO `id:` line.
    #[test]
    fn heartbeat_frame_has_no_id_line() {
        let frame = format_sse_heartbeat(1_700_000_000);
        assert_eq!(
            frame,
            "event: heartbeat\ndata: {\"ts\":1700000000}\n\n",
            "exact heartbeat frame mismatch"
        );
        assert!(
            !frame.contains("id:"),
            "heartbeat must not contain id line: {frame:?}"
        );
    }

    /// Memory frame JSON payload contains expected fields.
    #[test]
    fn memory_frame_json_payload() {
        let frame = format_sse_memory(&make_row());
        // Extract the data line
        let data_line = frame
            .lines()
            .find(|l| l.starts_with("data: "))
            .expect("data line present");
        let json_str = data_line.strip_prefix("data: ").unwrap();
        let v: serde_json::Value = serde_json::from_str(json_str).expect("valid JSON");

        assert_eq!(v["row_id"], 42);
        assert_eq!(v["memory_id"], "m_abc123");
        assert_eq!(v["bot_id"], "bot_1");
        assert_eq!(v["text"], "hello world");
        assert_eq!(v["ts"], 1_700_000_000_i64);
        // salience is 0.75 (f32)
        let s = v["salience"].as_f64().expect("salience is f64");
        assert!((s - 0.75).abs() < 0.001, "salience {s}");
    }

    /// Error frame shape.
    #[test]
    fn error_frame_shape() {
        let frame = format_sse_error("internal", "something broke");
        assert!(frame.starts_with("event: error\n"));
        assert!(frame.ends_with("\n\n"));
        assert!(frame.contains("\"code\":\"internal\""));
        assert!(frame.contains("\"message\":\"something broke\""));
    }

    /// Both memory and heartbeat frames end with exactly `\n\n`.
    #[test]
    fn all_frames_end_with_double_newline() {
        let mem = format_sse_memory(&make_row());
        let hb = format_sse_heartbeat(0);
        let err = format_sse_error("x", "y");
        for frame in &[&mem, &hb, &err] {
            assert!(
                frame.ends_with("\n\n"),
                "frame does not end with double newline: {frame:?}"
            );
        }
    }
}
