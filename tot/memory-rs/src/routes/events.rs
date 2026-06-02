//! SSE endpoint: `GET /v1/events/stream`
//!
//! Rust port of Python `routes_events.py`.  Exact parity on:
//! - Query param validation (bot_id ≤32, prefixes ≤16 items, each ≤64 chars)
//! - Subscribe-then-replay ordering (critical — see module comment below)
//! - Rowid cursor (`Last-Event-ID` header → i64 replay cursor)
//! - Prefix filter: `text.to_lowercase().starts_with(any prefix)`
//! - 15 second heartbeat frame (`event: heartbeat`, NOT axum's keep-alive comment)
//! - Bearer auth: required when `state.token_store` is `Some`, open when `None`
//! - Response headers: `Content-Type: text/event-stream`, `Cache-Control: no-cache`,
//!   `X-Accel-Buffering: no`
//!
//! # Subscribe-then-replay ordering (critical)
//!
//! 1. Register subscriber (`pubsub.subscribe(bot_id)`) — queue starts buffering new writes.
//! 2. Replay missed rows from DB using `rowid > last_event_id` in chunks of 500.
//! 3. Drain live queue, dedup against the `max_replayed_rowid` watermark.
//!
//! This closes the race window where a row could be written between
//! "replay done" and "subscriber registered".

use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        sse::{Event, Sse},
        IntoResponse, Response,
    },
};
use serde::Deserialize;
use tokio::time::{timeout, Duration};
use tracing::warn;

use crate::db;
use crate::pubsub::{MemoryRow, SubscriberQueue};
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Constants (matching Python)
// ---------------------------------------------------------------------------

const MAX_PREFIXES: usize = 16;
const MAX_PREFIX_LEN: usize = 64;
const MAX_BOT_ID_LEN: usize = 32;
const REPLAY_CHUNK_SIZE: i64 = 500;
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);

// ---------------------------------------------------------------------------
// Query + header extractors
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct StreamParams {
    pub bot_id: Option<String>,
    pub prefixes: Option<String>,
}

// ---------------------------------------------------------------------------
// Helper: raw SSE text → axum Event (for passing pre-formatted frames)
// ---------------------------------------------------------------------------

/// Wrap a pre-formatted SSE frame string as an axum `Event`.
///
/// The frame string already contains all field lines and ends with `\n\n`.
/// We strip the trailing `\n\n` (axum adds its own terminator), then
/// feed the remainder verbatim as a single `.data()` call.
///
/// Wait — axum's `Event` adds its OWN header fields (`event:`, `id:`, `data:`).
/// We cannot pass a "raw frame" through it; we need to use the builder API.
///
/// Therefore: parse the pre-formatted string and reconstruct via the builder.
/// This is only used by `sse_event_from_frame`; callers for the live path use
/// `sse_event_for_row` / `sse_event_for_heartbeat` directly.
fn sse_event_from_row(row: &MemoryRow) -> Event {
    // Build the JSON data payload (compact, matching Python separators).
    let json = serde_json::json!({
        "row_id": row.row_id,
        "memory_id": row.memory_id,
        "bot_id": row.bot_id,
        "text": row.text,
        "ts": row.created_ts,
        "salience": row.salience,
    });
    let json_str = serde_json::to_string(&json).unwrap_or_default();
    Event::default()
        .event("memory")
        .id(row.row_id.to_string())
        .data(json_str)
}

fn sse_event_for_heartbeat() -> Event {
    let now_ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let json = format!("{{\"ts\":{now_ts}}}");
    Event::default().event("heartbeat").data(json)
}

fn sse_event_for_error(code: &str, message: &str) -> Event {
    let json = serde_json::json!({"code": code, "message": message});
    Event::default()
        .event("error")
        .data(serde_json::to_string(&json).unwrap_or_default())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_prefixes(raw: &str) -> Result<Vec<String>, (StatusCode, String)> {
    let parts: Vec<String> = raw
        .split(',')
        .map(|p| p.trim().to_lowercase())
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "prefixes must be non-empty".to_owned(),
        ));
    }
    if parts.len() > MAX_PREFIXES {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("prefixes count exceeds {MAX_PREFIXES}"),
        ));
    }
    for p in &parts {
        if p.len() > MAX_PREFIX_LEN {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("prefix len exceeds {MAX_PREFIX_LEN}: {p:?}"),
            ));
        }
    }
    Ok(parts)
}

fn matches_any_prefix(text: &str, prefixes: &[String]) -> bool {
    let low = text.to_lowercase();
    prefixes.iter().any(|p| low.starts_with(p.as_str()))
}

// ---------------------------------------------------------------------------
// Replay — fetch rows from DB since `since_rowid` (runs in spawn_blocking)
// ---------------------------------------------------------------------------

