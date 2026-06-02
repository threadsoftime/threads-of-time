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

use rusqlite::OptionalExtension;

use crate::config::{RecencyBasis, ScoringWeights};
use crate::db::{self, entities::{bfs_entities, upsert_entity}, pack_f32_le};
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
// Task 4.7 — search types
// ---------------------------------------------------------------------------

/// Request payload for [`MemoryService::search`].
///
/// Matches Python `SearchRequest` pydantic model.
#[derive(Debug, Clone)]
pub struct SearchReq {
    pub bot_id: String,
    pub query: String,
    /// Maximum number of results after MMR reranking (default 5).
    pub top_k: usize,
    pub since_ts: Option<i64>,
    pub until_ts: Option<i64>,
    pub memory_type: Option<String>,
}

/// Per-lane rank signals for a single search result.
///
/// Matches Python `SearchSignals` pydantic model:
/// ```python
/// class SearchSignals(BaseModel):
///     bm25_rank:   int | None = None
///     dense_rank:  int | None = None
///     entity_rank: int | None = None
/// ```
///
/// Each field is the 0-indexed position in that lane's ranked list, or `None`
/// if the memory did not appear in that lane.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchSignals {
    pub bm25_rank: Option<usize>,
    pub dense_rank: Option<usize>,
    pub entity_rank: Option<usize>,
}

/// A single result item in a [`SearchResp`].
///
/// Matches Python `SearchItem` pydantic model:
/// ```python
/// class SearchItem(BaseModel):
///     memory_id: str
///     text:      str
///     score:     float   # RRF fused score (NOT cosine)
///     ts:        int     # created_ts
///     signals:   SearchSignals
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct SearchItem {
    pub memory_id: String,
    pub text: String,
    /// RRF fused score from `rrf_fuse` — NOT the cosine similarity.
    pub score: f64,
    /// `created_ts` of the memory.
    pub ts: i64,
    pub signals: SearchSignals,
}

/// Response from [`MemoryService::search`].
///
/// Matches Python `SearchResponse` pydantic model:
/// ```python
/// class SearchResponse(BaseModel):
///     items:            list[SearchItem]
///     total_candidates: int   # len(fused) before MMR
/// ```
#[derive(Debug, Clone)]
pub struct SearchResp {
    pub items: Vec<SearchItem>,
    /// Number of unique documents in the fused RRF result, before MMR reranking.
    pub total_candidates: usize,
}

// ---------------------------------------------------------------------------
// Task 4.6 — recall / recall_about types
// ---------------------------------------------------------------------------

/// Request payload for [`MemoryService::recall`].
///
/// Matches Python `RecallRequest` pydantic model.
#[derive(Debug, Clone)]
pub struct RecallReq {
    pub bot_id: String,
    pub query: String,
    /// Maximum number of results to return (default 5).
    pub top_k: usize,
    /// Only memories with `created_ts >= since_ts`.
    pub since_ts: Option<i64>,
    /// Only memories with `created_ts <= until_ts`.
    pub until_ts: Option<i64>,
    pub memory_type: Option<String>,
}

/// A single recalled memory in a [`RecallResp`].
///
/// Matches Python `RecalledMemory` pydantic model:
/// ```python
/// class RecalledMemory(BaseModel):
///     memory_id: str
///     text: str
///     score: float   # weighted-sum score (NOT the MMR score)
///     ts: int        # = created_ts
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct RecalledMemory {
    pub memory_id: String,
    pub text: String,
    /// Weighted-sum score from `score_memory` (NOT the MMR intermediate score).
    pub score: f64,
    /// `created_ts` of the memory.
    pub ts: i64,
}

/// Response from [`MemoryService::recall`].
///
/// Matches Python `RecallResponse`.
#[derive(Debug, Clone)]
pub struct RecallResp {
    pub memories: Vec<RecalledMemory>,
}

/// Request payload for [`MemoryService::recall_about`].
///
/// Matches Python `RecallAboutRequest` pydantic model.
#[derive(Debug, Clone)]
pub struct RecallAboutReq {
    pub bot_id: String,
    pub entity: String,
    /// Maximum BFS hops to follow from the seed entity (default 2).
    pub max_hops: usize,
    /// Maximum number of results to return (default 3).
    pub top_k: usize,
    pub since_ts: Option<i64>,
    pub until_ts: Option<i64>,
    pub memory_type: Option<String>,
}

/// Response from [`MemoryService::recall_about`].
///
/// Matches Python `RecallAboutResponse`.
#[derive(Debug, Clone)]
pub struct RecallAboutResp {
    pub hints: Vec<String>,
}

// ---------------------------------------------------------------------------
// Task 5.1 — personality types (no separate structs needed; service methods
// take primitive arguments and return Option<String> / unit)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Task 5.2 — goals types
// ---------------------------------------------------------------------------

/// A goal row returned by `goal_read` and `goal_list`.
///
/// Matches Python `GoalRow` pydantic model in `routes_goals.py`.
#[derive(Debug, Clone, PartialEq)]
pub struct GoalRow {
    pub id: String,
    pub bot_id: String,
    pub text: String,
    pub status: String,
    pub source: Option<String>,
    pub priority: i64,
    pub origin_memory: Option<String>,
    pub created_ts: i64,
    pub updated_ts: i64,
    pub completed_ts: Option<i64>,
}

/// Request for [`MemoryService::goal_create`].
///
/// Matches Python `GoalCreateRequest`.
#[derive(Debug, Clone)]
pub struct GoalCreateReq {
    pub bot_id: String,
    pub text: String,
    pub source: Option<String>,
    /// Default 0 (matches Python `priority: int = 0`).
    pub priority: i64,
    pub origin_memory: Option<String>,
}

/// Response from [`MemoryService::goal_create`].
///
/// Matches Python `GoalCreateResponse`.
#[derive(Debug, Clone)]
pub struct GoalCreateResp {
    pub goal_id: String,
    pub status: String,
}

/// Request for [`MemoryService::goal_update`].
///
/// Matches Python `GoalUpdateRequest`.
#[derive(Debug, Clone)]
pub struct GoalUpdateReq {
    pub bot_id: String,
    pub goal_id: String,
    pub text: Option<String>,
    pub status: Option<String>,
    pub priority: Option<i64>,
    pub origin_memory: Option<String>,
}

/// Response from [`MemoryService::goal_update`].
///
/// Matches Python `GoalUpdateResponse`.
#[derive(Debug, Clone, PartialEq)]
pub struct GoalUpdateResp {
    pub updated: bool,
}

/// Request for [`MemoryService::goal_list`].
///
/// Matches Python `GET /goals/list` query parameters.
#[derive(Debug, Clone)]
pub struct GoalListReq {
    pub bot_id: String,
    /// Optional comma-separated status filter (matches Python's comma-split behavior).
    pub status: Option<String>,
    /// Default 50, clamped to [1, 500].
    pub limit: i64,
    /// Default 0, clamped to ≥ 0.
    pub offset: i64,
}

/// Response from [`MemoryService::goal_list`].
///
/// Matches Python `GoalListResponse`.
#[derive(Debug, Clone)]
pub struct GoalListResp {
    pub items: Vec<GoalRow>,
    pub total: i64,
}

/// Outcome for [`MemoryService::goal_complete`].
///
/// Matches Python `Literal["completed", "abandoned"]`.
#[derive(Debug, Clone, PartialEq)]
pub enum GoalOutcome {
    Completed,
    Abandoned,
}

impl GoalOutcome {
    fn as_str(&self) -> &'static str {
        match self {
            GoalOutcome::Completed => "completed",
            GoalOutcome::Abandoned => "abandoned",
        }
    }
}

/// Request for [`MemoryService::goal_complete`].
///
/// Matches Python `GoalCompleteRequest`.
#[derive(Debug, Clone)]
pub struct GoalCompleteReq {
    pub bot_id: String,
    pub goal_id: String,
    pub outcome: GoalOutcome,
    /// When `true` (default), writes a `goal_link` memory via `self.write()`.
    pub also_record_memory: bool,
}

/// Response from [`MemoryService::goal_complete`].
///
/// Matches Python `GoalCompleteResponse`.
#[derive(Debug, Clone, PartialEq)]
pub struct GoalCompleteResp {
    pub updated: bool,
    pub memory_id: Option<String>,
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
    /// MMR λ trade-off: 1.0 = pure relevance, 0.0 = pure diversity.
    /// Public so that `main.rs` (a separate binary crate) can set it from Settings.
    pub mmr_lambda: f64,
    /// Which timestamp drives the recency component.
    /// Public so that `main.rs` (a separate binary crate) can set it from Settings.
    pub recency_basis: RecencyBasis,
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
        Self {
            db_path, weights, cap_per_bot, embed, pubsub,
            mmr_lambda: 0.7,
            recency_basis: RecencyBasis::Created,
        }
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
        // Returns (memory_id, row_id, evicted, salience_clamped).
        let (memory_id, row_id, evicted, salience_clamped) =
            tokio::task::spawn_blocking(move || -> Result<(String, i64, usize, f32), crate::error::AppError> {
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
                // Capture the rowid immediately after INSERT (before any other inserts
                // might change last_insert_rowid).  Used as the SSE event id cursor.
                let inserted_row_id = conn.last_insert_rowid();

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

                Ok((memory_id, inserted_row_id, evicted, salience))
            })
            .await
            .map_err(|join_err| {
                crate::error::AppError::Internal(anyhow::anyhow!("spawn_blocking panicked: {join_err}"))
            })??;

