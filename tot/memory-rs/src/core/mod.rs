//! Core memory service — the write path (Task 3.2).
//!
//! `MemoryService` owns the configuration and shared resources (embed cache,
//! pubsub bus).  It exposes async methods that combine embedding (HTTP, async)
//! with database work (blocking rusqlite, via `tokio::task::spawn_blocking`).
//!
//! # Connection model
//!
//! `rusqlite::Connection` is `!Send`, so it cannot be held across `.await`
//! points.  The pattern used here (matching Palmieri Ch 11 task-queue discipline):
//!
//! 1. Do all async work (embedding HTTP call) while holding no DB connection.
//! 2. Clone/copy all required data into owned values.
//! 3. `spawn_blocking(move || { let conn = open_db(&db_path)?; ... })`.
//! 4. `.await` on the `JoinHandle`, propagate the inner `Result`.
//!
//! This opens one connection per write.  Under expected load (≤ a few hundred
//! writes/s) this is acceptable; connection pooling is deferred to a later task.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::ScoringWeights;
use crate::db::{self, entities::upsert_entity, pack_f32_le};
use crate::embed_cache::EmbedCache;
use crate::ids::generate_memory_id;
use crate::pubsub::{MemoryRow as PubSubRow, PubSub};
use crate::retrieval::helpers::evict_if_over_cap;

// ---------------------------------------------------------------------------
// Public request / response types
// ---------------------------------------------------------------------------

/// A knowledge relation triple linking two named entities.
#[derive(Debug, Clone)]
pub struct RelationTriple {
    /// Source entity name.
    pub src: String,
    /// Relation label (e.g. "killed", "allied_with").
    pub rel: String,
    /// Destination entity name.
    pub dst: String,
}

/// Request payload for [`MemoryService::write`].
#[derive(Debug, Clone)]
pub struct WriteReq {
    pub bot_id: String,
    pub text: String,
    /// Clamped to `[0.0, 1.0]` inside `write`.
    pub salience: f32,
    pub entities: Vec<String>,
    pub relations: Vec<RelationTriple>,
    /// Memory classification.  `None` → stored as `"event"` (Python default).
    pub memory_type: Option<String>,
    /// Origin source string (e.g. `"goals:g_abc"`).  `None` → stored as SQL NULL.
    pub source: Option<String>,
}

/// Response from [`MemoryService::write`].
#[derive(Debug, Clone)]
pub struct WriteResp {
    pub memory_id: String,
    pub evicted: usize,
}

// ---------------------------------------------------------------------------
// Task 3.4 — read / update / forget / list types
// ---------------------------------------------------------------------------

/// A memory row returned by `read` and `list`.
///
/// Matches Python `MemoryRow` pydantic model in `routes_memory.py`.
/// The embedding blob is intentionally excluded — callers never need raw floats.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryRow {
    pub id: String,
    pub bot_id: String,
    pub text: String,
    pub salience: f32,
    pub memory_type: String,
    pub source: Option<String>,
    pub created_ts: i64,
    pub last_recalled_ts: i64,
}

/// Request payload for [`MemoryService::update`].
///
/// All fields except `bot_id` and `memory_id` are optional patches.
/// Matches Python `UpdateRequest` pydantic model.
#[derive(Debug, Clone)]
pub struct UpdateReq {
    pub bot_id: String,
    pub memory_id: String,
    /// When `Some`, the text is re-embedded and both `memories.embedding` and
    /// `vec_memories.embedding` are replaced.
    pub text: Option<String>,
    /// Clamped to `[0.0, 1.0]` inside `update`.
    pub salience: Option<f32>,
    pub memory_type: Option<String>,
    pub source: Option<String>,
}

/// Response from [`MemoryService::update`].
///
/// Matches Python `UpdateResponse`.
#[derive(Debug, Clone, PartialEq)]
pub struct UpdateResp {
    pub updated: bool,
    pub re_embedded: bool,
}

/// Request payload for [`MemoryService::forget`].
///
/// Matches Python `ForgetRequest`.
#[derive(Debug, Clone)]
pub struct ForgetReq {
    pub bot_id: String,
    pub memory_id: String,
}

/// Response from [`MemoryService::forget`].
///
/// Matches Python `ForgetResponse`.
#[derive(Debug, Clone, PartialEq)]
pub struct ForgetResp {
    pub forgotten: bool,
}

/// Request payload for [`MemoryService::list`].
///
/// Matches query parameters of the Python `GET /memory/list` handler.
#[derive(Debug, Clone)]
pub struct ListReq {
    pub bot_id: String,
    pub memory_type: Option<String>,
    pub source: Option<String>,
    pub since_ts: Option<i64>,
    pub until_ts: Option<i64>,
    /// Default 50, minimum 1, maximum 500 (clamped inside `list`).
    pub limit: i64,
    /// Default 0; clamped to ≥ 0 inside `list`.
    pub offset: i64,
}

/// Response from [`MemoryService::list`].
///
/// Matches Python `ListResponse`.
#[derive(Debug, Clone)]
pub struct ListResp {
    pub items: Vec<MemoryRow>,
    /// Total count of matching rows ignoring limit/offset.
    pub total: i64,
}

// ---------------------------------------------------------------------------
// MemoryService
// ---------------------------------------------------------------------------

/// The core memory service.
///
/// Cheap to clone: all heavy state is `Arc`-wrapped.
#[derive(Clone)]
pub struct MemoryService {
    pub(crate) db_path: PathBuf,
    pub(crate) weights: ScoringWeights,
    pub(crate) cap_per_bot: usize,
    pub(crate) embed: Arc<EmbedCache>,
    pub(crate) pubsub: Arc<PubSub>,
}

impl MemoryService {
    /// Construct a `MemoryService`.
    pub fn new(
        db_path: PathBuf,
        weights: ScoringWeights,
        cap_per_bot: usize,
        embed: Arc<EmbedCache>,
        pubsub: Arc<PubSub>,
    ) -> Self {
        Self { db_path, weights, cap_per_bot, embed, pubsub }
    }