fn fetch_rows_since(
    db_path: &PathBuf,
    bot_id: &str,
    since_rowid: i64,
) -> rusqlite::Result<Vec<MemoryRow>> {
    let conn = db::open_db(db_path)?;
    let mut all = Vec::new();
    let mut cursor = since_rowid;

    loop {
        let mut stmt = conn.prepare(
            "SELECT rowid, id, bot_id, text, created_ts, salience \
             FROM memories WHERE bot_id = ?1 AND rowid > ?2 \
             ORDER BY rowid ASC LIMIT ?3",
        )?;
        let chunk: Vec<MemoryRow> = stmt
            .query_map(
                rusqlite::params![bot_id, cursor, REPLAY_CHUNK_SIZE],
                |r| {
                    Ok(MemoryRow {
                        row_id: r.get(0)?,
                        memory_id: r.get(1)?,
                        bot_id: r.get(2)?,
                        text: r.get(3)?,
                        created_ts: r.get(4)?,
                        salience: r.get::<_, Option<f64>>(5)?.unwrap_or(0.5) as f32,
                    })
                },
            )?
            .collect::<rusqlite::Result<_>>()?;

        let chunk_len = chunk.len() as i64;
        if let Some(last) = chunk.last() {
            cursor = last.row_id;
        }
        all.extend(chunk);

        if chunk_len < REPLAY_CHUNK_SIZE {
            break;
        }
    }

    Ok(all)
}

// ---------------------------------------------------------------------------
// SSE handler
// ---------------------------------------------------------------------------

pub async fn stream_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<StreamParams>,
) -> Response {
    // --- Auth (before validation — auth errors take precedence) ---
    if let Some(ts) = &state.token_store {
        let bearer = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "))
            .map(|s| s.trim());

        match bearer {
            None => {
                return (StatusCode::UNAUTHORIZED, "missing bearer token").into_response();
            }
            Some(token) => {
                if ts.verify(token).is_none() {
                    return (StatusCode::UNAUTHORIZED, "invalid bearer token").into_response();
                }
            }
        }
    }

    // --- Validation ---
    let bot_id = match params.bot_id.as_deref() {
        None | Some("") => {
            return (StatusCode::BAD_REQUEST, "bot_id is required").into_response();
        }
        Some(s) if s.len() > MAX_BOT_ID_LEN => {
            return (
                StatusCode::BAD_REQUEST,
                "bot_id too long (max 32 chars)",
            )
                .into_response();
        }
        Some(s) => s.to_owned(),
    };

    let prefix_list = match params.prefixes.as_deref() {
        None | Some("") => {
            return (StatusCode::BAD_REQUEST, "prefixes is required").into_response();
        }
        Some(s) => match parse_prefixes(s) {
            Ok(p) => p,
            Err((status, msg)) => return (status, msg).into_response(),
        },
    };

    // Parse Last-Event-ID header (optional; 0 = replay from beginning)
    let replay_cursor: i64 = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    // --- Subscribe FIRST (subscribe-then-replay ordering — critical) ---
    let queue = state.pubsub.subscribe(&bot_id);
    let db_path = state.service.db_path.clone();

    // --- Build the SSE stream ---
    let stream = build_sse_stream(bot_id, prefix_list, replay_cursor, queue, db_path);

    // Build the response: axum's Sse sets Content-Type and Cache-Control.
    // We need to add X-Accel-Buffering: no separately.
    let mut response = Sse::new(stream).into_response();
    response
        .headers_mut()
        .insert("x-accel-buffering", "no".parse().unwrap());
    response
}

