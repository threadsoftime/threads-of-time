// SPDX-License-Identifier: AGPL-3.0
//! SSE consumer — faithful Rust port of `brain_sidecar/sse_consumer.py`.
//!
//! # Design
//!
//! One `SseConsumer` per enrolled bot.  It subscribes to the memory-sidecar
//! event stream, filters by prefix, coalesces bursts in a debounce window,
//! deduplicates via a shared `SeenMemoryIds`, persists the Last-Event-ID
//! cursor (watermark) in `StateStore`, and reconnects with backoff on stream
//! drop.
//!
//! ## Key ordering invariant
//!
//! For every `memory` event:
//!   1. `dedup.seen(memory_id)` → skip if already seen (early return).
//!   2. `dedup.mark(memory_id)` — mark BEFORE calling on_events.
//!   3. `state_store.write_last_event_id(bot_guid, row_id)` — monotonic.
//!   4. `coalesce.add(payload)` — hand to coalesce buffer.
//!
//! Step 2 before step 4 is the Python invariant: the memory is considered
//! processed as soon as it is marked, even if `on_events` hasn't fired yet.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use futures_util::StreamExt;
use serde_json::Value;
use tracing::{info, warn};

use crate::dedup::SeenMemoryIds;
use crate::sse_parser::{parse_sse_chunks, SseEvent};
use crate::state::StateStore;

// ---------------------------------------------------------------------------
// Backoff sequence (matches Python `_BACKOFF_SEQ`)
// ---------------------------------------------------------------------------

/// Reconnect delays in seconds: 0.5, 1.0, 2.0, 5.0, 5.0 (then pinned at 5.0).
const BACKOFF_SEQ: &[f64] = &[0.5, 1.0, 2.0, 5.0, 5.0];

// ---------------------------------------------------------------------------
// CoalesceBuffer
// ---------------------------------------------------------------------------

/// Debounce buffer: accumulates events; fires `on_drain` after `window_ms` of quiet.
///
/// Faithful port of Python `_CoalesceBuffer`:
/// - `add(item)` appends to buf and (re)starts a debounce task.
/// - `_wait_and_drain` sleeps `window_s`; if cancelled, returns without draining.
/// - On drain: copies buf, clears it, then calls `on_drain`.
///
/// The `on_drain` callback is a non-async `Arc<dyn Fn(Vec<Value>) + Send + Sync>`.
/// The drain task spawns its own async context.
pub struct CoalesceBuffer {
    window_ms: u64,
    buf: Arc<Mutex<Vec<Value>>>,
    drain_task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    on_drain: Arc<dyn Fn(Vec<Value>) + Send + Sync + 'static>,
}

impl CoalesceBuffer {
    /// Create a new CoalesceBuffer.
    ///
    /// `on_drain` is called with the drained items when the quiet window expires.
    /// It is called from a spawned task.
    pub fn new<F>(window_ms: u64, on_drain: F) -> Self
    where
        F: Fn(Vec<Value>) + Send + Sync + 'static,
    {
        Self {
            window_ms,
            buf: Arc::new(Mutex::new(Vec::new())),
            drain_task: Arc::new(Mutex::new(None)),
            on_drain: Arc::new(on_drain),
        }
    }

    /// Add an item; restart the debounce timer.
    ///
    /// Mirrors Python:
    /// ```python
    /// async with self._lock:
    ///     self._buf.append(item)
    ///     if self._task is not None and not self._task.done():
    ///         self._task.cancel()
    ///     self._task = asyncio.create_task(self._wait_and_drain())
    /// ```
    pub fn add(&self, item: Value) {
        {
            let mut buf = self.buf.lock().unwrap();
            buf.push(item);
        }
        // Cancel previous drain task.
        {
            let mut task_guard = self.drain_task.lock().unwrap();
            if let Some(h) = task_guard.take() {
                h.abort();
            }
        }
        // Spawn a new drain task.
        let window_ms = self.window_ms;
        let buf = Arc::clone(&self.buf);
        let on_drain = Arc::clone(&self.on_drain);
        let handle = tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(window_ms)).await;
            // Drain.
            let items = {
                let mut b = buf.lock().unwrap();
                if b.is_empty() {
                    return;
                }
                let items: Vec<Value> = b.drain(..).collect();
                items
            };
            on_drain(items);
        });
        let mut task_guard = self.drain_task.lock().unwrap();
        *task_guard = Some(handle);
    }
}

