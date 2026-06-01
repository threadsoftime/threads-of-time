// src/routes/write.rs
//! POST /v1/memory/:bot_guid/episodes
//!
//! Behavior per spec §5.2:
//!  1. Embed content_text on the async path (graceful degradation — any failure
//!     writes the episode with content_embedding_id = NULL; episode still FTS-searchable).
//!  2. Validate + coerce inputs in-handler (returning AppError::BadRequest) so
//!     the SQLite CHECK constraints are never the first line of defence.
//!  3. All DB work in spawn_blocking: open_bot_db -> run_migrations -> INSERT
//!     episode -> (if vec) INSERT embeddings_vec + UPDATE content_embedding_id
//!     -> upsert entities -> commit. Single transaction.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::db::{migrate::run_migrations, open_bot_db};
use crate::embeddings::EmbeddingsClient;
use crate::error::AppError;
use crate::state::AppState;

/// Roles accepted by the episode_entities CHECK constraint.
const ALLOWED_ROLES: &[&str] = &["subject", "participant", "target", "witness"];

#[derive(Debug, Deserialize)]
pub struct EntityRef {
    pub entity_kind: String,
    pub entity_key: String,
    pub display_name: String,
    pub role: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WriteEpisodeRequest {
    pub content_text: String,
    pub episode_type: String,
    pub timestamp: i64,
    pub salience_hint: Option<f64>,
    #[serde(default)]
    pub entities: Vec<EntityRef>,
    pub metadata: Option<JsonValue>,
    pub source: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct WriteEpisodeResponse {
    pub episode_id: i64,
    pub embedding_generated: bool,
    pub salience_score: f64,
}

pub async fn write_episode(
    State(st): State<AppState>,
    Path(bot_guid): Path<String>,
    Json(body): Json<WriteEpisodeRequest>,
) -> Result<(StatusCode, Json<WriteEpisodeResponse>), AppError> {
    // --- Validate inputs in-handler (spec §5.2 acceptable divergence from 422) ---
    if body.content_text.is_empty() || body.content_text.chars().count() > 4000 {
        return Err(AppError::BadRequest(
            "content_text must be 1–4000 characters".to_string(),
        ));
    }
    if body.timestamp <= 0 {
        return Err(AppError::BadRequest(
            "timestamp must be > 0".to_string(),
        ));
    }
    if let Some(sh) = body.salience_hint {
        if !(0.0..=1.0).contains(&sh) {
            return Err(AppError::BadRequest(
                "salience_hint must be 0.0–1.0".to_string(),
            ));
        }
    }

    // --- Step 1: embed on async path; graceful degradation on any error ---
    let client = EmbeddingsClient::new(&st.embed.url, &st.embed.model, &st.embed.api_key);
    let vec_result = client.embed(&body.content_text).await;
    let (vec_bytes, embedding_generated) = match vec_result {
        Ok(v) => {
            // Pack as little-endian f32 (matches Python struct.pack("{n}f")).
            let bytes: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
            (Some(bytes), true)
        }
        Err(e) => {
            tracing::warn!(
                bot_guid = %bot_guid,
                error = %e,
                "embed failed — writing episode with content_embedding_id=NULL"
            );
            (None, false)
        }
    };

    // --- Steps 2-5: all DB work in spawn_blocking ---
    let data_dir = st.data_dir.clone();

    // Capture fields needed inside the blocking closure (must be Send + 'static).
    let content_text = body.content_text.clone();
    let episode_type = body.episode_type.clone();
    let timestamp = body.timestamp;
    let salience_score = body.salience_hint.unwrap_or(0.5);
    let source = body.source.clone().unwrap_or_else(|| "self".to_string());
    let metadata_json = serialize_metadata(body.metadata.as_ref());
    let entities: Vec<(String, String, String, String)> = body
        .entities
        .into_iter()
        .map(|e| {
            let role = coerce_role(e.role);
            (e.entity_kind, e.entity_key, e.display_name, role)
        })
        .collect();

    let result = tokio::task::spawn_blocking(move || -> Result<(i64, f64), AppError> {
        let conn = open_bot_db(&data_dir, &bot_guid)?;
        run_migrations(&conn)?;

        // Begin explicit transaction for atomicity.
        conn.execute_batch("BEGIN")?;

        // INSERT episode row.
        let episode_id: i64 = conn
            .query_row(
                "INSERT INTO episodes \
                 (timestamp, content_text, episode_type, salience_score, source, metadata) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
                 RETURNING episode_id",
                params![
                    timestamp,
                    content_text,
                    episode_type,
                    salience_score,
                    source,
                    metadata_json,
                ],
                |row| row.get(0),
            )
            .map_err(AppError::Db)?;

        // INSERT vec row and update content_embedding_id if embedding succeeded.
        if let Some(ref bytes) = vec_bytes {
            conn.execute(
                "INSERT INTO embeddings_vec (rowid, embedding) VALUES (?1, ?2)",
                params![episode_id, bytes],
            )
            .map_err(AppError::Db)?;
            conn.execute(
                "UPDATE episodes SET content_embedding_id = ?1 WHERE episode_id = ?2",
                params![episode_id, episode_id],
            )
            .map_err(AppError::Db)?;
        }

        // Upsert entities + link via episode_entities.
        for (kind, key, display, role) in &entities {
            let maybe_id: Option<i64> = conn
                .query_row(
                    "SELECT entity_id FROM entities WHERE entity_kind = ?1 AND entity_key = ?2",
                    params![kind, key],
                    |row| row.get(0),
                )
                .optional()
                .map_err(AppError::Db)?;

            let entity_id = if let Some(eid) = maybe_id {
                conn.execute(
                    "UPDATE entities SET last_seen_at = ?1, display_name = ?2 WHERE entity_id = ?3",
                    params![timestamp, display, eid],
                )
                .map_err(AppError::Db)?;
                eid
            } else {
                conn.query_row(
                    "INSERT INTO entities (entity_kind, entity_key, display_name, last_seen_at) \
                     VALUES (?1, ?2, ?3, ?4) RETURNING entity_id",
                    params![kind, key, display, timestamp],
                    |row| row.get(0),
                )
                .map_err(AppError::Db)?
            };

            conn.execute(
                "INSERT OR IGNORE INTO episode_entities (episode_id, entity_id, role) \
                 VALUES (?1, ?2, ?3)",
                params![episode_id, entity_id, role],
            )
            .map_err(AppError::Db)?;
        }

        conn.execute_batch("COMMIT")?;
        Ok((episode_id, salience_score))
    })
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("spawn_blocking join error: {e}")))?;

