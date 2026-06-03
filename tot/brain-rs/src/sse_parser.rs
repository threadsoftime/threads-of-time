// SPDX-License-Identifier: AGPL-3.0
//! SSE frame parser — faithful Rust port of `brain_sidecar/sse_parser.py`.
//!
//! Handles the subset that memory-sidecar v0.3.0 emits:
//!   - `event: <type>`
//!   - `id: <str>`      (absent on heartbeat frames)
//!   - `data: <str>`    (multi-line joined with `\n`)
//!   - `: comment`      (ignored)
//!
//! Frames terminate on blank lines per the SSE spec.

use futures_util::Stream;
use async_stream::stream;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// One parsed SSE event frame.
///
/// Mirrors Python `SseEvent(type, id, data)`.
#[derive(Debug, Clone, PartialEq)]
pub struct SseEvent {
    /// Event type; defaults to `"message"` (SSE spec default).
    pub event_type: String,
    /// The `id:` field value, if present.
    pub id: Option<String>,
    /// The `data:` payload; multiple `data:` lines joined with `\n`.
    pub data: String,
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Parse an async stream of SSE text chunks into `SseEvent`s.
///
/// Faithful port of Python `parse_sse_chunks`:
/// - Buffer accumulates chunks; split on `\n`.
/// - `\r` stripped from each line (CRLF support).
/// - Blank line dispatches event (if data_lines non-empty, or event_id set, or
///   event_type != "message").
/// - Comment lines (starting with `:`) are ignored.
/// - `event:` / `id:` / `data:` lines update in-progress frame state.
pub fn parse_sse_chunks<S>(chunks: S) -> impl Stream<Item = SseEvent>
where
    S: Stream<Item = String>,
{
    stream! {
        let mut buf = String::new();
        let mut event_type = "message".to_string();
        let mut event_id: Option<String> = None;
        let mut data_lines: Vec<String> = Vec::new();

        tokio::pin!(chunks);

        use futures_util::StreamExt;

        while let Some(chunk) = chunks.next().await {
            buf.push_str(&chunk);

            // Process all complete lines in buf.
            while let Some(nl_pos) = buf.find('\n') {
                let line_raw = buf[..nl_pos].to_string();
                buf = buf[nl_pos + 1..].to_string();

                // Strip trailing \r (CRLF → LF normalisation).
                let line = line_raw.trim_end_matches('\r');

                if line.is_empty() {
                    // Blank line → dispatch event if there is something to emit.
                    // Mirrors Python: `if data_lines or event_id is not None or event_type != "message"`
                    if !data_lines.is_empty() || event_id.is_some() || event_type != "message" {
                        yield SseEvent {
                            event_type: event_type.clone(),
                            id: event_id.clone(),
                            data: data_lines.join("\n"),
                        };
                    }
                    // Reset to defaults.
                    event_type = "message".to_string();
                    event_id = None;
                    data_lines.clear();
                } else if line.starts_with(':') {
                    // Comment line — ignore (e.g. `: heartbeat`).
                } else if let Some(rest) = line.strip_prefix("event:") {
                    event_type = rest.trim_start().to_string();
                } else if let Some(rest) = line.strip_prefix("id:") {
                    event_id = Some(rest.trim_start().to_string());
                } else if let Some(rest) = line.strip_prefix("data:") {
                    // Python: `line[5:].lstrip(" ")` — strip one leading space.
                    // strip_prefix("data:") already removes "data:", then lstrip(" ").
                    data_lines.push(rest.trim_start_matches(' ').to_string());
                }
                // Other field names (e.g. `retry:`) are ignored (not used by memory-sidecar).
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;

    #[tokio::test]
    async fn test_parse_single_memory_event() {
        let raw = "event: memory\nid: 42\ndata: {\"row_id\":42,\"memory_id\":\"abc-uuid\"}\n\n";
        let chunks = tokio_stream::iter(vec![raw.to_string()]);
        let events: Vec<_> = parse_sse_chunks(chunks).collect().await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "memory");
        assert_eq!(events[0].id.as_deref(), Some("42"));
        assert!(events[0].data.contains("abc-uuid"));
    }

    #[tokio::test]
    async fn test_heartbeat_no_id_does_not_advance_cursor() {
        // Heartbeat: ": heartbeat\n\n" — comment lines ignored, no event yielded.
        let raw = ": heartbeat\n\n";
        let chunks = tokio_stream::iter(vec![raw.to_string()]);
        let events: Vec<_> = parse_sse_chunks(chunks).collect().await;
        assert!(events.is_empty(), "heartbeat comment must not yield event");
    }

    #[tokio::test]
    async fn test_multi_chunk_frame_reassembled() {
        // Frame split across two chunks.
        let chunk1 = "event: memory\n";
        let chunk2 = "data: {\"x\":1}\n\n";
        let chunks = tokio_stream::iter(vec![chunk1.to_string(), chunk2.to_string()]);
        let events: Vec<_> = parse_sse_chunks(chunks).collect().await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "memory");
    }

    #[tokio::test]
    async fn test_crlf_line_endings_stripped() {
        let raw = "event: memory\r\ndata: {\"x\":1}\r\n\r\n";
        let chunks = tokio_stream::iter(vec![raw.to_string()]);
        let events: Vec<_> = parse_sse_chunks(chunks).collect().await;
        assert_eq!(events.len(), 1);
    }

    #[tokio::test]
    async fn test_multi_line_data_joined_with_newline() {
        // Python: multiple data: lines joined with "\n".
        let raw = "event: memory\ndata: line1\ndata: line2\n\n";
        let chunks = tokio_stream::iter(vec![raw.to_string()]);
        let events: Vec<_> = parse_sse_chunks(chunks).collect().await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "line1\nline2");
    }

    #[tokio::test]
    async fn test_id_captured() {
        let raw = "event: memory\nid: 99\ndata: hello\n\n";
        let chunks = tokio_stream::iter(vec![raw.to_string()]);
        let events: Vec<_> = parse_sse_chunks(chunks).collect().await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id.as_deref(), Some("99"));
    }