    /// Write a new memory for `req.bot_id`.
    ///
    /// Exact insert order (ports `routes_memory.py::remember`):
    ///
    /// 1. `embed.embed(text).await` — async, before any DB work.
    /// 2. In `spawn_blocking`:
    ///    a. Open connection, begin implicit transaction.
    ///    b. `INSERT OR IGNORE INTO bots`.
    ///    c. Upsert each `entity` → collect `entity_ids`.
    ///    d. `INSERT INTO memories` (salience clamped to [0,1]).
    ///    e. `INSERT INTO vec_memories`.
    ///    f. `INSERT OR IGNORE INTO memory_entities` for each entity_id.
    ///    g. For each `RelationTriple`: upsert src + dst → `INSERT OR REPLACE INTO edges`.
    ///    h. `evict_if_over_cap`.
    ///    i. `conn.execute_batch("COMMIT")`.  (rusqlite auto-begins on first DML.)
    /// 3. `pubsub.publish(&row)`.
    /// 4. Return `WriteResp`.
    pub async fn write(&self, req: WriteReq) -> Result<WriteResp, crate::error::AppError> {
        // Step 1: Embed — must happen in async context before spawn_blocking.
        let embedding: Vec<f32> = match self.embed.embed(&req.text).await {
            Ok(arc) => arc.as_ref().clone(),
            Err(_) => {
                // Graceful degradation: write path stores NULL embedding (same as
                // Python — the embedder may be temporarily unavailable).
                vec![]
            }
        };

        // Clone all config values into owned types so they can move into the
        // blocking closure.  `PathBuf` + `ScoringWeights` are cheap to clone.
        let db_path = self.db_path.clone();
        let cap_per_bot = self.cap_per_bot;
        let tau_seconds = self.weights.tau_seconds;
        let req_owned = req.clone();
        let embedding_owned = embedding.clone();

        // Step 2: All blocking DB work.
        let (memory_id, evicted, salience_clamped) =
            tokio::task::spawn_blocking(move || -> Result<(String, usize, f32), crate::error::AppError> {
                let conn = db::open_db(&db_path)?;

                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);

                let memory_id = generate_memory_id();
                let salience = req_owned.salience.clamp(0.0, 1.0);
                let memory_type = req_owned
                    .memory_type
                    .as_deref()
                    .unwrap_or("event")
                    .to_owned();

                // Begin an explicit transaction.  rusqlite in autocommit mode (the
                // default) requires an explicit BEGIN before a matching COMMIT.
                conn.execute_batch("BEGIN")?;

                // (b) Ensure the bot row exists.
                conn.execute(
                    "INSERT OR IGNORE INTO bots (bot_id, created_ts) VALUES (?1, ?2)",
                    rusqlite::params![req_owned.bot_id, now],
                )?;

                // (c) Upsert entities listed in the request.
                let mut entity_ids: Vec<i64> = Vec::with_capacity(req_owned.entities.len());
                for name in &req_owned.entities {
                    let eid = upsert_entity(&conn, &req_owned.bot_id, name, None)?;
                    entity_ids.push(eid);
                }

                // (d) INSERT INTO memories.
                let embedding_blob = if embedding_owned.is_empty() {
                    None
                } else {
                    Some(pack_f32_le(&embedding_owned))
                };
                conn.execute(
                    "INSERT INTO memories \
                     (id, bot_id, text, salience, created_ts, last_recalled_ts, \
                      embedding, memory_type, source) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, ?7, ?8)",
                    rusqlite::params![
                        memory_id,
                        req_owned.bot_id,
                        req_owned.text,
                        salience as f64,
                        now,
                        embedding_blob,
                        memory_type,
                        req_owned.source,
                    ],
                )?;

                // (e) INSERT INTO vec_memories.
                if !embedding_owned.is_empty() {
                    let vec_blob = pack_f32_le(&embedding_owned);
                    conn.execute(
                        "INSERT INTO vec_memories (memory_id, bot_id, embedding) \
                         VALUES (?1, ?2, ?3)",
                        rusqlite::params![memory_id, req_owned.bot_id, vec_blob],
                    )?;
                }

                // (f) Link memory → entities.
                for eid in &entity_ids {
                    conn.execute(
                        "INSERT OR IGNORE INTO memory_entities (memory_id, entity_id) \
                         VALUES (?1, ?2)",
                        rusqlite::params![memory_id, eid],
                    )?;
                }

                // (g) Upsert relation edges (INSERT OR REPLACE).
                for rel in &req_owned.relations {
                    let src_id = upsert_entity(&conn, &req_owned.bot_id, &rel.src, None)?;
                    let dst_id = upsert_entity(&conn, &req_owned.bot_id, &rel.dst, None)?;
                    conn.execute(
                        "INSERT OR REPLACE INTO edges \
                         (src_entity_id, rel, dst_entity_id, weight, last_seen_ts) \
                         VALUES (?1, ?2, ?3, 1.0, ?4)",
                        rusqlite::params![src_id, rel.rel, dst_id, now],
                    )?;
                }

                // (h) Evict if over cap — runs before commit, includes new row.
                let evicted = evict_if_over_cap(&conn, &req_owned.bot_id, cap_per_bot, now, tau_seconds)?;

                // (i) Commit.
                conn.execute_batch("COMMIT")?;

                Ok((memory_id, evicted, salience))
            })
            .await
            .map_err(|join_err| {
                crate::error::AppError::Internal(anyhow::anyhow!("spawn_blocking panicked: {join_err}"))
            })??;

        // Step 3: publish (fire-and-forget; never propagates error).
        let row = PubSubRow {
            memory_id: memory_id.clone(),
            bot_id: req.bot_id,
            text: req.text,
            created_ts: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
            salience: salience_clamped,
        };
        self.pubsub.publish(&row);