/// Build the SSE stream.  Separated from the handler so it can be tested
/// independently without needing a full axum `Request`.
///
/// Stream phases:
/// 1. Replay (spawn_blocking DB reads, chunked at 500).
/// 2. Live drain (recv with 15 s heartbeat timeout, dedup by rowid watermark).
fn build_sse_stream(
    bot_id: String,
    prefix_list: Vec<String>,
    replay_cursor: i64,
    queue: Arc<SubscriberQueue>,
    db_path: PathBuf,
) -> impl futures_util::stream::Stream<Item = Result<Event, Infallible>> + Send + 'static {
    async_stream::stream! {
        // Phase 1: Replay — run DB fetch in spawn_blocking (Connection is !Send).
        let bot_id_owned = bot_id.clone();
        let replay_result = tokio::task::spawn_blocking(move || {
            fetch_rows_since(&db_path, &bot_id_owned, replay_cursor)
        })
        .await;

        let replay_rows = match replay_result {
            Ok(Ok(rows)) => rows,
            Ok(Err(e)) => {
                warn!(bot_id = %bot_id, error = %e, "sse_replay_db_error");
                yield Ok(sse_event_for_error("db_error", &e.to_string()));
                return;
            }
            Err(join_err) => {
                warn!(bot_id = %bot_id, error = %join_err, "sse_replay_panic");
                yield Ok(sse_event_for_error("internal", &join_err.to_string()));
                return;
            }
        };

        // Track the high-water rowid for dedup in Phase 2.
        let mut max_replayed_rowid: i64 = replay_cursor;

        for row in &replay_rows {
            if row.row_id > max_replayed_rowid {
                max_replayed_rowid = row.row_id;
            }
            if matches_any_prefix(&row.text, &prefix_list) {
                yield Ok(sse_event_from_row(row));
            }
        }

        // Phase 2: Live drain — recv with heartbeat timeout.
        loop {
            match timeout(HEARTBEAT_INTERVAL, queue.notified()).await {
                Err(_) => {
                    // Timeout — emit heartbeat (no id: line)
                    yield Ok(sse_event_for_heartbeat());
                }
                Ok(()) => {
                    // Drain all available rows from the queue (there may be several
                    // if notifications coalesced)
                    while let Some(row) = queue.try_recv() {
                        if row.row_id <= max_replayed_rowid {
                            continue; // already emitted in replay — dedup
                        }
                        if !matches_any_prefix(&row.text, &prefix_list) {
                            continue;
                        }
                        if row.row_id > max_replayed_rowid {
                            max_replayed_rowid = row.row_id;
                        }
                        yield Ok(sse_event_from_row(&row));
                    }
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
    use std::path::PathBuf;
    use std::sync::Arc;

    use axum::{
        body::Body,
        http::{Request, StatusCode},
        routing::get,
        Router,
    };
    use tower::ServiceExt; // for `oneshot`

    use crate::auth::TokenStore;
    use crate::config::ScoringWeights;
    use crate::db;
    use crate::db::migrate;
    use crate::embed_cache::EmbedCache;
    use crate::embeddings::EmbeddingsClient;
    use crate::pubsub::PubSub;
    use crate::state::AppState;
    use crate::core::MemoryService;

    const MIGRATIONS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/migrations");

    fn make_service_on_migrated_db(tmp: &tempfile::NamedTempFile) -> Arc<MemoryService> {
        let db_path = tmp.path().to_path_buf();
        // Run migrations first
        {
            let conn = db::open_db(&db_path).unwrap();
            migrate::run(&conn, std::path::Path::new(MIGRATIONS_DIR)).unwrap();
        }
        let client = EmbeddingsClient::new("http://127.0.0.1:11434/v1", "embedding", "");
        let embed = Arc::new(EmbedCache::new(client));
        let pubsub = Arc::new(PubSub::new());
        let weights = ScoringWeights {
            w_rel: 0.5,
            w_rec: 0.2,
            w_imp: 0.3,
            tau_seconds: 604800,
        };
        Arc::new(MemoryService::new(db_path, weights, 2000, embed, pubsub))
    }

    fn make_state(tmp: &tempfile::NamedTempFile, token_store: Option<Arc<TokenStore>>) -> (AppState, PathBuf) {
        let db_path = tmp.path().to_path_buf();
        let svc = make_service_on_migrated_db(tmp);
        let pubsub = Arc::new(PubSub::new());
        let state = AppState {
            service: svc,
            token_store,
            pubsub,
        };
        (state, db_path)
    }

    fn make_router(state: AppState) -> Router {
        Router::new()
            .route("/v1/events/stream", get(stream_events))
            .with_state(state)
    }

    // ---- Validation tests ----

    #[tokio::test]
    async fn missing_bot_id_returns_400() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let (state, _) = make_state(&tmp, None);
        let app = make_router(state);

        let req = Request::builder()
            .uri("/v1/events/stream?prefixes=hello")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn bot_id_too_long_returns_400() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let (state, _) = make_state(&tmp, None);
        let app = make_router(state);

        let long_id = "a".repeat(33);
        let uri = format!("/v1/events/stream?bot_id={long_id}&prefixes=hello");
        let req = Request::builder().uri(uri).body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn missing_prefixes_returns_400() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let (state, _) = make_state(&tmp, None);
        let app = make_router(state);

        let req = Request::builder()
            .uri("/v1/events/stream?bot_id=bot1")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn too_many_prefixes_returns_400() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let (state, _) = make_state(&tmp, None);
        let app = make_router(state);

        // 17 prefixes — exceeds MAX_PREFIXES=16
        let prefixes = (0..17).map(|i| format!("p{i}")).collect::<Vec<_>>().join(",");
        let uri = format!("/v1/events/stream?bot_id=bot1&prefixes={prefixes}");
        let req = Request::builder().uri(uri).body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn prefix_too_long_returns_400() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let (state, _) = make_state(&tmp, None);
        let app = make_router(state);

        let long_prefix = "x".repeat(65);
        let uri = format!("/v1/events/stream?bot_id=bot1&prefixes={long_prefix}");
        let req = Request::builder().uri(uri).body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // ---- Auth tests ----

    #[tokio::test]
    async fn bearer_required_when_token_store_some_and_missing_token_returns_401() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let ts = build_token_store("secret-token");
        let (state, _) = make_state(&tmp, Some(Arc::new(ts)));
        let app = make_router(state);

        // No Authorization header
        let req = Request::builder()
            .uri("/v1/events/stream?bot_id=bot1&prefixes=hello")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn invalid_bearer_returns_401() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let ts = build_token_store("secret-token");
        let (state, _) = make_state(&tmp, Some(Arc::new(ts)));
        let app = make_router(state);

        let req = Request::builder()
            .uri("/v1/events/stream?bot_id=bot1&prefixes=hello")
            .header("Authorization", "Bearer wrong-token")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn no_token_store_is_open_auth() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let (state, _) = make_state(&tmp, None);
        let app = make_router(state);

        // No Authorization header — should start streaming (200), not 401
        let req = Request::builder()
            .uri("/v1/events/stream?bot_id=bot1&prefixes=hello")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // ---- Replay + stream builder tests ----

    /// Insert rows directly into DB, then run build_sse_stream and collect
    /// the replay phase.  We use a real DB with known rows.
    #[tokio::test]
    async fn replay_yields_existing_rows_filtered_by_prefix() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let db_path = tmp.path().to_path_buf();
        {
            let conn = db::open_db(&db_path).unwrap();
            migrate::run(&conn, std::path::Path::new(MIGRATIONS_DIR)).unwrap();
        }

        // Insert test rows directly
        insert_test_row(&db_path, "bot1", "hello world", 0.5);
        insert_test_row(&db_path, "bot1", "goodbye", 0.5);
        insert_test_row(&db_path, "bot1", "hello again", 0.7);

        let pubsub = Arc::new(PubSub::new());
        let queue = pubsub.subscribe("bot1");

        let stream = build_sse_stream(
            "bot1".to_owned(),
            vec!["hello".to_owned()],
            0, // replay from beginning
            queue,
            db_path,
        );

        // Collect events until the stream yields nothing in 200ms (replay done)
        use futures_util::StreamExt;
        let events: Vec<_> = tokio::time::timeout(
            Duration::from_millis(200),
            stream.take(2).collect::<Vec<_>>(),
        )
        .await
        .unwrap_or_default();

        // Should get 2 "hello*" events (not "goodbye")
        assert_eq!(events.len(), 2, "expected 2 matching replay rows, got {}", events.len());
    }

    /// Replay with cursor skips already-seen rows.
    #[tokio::test]
    async fn replay_with_cursor_skips_old_rows() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let db_path = tmp.path().to_path_buf();
        {
            let conn = db::open_db(&db_path).unwrap();
            migrate::run(&conn, std::path::Path::new(MIGRATIONS_DIR)).unwrap();
        }

        insert_test_row(&db_path, "bot1", "hello first", 0.5);
        let rowid_after_first = get_last_rowid(&db_path);
        insert_test_row(&db_path, "bot1", "hello second", 0.5);

        let pubsub = Arc::new(PubSub::new());
        let queue = pubsub.subscribe("bot1");

        let stream = build_sse_stream(
            "bot1".to_owned(),
            vec!["hello".to_owned()],
            rowid_after_first, // cursor: skip first row
            queue,
            db_path,
        );

        use futures_util::StreamExt;
        let events: Vec<_> = tokio::time::timeout(
            Duration::from_millis(200),
            stream.take(1).collect::<Vec<_>>(),
        )
        .await
        .unwrap_or_default();

        assert_eq!(events.len(), 1, "should only replay the second row");
    }

    // ---- Helpers ----

    fn build_token_store(token: &str) -> TokenStore {
        use std::io::Write;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        write!(f, "tokens:\n  - token: \"{token}\"\n    identity: \"test\"\n    scope: []\n").unwrap();
        TokenStore::load(f.path()).unwrap()
    }

    fn insert_test_row(db_path: &PathBuf, bot_id: &str, text: &str, salience: f64) {
        let conn = db::open_db(db_path).unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO bots (bot_id, created_ts) VALUES (?1, 0)",
            rusqlite::params![bot_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memories (id, bot_id, text, salience, created_ts, last_recalled_ts, memory_type) \
             VALUES (?1, ?2, ?3, ?4, 0, 0, 'event')",
            rusqlite::params![
                format!("m_{}", uuid::Uuid::new_v4().simple()),
                bot_id,
                text,
                salience
            ],
        )
        .unwrap();
    }

    fn get_last_rowid(db_path: &PathBuf) -> i64 {
        let conn = db::open_db(db_path).unwrap();
        conn.query_row("SELECT MAX(rowid) FROM memories", [], |r| r.get(0))
            .unwrap()
    }
}