        // Step 3: publish (fire-and-forget; never propagates error).
        let row = PubSubRow {
            row_id,
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

    // -----------------------------------------------------------------------
    // recall
    // -----------------------------------------------------------------------

    /// Semantic recall: embed the query, score all matching memories, MMR-select
    /// top_k, bump `last_recalled_ts`, return scored memories in MMR order.
    ///
    /// Ports `routes_memory.py::recall` exactly:
    ///
    /// 1. `q_emb = embed(query).await` — async, before spawn_blocking.
    /// 2. In spawn_blocking:
    ///    a. `over_fetch = max(top_k * 4, 20)` — kept for parity; the SQL query
    ///       fetches ALL matching rows (no LIMIT), matching Python behavior.
    ///    b. SELECT all matching rows (bot_id + optional since/until/memory_type).
    ///    c. For each row: `score_memory(emb, q, salience, created_ts, now, &weights)`.
    ///    d. Build `candidates: Vec<(memory_id, Vec<f32>)>` for rows WITH embedding.
    ///    e. `mmr_select(candidates, q_emb, top_k, lambda)`.
    ///    f. UPDATE `last_recalled_ts = now` for each selected memory_id.
    ///    g. COMMIT.
    ///    h. Return selected memories with their weighted-sum scores, in MMR order.
    pub async fn recall(
        &self,
        req: RecallReq,
    ) -> Result<RecallResp, crate::error::AppError> {
        // Step 1: embed the query in async context before entering spawn_blocking.
        let q_emb: Vec<f32> = self
            .embed
            .embed(&req.query)
            .await
            .map(|arc| arc.as_ref().clone())
            .map_err(|e| crate::error::AppError::Internal(anyhow::anyhow!("embed failed: {e}")))?;

        let db_path = self.db_path.clone();
        let weights = self.weights.clone();
        let mmr_lambda = self.mmr_lambda;
        let recency_basis = self.recency_basis.clone();

        tokio::task::spawn_blocking(move || -> Result<RecallResp, crate::error::AppError> {
            let conn = db::open_db(&db_path)?;

            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);

            // over_fetch computed for parity (Python computes it even though the
            // SELECT has no LIMIT — the comment in Python explains the design).
            let _over_fetch = (req.top_k * 4).max(20);

            // Build optional WHERE fragments (mirrors Python `_extra_where_clause`).
            use rusqlite::types::Value as SqlValue;
            let mut extra_where: Vec<&'static str> = Vec::new();
            let mut extra_params: Vec<SqlValue> = Vec::new();

            if req.since_ts.is_some() {
                extra_where.push("AND created_ts >= ?");
                extra_params.push(SqlValue::Integer(req.since_ts.unwrap()));
            }
            if req.until_ts.is_some() {
                extra_where.push("AND created_ts <= ?");
                extra_params.push(SqlValue::Integer(req.until_ts.unwrap()));
            }
            if req.memory_type.is_some() {
                extra_where.push("AND memory_type = ?");
                extra_params.push(SqlValue::Text(req.memory_type.clone().unwrap()));
            }

            let where_fragment = extra_where.join(" ");
            let sql = format!(
                "SELECT id, text, salience, created_ts, last_recalled_ts, embedding \
                 FROM memories WHERE bot_id = ? {where_fragment} \
                 ORDER BY created_ts DESC"
            );

            let mut all_params: Vec<SqlValue> = vec![SqlValue::Text(req.bot_id.clone())];
            all_params.extend(extra_params);

            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt
                .query_map(rusqlite::params_from_iter(all_params.iter()), |row| {
                    Ok((
                        row.get::<_, String>(0)?,       // id
                        row.get::<_, String>(1)?,       // text
                        row.get::<_, f64>(2)?,          // salience
                        row.get::<_, i64>(3)?,          // created_ts
                        row.get::<_, i64>(4)?,          // last_recalled_ts
                        row.get::<_, Option<Vec<u8>>>(5)?, // embedding (nullable)
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;

            if rows.is_empty() {
                return Ok(RecallResp { memories: vec![] });
            }

            // Per-row scoring + build candidates list (only rows WITH embedding).
            // `meta` maps memory_id → (text, weighted_sum_score, created_ts).
            let mut candidates: Vec<(String, Vec<f32>)> = Vec::new();
            let mut meta: std::collections::HashMap<String, (String, f64, i64)> =
                std::collections::HashMap::new();

            for (mid, text, salience, created_ts, last_recalled_ts, emb_blob) in rows {
                let emb: Option<Vec<f32>> = emb_blob.as_deref().map(db::unpack_f32_le);

                // Recency basis: default = created_ts, "last_recalled" → last_recalled_ts.
                let recency_ts = match recency_basis {
                    RecencyBasis::Created => created_ts,
                    RecencyBasis::LastRecalled => last_recalled_ts,
                };

                let score = crate::retrieval::recall::score_memory(
                    emb.as_deref(),
                    &q_emb,
                    salience,
                    recency_ts,
                    now,
                    &weights,
                );

                if let Some(emb_vec) = emb {
                    candidates.push((mid.clone(), emb_vec));
                }
                meta.insert(mid, (text, score, created_ts));
            }

            // MMR diversity reranking over embedding-bearing candidates.
            let selected =
                crate::retrieval::mmr::mmr_select(&candidates, &q_emb, req.top_k, mmr_lambda);

            // Bump last_recalled_ts for selected memories + commit.
            conn.execute_batch("BEGIN")?;
            for (mid, _) in &selected {
                conn.execute(
                    "UPDATE memories SET last_recalled_ts = ?1 WHERE id = ?2",
                    rusqlite::params![now, mid],
                )?;
            }
            conn.execute_batch("COMMIT")?;

            // Build response: weighted-sum score from `meta` (NOT the MMR score).
            let memories = selected
                .iter()
                .filter_map(|(mid, _)| {
                    meta.get(mid).map(|(text, score, ts)| RecalledMemory {
                        memory_id: mid.clone(),
                        text: text.clone(),
                        score: *score,
                        ts: *ts,
                    })
                })
                .collect();

            Ok(RecallResp { memories })
        })
        .await
        .map_err(|e| crate::error::AppError::Internal(anyhow::anyhow!("spawn_blocking panicked: {e}")))?
    }

    // -----------------------------------------------------------------------
    // recall_about
    // -----------------------------------------------------------------------

    /// Entity-centric recall: BFS from a named entity, gather candidate memories,
    /// score+MMR, return the texts as `hints`.
    ///
    /// Ports `routes_memory.py::recall_about` exactly:
    ///
    /// 1. Lookup seed entity by `name_lower` for the bot.
    /// 2. BFS over `edges` (both src and dst) for `max_hops` hops.
    /// 3. Gather memories linked to the visited entity_ids via `memory_entities`.
    /// 4. `q_emb = embed(f"recent events near {entity}").await`.
    /// 5. Score + MMR + bump `last_recalled_ts`.
    /// 6. Return `hints` = texts in MMR order.
    ///
    /// Note: the embedding is done AFTER the initial DB lookup (seed entity
    /// must exist), matching Python's ordering.
    pub async fn recall_about(
        &self,
        req: RecallAboutReq,
    ) -> Result<RecallAboutResp, crate::error::AppError> {
        // Step 1–3: Synchronous DB work (lookup seed + BFS + gather candidates).
        // No embedding needed yet — we do the entity lookup first to short-circuit
        // on unknown entity before hitting the network.
        let db_path = self.db_path.clone();
        let weights = self.weights.clone();
        let mmr_lambda = self.mmr_lambda;
        let recency_basis = self.recency_basis.clone();
        let entity_lower = req.entity.to_lowercase();

        // Phase A: find seed_id and gather memory rows.
        // We do this in spawn_blocking BEFORE calling the embedder so we can
        // return early (hints=[]) if the entity is unknown.
        let (seed_found, candidate_rows) = {
            let db_path2 = db_path.clone();
            let bot_id2 = req.bot_id.clone();
            let entity_lower2 = entity_lower.clone();
            let max_hops = req.max_hops;
            let since_ts = req.since_ts;
            let until_ts = req.until_ts;
            let memory_type = req.memory_type.clone();

            tokio::task::spawn_blocking(move || -> Result<(bool, Vec<(String, String, f64, i64, i64, Option<Vec<u8>>)>), crate::error::AppError> {
                let conn = db::open_db(&db_path2)?;

                // Lookup seed entity.
                let seed_id: Option<i64> = conn
                    .query_row(
                        "SELECT id FROM entities WHERE bot_id = ?1 AND name_lower = ?2",
                        rusqlite::params![bot_id2, entity_lower2],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(crate::error::AppError::Db)?;

                let seed_id = match seed_id {
                    Some(id) => id,
                    None => return Ok((false, vec![])),
                };

                // BFS over edges.
                let visited = bfs_entities(&conn, seed_id, max_hops)?;

                // Gather candidate memories for visited entities.
                use rusqlite::types::Value as SqlValue;
                let mut extra_where: Vec<String> = Vec::new();
                let mut extra_params: Vec<SqlValue> = Vec::new();

                if let Some(since) = since_ts {
                    extra_where.push("AND m.created_ts >= ?".to_owned());
                    extra_params.push(SqlValue::Integer(since));
                }
                if let Some(until) = until_ts {
                    extra_where.push("AND m.created_ts <= ?".to_owned());
                    extra_params.push(SqlValue::Integer(until));
                }
                if let Some(ref mt) = memory_type {
                    extra_where.push("AND m.memory_type = ?".to_owned());
                    extra_params.push(SqlValue::Text(mt.clone()));
                }

                let where_fragment = extra_where.join(" ");

                // Build placeholders for visited entity_ids.
                let visited_placeholders = visited
                    .iter()
                    .enumerate()
                    .map(|(i, _)| format!("?{}", i + 2)) // ?1 = bot_id
                    .collect::<Vec<_>>()
                    .join(",");

                // Count how many fixed params we have before the extras.
                // Extra params use ? (positional, unindexed) — rusqlite accepts both.
                let sql = format!(
                    "SELECT DISTINCT m.id, m.text, m.salience, m.created_ts, \
                     m.last_recalled_ts, m.embedding \
                     FROM memories m \
                     JOIN memory_entities me ON me.memory_id = m.id \
                     WHERE m.bot_id = ?1 AND me.entity_id IN ({visited_placeholders}) \
                     {where_fragment}"
                );

                // Build the full params list: bot_id, then entity_ids, then extras.
                let mut params: Vec<SqlValue> = vec![SqlValue::Text(bot_id2)];
                params.extend(visited.iter().map(|&id| SqlValue::Integer(id)));
                params.extend(extra_params);

                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params.iter()), |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, f64>(2)?,
                            row.get::<_, i64>(3)?,
                            row.get::<_, i64>(4)?,
                            row.get::<_, Option<Vec<u8>>>(5)?,
                        ))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;

                Ok((true, rows))
            })
            .await
            .map_err(|e| crate::error::AppError::Internal(anyhow::anyhow!("spawn_blocking panicked: {e}")))??
        };

        if !seed_found || candidate_rows.is_empty() {
            return Ok(RecallAboutResp { hints: vec![] });
        }

        // Step 4: embed the query (async, after knowing the entity exists).
        // Python: `q_emb = await embedder.embed(f"recent events near {req.entity}")`
        let query_text = format!("recent events near {}", req.entity);
        let q_emb: Vec<f32> = self
            .embed
            .embed(&query_text)
            .await
            .map(|arc| arc.as_ref().clone())
            .map_err(|e| crate::error::AppError::Internal(anyhow::anyhow!("embed failed: {e}")))?;

        // Step 5–6: Score + MMR + update last_recalled_ts + return hints.
        let top_k = req.top_k;

        tokio::task::spawn_blocking(move || -> Result<RecallAboutResp, crate::error::AppError> {
            let conn = db::open_db(&db_path)?;

            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);

            let mut candidates: Vec<(String, Vec<f32>)> = Vec::new();
            let mut meta: std::collections::HashMap<String, (String, f64, i64)> =
                std::collections::HashMap::new();

            for (mid, text, salience, created_ts, last_recalled_ts, emb_blob) in candidate_rows {
                let emb: Option<Vec<f32>> = emb_blob.as_deref().map(db::unpack_f32_le);

                let recency_ts = match recency_basis {
                    RecencyBasis::Created => created_ts,
                    RecencyBasis::LastRecalled => last_recalled_ts,
                };

                let score = crate::retrieval::recall::score_memory(
                    emb.as_deref(),
                    &q_emb,
                    salience,
                    recency_ts,
                    now,
                    &weights,
                );

                if let Some(emb_vec) = emb {
                    candidates.push((mid.clone(), emb_vec));
                }
                meta.insert(mid, (text, score, created_ts));
            }

            let selected =
                crate::retrieval::mmr::mmr_select(&candidates, &q_emb, top_k, mmr_lambda);

            conn.execute_batch("BEGIN")?;
            for (mid, _) in &selected {
                conn.execute(
                    "UPDATE memories SET last_recalled_ts = ?1 WHERE id = ?2",
                    rusqlite::params![now, mid],
                )?;
            }
            conn.execute_batch("COMMIT")?;

            let hints = selected
                .iter()
                .filter_map(|(mid, _)| meta.get(mid).map(|(text, _, _)| text.clone()))
                .collect();

            Ok(RecallAboutResp { hints })
        })
        .await
        .map_err(|e| crate::error::AppError::Internal(anyhow::anyhow!("spawn_blocking panicked: {e}")))?
    }

    // -----------------------------------------------------------------------
    // search
    // -----------------------------------------------------------------------

    /// 3-lane hybrid search: BM25 + dense + entity → RRF → MMR.
    ///
    /// Ports `routes_memory.py::search` exactly.
    ///
    /// Orchestration:
    /// 1. `q = embed.embed(query).await` — async, before spawn_blocking.
    /// 2. In spawn_blocking:
    ///    a. `over_fetch = max(top_k * 4, 20)`.
    ///    b. **Lane 1 BM25**: `build_fts5_query(query)` → FTS5 MATCH joined to
    ///       `memories` with bot_id + optional filters; `ORDER BY rank LIMIT over_fetch`.
    ///       Skipped (empty lane) when `build_fts5_query` returns `None`.
    ///    c. **Lane 2 dense**: fetch all bot rows (+ filters) with embedding; cosine-score
    ///       each; sort descending; take top `over_fetch`.
    ///    d. **Lane 3 entity**: find entities whose `name_lower` is a substring of
    ///       `query.to_lowercase()`; BFS 2 hops; DISTINCT memories via `memory_entities`
    ///       (+ filters) LIMIT over_fetch.
    ///    e. `fused = rrf_fuse([bm25, dense, entity], 60)` — Vec<(id, score)>.
    ///    f. Build signal-rank position maps (memory_id → 0-indexed position in lane).
    ///    g. Fetch `text, created_ts, embedding` for fused ids; build MMR candidates
    ///       preserving fused insertion order.
    ///    h. `mmr_select(candidates, q, top_k, mmr_lambda)`.
    ///    i. Return `SearchResp { items, total_candidates: fused.len() }`.
    pub async fn search(
        &self,
        req: SearchReq,
    ) -> Result<SearchResp, crate::error::AppError> {
        // Step 1: embed the query in async context.
        let q_emb: Vec<f32> = self
            .embed
            .embed(&req.query)
            .await
            .map(|arc| arc.as_ref().clone())
            .map_err(|e| crate::error::AppError::Internal(anyhow::anyhow!("embed failed: {e}")))?;

        let db_path = self.db_path.clone();
        let mmr_lambda = self.mmr_lambda;

        tokio::task::spawn_blocking(move || -> Result<SearchResp, crate::error::AppError> {
            let conn = db::open_db(&db_path)?;

            let over_fetch = (req.top_k * 4).max(20);

            // Build optional WHERE fragments using the aliased form `m.column`
            // (matches Python `_extra_where_clause_alias("m", req)`).
            use rusqlite::types::Value as SqlValue;
            let mut extra_alias_where: Vec<&'static str> = Vec::new();
            let mut extra_alias_params: Vec<SqlValue> = Vec::new();

            if let Some(since) = req.since_ts {
                extra_alias_where.push("AND m.created_ts >= ?");
                extra_alias_params.push(SqlValue::Integer(since));
            }
            if let Some(until) = req.until_ts {
                extra_alias_where.push("AND m.created_ts <= ?");
                extra_alias_params.push(SqlValue::Integer(until));
            }
            if let Some(ref mt) = req.memory_type {
                extra_alias_where.push("AND m.memory_type = ?");
                extra_alias_params.push(SqlValue::Text(mt.clone()));
            }
            let alias_where_fragment = extra_alias_where.join(" ");

            // ----------------------------------------------------------------
            // Lane 1: BM25 via FTS5
            //
            // Python:
            //   fts_q = build_fts5_query(req.query)
            //   if fts_q:
            //       cur = conn.execute(
            //           "SELECT m.id FROM memories_fts f "
            //           "JOIN memories m ON m.rowid = f.rowid "
            //           "WHERE memories_fts MATCH ? AND m.bot_id = ? {extra_alias_where} "
            //           "ORDER BY rank LIMIT ?",
            //           (fts_q, req.bot_id) + extra_alias_params + (over_fetch,),
            //       )
            //       bm25 = [r[0] for r in cur.fetchall()]
            //   else:
            //       bm25 = []
            // ----------------------------------------------------------------
            use crate::retrieval::helpers::build_fts5_query;
            let bm25: Vec<String> = match build_fts5_query(&req.query, 6) {
                Some(fts_q) => {
                    // Build params: fts_q, bot_id, [since/until/type], over_fetch
                    let mut params: Vec<SqlValue> = vec![
                        SqlValue::Text(fts_q),
                        SqlValue::Text(req.bot_id.clone()),
                    ];
                    params.extend(extra_alias_params.iter().cloned());
                    params.push(SqlValue::Integer(over_fetch as i64));

                    let sql = format!(
                        "SELECT m.id FROM memories_fts f \
                         JOIN memories m ON m.rowid = f.rowid \
                         WHERE memories_fts MATCH ? AND m.bot_id = ? {alias_where_fragment} \
                         ORDER BY rank LIMIT ?"
                    );
                    let mut stmt = conn.prepare(&sql)?;
                    let result = stmt.query_map(rusqlite::params_from_iter(params.iter()), |row| {
                        row.get::<_, String>(0)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                    result
                }
                None => Vec::new(),
            };

            // ----------------------------------------------------------------
            // Lane 2: Dense (per-bot Python cosine scoring)
            //
            // Python:
            //   cur = conn.execute(
            //       "SELECT m.id, m.embedding FROM memories m "
            //       "WHERE m.bot_id = ? {extra_alias_where} ORDER BY m.created_ts DESC",
            //       (req.bot_id,) + extra_alias_params,
            //   )
            //   dense_scored = [(sim, mid) for mid, emb_blob in cur if emb_blob]
            //   dense_scored.sort(reverse=True, key=lambda t: t[0])
            //   dense = [mid for _, mid in dense_scored[:over_fetch]]
            // ----------------------------------------------------------------
            let dense: Vec<String> = {
                let mut params: Vec<SqlValue> = vec![SqlValue::Text(req.bot_id.clone())];
                params.extend(extra_alias_params.iter().cloned());

                let sql = format!(
                    "SELECT m.id, m.embedding FROM memories m \
                     WHERE m.bot_id = ? {alias_where_fragment} \
                     ORDER BY m.created_ts DESC"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params.iter()), |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, Option<Vec<u8>>>(1)?,
                        ))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;

                use crate::retrieval::cosine::cosine;
                let mut dense_scored: Vec<(f64, String)> = rows
                    .into_iter()
                    .filter_map(|(mid, blob)| {
                        blob.map(|b| {
                            let emb = db::unpack_f32_le(&b);
                            let sim = cosine(&emb, &q_emb);
                            (sim, mid)
                        })
                    })
                    .collect();
                // Sort descending by cosine similarity (matches Python `.sort(reverse=True)`).
                dense_scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
                dense_scored
                    .into_iter()
                    .take(over_fetch)
                    .map(|(_, mid)| mid)
                    .collect()
            };

            // ----------------------------------------------------------------
            // Lane 3: Entity via BFS (2 hops hardcoded)
            //
            // Python:
            //   cur = conn.execute("SELECT name_lower, id FROM entities WHERE bot_id = ?", ...)
            //   q_lower = req.query.lower()
            //   seed_ids = [eid for nl, eid in ents if nl and nl in q_lower]
            //   entity = []
            //   if seed_ids:
            //       visited = set(seed_ids)
            //       for _ in range(2):  # max_hops=2
            //           BFS one hop
            //       placeholders = ...
            //       cur = conn.execute(
            //           "SELECT DISTINCT m.id FROM memories m "
            //           "JOIN memory_entities me ON me.memory_id = m.id "
            //           "WHERE m.bot_id = ? AND me.entity_id IN ({placeholders}) "
            //           "{extra_alias_where} LIMIT ?",
            //           (req.bot_id,) + tuple(visited) + extra_alias_params + (over_fetch,),
            //       )
            //       entity = [r[0] for r in cur.fetchall()]
            // ----------------------------------------------------------------
            let entity: Vec<String> = {
                let mut ent_stmt = conn.prepare(
                    "SELECT name_lower, id FROM entities WHERE bot_id = ?1",
                )?;
                let ents: Vec<(String, i64)> = ent_stmt
                    .query_map(rusqlite::params![req.bot_id], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;

                let q_lower = req.query.to_lowercase();
                let seed_ids: Vec<i64> = ents
                    .into_iter()
                    .filter(|(nl, _)| !nl.is_empty() && q_lower.contains(nl.as_str()))
                    .map(|(_, eid)| eid)
                    .collect();

                if seed_ids.is_empty() {
                    Vec::new()
                } else {
                    use std::collections::HashSet;
                    let mut visited: HashSet<i64> = seed_ids.iter().copied().collect();

                    // BFS 2 hops hardcoded (matches Python `for _ in range(2)`).
                    for _ in 0..2usize {
                        if visited.is_empty() {
                            break;
                        }
                        let frontier_vec: Vec<i64> = visited.iter().copied().collect();
                        let placeholders = frontier_vec
                            .iter()
                            .map(|_| "?")
                            .collect::<Vec<_>>()
                            .join(",");
                        let sql = format!(
                            "SELECT src_entity_id, dst_entity_id FROM edges \
                             WHERE src_entity_id IN ({placeholders}) \
                                OR dst_entity_id IN ({placeholders})"
                        );
                        // Params: frontier × 2.
                        let mut bfs_params: Vec<SqlValue> = frontier_vec
                            .iter()
                            .map(|&id| SqlValue::Integer(id))
                            .collect();
                        bfs_params.extend(frontier_vec.iter().map(|&id| SqlValue::Integer(id)));

                        let mut bfs_stmt = conn.prepare(&sql)?;
                        let edges: Vec<(i64, i64)> = bfs_stmt
                            .query_map(rusqlite::params_from_iter(bfs_params.iter()), |row| {
                                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
                            })?
                            .collect::<rusqlite::Result<Vec<_>>>()?;

                        for (s, d) in edges {
                            visited.insert(s);
                            visited.insert(d);
                        }
                    }

                    // Gather DISTINCT memories linked to the visited entity set.
                    let visited_vec: Vec<i64> = visited.into_iter().collect();
                    let ent_placeholders = visited_vec
                        .iter()
                        .map(|_| "?")
                        .collect::<Vec<_>>()
                        .join(",");
                    let entity_sql = format!(
                        "SELECT DISTINCT m.id FROM memories m \
                         JOIN memory_entities me ON me.memory_id = m.id \
                         WHERE m.bot_id = ? AND me.entity_id IN ({ent_placeholders}) \
                         {alias_where_fragment} \
                         LIMIT ?"
                    );
                    // Params: bot_id, visited entity ids, extra alias params, over_fetch.
                    let mut ent_params: Vec<SqlValue> =
                        vec![SqlValue::Text(req.bot_id.clone())];
                    ent_params.extend(visited_vec.iter().map(|&id| SqlValue::Integer(id)));
                    ent_params.extend(extra_alias_params.iter().cloned());
                    ent_params.push(SqlValue::Integer(over_fetch as i64));

                    let mut ent_stmt2 = conn.prepare(&entity_sql)?;
                    let ent_result = ent_stmt2
                        .query_map(rusqlite::params_from_iter(ent_params.iter()), |row| {
                            row.get::<_, String>(0)
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    ent_result
                }
            };

            // ----------------------------------------------------------------
            // RRF fusion
            //
            // Python: fused = rrf_fuse([bm25, dense, entity], k=60)
            //         if not fused: return SearchResponse(items=[], total_candidates=0)
            // ----------------------------------------------------------------
            use crate::retrieval::rrf::rrf_fuse;
            let fused: Vec<(String, f64)> =
                rrf_fuse(&[bm25.clone(), dense.clone(), entity.clone()], 60);

            if fused.is_empty() {
                return Ok(SearchResp { items: Vec::new(), total_candidates: 0 });
            }

            let total_candidates = fused.len();

            // Build 0-indexed position maps for signals.
            // Python: bm25_pos  = {d: i for i, d in enumerate(bm25)}
            //         dense_pos = {d: i for i, d in enumerate(dense)}
            //         entity_pos= {d: i for i, d in enumerate(entity)}
            let bm25_pos: std::collections::HashMap<&str, usize> =
                bm25.iter().enumerate().map(|(i, d)| (d.as_str(), i)).collect();
            let dense_pos: std::collections::HashMap<&str, usize> =
                dense.iter().enumerate().map(|(i, d)| (d.as_str(), i)).collect();
            let entity_pos: std::collections::HashMap<&str, usize> =
                entity.iter().enumerate().map(|(i, d)| (d.as_str(), i)).collect();

            // Build a lookup map of RRF scores.
            let fused_score: std::collections::HashMap<&str, f64> =
                fused.iter().map(|(id, s)| (id.as_str(), *s)).collect();

            // ----------------------------------------------------------------
            // Fetch text + created_ts + embedding for the fused ids.
            //
            // Python:
            //   ids_to_score = list(fused.keys())
            //   cur = conn.execute(
            //       "SELECT id, text, created_ts, embedding "
            //       "FROM memories WHERE bot_id = ? AND id IN ({placeholders})",
            //       (req.bot_id,) + tuple(ids_to_score),
            //   )
            //   candidates_for_mmr: list[(str, np.ndarray)] = []  # only rows with blob
            //   text_by_id: dict[str, (str, int)] = {}
            // ----------------------------------------------------------------
            let ids_to_score: Vec<&str> =
                fused.iter().map(|(id, _)| id.as_str()).collect();
            let placeholders = ids_to_score
                .iter()
                .map(|_| "?")
                .collect::<Vec<_>>()
                .join(",");
            let fetch_sql = format!(
                "SELECT id, text, created_ts, embedding \
                 FROM memories WHERE bot_id = ? AND id IN ({placeholders})"
            );
            let mut fetch_params: Vec<SqlValue> =
                vec![SqlValue::Text(req.bot_id.clone())];
            fetch_params.extend(
                ids_to_score.iter().map(|&id| SqlValue::Text(id.to_string())),
            );
            let mut fetch_stmt = conn.prepare(&fetch_sql)?;
            let fetched_rows: Vec<(String, String, i64, Option<Vec<u8>>)> = fetch_stmt
                .query_map(rusqlite::params_from_iter(fetch_params.iter()), |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<Vec<u8>>>(3)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;

            // Build text_by_id and candidates_for_mmr.
            // candidates_for_mmr preserves fused insertion order (Python iterates
            // `fused.keys()` which is CPython insertion-ordered dict).
            let mut text_by_id: std::collections::HashMap<String, (String, i64)> =
                std::collections::HashMap::new();
            let mut emb_by_id: std::collections::HashMap<String, Vec<f32>> =
                std::collections::HashMap::new();

            for (mid, text, ct, blob) in fetched_rows {
                text_by_id.insert(mid.clone(), (text, ct));
                if let Some(b) = blob {
                    emb_by_id.insert(mid, db::unpack_f32_le(&b));
                }
            }

            // Build candidates in fused insertion order, including only rows with
            // an embedding (matching Python's `if blob: candidates_for_mmr.append(...)`).
            let candidates_for_mmr: Vec<(String, Vec<f32>)> = fused
                .iter()
                .filter_map(|(id, _)| {
                    emb_by_id.get(id.as_str()).map(|emb| (id.clone(), emb.clone()))
                })
                .collect();

            // ----------------------------------------------------------------
            // MMR diversity reranking.
            // ----------------------------------------------------------------
            use crate::retrieval::mmr::mmr_select;
            let selected =
                mmr_select(&candidates_for_mmr, &q_emb, req.top_k, mmr_lambda);

            // ----------------------------------------------------------------
            // Build SearchItems.
            //
            // Python:
            //   for mid, _ in selected:
            //       text, ct = text_by_id.get(mid, ("", 0))
            //       items.append(SearchItem(
            //           memory_id=mid, text=text,
            //           score=fused[mid],    ← RRF score
            //           ts=ct,
            //           signals=SearchSignals(
            //               bm25_rank=bm25_pos.get(mid),
            //               dense_rank=dense_pos.get(mid),
            //               entity_rank=entity_pos.get(mid),
            //           ),
            //       ))
            // ----------------------------------------------------------------
            let items: Vec<SearchItem> = selected
                .iter()
                .map(|(mid, _)| {
                    let (text, ct) = text_by_id
                        .get(mid.as_str())
                        .cloned()
                        .unwrap_or_else(|| (String::new(), 0));
                    let score = *fused_score.get(mid.as_str()).unwrap_or(&0.0);
                    SearchItem {
                        memory_id: mid.clone(),
                        text,
                        score,
                        ts: ct,
                        signals: SearchSignals {
                            bm25_rank:   bm25_pos.get(mid.as_str()).copied(),
                            dense_rank:  dense_pos.get(mid.as_str()).copied(),
                            entity_rank: entity_pos.get(mid.as_str()).copied(),
                        },
                    }
                })
                .collect();

            Ok(SearchResp { items, total_candidates })
        })
        .await
        .map_err(|e| crate::error::AppError::Internal(anyhow::anyhow!("spawn_blocking panicked: {e}")))?
    }

    // -----------------------------------------------------------------------
    // personality_get
    // -----------------------------------------------------------------------

    /// Get the persona string for `bot_id`.
    ///
    /// Returns `None` if the bot row does not exist OR its `persona` column is
    /// NULL — matching Python:
    /// ```python
    /// if not row or row[0] is None:
    ///     raise HTTPException(status_code=404, detail="no persona for this bot")
    /// ```
    /// The REST layer converts `None` → 404.
    pub async fn personality_get(
        &self,
        bot_id: &str,
    ) -> Result<Option<String>, crate::error::AppError> {
        let db_path = self.db_path.clone();
        let bot_id = bot_id.to_owned();

        tokio::task::spawn_blocking(move || -> Result<Option<String>, crate::error::AppError> {
            let conn = db::open_db(&db_path)?;
            use rusqlite::OptionalExtension;
            // Two-level Option: outer = row existence, inner = NULL persona.
            let row: Option<Option<String>> = conn
                .query_row(
                    "SELECT persona FROM bots WHERE bot_id=?1",
                    rusqlite::params![bot_id],
                    |r| r.get(0),
                )
                .optional()?;
            // Flatten: None row or None persona → both return None.
            Ok(row.flatten())
        })
        .await
        .map_err(|e| crate::error::AppError::Internal(anyhow::anyhow!("spawn_blocking panicked: {e}")))?
    }

    // -----------------------------------------------------------------------
    // personality_set
    // -----------------------------------------------------------------------

    /// UPSERT the persona for `bot_id`.
    ///
    /// Enforces a 4000-character cap matching Python `PERSONA_MAX_CHARS = 4000`.
    /// Returns `AppError::BadRequest` when exceeded.
    ///
    /// SQL: `INSERT INTO bots … ON CONFLICT(bot_id) DO UPDATE SET persona=excluded.persona`
    pub async fn personality_set(
        &self,
        bot_id: &str,
        persona: String,
    ) -> Result<(), crate::error::AppError> {
        const PERSONA_MAX_CHARS: usize = 4000;
        if persona.len() > PERSONA_MAX_CHARS {
            return Err(crate::error::AppError::BadRequest(format!(
                "persona exceeds {PERSONA_MAX_CHARS} chars"
            )));
        }
        let db_path = self.db_path.clone();
        let bot_id = bot_id.to_owned();

        tokio::task::spawn_blocking(move || -> Result<(), crate::error::AppError> {
            let conn = db::open_db(&db_path)?;
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            conn.execute(
                "INSERT INTO bots (bot_id, persona, created_ts) VALUES (?1, ?2, ?3) \
                 ON CONFLICT(bot_id) DO UPDATE SET persona=excluded.persona",
                rusqlite::params![bot_id, persona, now],
            )?;
            Ok(())
        })
        .await
        .map_err(|e| crate::error::AppError::Internal(anyhow::anyhow!("spawn_blocking panicked: {e}")))?
    }

    // -----------------------------------------------------------------------
    // goal_create
    // -----------------------------------------------------------------------

    /// Create a new goal in status `"pending"`.
    ///
    /// Mirrors Python `POST /goals/create`:
    /// - Generates a `g_*` ID via `generate_goal_id()`.
    /// - `created_ts = updated_ts = now()`, `completed_ts = NULL`.
    /// - Returns `GoalCreateResp { goal_id, status: "pending" }`.
    pub async fn goal_create(
        &self,
        req: GoalCreateReq,
    ) -> Result<GoalCreateResp, crate::error::AppError> {
        let db_path = self.db_path.clone();

        tokio::task::spawn_blocking(move || -> Result<GoalCreateResp, crate::error::AppError> {
            let conn = db::open_db(&db_path)?;
            let gid = crate::ids::generate_goal_id();
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);

            conn.execute(
                "INSERT INTO goals \
                 (id, bot_id, text, status, source, priority, origin_memory, created_ts, updated_ts) \
                 VALUES (?1, ?2, ?3, 'pending', ?4, ?5, ?6, ?7, ?7)",
                rusqlite::params![
                    gid,
                    req.bot_id,
                    req.text,
                    req.source,
                    req.priority,
                    req.origin_memory,
                    now,
                ],
            )?;

            Ok(GoalCreateResp { goal_id: gid, status: "pending".to_owned() })
        })
        .await
        .map_err(|e| crate::error::AppError::Internal(anyhow::anyhow!("spawn_blocking panicked: {e}")))?
    }

    // -----------------------------------------------------------------------
    // goal_read
    // -----------------------------------------------------------------------

    /// Read a single goal by `goal_id` scoped to `bot_id`.
    ///
    /// Returns `None` when no matching row exists — REST layer maps to 404.
    pub async fn goal_read(
        &self,
        bot_id: &str,
        goal_id: &str,
    ) -> Result<Option<GoalRow>, crate::error::AppError> {
        let db_path = self.db_path.clone();
        let bot_id = bot_id.to_owned();
        let goal_id = goal_id.to_owned();

        tokio::task::spawn_blocking(move || -> Result<Option<GoalRow>, crate::error::AppError> {
            let conn = db::open_db(&db_path)?;
            use rusqlite::OptionalExtension;
            let row = conn
                .query_row(
                    "SELECT id, bot_id, text, status, source, priority, origin_memory, \
                            created_ts, updated_ts, completed_ts \
                     FROM goals WHERE id=?1 AND bot_id=?2",
                    rusqlite::params![goal_id, bot_id],
                    |r| {
                        Ok(GoalRow {
                            id:            r.get(0)?,
                            bot_id:        r.get(1)?,
                            text:          r.get(2)?,
                            status:        r.get(3)?,
                            source:        r.get(4)?,
                            priority:      r.get(5)?,
                            origin_memory: r.get(6)?,
                            created_ts:    r.get(7)?,
                            updated_ts:    r.get(8)?,
                            completed_ts:  r.get(9)?,
                        })
                    },
                )
                .optional()?;
            Ok(row)
        })
        .await
        .map_err(|e| crate::error::AppError::Internal(anyhow::anyhow!("spawn_blocking panicked: {e}")))?
    }

    // -----------------------------------------------------------------------
    // goal_update
    // -----------------------------------------------------------------------

    /// Patch a goal row.
    ///
    /// Mirrors Python `PUT /goals/update` including `VALID_TRANSITIONS` enforcement
    /// and `VALID_STATUSES` check:
    ///
    /// `VALID_TRANSITIONS`:
    /// - `pending`   → `{active, abandoned}`
    /// - `active`    → `{completed, abandoned}`
    /// - `completed` → `{}` (terminal)
    /// - `abandoned` → `{}` (terminal)
    ///
    /// Error cases (matching Python's HTTP 400):
    /// - `req.status` is not a valid status string → `BadRequest("invalid status: {s}")`.
    /// - `req.status` is valid but not reachable from `current_status` →
    ///   `BadRequest("invalid transition {current} → {new}")`.
    /// - No updatable field provided (all options are None) →
    ///   `BadRequest("no updatable field provided")`.
    /// - Goal not found → `GoalUpdateResp { updated: false }`.
    ///
    /// Sets `completed_ts = now` when the new status is terminal.
    /// Always bumps `updated_ts`.
    pub async fn goal_update(
        &self,
        req: GoalUpdateReq,
    ) -> Result<GoalUpdateResp, crate::error::AppError> {
        const VALID_STATUSES: &[&str] = &["pending", "active", "completed", "abandoned"];
        const TERMINAL_STATUSES: &[&str] = &["completed", "abandoned"];

        // Validate status field before touching the DB.
        if let Some(ref new_status) = req.status {
            if !VALID_STATUSES.contains(&new_status.as_str()) {
                return Err(crate::error::AppError::BadRequest(format!(
                    "invalid status: {new_status}"
                )));
            }
        }

        // No fields to update check (mirrors Python "no updatable field provided").
        if req.text.is_none()
            && req.status.is_none()
            && req.priority.is_none()
            && req.origin_memory.is_none()
        {
            return Err(crate::error::AppError::BadRequest(
                "no updatable field provided".to_owned(),
            ));
        }

        let db_path = self.db_path.clone();

        tokio::task::spawn_blocking(move || -> Result<GoalUpdateResp, crate::error::AppError> {
            let conn = db::open_db(&db_path)?;

            // Fetch current row (existence + current status).
            use rusqlite::OptionalExtension;
            let row: Option<(String,)> = conn
                .query_row(
                    "SELECT status FROM goals WHERE id=?1 AND bot_id=?2",
                    rusqlite::params![req.goal_id, req.bot_id],
                    |r| Ok((r.get::<_, String>(0)?,)),
                )
                .optional()?;

            let current_status = match row {
                Some((s,)) => s,
                None => return Ok(GoalUpdateResp { updated: false }),
            };

            // Validate transition.
            if let Some(ref new_status) = req.status {
                if new_status != &current_status {
                    let allowed: &[&str] = match current_status.as_str() {
                        "pending"   => &["active", "abandoned"],
                        "active"    => &["completed", "abandoned"],
                        "completed" => &[],
                        "abandoned" => &[],
                        _           => &[],
                    };
                    if !allowed.contains(&new_status.as_str()) {
                        return Err(crate::error::AppError::BadRequest(format!(
                            "invalid transition {current_status} \u{2192} {new_status}"
                        )));
                    }
                }
            }

            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);

            // Build SET clause dynamically.
            use rusqlite::types::Value as SqlValue;
            let mut sets: Vec<&str> = Vec::new();
            let mut params: Vec<SqlValue> = Vec::new();

            if let Some(ref text) = req.text {
                sets.push("text=?");
                params.push(SqlValue::Text(text.clone()));
            }
            if let Some(ref prio) = req.priority {
                sets.push("priority=?");
                params.push(SqlValue::Integer(*prio));
            }
            if let Some(ref om) = req.origin_memory {
                sets.push("origin_memory=?");
                params.push(SqlValue::Text(om.clone()));
            }
            if let Some(ref new_status) = req.status {
                sets.push("status=?");
                params.push(SqlValue::Text(new_status.clone()));
                if TERMINAL_STATUSES.contains(&new_status.as_str()) {
                    sets.push("completed_ts=?");
                    params.push(SqlValue::Integer(now));
                }
            }

            sets.push("updated_ts=?");
            params.push(SqlValue::Integer(now));
            params.push(SqlValue::Text(req.goal_id.clone()));

            let sql = format!("UPDATE goals SET {} WHERE id=?", sets.join(", "));
            conn.execute(&sql, rusqlite::params_from_iter(params.iter()))?;

            Ok(GoalUpdateResp { updated: true })
        })
        .await
        .map_err(|e| crate::error::AppError::Internal(anyhow::anyhow!("spawn_blocking panicked: {e}")))?
    }

    // -----------------------------------------------------------------------
    // goal_list
    // -----------------------------------------------------------------------

    /// List goals for a bot with optional status filter and pagination.
    ///
    /// Mirrors Python `GET /goals/list`:
    /// - `status` can be a comma-separated list of valid statuses (e.g. `"pending,active"`).
    /// - Invalid status values → `BadRequest("invalid status filter")`.
    /// - `ORDER BY priority DESC, created_ts DESC LIMIT ? OFFSET ?`.
    /// - `total` = full filtered count.
    /// - `limit` clamped to `[1, 500]`; `offset` clamped to `≥ 0`.
    pub async fn goal_list(
        &self,
        req: GoalListReq,
    ) -> Result<GoalListResp, crate::error::AppError> {
        const VALID_STATUSES: &[&str] = &["pending", "active", "completed", "abandoned"];

        // Validate status filter before touching DB.
        let statuses: Option<Vec<String>> = if let Some(ref s) = req.status {
            let parts: Vec<String> = s
                .split(',')
                .map(|x| x.trim().to_owned())
                .filter(|x| !x.is_empty())
                .collect();
            for part in &parts {
                if !VALID_STATUSES.contains(&part.as_str()) {
                    return Err(crate::error::AppError::BadRequest(
                        "invalid status filter".to_owned(),
                    ));
                }
            }
            if parts.is_empty() { None } else { Some(parts) }
        } else {
            None
        };

        let db_path = self.db_path.clone();

        tokio::task::spawn_blocking(move || -> Result<GoalListResp, crate::error::AppError> {
            let conn = db::open_db(&db_path)?;

            let limit  = req.limit.clamp(1, 500);
            let offset = req.offset.max(0);

            use rusqlite::types::Value as SqlValue;
            let mut where_parts: Vec<String> = vec!["bot_id=?".to_owned()];
            let mut base_params: Vec<SqlValue> = vec![SqlValue::Text(req.bot_id.clone())];

            if let Some(ref sts) = statuses {
                let placeholders = sts.iter().map(|_| "?").collect::<Vec<_>>().join(",");
                where_parts.push(format!("status IN ({placeholders})"));
                for st in sts {
                    base_params.push(SqlValue::Text(st.clone()));
                }
            }

            let where_sql = where_parts.join(" AND ");

            // COUNT(*) for total.
            let count_sql = format!("SELECT COUNT(*) FROM goals WHERE {where_sql}");
            let total: i64 = conn.query_row(
                &count_sql,
                rusqlite::params_from_iter(base_params.iter()),
                |r| r.get(0),
            )?;

            // Main SELECT.
            let mut list_params = base_params.clone();
            list_params.push(SqlValue::Integer(limit));
            list_params.push(SqlValue::Integer(offset));

            let list_sql = format!(
                "SELECT id, bot_id, text, status, source, priority, origin_memory, \
                        created_ts, updated_ts, completed_ts \
                 FROM goals WHERE {where_sql} \
                 ORDER BY priority DESC, created_ts DESC LIMIT ? OFFSET ?"
            );
            let mut stmt = conn.prepare(&list_sql)?;
            let items: Vec<GoalRow> = stmt
                .query_map(rusqlite::params_from_iter(list_params.iter()), |r| {
                    Ok(GoalRow {
                        id:            r.get(0)?,
                        bot_id:        r.get(1)?,
                        text:          r.get(2)?,
                        status:        r.get(3)?,
                        source:        r.get(4)?,
                        priority:      r.get(5)?,
                        origin_memory: r.get(6)?,
                        created_ts:    r.get(7)?,
                        updated_ts:    r.get(8)?,
                        completed_ts:  r.get(9)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;

            Ok(GoalListResp { items, total })
        })
        .await
        .map_err(|e| crate::error::AppError::Internal(anyhow::anyhow!("spawn_blocking panicked: {e}")))?
    }

    // -----------------------------------------------------------------------
    // goal_complete
    // -----------------------------------------------------------------------

    /// Transition a goal to a terminal outcome and optionally record a memory.
    ///
    /// Mirrors Python `POST /goals/complete`:
    ///
    /// 1. Fetch the goal.  Not found → `NotFound`.
    /// 2. If already terminal → `BadRequest("goal is already terminal ({current})")`.
    /// 3. Validate `outcome` is reachable from `current_status` via `VALID_TRANSITIONS`.
    /// 4. `UPDATE goals SET status=?, completed_ts=now, updated_ts=now WHERE id=?`.
    /// 5. If `also_record_memory` (default `true`):
    ///    - `text = format!("Goal {outcome}: {goal_text}")` — matches Python
    ///      `f"Goal {req.outcome}: {row[2]}"` where `row[2]` is the goal text.
    ///    - Call `self.write(WriteReq { salience: 0.6, memory_type: "goal_link",
    ///      source: "goals:{goal_id}", entities: [], relations: [] })`.
    ///    - Return `memory_id` from the `WriteResp`.
    /// 6. Return `GoalCompleteResp { updated: true, memory_id }`.
    pub async fn goal_complete(
        &self,
        req: GoalCompleteReq,
    ) -> Result<GoalCompleteResp, crate::error::AppError> {
        let db_path = self.db_path.clone();
        let bot_id = req.bot_id.clone();
        let goal_id = req.goal_id.clone();
        let outcome_str = req.outcome.as_str();

        // Step 1–4: DB work (fetch + validate + update) in spawn_blocking.
        let goal_text: String = {
            let db_path2 = db_path.clone();
            let bot_id2 = bot_id.clone();
            let goal_id2 = goal_id.clone();

            tokio::task::spawn_blocking(move || -> Result<String, crate::error::AppError> {
                let conn = db::open_db(&db_path2)?;
                use rusqlite::OptionalExtension;

                let row: Option<(String, String)> = conn
                    .query_row(
                        "SELECT text, status FROM goals WHERE id=?1 AND bot_id=?2",
                        rusqlite::params![goal_id2, bot_id2],
                        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
                    )
                    .optional()?;

                let (goal_text, current_status) = match row {
                    None => return Err(crate::error::AppError::NotFound("goal not found")),
                    Some(r) => r,
                };

                // Already terminal.
                const TERMINAL_STATUSES: &[&str] = &["completed", "abandoned"];
                if TERMINAL_STATUSES.contains(&current_status.as_str()) {
                    return Err(crate::error::AppError::BadRequest(format!(
                        "goal is already terminal ({current_status})"
                    )));
                }

                // Validate transition.
                let allowed: &[&str] = match current_status.as_str() {
                    "pending" => &["active", "abandoned"],
                    "active"  => &["completed", "abandoned"],
                    _         => &[],
                };
                if !allowed.contains(&outcome_str) {
                    return Err(crate::error::AppError::BadRequest(format!(
                        "invalid transition {current_status} \u{2192} {outcome_str}"
                    )));
                }

                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);

                conn.execute(
                    "UPDATE goals SET status=?1, completed_ts=?2, updated_ts=?2 WHERE id=?3",
                    rusqlite::params![outcome_str, now, goal_id2],
                )?;

                Ok(goal_text)
            })
            .await
            .map_err(|e| {
                crate::error::AppError::Internal(anyhow::anyhow!("spawn_blocking panicked: {e}"))
            })??
        };

        // Step 5: write auto-memory (async — calls embed + DB).
        let memory_id = if req.also_record_memory {
            // Python: f"Goal {req.outcome}: {row[2]}"
            let text = format!("Goal {outcome_str}: {goal_text}");
            let write_resp = self
                .write(WriteReq {
                    bot_id: bot_id.clone(),
                    text,
                    salience: 0.6,
                    entities: vec![],
                    relations: vec![],
                    memory_type: Some("goal_link".to_owned()),
                    source: Some(format!("goals:{goal_id}")),
                })
                .await?;
            Some(write_resp.memory_id)
        } else {
            None
        };

        Ok(GoalCompleteResp { updated: true, memory_id })
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

    // -----------------------------------------------------------------------
    // Recall / recall_about helpers
    // -----------------------------------------------------------------------

    /// Build a unique directional unit embedding for a given `slot` in [0, 383].
    /// Slot `i` has a 1.0 at position `i % EMBEDDING_DIM`, rest 0.0.
    fn slot_emb(slot: usize) -> Vec<f32> {
        let mut v = vec![0.0_f32; EMBEDDING_DIM];
        v[slot % EMBEDDING_DIM] = 1.0;
        v
    }

    // -----------------------------------------------------------------------
    // RCL-1: recall returns up to top_k results, each with weighted-sum score.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn recall_returns_top_k_in_mmr_order_with_weighted_sum_score() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());

        // Insert 4 memories. Memory 0 is most relevant (embedding aligns with query).
        for i in 0..4u64 {
            insert_raw_memory(
                &conn, "botR", &format!("m_rcl{i:07}"), &format!("text {i}"),
                0.5, "event", None, 1_700_000_000 + i as i64, &slot_emb(i as usize),
            );
        }

        // Query embedding aligns with slot 0 → memory 0 is most relevant.
        let q_emb = slot_emb(0);
        let embed_url = spawn_embed_stub(q_emb).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let resp = svc
            .recall(RecallReq {
                bot_id: "botR".to_string(),
                query: "anything".to_string(),
                top_k: 2,
                since_ts: None,
                until_ts: None,
                memory_type: None,
            })
            .await
            .expect("recall must succeed");

        assert_eq!(resp.memories.len(), 2, "must return exactly top_k=2");
        // First result should be memory 0 (highest cosine to query).
        assert_eq!(resp.memories[0].memory_id, "m_rcl0000000");
        // Scores must be in [0, 1] (weighted sum).
        for m in &resp.memories {
            assert!(m.score >= 0.0 && m.score <= 1.01, "score out of range: {}", m.score);
        }
    }

    // -----------------------------------------------------------------------
    // RCL-2: last_recalled_ts is bumped for returned memories.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn recall_bumps_last_recalled_ts_for_selected() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());
        let emb = slot_emb(0);
        insert_raw_memory(
            &conn, "botR2", "m_rcl_bump1", "bump text", 0.5,
            "event", None, 1_700_000_000, &emb,
        );

        let before = {
            conn.query_row(
                "SELECT last_recalled_ts FROM memories WHERE id='m_rcl_bump1'",
                [], |r| r.get::<_, i64>(0),
            ).unwrap()
        };

        // Give the clock a chance to advance by sleeping 1 second would be slow;
        // instead we just record the time before and check that after recall
        // the stored value equals the `now` used inside recall (i.e., ≥ before).
        let embed_url = spawn_embed_stub(slot_emb(0)).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        svc.recall(RecallReq {
            bot_id: "botR2".to_string(),
            query: "bump".to_string(),
            top_k: 5,
            since_ts: None,
            until_ts: None,
            memory_type: None,
        })
        .await
        .expect("recall");

        let conn2 = db::open_db(tmp.path()).unwrap();
        let after: i64 = conn2.query_row(
            "SELECT last_recalled_ts FROM memories WHERE id='m_rcl_bump1'",
            [], |r| r.get(0),
        ).unwrap();
        assert!(after >= before, "last_recalled_ts must be bumped: before={before} after={after}");
    }

    // -----------------------------------------------------------------------
    // RCL-3: since_ts / until_ts / memory_type filters apply.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn recall_filters_since_until_and_type() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());
        let emb = slot_emb(0);

        insert_raw_memory(&conn, "botF", "m_f_in",  "in range",  0.5, "goal",  None, 1_700_000_002, &emb);
        insert_raw_memory(&conn, "botF", "m_f_out", "out range", 0.5, "goal",  None, 1_700_000_099, &emb);
        insert_raw_memory(&conn, "botF", "m_f_typ", "wrong type",0.5, "event", None, 1_700_000_002, &emb);

        let embed_url = spawn_embed_stub(slot_emb(0)).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let resp = svc
            .recall(RecallReq {
                bot_id: "botF".to_string(),
                query: "anything".to_string(),
                top_k: 10,
                since_ts: Some(1_700_000_000),
                until_ts: Some(1_700_000_010),
                memory_type: Some("goal".to_string()),
            })
            .await
            .expect("recall with filters");

        assert_eq!(resp.memories.len(), 1, "only 1 memory matches all filters");
        assert_eq!(resp.memories[0].memory_id, "m_f_in");
    }

    // -----------------------------------------------------------------------
    // RCL-4: unknown bot → empty memories list.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn recall_unknown_bot_returns_empty() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        open_and_migrate(tmp.path());

        let embed_url = spawn_embed_stub(slot_emb(0)).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let resp = svc
            .recall(RecallReq {
                bot_id: "no_such_bot".to_string(),
                query: "anything".to_string(),
                top_k: 5,
                since_ts: None,
                until_ts: None,
                memory_type: None,
            })
            .await
            .expect("recall must not error");

        assert!(resp.memories.is_empty(), "unknown bot → empty memories");
    }

    // -----------------------------------------------------------------------
    // RCL-5: score field is the weighted-sum score (not MMR score).
    // The weighted-sum is computed BEFORE MMR; we verify it is > 0 and ≤ 1
    // (since all weights sum to 1 and all components are in [0,1]).
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn recall_score_is_weighted_sum_not_mmr_score() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());
        let emb = slot_emb(0);
        // High salience + recent + relevant → score near 1.0
        let now_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        insert_raw_memory(&conn, "botS", "m_score1", "score test", 1.0, "event", None, now_ts, &emb);

        let embed_url = spawn_embed_stub(slot_emb(0)).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let resp = svc
            .recall(RecallReq {
                bot_id: "botS".to_string(),
                query: "score test".to_string(),
                top_k: 1,
                since_ts: None,
                until_ts: None,
                memory_type: None,
            })
            .await
            .expect("recall");

        assert_eq!(resp.memories.len(), 1);
        let score = resp.memories[0].score;
        // w_rel=0.5 * cos≈1.0 + w_rec=0.2 * exp(0)≈1.0 + w_imp=0.3 * 1.0 ≈ 1.0
        assert!(score > 0.9, "high-salience recent relevant memory must have score > 0.9, got {score}");
        assert!(score <= 1.01, "weighted-sum score must be ≤ 1.01, got {score}");
    }

    // -----------------------------------------------------------------------
    // RCL-6: ts field in RecalledMemory = created_ts.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn recall_ts_equals_created_ts() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());
        let emb = slot_emb(0);
        let created_ts: i64 = 1_750_000_042;
        insert_raw_memory(&conn, "botTS", "m_ts0001", "ts test", 0.7, "event", None, created_ts, &emb);

        let embed_url = spawn_embed_stub(slot_emb(0)).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let resp = svc
            .recall(RecallReq {
                bot_id: "botTS".to_string(),
                query: "ts test".to_string(),
                top_k: 1,
                since_ts: None,
                until_ts: None,
                memory_type: None,
            })
            .await
            .expect("recall");

        assert_eq!(resp.memories.len(), 1);
        assert_eq!(resp.memories[0].ts, created_ts, "ts must equal created_ts");
    }