    #[tokio::test]
    async fn test_no_event_type_defaults_to_message() {
        // No "event:" line — defaults to "message".
        let raw = "data: hello\n\n";
        let chunks = tokio_stream::iter(vec![raw.to_string()]);
        let events: Vec<_> = parse_sse_chunks(chunks).collect().await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "message");
        assert_eq!(events[0].data, "hello");
    }

    #[tokio::test]
    async fn test_blank_line_without_data_does_not_emit() {
        // Blank line after blank line (or at start) with no content → no event.
        let raw = "\n\n";
        let chunks = tokio_stream::iter(vec![raw.to_string()]);
        let events: Vec<_> = parse_sse_chunks(chunks).collect().await;
        assert!(events.is_empty(), "blank frame must not yield event");
    }

    #[tokio::test]
    async fn test_multiple_events_in_one_chunk() {
        let raw = "event: memory\ndata: first\n\nevent: memory\ndata: second\n\n";
        let chunks = tokio_stream::iter(vec![raw.to_string()]);
        let events: Vec<_> = parse_sse_chunks(chunks).collect().await;
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].data, "first");
        assert_eq!(events[1].data, "second");
    }

    #[tokio::test]
    async fn test_event_type_resets_between_frames() {
        // First frame sets event_type; second frame has none → must default to "message".
        let raw = "event: memory\ndata: a\n\ndata: b\n\n";
        let chunks = tokio_stream::iter(vec![raw.to_string()]);
        let events: Vec<_> = parse_sse_chunks(chunks).collect().await;
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type, "memory");
        assert_eq!(events[1].event_type, "message");
    }

    #[tokio::test]
    async fn test_partial_chunk_buffering() {
        // Frame split mid-field line.
        let chunk1 = "event: mem";
        let chunk2 = "ory\ndata: {}\n\n";
        let chunks = tokio_stream::iter(vec![chunk1.to_string(), chunk2.to_string()]);
        let events: Vec<_> = parse_sse_chunks(chunks).collect().await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "memory");
    }
}