        Ok(WriteResp { memory_id, evicted })
    }

    // -----------------------------------------------------------------------
    // read
    // -----------------------------------------------------------------------

    /// Read a single memory row by `memory_id` scoped to `bot_id`.
    ///
    /// Returns `None` if no row matches — the REST layer maps this to 404.
    /// The embedding blob is never returned (matches Python `GET /memory/{id}`).
    pub async fn read(
        &self,
        bot_id: &str,
        memory_id: &str,
    ) -> Result<Option<MemoryRow>, crate::error::AppError> {
        let db_path = self.db_path.clone();
        let bot_id = bot_id.to_owned();
        let memory_id = memory_id.to_owned();

        tokio::task::spawn_blocking(move || -> Result<Option<MemoryRow>, crate::error::AppError> {
            let conn = db::open_db(&db_path)?;
            let mut stmt = conn.prepare(
                "SELECT id, bot_id, text, salience, memory_type, source, \
                        created_ts, last_recalled_ts \
                 FROM memories WHERE id=?1 AND bot_id=?2",
            )?;
            let row = stmt.query_row(rusqlite::params![memory_id, bot_id], |r| {
                Ok(MemoryRow {
                    id:               r.get(0)?,
                    bot_id:           r.get(1)?,
                    text:             r.get(2)?,
                    salience:         r.get::<_, f64>(3)? as f32,
                    memory_type:      r.get(4)?,
                    source:           r.get(5)?,
                    created_ts:       r.get(6)?,
                    last_recalled_ts: r.get(7)?,
                })
            });
            match row {
                Ok(m) => Ok(Some(m)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(crate::error::AppError::Db(e)),
            }
        })
        .await
        .map_err(|e| crate::error::AppError::Internal(anyhow::anyhow!("spawn_blocking panicked: {e}")))?
    }

    // -----------------------------------------------------------------------
    // update
    // -----------------------------------------------------------------------

    /// Patch a memory row.
    ///
    /// Semantics (mirrors Python `PUT /memory/update`):
    /// - If `text` is `Some`, re-embed it (async, before `spawn_blocking`),
    ///   update `memories.text`, `memories.embedding`, and replace the
    ///   `vec_memories` row.  Sets `re_embedded = true`.
    /// - `salience` is clamped to `[0.0, 1.0]`.
    /// - `memory_type` and `source` are patched when `Some`.
    /// - `last_recalled_ts` is always set to `now` on a successful update.
    /// - Returns `UpdateResp { updated: false, re_embedded: false }` when the
    ///   row does not exist (no 404 here — REST layer decides status).
    pub async fn update(
        &self,
        r: UpdateReq,
    ) -> Result<UpdateResp, crate::error::AppError> {
        // Embed first, before entering spawn_blocking, if text is changing.
        // Degrade gracefully (empty embedding) on embedder error — same pattern
        // as write().
        let (new_embedding, re_embedded) = if let Some(ref text) = r.text {
            match self.embed.embed(text).await {
                Ok(arc) => (arc.as_ref().clone(), true),
                Err(_) => (vec![], true),   // still re_embedded=true; text changed even if embed failed
            }
        } else {
            (vec![], false)
        };

        let db_path = self.db_path.clone();
        let req = r;

        tokio::task::spawn_blocking(move || -> Result<UpdateResp, crate::error::AppError> {
            let conn = db::open_db(&db_path)?;

            // Check existence first (mirrors Python 404 guard).
            let exists: bool = conn
                .query_row(
                    "SELECT 1 FROM memories WHERE id=?1 AND bot_id=?2",
                    rusqlite::params![req.memory_id, req.bot_id],
                    |_| Ok(true),
                )
                .unwrap_or(false);

            if !exists {
                return Ok(UpdateResp { updated: false, re_embedded: false });
            }

            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);

            conn.execute_batch("BEGIN")?;

            // Build SET clause dynamically.
            // We always update last_recalled_ts at the end.
            let mut sql_sets: Vec<String> = Vec::new();
            // We'll bind values as a Vec<rusqlite::types::Value>.
            use rusqlite::types::Value as SqlValue;
            let mut bind_vals: Vec<SqlValue> = Vec::new();

            if let Some(ref text) = req.text {
                sql_sets.push("text=?".to_owned());
                bind_vals.push(SqlValue::Text(text.clone()));

                let blob = if new_embedding.is_empty() {
                    SqlValue::Null
                } else {
                    SqlValue::Blob(db::pack_f32_le(&new_embedding))
                };
                sql_sets.push("embedding=?".to_owned());
                bind_vals.push(blob);

                // Replace vec_memories row.
                conn.execute(
                    "DELETE FROM vec_memories WHERE memory_id=?1",
                    rusqlite::params![req.memory_id],
                )?;
                if !new_embedding.is_empty() {
                    conn.execute(
                        "INSERT INTO vec_memories (memory_id, bot_id, embedding) VALUES (?1, ?2, ?3)",
                        rusqlite::params![req.memory_id, req.bot_id, db::pack_f32_le(&new_embedding)],
                    )?;
                }
            }

            if let Some(sal) = req.salience {
                let clamped = sal.clamp(0.0, 1.0) as f64;
                sql_sets.push("salience=?".to_owned());
                bind_vals.push(SqlValue::Real(clamped));
            }

            if let Some(ref mt) = req.memory_type {
                sql_sets.push("memory_type=?".to_owned());
                bind_vals.push(SqlValue::Text(mt.clone()));
            }

            if let Some(ref src) = req.source {
                sql_sets.push("source=?".to_owned());
                bind_vals.push(SqlValue::Text(src.clone()));
            }

            // Always update last_recalled_ts.
            sql_sets.push("last_recalled_ts=?".to_owned());
            bind_vals.push(SqlValue::Integer(now));

            // Trailing WHERE param.
            bind_vals.push(SqlValue::Text(req.memory_id.clone()));

            let sql = format!(
                "UPDATE memories SET {} WHERE id=?",
                sql_sets.join(", ")
            );
            // Renumber placeholders — rusqlite uses ?1, ?2, … but we built with bare `?`.
            // rusqlite accepts bare `?` as positional (same as SQLite), so this is fine.
            conn.execute(&sql, rusqlite::params_from_iter(bind_vals.iter()))?;

            conn.execute_batch("COMMIT")?;

            Ok(UpdateResp { updated: true, re_embedded })
        })
        .await
        .map_err(|e| crate::error::AppError::Internal(anyhow::anyhow!("spawn_blocking panicked: {e}")))?
    }

    // -----------------------------------------------------------------------
    // forget
    // -----------------------------------------------------------------------

    /// Delete a memory row from `memories`, `vec_memories`, and `memory_entities`.
    ///
    /// Returns `ForgetResp { forgotten: true }` when a row was deleted,
    /// `forgotten: false` if no matching row existed.
    ///
    /// Mirrors Python `POST /memory/forget`:
    /// ```python
    /// cur = conn.execute("DELETE FROM memories WHERE id=? AND bot_id=?", ...)
    /// deleted = cur.rowcount > 0
    /// if deleted:
    ///     conn.execute("DELETE FROM vec_memories WHERE memory_id=?", ...)
    ///     conn.execute("DELETE FROM memory_entities WHERE memory_id=?", ...)
    ///     conn.commit()
    /// ```
    pub async fn forget(
        &self,
        r: ForgetReq,
    ) -> Result<ForgetResp, crate::error::AppError> {
        let db_path = self.db_path.clone();

        tokio::task::spawn_blocking(move || -> Result<ForgetResp, crate::error::AppError> {
            let conn = db::open_db(&db_path)?;

            conn.execute_batch("BEGIN")?;

            let deleted = conn.execute(
                "DELETE FROM memories WHERE id=?1 AND bot_id=?2",
                rusqlite::params![r.memory_id, r.bot_id],
            )? > 0;

            if deleted {
                conn.execute(
                    "DELETE FROM vec_memories WHERE memory_id=?1",
                    rusqlite::params![r.memory_id],
                )?;
                conn.execute(
                    "DELETE FROM memory_entities WHERE memory_id=?1",
                    rusqlite::params![r.memory_id],
                )?;
                conn.execute_batch("COMMIT")?;
            } else {
                conn.execute_batch("ROLLBACK")?;
            }

            Ok(ForgetResp { forgotten: deleted })
        })
        .await
        .map_err(|e| crate::error::AppError::Internal(anyhow::anyhow!("spawn_blocking panicked: {e}")))?
    }

    // -----------------------------------------------------------------------
    // list
    // -----------------------------------------------------------------------

    /// List memories for a bot with optional filters.
    ///
    /// Mirrors Python `GET /memory/list`:
    /// - Filters: `memory_type`, `source`, `since_ts` (created_ts >=), `until_ts` (created_ts <=).
    /// - `ORDER BY created_ts DESC LIMIT ? OFFSET ?`.
    /// - `total` = COUNT(*) of the same WHERE without limit/offset.
    /// - `limit` clamped to `[1, 500]`; `offset` clamped to `≥ 0`.
    pub async fn list(
        &self,
        r: ListReq,
    ) -> Result<ListResp, crate::error::AppError> {
        let db_path = self.db_path.clone();

        tokio::task::spawn_blocking(move || -> Result<ListResp, crate::error::AppError> {
            let conn = db::open_db(&db_path)?;

            let limit = r.limit.clamp(1, 500);
            let offset = r.offset.max(0);

            // Build WHERE clause dynamically.
            use rusqlite::types::Value as SqlValue;
            let mut where_parts: Vec<&str> = vec!["bot_id=?"];
            let mut params: Vec<SqlValue> = vec![SqlValue::Text(r.bot_id.clone())];

            if let Some(ref mt) = r.memory_type {
                where_parts.push("memory_type=?");
                params.push(SqlValue::Text(mt.clone()));
            }
            if let Some(ref src) = r.source {
                where_parts.push("source=?");
                params.push(SqlValue::Text(src.clone()));
            }
            if let Some(since) = r.since_ts {
                where_parts.push("created_ts>=?");
                params.push(SqlValue::Integer(since));
            }
            if let Some(until) = r.until_ts {
                where_parts.push("created_ts<=?");
                params.push(SqlValue::Integer(until));
            }

            let where_sql = where_parts.join(" AND ");

            // COUNT(*) for total (no LIMIT/OFFSET).
            let count_sql = format!("SELECT COUNT(*) FROM memories WHERE {where_sql}");
            let total: i64 = conn.query_row(
                &count_sql,
                rusqlite::params_from_iter(params.iter()),
                |r| r.get(0),
            )?;

            // Main SELECT with LIMIT/OFFSET.
            let mut list_params = params.clone();
            list_params.push(SqlValue::Integer(limit));
            list_params.push(SqlValue::Integer(offset));

            let list_sql = format!(
                "SELECT id, bot_id, text, salience, memory_type, source, \
                        created_ts, last_recalled_ts \
                 FROM memories WHERE {where_sql} \
                 ORDER BY created_ts DESC LIMIT ? OFFSET ?"
            );
            let mut stmt = conn.prepare(&list_sql)?;
            let items: Vec<MemoryRow> = stmt
                .query_map(rusqlite::params_from_iter(list_params.iter()), |row| {
                    Ok(MemoryRow {
                        id:               row.get(0)?,
                        bot_id:           row.get(1)?,
                        text:             row.get(2)?,
                        salience:         row.get::<_, f64>(3)? as f32,
                        memory_type:      row.get(4)?,
                        source:           row.get(5)?,
                        created_ts:       row.get(6)?,
                        last_recalled_ts: row.get(7)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;

            Ok(ListResp { items, total })
        })
        .await
        .map_err(|e| crate::error::AppError::Internal(anyhow::anyhow!("spawn_blocking panicked: {e}")))?
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ScoringWeights;
    use crate::embed_cache::EmbedCache;
    use crate::embeddings::EmbeddingsClient;
    use crate::{db, EMBEDDING_DIM};
    use axum::{extract::Json as AxumJson, http::StatusCode, routing::post, Router};
    use std::sync::Arc;

    // ---- Mock embed server ----

    /// Spin up a stub embed server that returns a constant 384-dim vector.
    /// Returns the base URL (e.g. `http://127.0.0.1:<port>/v1`).
    async fn spawn_embed_stub(vec: Vec<f32>) -> String {
        let handler = move |AxumJson(_body): AxumJson<serde_json::Value>| {
            let v = vec.clone();
            async move {
                let arr: Vec<serde_json::Value> =
                    v.iter().map(|&x| serde_json::json!(x)).collect();
                (
                    StatusCode::OK,
                    AxumJson(serde_json::json!({
                        "object": "list",
                        "data": [{"object": "embedding", "embedding": arr, "index": 0}],
                        "model": "test",
                    })),
                )
            }
        };
        let app = Router::new().route("/v1/embeddings", post(handler));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}/v1")
    }

    fn make_service(db_path: PathBuf, embed_url: &str) -> MemoryService {
        let client = EmbeddingsClient::new(embed_url, "embedding", "");
        let cache = Arc::new(EmbedCache::new(client));
        MemoryService::new(
            db_path,
            ScoringWeights { w_rel: 0.5, w_rec: 0.2, w_imp: 0.3, tau_seconds: 604800 },
            2000,
            cache,
            Arc::new(PubSub::new()),
        )
    }

    fn open_and_migrate(path: &std::path::Path) -> rusqlite::Connection {
        db::register_vec0();
        let conn = db::open_db(path).expect("open_db");
        let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/migrations"));
        db::migrate::run(&conn, dir).expect("migrate::run");
        conn
    }

    // W1: salience > 1.0 is clamped to 1.0.
    #[tokio::test]
    async fn write_salience_clamped_to_one() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        open_and_migrate(tmp.path());

        let embed_vec: Vec<f32> = vec![0.1_f32; EMBEDDING_DIM];
        let embed_url = spawn_embed_stub(embed_vec).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let req = WriteReq {
            bot_id: "bot1".to_string(),
            text: "hello world".to_string(),
            salience: 1.5,
            entities: vec![],
            relations: vec![],
            memory_type: None,
            source: None,
        };
        let resp = svc.write(req).await.expect("write must succeed");

        let conn = db::open_db(tmp.path()).unwrap();
        let sal: f64 = conn
            .query_row(
                "SELECT salience FROM memories WHERE id=?1",
                rusqlite::params![resp.memory_id],
                |r| r.get(0),
            )
            .expect("SELECT salience");
        assert!(
            (sal - 1.0).abs() < 1e-6,
            "salience > 1.0 must be clamped to 1.0, got {sal}"
        );
    }

    // W2: salience < 0.0 is clamped to 0.0.
    #[tokio::test]
    async fn write_salience_clamped_to_zero() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        open_and_migrate(tmp.path());

        let embed_vec: Vec<f32> = vec![0.1_f32; EMBEDDING_DIM];
        let embed_url = spawn_embed_stub(embed_vec).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let req = WriteReq {
            bot_id: "bot1".to_string(),
            text: "something".to_string(),
            salience: -0.5,
            entities: vec![],
            relations: vec![],
            memory_type: None,
            source: None,
        };
        let resp = svc.write(req).await.expect("write must succeed");

        let conn = db::open_db(tmp.path()).unwrap();
        let sal: f64 = conn
            .query_row(
                "SELECT salience FROM memories WHERE id=?1",
                rusqlite::params![resp.memory_id],
                |r| r.get(0),
            )
            .expect("SELECT salience");
        assert!(
            sal.abs() < 1e-6,
            "salience < 0.0 must be clamped to 0.0, got {sal}"
        );
    }

    // W3: vec_memories row is present with the embedding.
    #[tokio::test]
    async fn write_inserts_vec_memories() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        open_and_migrate(tmp.path());

        let embed_vec: Vec<f32> = (0..EMBEDDING_DIM).map(|i| i as f32 * 0.001).collect();
        let embed_url = spawn_embed_stub(embed_vec.clone()).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let req = WriteReq {
            bot_id: "bot1".to_string(),
            text: "test text".to_string(),
            salience: 0.7,
            entities: vec![],
            relations: vec![],
            memory_type: None,
            source: None,
        };
        let resp = svc.write(req).await.expect("write");

        let conn = db::open_db(tmp.path()).unwrap();
        let blob: Vec<u8> = conn
            .query_row(
                "SELECT embedding FROM vec_memories WHERE memory_id=?1",
                rusqlite::params![resp.memory_id],
                |r| r.get(0),
            )
            .expect("SELECT embedding from vec_memories");

        let decoded = db::unpack_f32_le(&blob);
        assert_eq!(decoded.len(), EMBEDDING_DIM, "embedding must be {EMBEDDING_DIM} floats");
    }

    // W4: entities are upserted and linked to the memory.
    #[tokio::test]
    async fn write_upserts_entities_and_memory_entities() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        open_and_migrate(tmp.path());

        let embed_vec: Vec<f32> = vec![0.1_f32; EMBEDDING_DIM];
        let embed_url = spawn_embed_stub(embed_vec).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let req = WriteReq {
            bot_id: "bot1".to_string(),
            text: "Alice met Bob".to_string(),
            salience: 0.8,
            entities: vec!["Alice".to_string(), "Bob".to_string()],
            relations: vec![],
            memory_type: None,
            source: None,
        };
        let resp = svc.write(req).await.expect("write");

        let conn = db::open_db(tmp.path()).unwrap();

        // Both entities must exist.
        for name in ["alice", "bob"] {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM entities WHERE bot_id='bot1' AND name_lower=?1",
                    rusqlite::params![name],
                    |r| r.get(0),
                )
                .expect("count entities");
            assert_eq!(count, 1, "entity '{name}' must be upserted");
        }

        // Both must be linked via memory_entities.
        let me_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memory_entities WHERE memory_id=?1",
                rusqlite::params![resp.memory_id],
                |r| r.get(0),
            )
            .expect("count memory_entities");
        assert_eq!(me_count, 2, "must have 2 memory_entity links");
    }

    // W5: relation triples produce edges rows.
    #[tokio::test]
    async fn write_inserts_edges_for_relations() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        open_and_migrate(tmp.path());

        let embed_vec: Vec<f32> = vec![0.1_f32; EMBEDDING_DIM];
        let embed_url = spawn_embed_stub(embed_vec).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let req = WriteReq {
            bot_id: "bot1".to_string(),
            text: "Alice killed Arthas".to_string(),
            salience: 0.9,
            entities: vec![],
            relations: vec![RelationTriple {
                src: "Alice".to_string(),
                rel: "killed".to_string(),
                dst: "Arthas".to_string(),
            }],
            memory_type: None,
            source: None,
        };
        svc.write(req).await.expect("write");

        let conn = db::open_db(tmp.path()).unwrap();

        // src and dst entities must exist.
        for name in ["alice", "arthas"] {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM entities WHERE bot_id='bot1' AND name_lower=?1",
                    rusqlite::params![name],
                    |r| r.get(0),
                )
                .expect("count entity");
            assert_eq!(count, 1, "entity '{name}' must exist after write with relation");
        }

        // Edge must exist.
        let edge_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))
            .expect("count edges");
        assert_eq!(edge_count, 1, "must have 1 edge row");

        // Edge rel must be 'killed'.
        let rel: String = conn
            .query_row("SELECT rel FROM edges LIMIT 1", [], |r| r.get(0))
            .expect("SELECT rel");
        assert_eq!(rel, "killed");
    }

    // W6: memory_type defaults to "event" when None.
    #[tokio::test]
    async fn write_default_memory_type_is_event() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        open_and_migrate(tmp.path());

        let embed_vec: Vec<f32> = vec![0.1_f32; EMBEDDING_DIM];
        let embed_url = spawn_embed_stub(embed_vec).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let req = WriteReq {
            bot_id: "bot1".to_string(),
            text: "an event".to_string(),
            salience: 0.5,
            entities: vec![],
            relations: vec![],
            memory_type: None,
            source: None,
        };
        let resp = svc.write(req).await.expect("write");

        let conn = db::open_db(tmp.path()).unwrap();
        let mt: String = conn
            .query_row(
                "SELECT memory_type FROM memories WHERE id=?1",
                rusqlite::params![resp.memory_id],
                |r| r.get(0),
            )
            .expect("SELECT memory_type");
        assert_eq!(mt, "event");
    }

    // W7: explicit memory_type is stored.
    #[tokio::test]
    async fn write_explicit_memory_type_stored() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        open_and_migrate(tmp.path());

        let embed_vec: Vec<f32> = vec![0.1_f32; EMBEDDING_DIM];
        let embed_url = spawn_embed_stub(embed_vec).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let req = WriteReq {
            bot_id: "bot1".to_string(),
            text: "goal completed".to_string(),
            salience: 0.6,
            entities: vec![],
            relations: vec![],
            memory_type: Some("goal_link".to_string()),
            source: Some("goals:g_abc12345678".to_string()),
        };
        let resp = svc.write(req).await.expect("write");

        let conn = db::open_db(tmp.path()).unwrap();
        let (mt, src): (String, Option<String>) = conn
            .query_row(
                "SELECT memory_type, source FROM memories WHERE id=?1",
                rusqlite::params![resp.memory_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("SELECT memory_type, source");
        assert_eq!(mt, "goal_link");
        assert_eq!(src.as_deref(), Some("goals:g_abc12345678"));
    }

    // W8: returned memory_id matches what's stored in DB.
    #[tokio::test]
    async fn write_returned_memory_id_matches_db() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        open_and_migrate(tmp.path());

        let embed_vec: Vec<f32> = vec![0.1_f32; EMBEDDING_DIM];
        let embed_url = spawn_embed_stub(embed_vec).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let req = WriteReq {
            bot_id: "bot1".to_string(),
            text: "text".to_string(),
            salience: 0.5,
            entities: vec![],
            relations: vec![],
            memory_type: None,
            source: None,
        };
        let resp = svc.write(req).await.expect("write");

        let conn = db::open_db(tmp.path()).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE id=?1",
                rusqlite::params![resp.memory_id],
                |r| r.get(0),
            )
            .expect("count by id");
        assert_eq!(count, 1, "the returned memory_id must exist in the DB");
    }

    // W9: eviction fires when over cap.
    #[tokio::test]
    async fn write_evicts_when_over_cap() {
        let tmp = tempfile::NamedTempFile::new().unwrap();

        // Pre-populate with cap rows at low salience, then write one more.
        {
            let conn = open_and_migrate(tmp.path());
            conn.execute(
                "INSERT OR IGNORE INTO bots (bot_id, created_ts) VALUES ('botE', 0)",
                [],
            )
            .unwrap();
            let blob = db::pack_f32_le(&vec![0.0_f32; EMBEDDING_DIM]);
            for i in 0..3usize {
                conn.execute(
                    "INSERT INTO memories \
                     (id, bot_id, text, salience, created_ts, last_recalled_ts, embedding, memory_type) \
                     VALUES (?1, 'botE', 'old', 0.01, ?2, ?2, ?3, 'event')",
                    rusqlite::params![format!("m_old{i:07}"), i as i64, blob],
                )
                .unwrap();
                conn.execute(
                    "INSERT INTO vec_memories (memory_id, bot_id, embedding) VALUES (?1, 'botE', ?2)",
                    rusqlite::params![format!("m_old{i:07}"), blob],
                )
                .unwrap();
            }
            conn.execute_batch("COMMIT").ok();
        }

        // cap=3 — the write will push us to 4, so 1 must be evicted.
        let embed_vec: Vec<f32> = vec![0.9_f32; EMBEDDING_DIM];
        let embed_url = spawn_embed_stub(embed_vec).await;

        let client = EmbeddingsClient::new(&embed_url, "embedding", "");
        let cache = Arc::new(EmbedCache::new(client));
        let svc = MemoryService::new(
            tmp.path().to_path_buf(),
            ScoringWeights { w_rel: 0.5, w_rec: 0.2, w_imp: 0.3, tau_seconds: 604800 },
            3, // cap = 3
            cache,
            Arc::new(PubSub::new()),
        );

        let req = WriteReq {
            bot_id: "botE".to_string(),
            text: "new high-salience memory".to_string(),
            salience: 0.99,
            entities: vec![],
            relations: vec![],
            memory_type: None,
            source: None,
        };
        let resp = svc.write(req).await.expect("write");
        assert_eq!(resp.evicted, 1, "writing to a bot at cap must evict 1");

        let conn = db::open_db(tmp.path()).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM memories WHERE bot_id='botE'", [], |r| r.get(0))
            .expect("count");
        assert_eq!(count, 3, "after write+evict, bot must have exactly cap=3 rows");
    }

    // -----------------------------------------------------------------------
    // Helpers shared by read/update/forget/list tests
    // -----------------------------------------------------------------------

    /// Write a memory row directly into the DB (bypasses the service write path).
    /// Returns the generated `memory_id`.
    fn insert_raw_memory(
        conn: &rusqlite::Connection,
        bot_id: &str,
        memory_id: &str,
        text: &str,
        salience: f64,
        memory_type: &str,
        source: Option<&str>,
        created_ts: i64,
        embedding: &[f32],
    ) {
        conn.execute(
            "INSERT OR IGNORE INTO bots (bot_id, created_ts) VALUES (?1, ?2)",
            rusqlite::params![bot_id, created_ts],
        )
        .unwrap();
        let blob = db::pack_f32_le(embedding);
        conn.execute(
            "INSERT INTO memories \
             (id, bot_id, text, salience, created_ts, last_recalled_ts, embedding, memory_type, source) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                memory_id, bot_id, text, salience, created_ts,
                blob, memory_type, source,
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO vec_memories (memory_id, bot_id, embedding) VALUES (?1, ?2, ?3)",
            rusqlite::params![memory_id, bot_id, blob],
        )
        .unwrap();
    }

    // -----------------------------------------------------------------------
    // R1: read existing row returns it (no embedding field).
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn read_existing_row_returns_memory_row() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());
        let emb: Vec<f32> = vec![0.1_f32; EMBEDDING_DIM];
        insert_raw_memory(
            &conn, "bot1", "m_read00001", "read test", 0.75,
            "event", Some("src:x"), 1_700_000_000, &emb,
        );

        let embed_url = spawn_embed_stub(emb).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let row = svc.read("bot1", "m_read00001").await.expect("read").expect("must be Some");
        assert_eq!(row.id, "m_read00001");
        assert_eq!(row.bot_id, "bot1");
        assert_eq!(row.text, "read test");
        assert!((row.salience - 0.75_f32).abs() < 1e-5, "salience mismatch: {}", row.salience);
        assert_eq!(row.memory_type, "event");
        assert_eq!(row.source.as_deref(), Some("src:x"));
        assert_eq!(row.created_ts, 1_700_000_000);
    }

    // R2: read with unknown id returns None.
    #[tokio::test]
    async fn read_unknown_id_returns_none() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        open_and_migrate(tmp.path());

        let embed_url = spawn_embed_stub(vec![0.1_f32; EMBEDDING_DIM]).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let result = svc.read("bot1", "no_such_id_xyz").await.expect("read must not error");
        assert!(result.is_none(), "non-existent id must return None");
    }

    // R3: read is scoped to bot_id — same memory_id, different bot → None.
    #[tokio::test]
    async fn read_wrong_bot_returns_none() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());
        let emb: Vec<f32> = vec![0.2_f32; EMBEDDING_DIM];
        insert_raw_memory(
            &conn, "botA", "m_scope0001", "scoped text", 0.5,
            "event", None, 1_700_000_001, &emb,
        );

        let embed_url = spawn_embed_stub(emb).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        // botB must not see botA's memory.
        let result = svc.read("botB", "m_scope0001").await.expect("read");
        assert!(result.is_none(), "different bot_id must not see the row");
    }

    // -----------------------------------------------------------------------
    // U1: update text re-embeds + updates vec_memories.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn update_text_change_re_embeds_and_updates_vec_memories() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());
        let orig_emb: Vec<f32> = vec![0.1_f32; EMBEDDING_DIM];
        insert_raw_memory(
            &conn, "bot1", "m_upd00001", "original text", 0.5,
            "event", None, 1_700_000_000, &orig_emb,
        );

        // New embedding is different from original — all 0.9.
        let new_emb: Vec<f32> = vec![0.9_f32; EMBEDDING_DIM];
        let embed_url = spawn_embed_stub(new_emb.clone()).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let resp = svc
            .update(UpdateReq {
                bot_id: "bot1".to_string(),
                memory_id: "m_upd00001".to_string(),
                text: Some("updated text".to_string()),
                salience: None,
                memory_type: None,
                source: None,
            })
            .await
            .expect("update must succeed");

        assert!(resp.updated, "updated must be true");
        assert!(resp.re_embedded, "re_embedded must be true when text changes");

        // Verify memories.text updated.
        let conn2 = db::open_db(tmp.path()).unwrap();
        let stored_text: String = conn2
            .query_row(
                "SELECT text FROM memories WHERE id='m_upd00001'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored_text, "updated text");

        // Verify vec_memories embedding updated.
        let vec_blob: Vec<u8> = conn2
            .query_row(
                "SELECT embedding FROM vec_memories WHERE memory_id='m_upd00001'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let decoded = db::unpack_f32_le(&vec_blob);
        assert_eq!(decoded.len(), EMBEDDING_DIM);
        // The stub always returns new_emb (0.9s).
        assert!((decoded[0] - 0.9_f32).abs() < 1e-5, "vec_memories must hold new embedding");
    }

    // U2: salience-only patch does not re-embed; salience is clamped.
    #[tokio::test]
    async fn update_salience_only_no_re_embed_clamped() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());
        let emb: Vec<f32> = vec![0.5_f32; EMBEDDING_DIM];
        insert_raw_memory(
            &conn, "bot1", "m_sal00001", "sal test", 0.3,
            "event", None, 1_700_000_000, &emb,
        );

        let embed_url = spawn_embed_stub(emb).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let resp = svc
            .update(UpdateReq {
                bot_id: "bot1".to_string(),
                memory_id: "m_sal00001".to_string(),
                text: None,
                salience: Some(1.8),  // over 1 → must clamp to 1.0
                memory_type: None,
                source: None,
            })
            .await
            .expect("update");

        assert!(resp.updated);
        assert!(!resp.re_embedded, "salience-only update must not re-embed");

        let conn2 = db::open_db(tmp.path()).unwrap();
        let sal: f64 = conn2
            .query_row("SELECT salience FROM memories WHERE id='m_sal00001'", [], |r| r.get(0))
            .unwrap();
        assert!((sal - 1.0).abs() < 1e-6, "salience must be clamped to 1.0, got {sal}");
    }

    // U3: unknown memory_id returns updated=false.
    #[tokio::test]
    async fn update_unknown_id_returns_not_updated() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        open_and_migrate(tmp.path());

        let embed_url = spawn_embed_stub(vec![0.1_f32; EMBEDDING_DIM]).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let resp = svc
            .update(UpdateReq {
                bot_id: "bot1".to_string(),
                memory_id: "no_such_id_zzz".to_string(),
                text: Some("irrelevant".to_string()),
                salience: None,
                memory_type: None,
                source: None,
            })
            .await
            .expect("update must not error");

        assert!(!resp.updated, "non-existent id must return updated=false");
        assert!(!resp.re_embedded, "non-existent id must return re_embedded=false");
    }

    // -----------------------------------------------------------------------
    // F1: forget deletes from memories + vec_memories + memory_entities; returns true.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn forget_deletes_all_tables_returns_true() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());
        let emb: Vec<f32> = vec![0.3_f32; EMBEDDING_DIM];
        insert_raw_memory(
            &conn, "bot1", "m_fgt00001", "forget me", 0.5,
            "event", None, 1_700_000_000, &emb,
        );
        // Add a memory_entities link via the upsert helper (uses the correct schema).
        let entity_id =
            db::entities::upsert_entity(&conn, "bot1", "SomeName", None).unwrap();
        conn.execute(
            "INSERT INTO memory_entities (memory_id, entity_id) VALUES ('m_fgt00001', ?1)",
            rusqlite::params![entity_id],
        )
        .unwrap();

        let embed_url = spawn_embed_stub(emb).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let resp = svc
            .forget(ForgetReq { bot_id: "bot1".to_string(), memory_id: "m_fgt00001".to_string() })
            .await
            .expect("forget");
        assert!(resp.forgotten, "forgotten must be true");

        let conn2 = db::open_db(tmp.path()).unwrap();

        let mem_count: i64 = conn2
            .query_row("SELECT COUNT(*) FROM memories WHERE id='m_fgt00001'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mem_count, 0, "memories row must be deleted");

        let vec_count: i64 = conn2
            .query_row("SELECT COUNT(*) FROM vec_memories WHERE memory_id='m_fgt00001'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(vec_count, 0, "vec_memories row must be deleted");

        let me_count: i64 = conn2
            .query_row("SELECT COUNT(*) FROM memory_entities WHERE memory_id='m_fgt00001'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(me_count, 0, "memory_entities rows must be deleted");
    }

    // F2: second forget on same id returns forgotten=false.
    #[tokio::test]
    async fn forget_second_call_returns_false() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());
        let emb: Vec<f32> = vec![0.3_f32; EMBEDDING_DIM];
        insert_raw_memory(
            &conn, "bot1", "m_fgt00002", "forget twice", 0.5,
            "event", None, 1_700_000_000, &emb,
        );

        let embed_url = spawn_embed_stub(emb).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let r1 = svc
            .forget(ForgetReq { bot_id: "bot1".to_string(), memory_id: "m_fgt00002".to_string() })
            .await
            .expect("first forget");
        assert!(r1.forgotten);

        let r2 = svc
            .forget(ForgetReq { bot_id: "bot1".to_string(), memory_id: "m_fgt00002".to_string() })
            .await
            .expect("second forget must not error");
        assert!(!r2.forgotten, "second forget must return forgotten=false");
    }

    // -----------------------------------------------------------------------
    // L1: list returns all rows for bot when no filters.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn list_all_rows_for_bot() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());
        let emb: Vec<f32> = vec![0.1_f32; EMBEDDING_DIM];
        for i in 0..3u64 {
            insert_raw_memory(
                &conn,
                "bot1",
                &format!("m_lst{i:07}"),
                &format!("text {i}"),
                0.5,
                "event",
                None,
                1_700_000_000 + i as i64,
                &emb,
            );
        }

        let embed_url = spawn_embed_stub(emb).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let resp = svc
            .list(ListReq {
                bot_id: "bot1".to_string(),
                memory_type: None,
                source: None,
                since_ts: None,
                until_ts: None,
                limit: 50,
                offset: 0,
            })
            .await
            .expect("list");

        assert_eq!(resp.total, 3, "total must count all 3 rows");
        assert_eq!(resp.items.len(), 3, "items must contain all 3 rows");
    }

    // L2: filter by memory_type.
    #[tokio::test]
    async fn list_filter_by_memory_type() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());
        let emb: Vec<f32> = vec![0.1_f32; EMBEDDING_DIM];
        insert_raw_memory(&conn, "bot1", "m_lstA00001", "event row", 0.5, "event", None, 1_700_000_001, &emb);
        insert_raw_memory(&conn, "bot1", "m_lstB00001", "goal row",  0.5, "goal",  None, 1_700_000_002, &emb);
        insert_raw_memory(&conn, "bot1", "m_lstB00002", "goal row2", 0.5, "goal",  None, 1_700_000_003, &emb);

        let embed_url = spawn_embed_stub(emb).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let resp = svc
            .list(ListReq {
                bot_id: "bot1".to_string(),
                memory_type: Some("goal".to_string()),
                source: None,
                since_ts: None,
                until_ts: None,
                limit: 50,
                offset: 0,
            })
            .await
            .expect("list by type");

        assert_eq!(resp.total, 2, "total must be 2 for memory_type=goal");
        assert_eq!(resp.items.len(), 2);
        assert!(resp.items.iter().all(|r| r.memory_type == "goal"));
    }

    // L3: filter by source.
    #[tokio::test]
    async fn list_filter_by_source() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());
        let emb: Vec<f32> = vec![0.1_f32; EMBEDDING_DIM];
        insert_raw_memory(&conn, "bot1", "m_lstS00001", "src a", 0.5, "event", Some("src:a"), 1_700_000_001, &emb);
        insert_raw_memory(&conn, "bot1", "m_lstS00002", "src b", 0.5, "event", Some("src:b"), 1_700_000_002, &emb);
        insert_raw_memory(&conn, "bot1", "m_lstS00003", "src a2", 0.5, "event", Some("src:a"), 1_700_000_003, &emb);

        let embed_url = spawn_embed_stub(emb).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let resp = svc
            .list(ListReq {
                bot_id: "bot1".to_string(),
                memory_type: None,
                source: Some("src:a".to_string()),
                since_ts: None,
                until_ts: None,
                limit: 50,
                offset: 0,
            })
            .await
            .expect("list by source");

        assert_eq!(resp.total, 2);
        assert!(resp.items.iter().all(|r| r.source.as_deref() == Some("src:a")));
    }

    // L4: filter by since_ts / until_ts.
    #[tokio::test]
    async fn list_filter_by_since_until_ts() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());
        let emb: Vec<f32> = vec![0.1_f32; EMBEDDING_DIM];
        insert_raw_memory(&conn, "bot1", "m_lstT00001", "t1", 0.5, "event", None, 1_700_000_001, &emb);
        insert_raw_memory(&conn, "bot1", "m_lstT00002", "t2", 0.5, "event", None, 1_700_000_002, &emb);
        insert_raw_memory(&conn, "bot1", "m_lstT00003", "t3", 0.5, "event", None, 1_700_000_003, &emb);

        let embed_url = spawn_embed_stub(emb).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let resp = svc
            .list(ListReq {
                bot_id: "bot1".to_string(),
                memory_type: None,
                source: None,
                since_ts: Some(1_700_000_002),
                until_ts: Some(1_700_000_002),
                limit: 50,
                offset: 0,
            })
            .await
            .expect("list by ts range");

        assert_eq!(resp.total, 1, "only ts=1_700_000_002 should match");
        assert_eq!(resp.items[0].created_ts, 1_700_000_002);
    }

    // L5: limit + offset paginate; total reflects full filtered count.
    #[tokio::test]
    async fn list_limit_offset_pagination() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());
        let emb: Vec<f32> = vec![0.1_f32; EMBEDDING_DIM];
        for i in 0..5u64 {
            insert_raw_memory(
                &conn,
                "bot1",
                &format!("m_lstP{i:07}"),
                &format!("page {i}"),
                0.5,
                "event",
                None,
                1_700_000_000 + i as i64,
                &emb,
            );
        }

        let embed_url = spawn_embed_stub(emb).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let resp = svc
            .list(ListReq {
                bot_id: "bot1".to_string(),
                memory_type: None,
                source: None,
                since_ts: None,
                until_ts: None,
                limit: 2,
                offset: 1,
            })
            .await
            .expect("list paginated");

        assert_eq!(resp.total, 5, "total must be full count of 5");
        assert_eq!(resp.items.len(), 2, "items must respect limit=2");
    }

    // L6: order is DESC by created_ts.
    #[tokio::test]
    async fn list_order_is_created_ts_desc() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());
        let emb: Vec<f32> = vec![0.1_f32; EMBEDDING_DIM];
        insert_raw_memory(&conn, "bot1", "m_lstO00001", "old", 0.5, "event", None, 1_700_000_001, &emb);
        insert_raw_memory(&conn, "bot1", "m_lstO00002", "new", 0.5, "event", None, 1_700_000_002, &emb);

        let embed_url = spawn_embed_stub(emb).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let resp = svc
            .list(ListReq {
                bot_id: "bot1".to_string(),
                memory_type: None,
                source: None,
                since_ts: None,
                until_ts: None,
                limit: 50,
                offset: 0,
            })
            .await
            .expect("list order");

        assert_eq!(resp.items[0].created_ts, 1_700_000_002, "most recent first");
        assert_eq!(resp.items[1].created_ts, 1_700_000_001, "older second");
    }
}