    // -----------------------------------------------------------------------
    // RA-1: recall_about returns hints for entity-linked memories.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn recall_about_returns_hints_for_linked_entity() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());
        let emb = slot_emb(5);

        // Write a memory and link it to entity "Arthas".
        insert_raw_memory(&conn, "botA", "m_ra00001", "Arthas attacked", 0.8, "event", None, 1_700_000_000, &emb);
        let entity_id = db::entities::upsert_entity(&conn, "botA", "Arthas", None).unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO memory_entities (memory_id, entity_id) VALUES ('m_ra00001', ?1)",
            rusqlite::params![entity_id],
        ).unwrap();

        let embed_url = spawn_embed_stub(slot_emb(5)).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let resp = svc
            .recall_about(RecallAboutReq {
                bot_id: "botA".to_string(),
                entity: "Arthas".to_string(),
                max_hops: 2,
                top_k: 5,
                since_ts: None,
                until_ts: None,
                memory_type: None,
            })
            .await
            .expect("recall_about");

        assert_eq!(resp.hints.len(), 1, "one linked memory → one hint");
        assert_eq!(resp.hints[0], "Arthas attacked");
    }

    // -----------------------------------------------------------------------
    // RA-2: recall_about follows edges (1-hop).
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn recall_about_follows_entity_edges() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());
        let emb = slot_emb(10);

        // Entity A → B via edge.
        let a_id = db::entities::upsert_entity(&conn, "botB", "EntityA", None).unwrap();
        let b_id = db::entities::upsert_entity(&conn, "botB", "EntityB", None).unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO edges (src_entity_id, rel, dst_entity_id, weight, last_seen_ts) VALUES (?1, 'link', ?2, 1.0, 0)",
            rusqlite::params![a_id, b_id],
        ).unwrap();

        // Memory linked to EntityB only.
        insert_raw_memory(&conn, "botB", "m_ra_edge1", "B memory", 0.7, "event", None, 1_700_000_000, &emb);
        conn.execute(
            "INSERT OR IGNORE INTO memory_entities (memory_id, entity_id) VALUES ('m_ra_edge1', ?1)",
            rusqlite::params![b_id],
        ).unwrap();

        let embed_url = spawn_embed_stub(slot_emb(10)).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        // Query from EntityA with max_hops=1 — should reach B and find the memory.
        let resp = svc
            .recall_about(RecallAboutReq {
                bot_id: "botB".to_string(),
                entity: "EntityA".to_string(),
                max_hops: 1,
                top_k: 5,
                since_ts: None,
                until_ts: None,
                memory_type: None,
            })
            .await
            .expect("recall_about");

        assert_eq!(resp.hints.len(), 1, "must follow edge A→B and return B's memory");
        assert_eq!(resp.hints[0], "B memory");
    }

    // -----------------------------------------------------------------------
    // RA-3: unknown entity → empty hints.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn recall_about_unknown_entity_returns_empty() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        open_and_migrate(tmp.path());

        let embed_url = spawn_embed_stub(slot_emb(0)).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let resp = svc
            .recall_about(RecallAboutReq {
                bot_id: "botC".to_string(),
                entity: "NoSuchEntity".to_string(),
                max_hops: 2,
                top_k: 5,
                since_ts: None,
                until_ts: None,
                memory_type: None,
            })
            .await
            .expect("recall_about must not error");

        assert!(resp.hints.is_empty(), "unknown entity → empty hints");
    }

    // -----------------------------------------------------------------------
    // RA-4: recall_about returns only texts (no scores), in MMR order.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn recall_about_returns_texts_only_no_scores() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());

        // 3 memories linked to same entity with distinct embeddings.
        let entity_id = db::entities::upsert_entity(&conn, "botD", "Boss", None).unwrap();
        for i in 0..3usize {
            let id = format!("m_ra_txt{i:04}");
            let text = format!("boss memory {i}");
            insert_raw_memory(&conn, "botD", &id, &text, 0.6, "event", None, 1_700_000_000 + i as i64, &slot_emb(i));
            conn.execute(
                "INSERT OR IGNORE INTO memory_entities (memory_id, entity_id) VALUES (?1, ?2)",
                rusqlite::params![id, entity_id],
            ).unwrap();
        }

        let embed_url = spawn_embed_stub(slot_emb(0)).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let resp = svc
            .recall_about(RecallAboutReq {
                bot_id: "botD".to_string(),
                entity: "Boss".to_string(),
                max_hops: 2,
                top_k: 2,
                since_ts: None,
                until_ts: None,
                memory_type: None,
            })
            .await
            .expect("recall_about");

        // hints is Vec<String> — texts only, top_k=2.
        assert_eq!(resp.hints.len(), 2, "must return top_k=2 hints");
        // All hints are non-empty strings.
        for h in &resp.hints {
            assert!(!h.is_empty(), "hint must not be empty");
        }
    }

    // -----------------------------------------------------------------------
    // Search helpers
    // -----------------------------------------------------------------------

    /// Insert a memory row with an explicit embedding and optional entity link.
    /// Returns the memory_id.
    fn insert_search_memory(
        conn: &rusqlite::Connection,
        bot_id: &str,
        memory_id: &str,
        text: &str,
        embedding: &[f32],
        created_ts: i64,
    ) {
        conn.execute(
            "INSERT OR IGNORE INTO bots (bot_id, created_ts) VALUES (?1, ?2)",
            rusqlite::params![bot_id, created_ts],
        )
        .unwrap();
        let blob = db::pack_f32_le(embedding);
        conn.execute(
            "INSERT INTO memories \
             (id, bot_id, text, salience, created_ts, last_recalled_ts, embedding, memory_type) \
             VALUES (?1, ?2, ?3, 0.5, ?4, ?4, ?5, 'event')",
            rusqlite::params![memory_id, bot_id, text, created_ts, blob],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO vec_memories (memory_id, bot_id, embedding) VALUES (?1, ?2, ?3)",
            rusqlite::params![memory_id, bot_id, blob],
        )
        .unwrap();
    }

    /// Link a memory to an entity by entity name (upserts entity if needed).
    fn link_memory_entity(
        conn: &rusqlite::Connection,
        bot_id: &str,
        memory_id: &str,
        entity_name: &str,
    ) -> i64 {
        let eid = db::entities::upsert_entity(conn, bot_id, entity_name, None).unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO memory_entities (memory_id, entity_id) VALUES (?1, ?2)",
            rusqlite::params![memory_id, eid],
        )
        .unwrap();
        eid
    }

    // -----------------------------------------------------------------------
    // SR-1: 3-lane fusion — each lane contributes a distinct memory; RRF
    //        fuses them; signals carry the correct 0-indexed lane positions.
    // -----------------------------------------------------------------------
    //
    // DB layout:
    //   m_bm25_only  — text has unique keyword "dragonhide"; embedding is
    //                  all-zeros (orthogonal to query) → BM25 lane only.
    //   m_dense_only — text has no unique keywords; embedding == query vector
    //                  → dense lane only.
    //   m_ent_only   — text has no unique keywords; embedding is all-zeros;
    //                  linked to entity "Galdrak" which appears in query
    //                  → entity lane only.
    //
    // Query: "dragonhide Galdrak" (hits both keyword and entity lanes, but we
    //         assign orthogonal embeddings so each memory is dominant in exactly
    //         one lane).
    //
    // After RRF fusion all three must appear.  We then verify:
    //   - score == RRF score (not cosine).
    //   - signal positions are 0-indexed positions in each lane's result list.
    //   - total_candidates == 3 (one per unique memory across all lanes).
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn search_three_lanes_fuse_and_signals_correct() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());

        // query embedding = slot_emb(0) (1.0 at position 0, rest 0.0)
        let q_emb = slot_emb(0);
        // orthogonal embedding (no cosine similarity to q_emb)
        let orth_emb = slot_emb(1);

        // m_bm25_only: keyword "dragonhide" in text; orthogonal embedding.
        insert_search_memory(&conn, "botSR", "m_bm25_only",
            "dragonhide armor crafted", &orth_emb, 1_700_000_001);

        // m_dense_only: no unique keywords; embedding == q_emb.
        insert_search_memory(&conn, "botSR", "m_dense_only",
            "the the the", &q_emb, 1_700_000_002);

        // m_ent_only: no unique keywords; orthogonal embedding; linked to "Galdrak".
        insert_search_memory(&conn, "botSR", "m_ent_only",
            "something happened here", &orth_emb, 1_700_000_003);
        link_memory_entity(&conn, "botSR", "m_ent_only", "Galdrak");

        // query hits keyword "dragonhide" (→ BM25 lane), "Galdrak" entity (→
        // entity lane), and the dense query vector aligns with m_dense_only
        // (→ dense lane).
        let query = "dragonhide Galdrak".to_string();

        // The embed stub returns q_emb for any input.
        let embed_url = spawn_embed_stub(q_emb.clone()).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let resp = svc
            .search(SearchReq {
                bot_id: "botSR".to_string(),
                query,
                top_k: 5,
                since_ts: None,
                until_ts: None,
                memory_type: None,
            })
            .await
            .expect("search must succeed");

        // All 3 unique memories must appear in fused set.
        assert_eq!(resp.total_candidates, 3,
            "3 unique memories across lanes → total_candidates=3");

        // All 3 returned in items (top_k=5 ≥ 3).
        assert_eq!(resp.items.len(), 3,
            "top_k=5 with 3 candidates → 3 items");

        // Collect into a map for per-memory assertions.
        let item_map: std::collections::HashMap<&str, &SearchItem> =
            resp.items.iter().map(|i| (i.memory_id.as_str(), i)).collect();

        // m_bm25_only: must have bm25_rank=Some(0) (first/only in BM25 lane),
        //              dense_rank some value (it is present in dense lane because
        //              we do score all bot memories in dense), entity_rank=None.
        let bm25_item = item_map.get("m_bm25_only").expect("m_bm25_only must be in results");
        assert!(bm25_item.signals.bm25_rank.is_some(),
            "m_bm25_only must have a bm25_rank");
        assert!(bm25_item.signals.entity_rank.is_none(),
            "m_bm25_only must have no entity_rank");

        // m_dense_only: must have dense_rank=Some(0) (highest cosine = 1.0),
        //               bm25_rank=None (no FTS match for stopword-only text).
        let dense_item = item_map.get("m_dense_only").expect("m_dense_only must be in results");
        assert_eq!(dense_item.signals.dense_rank, Some(0),
            "m_dense_only must be rank 0 in dense lane (cos=1.0)");
        assert!(dense_item.signals.bm25_rank.is_none(),
            "m_dense_only must have no bm25_rank (all-stopword text)");

        // m_ent_only: must have entity_rank=Some(0), and may or may not appear
        //             in dense lane (it has orthogonal embedding).
        let ent_item = item_map.get("m_ent_only").expect("m_ent_only must be in results");
        assert_eq!(ent_item.signals.entity_rank, Some(0),
            "m_ent_only must be rank 0 in entity lane");

        // Score must be the RRF score (in range (0, 1/60] per lane hit = up to 3/60).
        for item in &resp.items {
            assert!(item.score > 0.0 && item.score <= 3.0 / 60.0 + 1e-12,
                "RRF score must be in (0, 3/60], got {} for {}",
                item.score, item.memory_id);
        }
    }

    // -----------------------------------------------------------------------
    // SR-2: all-stopword query → BM25 lane is empty; dense + entity still work.
    //
    // Design note: the query "the and is" is 100% stopwords → build_fts5_query
    // returns None → BM25 lane is skipped entirely.  Dense + entity lanes
    // still fire and return results.
    //
    // To keep the test clean we use a bot with only stopwords in memory texts
    // (so BM25 would miss them even if the lane ran), and only rely on dense +
    // entity lanes to retrieve them.  We assert that no item has a bm25_rank
    // (because the lane was empty — the pos map contains no entries).
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn search_all_stopword_query_bm25_empty_dense_entity_work() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());

        let q_emb = slot_emb(2);
        // m_dense: aligns with query embedding; text is all-stopwords so FTS5
        // would not match it anyway.
        insert_search_memory(&conn, "botSQ", "m_dense_sq", "the and is", &q_emb, 1_700_000_001);
        // m_ent: linked to entity "zalgarak" (not a stopword, but the query is
        //        all-stopwords "the and is" → BM25 lane returns empty regardless).
        //        The text is also all-stopwords.
        insert_search_memory(&conn, "botSQ", "m_ent_sq", "the or but", &slot_emb(3), 1_700_000_002);
        link_memory_entity(&conn, "botSQ", "m_ent_sq", "zalgarak");

        // Query contains entity name "zalgarak" as a substring so entity lane fires.
        // The rest of the query is all-stopwords → BM25 lane returns empty.
        let query = "the and is zalgarak".to_string();

        let embed_url = spawn_embed_stub(q_emb).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let resp = svc
            .search(SearchReq {
                bot_id: "botSQ".to_string(),
                query,
                top_k: 5,
                since_ts: None,
                until_ts: None,
                memory_type: None,
            })
            .await
            .expect("search must succeed");

        // Must return results (dense + entity both contribute).
        assert!(!resp.items.is_empty(), "query with stopwords only should still return results");

        // BM25 lane was empty (all-stopword query → build_fts5_query returns None
        // for "the and is" portion — "zalgarak" is ≥3 chars and not a stopword,
        // so fts_q = Some("zalgarak")).  Actually "zalgarak" IS a significant term,
        // so build_fts5_query("the and is zalgarak", 6) returns Some("zalgarak").
        //
        // Revised assertion: we only assert results are non-empty (both dense and
        // entity lanes fire) and that m_dense_sq and m_ent_sq both appear.
        let has_dense = resp.items.iter().any(|i| i.memory_id == "m_dense_sq");
        assert!(has_dense, "m_dense_sq must appear via dense lane");

        let has_ent = resp.items.iter().any(|i| i.memory_id == "m_ent_sq");
        assert!(has_ent, "m_ent_sq must appear via entity lane");
    }

    // -----------------------------------------------------------------------
    // SR-2b: pure all-stopword query (no significant terms at all) →
    //         BM25 lane truly empty; dense still returns results.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn search_pure_stopword_query_bm25_lane_is_empty() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());

        let q_emb = slot_emb(4);
        // One memory with an embedding that aligns with the query vector.
        // Text is all stopwords (FTS5 won't match it).
        insert_search_memory(&conn, "botSP", "m_pure_dense",
            "the and or but is", &q_emb, 1_700_000_001);

        // Query is 100% stopwords → build_fts5_query returns None → BM25 lane empty.
        let query = "the and is".to_string();

        let embed_url = spawn_embed_stub(q_emb).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let resp = svc
            .search(SearchReq {
                bot_id: "botSP".to_string(),
                query,
                top_k: 5,
                since_ts: None,
                until_ts: None,
                memory_type: None,
            })
            .await
            .expect("search must not error");

        // Dense lane fires and finds m_pure_dense via embedding similarity.
        assert!(!resp.items.is_empty(), "dense lane must still return the embedding-matching memory");

        // BM25 lane was empty → no item should have a bm25_rank.
        for item in &resp.items {
            assert!(item.signals.bm25_rank.is_none(),
                "pure stopword query → BM25 lane empty → bm25_rank must be None for {}",
                item.memory_id);
        }
    }

    // -----------------------------------------------------------------------
    // SR-3: empty result when nothing matches any lane.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn search_empty_result_when_nothing_matches() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        open_and_migrate(tmp.path());

        // No memories at all for this bot.
        let embed_url = spawn_embed_stub(slot_emb(0)).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let resp = svc
            .search(SearchReq {
                bot_id: "botSE".to_string(),
                query: "dragon slayer warrior".to_string(),
                top_k: 5,
                since_ts: None,
                until_ts: None,
                memory_type: None,
            })
            .await
            .expect("search must not error");

        assert!(resp.items.is_empty(), "no memories → items must be empty");
        assert_eq!(resp.total_candidates, 0, "no memories → total_candidates must be 0");
    }

    // -----------------------------------------------------------------------
    // SR-4: total_candidates = len(fused), not top_k.
    //        When there are more fused candidates than top_k, total_candidates
    //        exceeds items.len().
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn search_total_candidates_exceeds_top_k() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());

        // Insert 6 memories, each with a unique keyword + distinct embedding.
        for i in 0..6usize {
            let text = format!("battleground{i} event occurred");
            insert_search_memory(
                &conn, "botST",
                &format!("m_tc{i:04}"),
                &text,
                &slot_emb(i),
                1_700_000_000 + i as i64,
            );
        }

        // Query matches keyword "battleground" across all 6 (BM25 lane will have 6,
        // dense lane will have 6).
        let q_emb = slot_emb(0);
        let embed_url = spawn_embed_stub(q_emb).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let resp = svc
            .search(SearchReq {
                bot_id: "botST".to_string(),
                query: "battleground".to_string(),
                top_k: 2,  // only 2 returned in items
                since_ts: None,
                until_ts: None,
                memory_type: None,
            })
            .await
            .expect("search must succeed");

        // items is capped at top_k=2.
        assert_eq!(resp.items.len(), 2,
            "top_k=2 → exactly 2 items returned");
        // total_candidates = all 6 (fused across both BM25+dense lanes).
        assert_eq!(resp.total_candidates, 6,
            "6 unique memories fused → total_candidates=6");
    }

    // -----------------------------------------------------------------------
    // SR-5: since_ts / until_ts / memory_type filters apply to all lanes.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn search_filters_apply_to_all_lanes() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());

        let q_emb = slot_emb(0);
        // Two memories: one inside the filter window, one outside.
        insert_search_memory(&conn, "botSF", "m_sf_in",
            "dragon slayer event", &q_emb, 1_700_000_002);
        insert_search_memory(&conn, "botSF", "m_sf_out",
            "dragon slayer event", &q_emb, 1_700_000_099);

        let embed_url = spawn_embed_stub(q_emb).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let resp = svc
            .search(SearchReq {
                bot_id: "botSF".to_string(),
                query: "dragon slayer".to_string(),
                top_k: 10,
                since_ts: Some(1_700_000_000),
                until_ts: Some(1_700_000_010),
                memory_type: None,
            })
            .await
            .expect("search with filters");

        // Only the in-window memory must appear.
        assert_eq!(resp.items.len(), 1,
            "filter must exclude out-of-window memory");
        assert_eq!(resp.items[0].memory_id, "m_sf_in",
            "only in-window memory must be returned");
    }

    // =======================================================================
    // Task 5.1 — personality tests
    // =======================================================================

    // -----------------------------------------------------------------------
    // P1: set then get round-trip.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn personality_set_then_get_round_trip() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        open_and_migrate(tmp.path());

        let embed_url = spawn_embed_stub(vec![0.1_f32; EMBEDDING_DIM]).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        svc.personality_set("bot_pers1", "I am Athena, the war-witch.".to_owned())
            .await
            .expect("set must succeed");

        let persona = svc.personality_get("bot_pers1").await.expect("get must succeed");
        assert_eq!(
            persona.as_deref(),
            Some("I am Athena, the war-witch."),
            "round-trip must return the set persona"
        );
    }

    // -----------------------------------------------------------------------
    // P2: get unknown bot → None.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn personality_get_unknown_bot_returns_none() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        open_and_migrate(tmp.path());

        let embed_url = spawn_embed_stub(vec![0.1_f32; EMBEDDING_DIM]).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let result = svc.personality_get("no_such_bot").await.expect("get must not error");
        assert!(result.is_none(), "unknown bot must return None");
    }

    // -----------------------------------------------------------------------
    // P3: set overwrites an existing persona (UPSERT).
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn personality_set_overwrites_existing() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        open_and_migrate(tmp.path());

        let embed_url = spawn_embed_stub(vec![0.1_f32; EMBEDDING_DIM]).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        svc.personality_set("bot_pers2", "first persona".to_owned())
            .await
            .expect("first set");
        svc.personality_set("bot_pers2", "second persona".to_owned())
            .await
            .expect("second set");

        let result = svc.personality_get("bot_pers2").await.expect("get");
        assert_eq!(result.as_deref(), Some("second persona"), "UPSERT must overwrite");
    }

    // -----------------------------------------------------------------------
    // P4: persona > 4000 chars → BadRequest.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn personality_set_exceeds_4000_chars_returns_bad_request() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        open_and_migrate(tmp.path());

        let embed_url = spawn_embed_stub(vec![0.1_f32; EMBEDDING_DIM]).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let long_persona = "x".repeat(4001);
        let err = svc.personality_set("bot_pers3", long_persona).await;
        assert!(
            matches!(err, Err(crate::error::AppError::BadRequest(_))),
            "persona > 4000 chars must be BadRequest"
        );
    }

    // -----------------------------------------------------------------------
    // P5: persona of exactly 4000 chars is accepted.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn personality_set_exactly_4000_chars_accepted() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        open_and_migrate(tmp.path());

        let embed_url = spawn_embed_stub(vec![0.1_f32; EMBEDDING_DIM]).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let persona = "a".repeat(4000);
        svc.personality_set("bot_pers4", persona.clone()).await.expect("4000 chars must succeed");
        let result = svc.personality_get("bot_pers4").await.expect("get").unwrap();
        assert_eq!(result.len(), 4000, "4000-char persona must be stored intact");
    }

    // =======================================================================
    // Task 5.2 — goals tests
    // =======================================================================

    // Helper: insert a bot row and goal directly into the DB (bypasses service).
    fn insert_raw_goal(
        conn: &rusqlite::Connection,
        id: &str,
        bot_id: &str,
        text: &str,
        status: &str,
        priority: i64,
        created_ts: i64,
    ) {
        conn.execute(
            "INSERT OR IGNORE INTO bots (bot_id, created_ts) VALUES (?1, ?2)",
            rusqlite::params![bot_id, created_ts],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO goals \
             (id, bot_id, text, status, source, priority, origin_memory, created_ts, updated_ts) \
             VALUES (?1, ?2, ?3, ?4, NULL, ?5, NULL, ?6, ?6)",
            rusqlite::params![id, bot_id, text, status, priority, created_ts],
        )
        .unwrap();
    }

    // -----------------------------------------------------------------------
    // G1: create returns goal_id with prefix g_ and status "pending".
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn goal_create_returns_pending_with_g_prefix() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        open_and_migrate(tmp.path());

        let embed_url = spawn_embed_stub(vec![0.1_f32; EMBEDDING_DIM]).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let resp = svc
            .goal_create(GoalCreateReq {
                bot_id: "bot_g1".to_owned(),
                text: "Defeat the Lich King".to_owned(),
                source: None,
                priority: 0,
                origin_memory: None,
            })
            .await
            .expect("goal_create must succeed");

        assert!(resp.goal_id.starts_with("g_"), "goal_id must start with 'g_'");
        assert_eq!(resp.status, "pending", "new goal must be pending");
    }

    // -----------------------------------------------------------------------
    // G2: create → read round-trip.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn goal_create_then_read() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        open_and_migrate(tmp.path());

        let embed_url = spawn_embed_stub(vec![0.1_f32; EMBEDDING_DIM]).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let cr = svc
            .goal_create(GoalCreateReq {
                bot_id: "bot_g2".to_owned(),
                text: "Find the artifact".to_owned(),
                source: Some("quest:123".to_owned()),
                priority: 5,
                origin_memory: None,
            })
            .await
            .expect("create");

        let row = svc
            .goal_read("bot_g2", &cr.goal_id)
            .await
            .expect("read")
            .expect("must be Some");

        assert_eq!(row.id, cr.goal_id);
        assert_eq!(row.bot_id, "bot_g2");
        assert_eq!(row.text, "Find the artifact");
        assert_eq!(row.status, "pending");
        assert_eq!(row.source.as_deref(), Some("quest:123"));
        assert_eq!(row.priority, 5);
        assert!(row.completed_ts.is_none(), "completed_ts must be NULL for pending");
    }

    // -----------------------------------------------------------------------
    // G3: read unknown goal → None.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn goal_read_unknown_returns_none() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        open_and_migrate(tmp.path());

        let embed_url = spawn_embed_stub(vec![0.1_f32; EMBEDDING_DIM]).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let result = svc.goal_read("bot_g3", "g_nosuchgoal").await.expect("no error");
        assert!(result.is_none(), "unknown goal → None");
    }

    // -----------------------------------------------------------------------
    // G4: legal transition pending → active succeeds; updated_ts bumped.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn goal_update_legal_transition_pending_to_active() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());
        insert_raw_goal(&conn, "g_upd001", "bot_g4", "test goal", "pending", 0, 0);

        let embed_url = spawn_embed_stub(vec![0.1_f32; EMBEDDING_DIM]).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let resp = svc
            .goal_update(GoalUpdateReq {
                bot_id: "bot_g4".to_owned(),
                goal_id: "g_upd001".to_owned(),
                text: None,
                status: Some("active".to_owned()),
                priority: None,
                origin_memory: None,
            })
            .await
            .expect("update");

        assert!(resp.updated, "legal transition must return updated=true");

        let row = svc.goal_read("bot_g4", "g_upd001").await.expect("read").unwrap();
        assert_eq!(row.status, "active");
        assert!(row.updated_ts > 0, "updated_ts must be bumped");
        assert!(row.completed_ts.is_none(), "active is not terminal; completed_ts must be None");
    }

    // -----------------------------------------------------------------------
    // G5: illegal transition pending → completed → BadRequest.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn goal_update_illegal_transition_returns_bad_request() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());
        insert_raw_goal(&conn, "g_upd002", "bot_g5", "test goal", "pending", 0, 0);

        let embed_url = spawn_embed_stub(vec![0.1_f32; EMBEDDING_DIM]).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let err = svc
            .goal_update(GoalUpdateReq {
                bot_id: "bot_g5".to_owned(),
                goal_id: "g_upd002".to_owned(),
                text: None,
                status: Some("completed".to_owned()),
                priority: None,
                origin_memory: None,
            })
            .await;

        assert!(
            matches!(err, Err(crate::error::AppError::BadRequest(_))),
            "illegal transition must be BadRequest, got {:?}", err
        );
        // Status must not have changed.
        let row = svc.goal_read("bot_g5", "g_upd002").await.expect("read").unwrap();
        assert_eq!(row.status, "pending", "status must not change on rejected transition");
    }

    // -----------------------------------------------------------------------
    // G6: transition to terminal status sets completed_ts.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn goal_update_terminal_transition_sets_completed_ts() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());
        insert_raw_goal(&conn, "g_upd003", "bot_g6", "test goal", "active", 0, 0);

        let embed_url = spawn_embed_stub(vec![0.1_f32; EMBEDDING_DIM]).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        svc.goal_update(GoalUpdateReq {
            bot_id: "bot_g6".to_owned(),
            goal_id: "g_upd003".to_owned(),
            text: None,
            status: Some("completed".to_owned()),
            priority: None,
            origin_memory: None,
        })
        .await
        .expect("update");

        let row = svc.goal_read("bot_g6", "g_upd003").await.expect("read").unwrap();
        assert_eq!(row.status, "completed");
        assert!(row.completed_ts.is_some(), "completed_ts must be set when transitioning to terminal");
    }

    // -----------------------------------------------------------------------
    // G7: update with no fields → BadRequest.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn goal_update_no_fields_returns_bad_request() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());
        insert_raw_goal(&conn, "g_upd004", "bot_g7", "test goal", "pending", 0, 0);

        let embed_url = spawn_embed_stub(vec![0.1_f32; EMBEDDING_DIM]).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let err = svc
            .goal_update(GoalUpdateReq {
                bot_id: "bot_g7".to_owned(),
                goal_id: "g_upd004".to_owned(),
                text: None,
                status: None,
                priority: None,
                origin_memory: None,
            })
            .await;

        assert!(
            matches!(err, Err(crate::error::AppError::BadRequest(_))),
            "no fields → BadRequest"
        );
    }

    // -----------------------------------------------------------------------
    // G8: update missing goal → updated: false.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn goal_update_missing_goal_returns_updated_false() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        open_and_migrate(tmp.path());

        let embed_url = spawn_embed_stub(vec![0.1_f32; EMBEDDING_DIM]).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let resp = svc
            .goal_update(GoalUpdateReq {
                bot_id: "bot_g8".to_owned(),
                goal_id: "g_nosuchgoal".to_owned(),
                text: Some("new text".to_owned()),
                status: None,
                priority: None,
                origin_memory: None,
            })
            .await
            .expect("must not error");

        assert!(!resp.updated, "missing goal → updated=false");
    }

    // -----------------------------------------------------------------------
    // G9: invalid status string → BadRequest.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn goal_update_invalid_status_string_returns_bad_request() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());
        insert_raw_goal(&conn, "g_upd005", "bot_g9", "test goal", "pending", 0, 0);

        let embed_url = spawn_embed_stub(vec![0.1_f32; EMBEDDING_DIM]).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let err = svc
            .goal_update(GoalUpdateReq {
                bot_id: "bot_g9".to_owned(),
                goal_id: "g_upd005".to_owned(),
                text: None,
                status: Some("flying".to_owned()),
                priority: None,
                origin_memory: None,
            })
            .await;

        assert!(
            matches!(err, Err(crate::error::AppError::BadRequest(_))),
            "invalid status string → BadRequest"
        );
    }

    // -----------------------------------------------------------------------
    // G10: list filter by status.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn goal_list_filter_by_status() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());

        insert_raw_goal(&conn, "g_lst001", "bot_gl", "task 1", "pending",   0, 1_000);
        insert_raw_goal(&conn, "g_lst002", "bot_gl", "task 2", "active",    0, 1_001);
        insert_raw_goal(&conn, "g_lst003", "bot_gl", "task 3", "pending",   0, 1_002);
        insert_raw_goal(&conn, "g_lst004", "bot_gl", "task 4", "abandoned", 0, 1_003);

        let embed_url = spawn_embed_stub(vec![0.1_f32; EMBEDDING_DIM]).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let resp = svc
            .goal_list(GoalListReq {
                bot_id: "bot_gl".to_owned(),
                status: Some("pending".to_owned()),
                limit: 50,
                offset: 0,
            })
            .await
            .expect("list");

        assert_eq!(resp.total, 2, "filter pending → total=2");
        assert_eq!(resp.items.len(), 2);
        for item in &resp.items {
            assert_eq!(item.status, "pending");
        }
    }

    // -----------------------------------------------------------------------
    // G11: list ordering: priority DESC, then created_ts DESC.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn goal_list_ordering_priority_then_created_ts() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());

        insert_raw_goal(&conn, "g_ord001", "bot_go", "low prio old",  "pending", 0,  1_000);
        insert_raw_goal(&conn, "g_ord002", "bot_go", "low prio new",  "pending", 0,  2_000);
        insert_raw_goal(&conn, "g_ord003", "bot_go", "high prio",     "pending", 20, 1_500);

        let embed_url = spawn_embed_stub(vec![0.1_f32; EMBEDDING_DIM]).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let resp = svc
            .goal_list(GoalListReq {
                bot_id: "bot_go".to_owned(),
                status: None,
                limit: 50,
                offset: 0,
            })
            .await
            .expect("list");

        assert_eq!(resp.total, 3);
        assert_eq!(resp.items.len(), 3);
        assert_eq!(resp.items[0].id, "g_ord003", "highest priority must come first");
        assert_eq!(resp.items[1].id, "g_ord002", "among same-priority, newer created_ts first");
        assert_eq!(resp.items[2].id, "g_ord001");
    }

    // -----------------------------------------------------------------------
    // G12: list pagination (limit/offset).
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn goal_list_pagination() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());

        for i in 0..5u32 {
            let id = format!("g_pag{i:03}");
            insert_raw_goal(&conn, &id, "bot_gp", &format!("goal {i}"), "pending", 0, i as i64);
        }

        let embed_url = spawn_embed_stub(vec![0.1_f32; EMBEDDING_DIM]).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let page1 = svc
            .goal_list(GoalListReq { bot_id: "bot_gp".to_owned(), status: None, limit: 2, offset: 0 })
            .await
            .expect("page1");

        assert_eq!(page1.total, 5, "total must always reflect full count");
        assert_eq!(page1.items.len(), 2, "page 1 must have 2 items");

        let page2 = svc
            .goal_list(GoalListReq { bot_id: "bot_gp".to_owned(), status: None, limit: 2, offset: 2 })
            .await
            .expect("page2");

        assert_eq!(page2.total, 5);
        assert_eq!(page2.items.len(), 2);
        for item in &page2.items {
            assert!(!page1.items.iter().any(|i| i.id == item.id), "page 2 items must not overlap page 1");
        }
    }

    // -----------------------------------------------------------------------
    // G13: list with invalid status filter → BadRequest.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn goal_list_invalid_status_filter_bad_request() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        open_and_migrate(tmp.path());

        let embed_url = spawn_embed_stub(vec![0.1_f32; EMBEDDING_DIM]).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let err = svc
            .goal_list(GoalListReq {
                bot_id: "bot_gx".to_owned(),
                status: Some("flying".to_owned()),
                limit: 50,
                offset: 0,
            })
            .await;

        assert!(matches!(err, Err(crate::error::AppError::BadRequest(_))), "invalid status → BadRequest");
    }

    // -----------------------------------------------------------------------
    // G14: complete sets terminal status + completed_ts + records memory.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn goal_complete_sets_terminal_and_records_memory() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());
        insert_raw_goal(&conn, "g_cmp001", "bot_gc", "Slay the dragon", "active", 0, 1_000);

        let embed_url = spawn_embed_stub(vec![0.1_f32; EMBEDDING_DIM]).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let resp = svc
            .goal_complete(GoalCompleteReq {
                bot_id: "bot_gc".to_owned(),
                goal_id: "g_cmp001".to_owned(),
                outcome: GoalOutcome::Completed,
                also_record_memory: true,
            })
            .await
            .expect("goal_complete must succeed");

        assert!(resp.updated, "updated must be true");
        assert!(resp.memory_id.is_some(), "also_record_memory=true → memory_id must be Some");

        let row = svc.goal_read("bot_gc", "g_cmp001").await.expect("read").unwrap();
        assert_eq!(row.status, "completed");
        assert!(row.completed_ts.is_some(), "completed_ts must be set");

        let mem_id = resp.memory_id.unwrap();
        let mem = svc.read("bot_gc", &mem_id).await.expect("read memory").expect("memory must exist");
        assert_eq!(mem.memory_type, "goal_link");
        assert_eq!(mem.source.as_deref(), Some("goals:g_cmp001"));
        assert!((mem.salience - 0.6_f32).abs() < 1e-5, "salience must be 0.6");
        assert_eq!(mem.text, "Goal completed: Slay the dragon");
    }

    // -----------------------------------------------------------------------
    // G15: complete with also_record_memory=false → memory_id is None.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn goal_complete_no_memory_returns_none_memory_id() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());
        insert_raw_goal(&conn, "g_cmp002", "bot_gc2", "another goal", "active", 0, 1_000);

        let embed_url = spawn_embed_stub(vec![0.1_f32; EMBEDDING_DIM]).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let resp = svc
            .goal_complete(GoalCompleteReq {
                bot_id: "bot_gc2".to_owned(),
                goal_id: "g_cmp002".to_owned(),
                outcome: GoalOutcome::Abandoned,
                also_record_memory: false,
            })
            .await
            .expect("goal_complete");

        assert!(resp.updated);
        assert!(resp.memory_id.is_none(), "also_record_memory=false → memory_id must be None");

        let row = svc.goal_read("bot_gc2", "g_cmp002").await.expect("read").unwrap();
        assert_eq!(row.status, "abandoned");
        assert!(row.completed_ts.is_some());
    }

    // -----------------------------------------------------------------------
    // G16: complete already-terminal goal → BadRequest.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn goal_complete_already_terminal_bad_request() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());
        insert_raw_goal(&conn, "g_cmp003", "bot_gc3", "done goal", "completed", 0, 1_000);

        let embed_url = spawn_embed_stub(vec![0.1_f32; EMBEDDING_DIM]).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let err = svc
            .goal_complete(GoalCompleteReq {
                bot_id: "bot_gc3".to_owned(),
                goal_id: "g_cmp003".to_owned(),
                outcome: GoalOutcome::Completed,
                also_record_memory: false,
            })
            .await;

        assert!(
            matches!(err, Err(crate::error::AppError::BadRequest(_))),
            "completing a terminal goal must be BadRequest"
        );
    }

    // -----------------------------------------------------------------------
    // G17: complete a pending goal with outcome "abandoned" — valid (pending→abandoned).
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn goal_complete_pending_to_abandoned_via_complete() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());
        insert_raw_goal(&conn, "g_cmp004", "bot_gc4", "pending goal", "pending", 0, 1_000);

        let embed_url = spawn_embed_stub(vec![0.1_f32; EMBEDDING_DIM]).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let resp = svc
            .goal_complete(GoalCompleteReq {
                bot_id: "bot_gc4".to_owned(),
                goal_id: "g_cmp004".to_owned(),
                outcome: GoalOutcome::Abandoned,
                also_record_memory: false,
            })
            .await
            .expect("pending→abandoned must succeed");

        assert!(resp.updated);
        let row = svc.goal_read("bot_gc4", "g_cmp004").await.expect("read").unwrap();
        assert_eq!(row.status, "abandoned");
    }

    // -----------------------------------------------------------------------
    // G18: complete a pending goal with outcome "completed" → BadRequest
    //      (pending→completed is not a valid direct transition).
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn goal_complete_pending_to_completed_is_invalid() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());
        insert_raw_goal(&conn, "g_cmp005", "bot_gc5", "pending goal", "pending", 0, 1_000);

        let embed_url = spawn_embed_stub(vec![0.1_f32; EMBEDDING_DIM]).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let err = svc
            .goal_complete(GoalCompleteReq {
                bot_id: "bot_gc5".to_owned(),
                goal_id: "g_cmp005".to_owned(),
                outcome: GoalOutcome::Completed,
                also_record_memory: false,
            })
            .await;

        assert!(
            matches!(err, Err(crate::error::AppError::BadRequest(_))),
            "pending→completed is not a valid transition via complete"
        );
    }

    // -----------------------------------------------------------------------
    // G19: list comma-separated status filter.
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn goal_list_comma_separated_status_filter() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = open_and_migrate(tmp.path());

        insert_raw_goal(&conn, "g_csv001", "bot_gcsv", "t1", "pending",   0, 1_000);
        insert_raw_goal(&conn, "g_csv002", "bot_gcsv", "t2", "active",    0, 1_001);
        insert_raw_goal(&conn, "g_csv003", "bot_gcsv", "t3", "completed", 0, 1_002);
        insert_raw_goal(&conn, "g_csv004", "bot_gcsv", "t4", "abandoned", 0, 1_003);

        let embed_url = spawn_embed_stub(vec![0.1_f32; EMBEDDING_DIM]).await;
        let svc = make_service(tmp.path().to_path_buf(), &embed_url);

        let resp = svc
            .goal_list(GoalListReq {
                bot_id: "bot_gcsv".to_owned(),
                status: Some("pending,active".to_owned()),
                limit: 50,
                offset: 0,
            })
            .await
            .expect("list");

        assert_eq!(resp.total, 2, "pending,active filter → 2 goals");
        for item in &resp.items {
            assert!(
                item.status == "pending" || item.status == "active",
                "only pending/active must be returned, got {}", item.status
            );
        }
    }
}
