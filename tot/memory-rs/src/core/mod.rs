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
use crate::pubsub::{MemoryRow, PubSub};
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
        let row = MemoryRow {
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
}