// ---------------------------------------------------------------------------
// SseConsumer
// ---------------------------------------------------------------------------

/// Per-bot SSE subscriber.
///
/// Faithful port of Python `SseConsumer` + `_CoalesceBuffer`.
pub struct SseConsumer {
    pub bot_guid: i64,
    pub memory_url: String,
    pub bearer: String,
    pub state_store: Arc<StateStore>,
    /// Called with coalesced payloads (one or more `Value`s per burst).
    pub on_events: Arc<dyn Fn(Vec<Value>) + Send + Sync + 'static>,
    pub prefixes: Vec<String>,
    pub coalesce_window_ms: u64,
    /// Shared dedup set — injected so SSE path and polling path share the same set.
    pub dedup_set: Arc<Mutex<SeenMemoryIds>>,
    /// Set to `true` to stop the run loop on the next iteration.
    pub cancel: Arc<AtomicBool>,
}

impl SseConsumer {
    /// Process one parsed SSE frame.
    ///
    /// Port of Python `_handle_event`.
    ///
    /// Only `memory` events are processed.  For each:
    ///   1. Parse JSON payload.
    ///   2. Skip if `dedup.seen(memory_id)`.
    ///   3. `dedup.mark(memory_id)` — mark BEFORE callback.
    ///   4. `state_store.write_last_event_id` — monotonic persist of cursor.
    ///   5. `coalesce.add(payload)` — hand to coalesce buffer.
    pub fn handle_event(&self, event: &SseEvent, coalesce: &CoalesceBuffer) {
        if event.event_type != "memory" {
            return;
        }

        let payload: Value = match serde_json::from_str(&event.data) {
            Ok(v) => v,
            Err(e) => {
                warn!(
                    bot_guid = self.bot_guid,
                    data = %event.data.chars().take(200).collect::<String>(),
                    error = %e,
                    "sse_bad_payload"
                );
                return;
            }
        };

        let row_id = match payload.get("row_id").and_then(|v| v.as_i64()) {
            Some(id) => id,
            None => {
                warn!(bot_guid = self.bot_guid, "sse_missing_row_id");
                return;
            }
        };
        let memory_id = match payload.get("memory_id").and_then(|v| v.as_str()) {
            Some(id) => id.to_string(),
            None => {
                warn!(bot_guid = self.bot_guid, "sse_missing_memory_id");
                return;
            }
        };

        // Dedup check + mark (mark BEFORE coalesce.add — Python invariant).
        {
            let mut dedup = self.dedup_set.lock().unwrap();
            if dedup.seen(&memory_id) {
                return;
            }
            dedup.mark(&memory_id);
        }

        // Persist watermark monotonically (ignore failures — log only).
        if let Err(e) = self.state_store.write_last_event_id(self.bot_guid, row_id) {
            warn!(
                bot_guid = self.bot_guid,
                row_id = row_id,
                error = %e,
                "sse_state_write_failed"
            );
        }

        coalesce.add(payload);
    }