    let (episode_id, salience_score) = result?;

    Ok((
        StatusCode::CREATED,
        Json(WriteEpisodeResponse {
            episode_id,
            embedding_generated,
            salience_score,
        }),
    ))
}

/// Compact JSON serialization matching Python's `json.dumps(separators=(",", ":"))`.
fn serialize_metadata(metadata: Option<&JsonValue>) -> Option<String> {
    metadata.map(|v| {
        // serde_json compact output uses no whitespace — equivalent to Python's
        // separators=(",",":")
        v.to_string()
    })
}

/// Coerce an entity role to a valid value; default to "participant".
fn coerce_role(role: Option<String>) -> String {
    match role {
        Some(r) if ALLOWED_ROLES.contains(&r.as_str()) => r,
        _ => "participant".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use serde_json::json;
    use tempfile::TempDir;
    use tower::ServiceExt;
    use wiremock::{
        matchers::{method, path},
        Mock, MockServer, ResponseTemplate,
    };

    use crate::app::build_router;
    use crate::state::{AppState, EmbedConfig};

    /// Build a test AppState with a temporary data_dir and a given embed URL.
    fn make_state(data_dir: &TempDir, embed_url: &str) -> AppState {
        AppState {
            data_dir: data_dir.path().to_path_buf(),
            embed: EmbedConfig {
                url: embed_url.to_string(),
                model: "test-model".to_string(),
                api_key: String::new(),
            },
        }
    }

    /// Build a 768-element f32 embedding response body for wiremock.
    fn embed_response_body() -> serde_json::Value {
        let embedding: Vec<f64> = (0..768).map(|i| (i as f64) / 768.0).collect();
        json!({
            "data": [{"embedding": embedding}]
        })
    }

    // ── Happy path ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn write_returns_201_embedding_generated_vec_row_and_entity_link() {
        let dir = TempDir::new().unwrap();
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(embed_response_body()),
            )
            .mount(&mock_server)
            .await;

        let state = make_state(&dir, &mock_server.uri());
        let app = build_router(state);

        let body = json!({
            "content_text": "Killed the final boss with Arthas.",
            "episode_type": "combat",
            "timestamp": 1_700_000_000_000i64,
            "salience_hint": 0.9,
            "entities": [
                {
                    "entity_kind": "npc",
                    "entity_key": "36597",
                    "display_name": "The Lich King",
                    "role": "target"
                }
            ],
            "metadata": {"dungeon": "ICC", "wipe_count": 3},
            // NOTE: `source` must be one of self|chat|observed|system per the
            // episodes.source CHECK constraint (002 schema). "combat" is an
            // episode_type, not a source — the plan's draft used it here by
            // mistake; the Python parity test (tests/unit/test_write.py) uses
            // "chat". The episode_type above is still "combat".
            "source": "chat"
        });

        let req = Request::builder()
            .method("POST")
            .uri("/v1/memory/bot123/episodes")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(parsed["embedding_generated"], true);
        assert_eq!(parsed["salience_score"], 0.9);
        let episode_id = parsed["episode_id"].as_i64().unwrap();
        assert!(episode_id > 0);

        // Verify vec row was inserted by opening the DB directly.
        use rusqlite::Connection;
        let db_path = dir.path().join("bot123").join("memory.sqlite");
        let conn = Connection::open(&db_path).unwrap();
        let vec_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM embeddings_vec WHERE rowid = ?1",
                [episode_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(vec_count, 1, "vec row must be present for embedding_generated=true");

        // Verify content_embedding_id is set.
        let emb_id: Option<i64> = conn
            .query_row(
                "SELECT content_embedding_id FROM episodes WHERE episode_id = ?1",
                [episode_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(emb_id, Some(episode_id));

        // Verify entity upsert + link (total_episodes trigger).
        let total: i64 = conn
            .query_row(
                "SELECT e.total_episodes FROM entities e WHERE e.display_name = 'The Lich King'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(total, 1);

        // Verify link row exists with coerced role.
        let link_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM episode_entities ee \
                 JOIN entities e ON e.entity_id = ee.entity_id \
                 WHERE ee.episode_id = ?1 AND e.display_name = 'The Lich King' AND ee.role = 'target'",
                [episode_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(link_count, 1);
    }

    // ── Graceful embed degradation ────────────────────────────────────────────

    #[tokio::test]
    async fn write_degrades_gracefully_when_embed_endpoint_down() {
        let dir = TempDir::new().unwrap();
        // Use a port that has no listener — connection refused.
        let state = make_state(&dir, "http://127.0.0.1:19999");
        let app = build_router(state);

        let body = json!({
            "content_text": "Talked to the innkeeper about the war.",
            "episode_type": "chat",
            "timestamp": 1_700_000_001_000i64
        });

        let req = Request::builder()
            .method("POST")
            .uri("/v1/memory/bot456/episodes")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(parsed["embedding_generated"], false);
        let episode_id = parsed["episode_id"].as_i64().unwrap();

        // Verify content_embedding_id IS NULL (no vec row).
        use rusqlite::Connection;
        let db_path = dir.path().join("bot456").join("memory.sqlite");
        let conn = Connection::open(&db_path).unwrap();
        let emb_id: Option<i64> = conn
            .query_row(
                "SELECT content_embedding_id FROM episodes WHERE episode_id = ?1",
                [episode_id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(emb_id.is_none(), "content_embedding_id must be NULL on embed failure");

        // Verify the episode IS still BM25-indexed (FTS row present).
        let fts_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM episodes_fts WHERE episodes_fts MATCH ?1",
                ["innkeeper"],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(fts_count, 1, "episode must be BM25-searchable even with NULL embedding");
    }

    // ── Metadata compact serialization ────────────────────────────────────────

    #[tokio::test]
    async fn metadata_is_stored_as_compact_json() {
        let dir = TempDir::new().unwrap();
        let state = make_state(&dir, "http://127.0.0.1:19999");
        let app = build_router(state);

        let body = json!({
            "content_text": "Completed the quest.",
            "episode_type": "quest",
            "timestamp": 1_700_000_002_000i64,
            "metadata": {"a": 1, "b": "two"}
        });

        let req = Request::builder()
            .method("POST")
            .uri("/v1/memory/botmeta/episodes")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let episode_id = parsed["episode_id"].as_i64().unwrap();

        use rusqlite::Connection;
        let db_path = dir.path().join("botmeta").join("memory.sqlite");
        let conn = Connection::open(&db_path).unwrap();
        let raw: String = conn
            .query_row(
                "SELECT metadata FROM episodes WHERE episode_id = ?1",
                [episode_id],
                |r| r.get(0),
            )
            .unwrap();
        // No spaces: compact JSON — no whitespace after : or ,
        assert!(!raw.contains(' '), "metadata must be compact JSON, got: {raw}");
        // Roundtrip parses to same value.
        let reparsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(reparsed["a"], 1);
        assert_eq!(reparsed["b"], "two");
    }

    // ── Role coercion ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn unknown_entity_role_is_coerced_to_participant() {
        let dir = TempDir::new().unwrap();
        let state = make_state(&dir, "http://127.0.0.1:19999");
        let app = build_router(state);

        let body = json!({
            "content_text": "Saw a stranger at the crossroads.",
            "episode_type": "observation",
            "timestamp": 1_700_000_003_000i64,
            "entities": [{
                "entity_kind": "npc",
                "entity_key": "stranger_1",
                "display_name": "Mysterious Stranger",
                "role": "villain"
            }]
        });

        let req = Request::builder()
            .method("POST")
            .uri("/v1/memory/botcoerce/episodes")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let episode_id = parsed["episode_id"].as_i64().unwrap();

        use rusqlite::Connection;
        let db_path = dir.path().join("botcoerce").join("memory.sqlite");
        let conn = Connection::open(&db_path).unwrap();
        let role: String = conn
            .query_row(
                "SELECT ee.role FROM episode_entities ee \
                 JOIN entities e ON e.entity_id = ee.entity_id \
                 WHERE ee.episode_id = ?1 AND e.display_name = 'Mysterious Stranger'",
                [episode_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(role, "participant", "unknown role must be coerced to participant");
    }

    // ── Salience default ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn salience_defaults_to_0_5_when_omitted() {
        let dir = TempDir::new().unwrap();
        let state = make_state(&dir, "http://127.0.0.1:19999");
        let app = build_router(state);

        let body = json!({
            "content_text": "Nothing in particular happened.",
            "episode_type": "observation",
            "timestamp": 1_700_000_004_000i64
        });

        let req = Request::builder()
            .method("POST")
            .uri("/v1/memory/botsal/episodes")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        // Floating-point equality is fine here: 0.5 is exactly representable in f64.
        assert_eq!(parsed["salience_score"], 0.5);
    }
}