    /// Open one SSE stream and consume until closed or cancelled.
    ///
    /// Port of Python `_stream_once`.  Uses reqwest 0.12 streaming via
    /// `Response::bytes_stream()`, decoded as UTF-8, fed into `parse_sse_chunks`.
    pub async fn stream_once(&self) -> anyhow::Result<()> {
        let last_id = self
            .state_store
            .read_last_event_id(self.bot_guid)
            .unwrap_or(0);

        let url = format!("{}/v1/events/stream", self.memory_url.trim_end_matches('/'));
        let prefixes_str = self.prefixes.join(",");

        let client = reqwest::Client::builder()
            // No timeout — SSE is a long-lived streaming connection.
            // Leaving timeout unset uses reqwest's default of no timeout.
            .build()
            .map_err(|e| anyhow::anyhow!("SseConsumer: build client: {e}"))?;

        let mut req = client
            .get(&url)
            .query(&[
                ("bot_id", self.bot_guid.to_string()),
                ("prefixes", prefixes_str),
            ])
            .header("Authorization", format!("Bearer {}", self.bearer))
            .header("Accept", "text/event-stream");

        if last_id > 0 {
            req = req.header("Last-Event-ID", last_id.to_string());
        }

        let response = req
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("SseConsumer: connect: {e}"))?;
        response
            .error_for_status_ref()
            .map_err(|e| anyhow::anyhow!("SseConsumer: HTTP status: {e}"))?;

        info!(
            bot_guid = self.bot_guid,
            last_event_id = last_id,
            "sse_subscribed"
        );

        let on_events = Arc::clone(&self.on_events);
        let coalesce = CoalesceBuffer::new(self.coalesce_window_ms, move |items| {
            on_events(items);
        });

        // reqwest 0.12: `bytes_stream()` → Stream<Item = Result<Bytes, Error>>.
        // Decode each chunk as UTF-8 (lossy) and feed as String to parse_sse_chunks.
        let byte_stream = response.bytes_stream();
        let text_stream = byte_stream.filter_map(|chunk_result| async move {
            match chunk_result {
                Ok(bytes) => Some(String::from_utf8_lossy(&bytes).into_owned()),
                Err(e) => {
                    warn!(error = %e, "sse_chunk_error");
                    None
                }
            }
        });

        let mut events = std::pin::pin!(parse_sse_chunks(text_stream));
        while let Some(ev) = events.next().await {
            if self.cancel.load(Ordering::Relaxed) {
                return Ok(());
            }
            self.handle_event(&ev, &coalesce);
        }

        Ok(())
    }

    /// Run the consumer loop until cancelled or `max_attempts` exhausted.
    ///
    /// Port of Python `run_until_complete`:
    /// - On successful stream completion, `backoff_idx` resets to 0.
    /// - On failure, backs off using `BACKOFF_SEQ[min(backoff_idx, last)]`.
    /// - `max_attempts` is for unit-test use only; production calls without it.
    pub async fn run_until_complete(&self, max_attempts: Option<usize>) {
        let mut attempts: usize = 0;
        let mut backoff_idx: usize = 0;

        while !self.cancel.load(Ordering::Relaxed) {
            match self.stream_once().await {
                Ok(()) => {
                    backoff_idx = 0; // successful run → reset backoff
                    if max_attempts.is_some() {
                        return; // test mode: one successful run → done
                    }
                }
                Err(e) => {
                    warn!(bot_guid = self.bot_guid, error = %e, "sse_disconnect");
                    attempts += 1;
                    if let Some(max) = max_attempts {
                        if attempts >= max {
                            return;
                        }
                    }
                    let delay_s = BACKOFF_SEQ[backoff_idx.min(BACKOFF_SEQ.len() - 1)];
                    backoff_idx += 1;
                    tokio::time::sleep(tokio::time::Duration::from_secs_f64(delay_s)).await;
                }
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
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};
    use tempfile::NamedTempFile;

    use axum::{
        Router,
        routing::get,
        response::{
            sse::{Event, Sse},
            IntoResponse,
        },
        http::HeaderMap,
    };
    use futures_util::stream;

    use crate::dedup::{SeenMemoryIds, DEFAULT_CAPACITY};
    use crate::models::PersonalityCard;
    use crate::state::StateStore;

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn temp_store_with_bot(bot_guid: i64) -> (Arc<StateStore>, NamedTempFile) {
        let f = NamedTempFile::new().unwrap();
        let s = StateStore::open(f.path().to_str().unwrap()).unwrap();
        s.migrate().unwrap();
        let card = PersonalityCard {
            name: "T".into(),
            race: "Human".into(),
            class_: "Warrior".into(),
            backstory: "x".into(),
            talkativeness: 0.5,
            courage: 0.5,
            greed: 0.0,
            attitude_to_master: 0.0,
            party_invite_policy: "accept_all".into(),
            pvp_appetite: None,
            raid_appetite: None,
            completionist_streak: None,
            gold_motivation: None,
            profession_appetite: None,
        };
        s.enroll(bot_guid, 0, &card).unwrap();
        (Arc::new(s), f)
    }

    fn memory_event_frame(row_id: i64, memory_id: &str) -> String {
        let data = serde_json::json!({
            "row_id": row_id,
            "memory_id": memory_id,
        });
        format!(
            "event: memory\nid: {}\ndata: {}\n\n",
            row_id,
            serde_json::to_string(&data).unwrap()
        )
    }

    /// Spawn a mock SSE server that streams the given frames then closes.
    /// Returns the base URL.
    async fn spawn_sse_server(frames: Vec<String>) -> String {
        let app = Router::new().route(
            "/v1/events/stream",
            get(move || {
                let frames = frames.clone();
                async move {
                    let s = stream::iter(
                        frames
                            .into_iter()
                            .map(|f| Ok::<Event, std::convert::Infallible>(Event::default().data(f)))
                    );
                    Sse::new(s).into_response()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://127.0.0.1:{}", addr.port())
    }

    /// Spawn a mock SSE server that serves raw SSE text (NOT through axum's Event
    /// builder — so we control the exact bytes including `event:`/`id:`/`data:` lines).
    async fn spawn_raw_sse_server(raw_body: String) -> String {
        use axum::response::Response;
        use axum::body::Body;
        use axum::http::header;

        let app = Router::new().route(
            "/v1/events/stream",
            get(move || {
                let body = raw_body.clone();
                async move {
                    Response::builder()
                        .header(header::CONTENT_TYPE, "text/event-stream")
                        .header(header::CACHE_CONTROL, "no-cache")
                        .body(Body::from(body))
                        .unwrap()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://127.0.0.1:{}", addr.port())
    }

    // -----------------------------------------------------------------------
    // CoalesceBuffer tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_coalesce_buffer_drains_after_window() {
        let received: Arc<Mutex<Vec<Vec<Value>>>> = Arc::new(Mutex::new(Vec::new()));
        let recv_clone = Arc::clone(&received);

        let buf = CoalesceBuffer::new(50, move |items| {
            recv_clone.lock().unwrap().push(items);
        });

        // Add 3 items within the window.
        let item = serde_json::json!({"row_id": 1});
        buf.add(item.clone());
        buf.add(item.clone());
        buf.add(item.clone());

        // Wait longer than the window for drain to fire.
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        let calls = received.lock().unwrap();
        assert_eq!(calls.len(), 1, "drain fires exactly once");
        assert_eq!(calls[0].len(), 3, "all 3 items in one drain");
    }

    #[tokio::test]
    async fn test_coalesce_buffer_debounces() {
        // Add items spaced apart — each reset extends the timer.
        // Total: 3 items, but timer resets each time; one batch at end.
        let received: Arc<Mutex<Vec<Vec<Value>>>> = Arc::new(Mutex::new(Vec::new()));
        let recv_clone = Arc::clone(&received);

        let buf = CoalesceBuffer::new(80, move |items| {
            recv_clone.lock().unwrap().push(items);
        });

        let item = serde_json::json!({"row_id": 1});
        buf.add(item.clone());
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
        buf.add(item.clone());
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
        buf.add(item.clone());

        // Wait for the final window to expire.
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        let calls = received.lock().unwrap();
        assert_eq!(calls.len(), 1, "debounce: only one drain despite multiple adds");
        assert_eq!(calls[0].len(), 3);
    }

    // -----------------------------------------------------------------------
    // handle_event / dedup tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_dedup_prevents_double_processing() {
        let (store, _f) = temp_store_with_bot(1001);

        let consumer = SseConsumer {
            bot_guid: 1001,
            memory_url: "http://localhost".into(),
            bearer: "tok".into(),
            state_store: Arc::clone(&store),
            on_events: Arc::new(|_| {}),
            prefixes: vec![],
            coalesce_window_ms: 50,
            dedup_set: Arc::new(Mutex::new(SeenMemoryIds::new(DEFAULT_CAPACITY))),
            cancel: Arc::new(AtomicBool::new(false)),
        };

        // We test handle_event directly (exposed as pub for unit testing — mirrors Python).
        let coalesce = CoalesceBuffer::new(50, |_| {});

        let ev = SseEvent {
            event_type: "memory".into(),
            id: Some("1".into()),
            data: r#"{"row_id":1,"memory_id":"uuid-aaa"}"#.into(),
        };

        // First handle — should be processed.
        consumer.handle_event(&ev, &coalesce);
        // Second handle — dedup should skip.
        consumer.handle_event(&ev, &coalesce);

        assert!(
            consumer.dedup_set.lock().unwrap().seen("uuid-aaa"),
            "memory_id must be marked"
        );

        // Watermark updated by first handle; second is deduped so no regression.
        let last_id = store.read_last_event_id(1001).unwrap();
        assert_eq!(last_id, 1, "row_id=1 must be persisted after first handle");
    }

    #[tokio::test]
    async fn test_dedup_mark_before_callback_ordering() {
        // The dedup mark must happen BEFORE on_events is called.
        // We verify by checking that after handle_event the memory_id is seen
        // even if on_events hasn't been awaited (coalesce window still open).
        let (store, _f) = temp_store_with_bot(2001);
        let consumer = SseConsumer {
            bot_guid: 2001,
            memory_url: "http://localhost".into(),
            bearer: "tok".into(),
            state_store: Arc::clone(&store),
            on_events: Arc::new(|_| {}),
            prefixes: vec![],
            coalesce_window_ms: 5000, // long window — drain won't fire during test
            dedup_set: Arc::new(Mutex::new(SeenMemoryIds::new(DEFAULT_CAPACITY))),
            cancel: Arc::new(AtomicBool::new(false)),
        };

        // Long coalesce window so on_events is never called during the test.
        let coalesce = CoalesceBuffer::new(5000, |_| {});

        let ev = SseEvent {
            event_type: "memory".into(),
            id: Some("5".into()),
            data: r#"{"row_id":5,"memory_id":"uuid-mark-order"}"#.into(),
        };

        consumer.handle_event(&ev, &coalesce);

        // Even though the coalesce window hasn't elapsed (on_events not yet called),
        // the dedup mark must already be set.
        assert!(
            consumer.dedup_set.lock().unwrap().seen("uuid-mark-order"),
            "memory_id must be marked before on_events fires"
        );
    }

    #[tokio::test]
    async fn test_non_memory_events_ignored() {
        let (store, _f) = temp_store_with_bot(3001);
        let consumer = SseConsumer {
            bot_guid: 3001,
            memory_url: "http://localhost".into(),
            bearer: "tok".into(),
            state_store: Arc::clone(&store),
            on_events: Arc::new(|_| {}),
            prefixes: vec![],
            coalesce_window_ms: 50,
            dedup_set: Arc::new(Mutex::new(SeenMemoryIds::new(DEFAULT_CAPACITY))),
            cancel: Arc::new(AtomicBool::new(false)),
        };

        let coalesce = CoalesceBuffer::new(50, |_| {});

        let heartbeat = SseEvent {
            event_type: "heartbeat".into(),
            id: None,
            data: r#"{"ts":12345}"#.into(),
        };
        consumer.handle_event(&heartbeat, &coalesce);

        // No state written.
        let last_id = store.read_last_event_id(3001).unwrap();
        assert_eq!(last_id, 0, "heartbeat must not advance cursor");
    }

    // -----------------------------------------------------------------------
    // Integration tests — local axum mock SSE server
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_watermark_resume_sends_last_event_id_header() {
        // state_store has last_event_id=100; verify the request carries
        // Last-Event-ID: 100 header on connect.
        let captured_headers: Arc<Mutex<Option<HeaderMap>>> = Arc::new(Mutex::new(None));
        let cap = Arc::clone(&captured_headers);

        let (store, _f) = temp_store_with_bot(4001);
        // Write a known watermark.
        store.write_last_event_id(4001, 100).unwrap();

        let app = Router::new().route(
            "/v1/events/stream",
            get(move |headers: HeaderMap| {
                let cap = Arc::clone(&cap);
                async move {
                    *cap.lock().unwrap() = Some(headers);
                    // Return empty SSE stream (just double-newline to close).
                    use axum::response::Response;
                    use axum::body::Body;
                    Response::builder()
                        .header("Content-Type", "text/event-stream")
                        .body(Body::from("\n\n"))
                        .unwrap()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let base_url = format!("http://127.0.0.1:{}", addr.port());

        let consumer = SseConsumer {
            bot_guid: 4001,
            memory_url: base_url,
            bearer: "secret".into(),
            state_store: Arc::clone(&store),
            on_events: Arc::new(|_| {}),
            prefixes: vec!["hello".into()],
            coalesce_window_ms: 50,
            dedup_set: Arc::new(Mutex::new(SeenMemoryIds::new(DEFAULT_CAPACITY))),
            cancel: Arc::new(AtomicBool::new(false)),
        };

        consumer.stream_once().await.unwrap();

        let headers = captured_headers.lock().unwrap();
        let headers = headers.as_ref().expect("headers captured");
        let lei = headers
            .get("last-event-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(lei, "100", "Last-Event-ID: 100 must be sent on resume");
    }

    #[tokio::test]
    async fn test_reconnect_backoff_sequence() {
        // Mock server that returns 500 on first two attempts, then 200 + empty stream.
        let attempt_count: Arc<Mutex<usize>> = Arc::new(Mutex::new(0));
        let ac = Arc::clone(&attempt_count);
        let delays: Arc<Mutex<Vec<std::time::Instant>>> = Arc::new(Mutex::new(Vec::new()));
        let dl = Arc::clone(&delays);

        let app = Router::new().route(
            "/v1/events/stream",
            get(move || {
                let ac = Arc::clone(&ac);
                let dl = Arc::clone(&dl);
                async move {
                    dl.lock().unwrap().push(std::time::Instant::now());
                    let mut count = ac.lock().unwrap();
                    *count += 1;
                    let c = *count;
                    drop(count);
                    use axum::http::StatusCode;
                    use axum::response::Response;
                    use axum::body::Body;
                    if c <= 2 {
                        Response::builder()
                            .status(StatusCode::INTERNAL_SERVER_ERROR)
                            .body(Body::from("error"))
                            .unwrap()
                    } else {
                        Response::builder()
                            .header("Content-Type", "text/event-stream")
                            .body(Body::from("\n\n"))
                            .unwrap()
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let base_url = format!("http://127.0.0.1:{}", addr.port());

        let (store, _f) = temp_store_with_bot(5001);

        let consumer = SseConsumer {
            bot_guid: 5001,
            memory_url: base_url,
            bearer: "".into(),
            state_store: Arc::clone(&store),
            on_events: Arc::new(|_| {}),
            prefixes: vec!["x".into()],
            coalesce_window_ms: 50,
            dedup_set: Arc::new(Mutex::new(SeenMemoryIds::new(DEFAULT_CAPACITY))),
            cancel: Arc::new(AtomicBool::new(false)),
        };

        // max_attempts=2 means: 2 failures → return.
        consumer.run_until_complete(Some(2)).await;

        let count = *attempt_count.lock().unwrap();
        assert!(count >= 2, "must attempt at least 2 times; got {count}");

        // Verify backoff delay between first and second attempt >= 0.4s (nominal 0.5s).
        let times = delays.lock().unwrap();
        if times.len() >= 2 {
            let gap = times[1].duration_since(times[0]);
            assert!(
                gap >= std::time::Duration::from_millis(400),
                "first backoff gap {gap:?} must be >= 400ms (nominal 500ms)"
            );
        }
    }

    #[tokio::test]
    async fn test_stream_once_processes_memory_event() {
        // Mock SSE server emits one memory event.
        let raw_body = memory_event_frame(42, "uuid-test-process");
        let base_url = spawn_raw_sse_server(raw_body).await;

        let (store, _f) = temp_store_with_bot(6001);
        let received: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let recv_clone = Arc::clone(&received);

        let consumer = SseConsumer {
            bot_guid: 6001,
            memory_url: base_url,
            bearer: "".into(),
            state_store: Arc::clone(&store),
            on_events: Arc::new(move |items| {
                recv_clone.lock().unwrap().extend(items);
            }),
            prefixes: vec![],
            coalesce_window_ms: 50,
            dedup_set: Arc::new(Mutex::new(SeenMemoryIds::new(DEFAULT_CAPACITY))),
            cancel: Arc::new(AtomicBool::new(false)),
        };

        consumer.stream_once().await.unwrap();

        // Wait for coalesce drain.
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        // Dedup mark must be set.
        assert!(
            consumer.dedup_set.lock().unwrap().seen("uuid-test-process"),
            "memory_id must be marked after processing"
        );

        // Watermark must be updated.
        let last_id = store.read_last_event_id(6001).unwrap();
        assert_eq!(last_id, 42, "last_event_id must be updated to row_id=42");
    }

    #[tokio::test]
    async fn test_last_event_id_zero_not_sent() {
        // When last_event_id=0 (no watermark), the header must NOT be sent.
        let captured_headers: Arc<Mutex<Option<HeaderMap>>> = Arc::new(Mutex::new(None));
        let cap = Arc::clone(&captured_headers);

        let app = Router::new().route(
            "/v1/events/stream",
            get(move |headers: HeaderMap| {
                let cap = Arc::clone(&cap);
                async move {
                    *cap.lock().unwrap() = Some(headers);
                    use axum::response::Response;
                    use axum::body::Body;
                    Response::builder()
                        .header("Content-Type", "text/event-stream")
                        .body(Body::from("\n\n"))
                        .unwrap()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let base_url = format!("http://127.0.0.1:{}", addr.port());

        let (store, _f) = temp_store_with_bot(7001);

        let consumer = SseConsumer {
            bot_guid: 7001,
            memory_url: base_url,
            bearer: "".into(),
            state_store: Arc::clone(&store),
            on_events: Arc::new(|_| {}),
            prefixes: vec!["x".into()],
            coalesce_window_ms: 50,
            dedup_set: Arc::new(Mutex::new(SeenMemoryIds::new(DEFAULT_CAPACITY))),
            cancel: Arc::new(AtomicBool::new(false)),
        };

        consumer.stream_once().await.unwrap();

        let headers = captured_headers.lock().unwrap();
        let headers = headers.as_ref().expect("headers captured");
        let lei = headers.get("last-event-id");
        assert!(
            lei.is_none(),
            "Last-Event-ID must NOT be sent when watermark is 0"
        );
    }
}
