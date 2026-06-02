// src/routes/read.rs
//! GET /v1/memory/:bot_guid/episodes/:episode_id
//!
//! Per spec §5.3: full episode row + linked entities. No side effects —
//! last_recalled_at is NOT bumped (that is reserved for recall, §5.5).
//! Returns 404 {"detail":"episode_not_found"} when the episode does not exist.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use serde_json::Value as JsonValue;

use crate::db::{migrate::run_migrations, open_bot_db};
use crate::error::AppError;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct EntityOut {
    pub entity_id: i64,
    pub entity_kind: String,
    pub entity_key: String,
    pub display_name: String,
    pub role: String,
}

#[derive(Debug, Serialize)]
pub struct EpisodeReadResponse {
    pub episode_id: i64,
    pub timestamp: i64,
    pub created_at: i64,
    pub content_text: String,
    pub episode_type: String,
    pub salience_score: f64,
    pub last_recalled_at: Option<i64>,
    pub recall_count: i64,
    pub source: String,
    pub embedding_generated: bool,
    pub metadata: Option<JsonValue>,
    pub entities: Vec<EntityOut>,
}

pub async fn read_episode(
    State(st): State<AppState>,
    Path((bot_guid, episode_id)): Path<(String, i64)>,
) -> Result<(StatusCode, Json<EpisodeReadResponse>), AppError> {
    let data_dir = st.data_dir.clone();

    let result = tokio::task::spawn_blocking(move || -> Result<EpisodeReadResponse, AppError> {
        let conn = open_bot_db(&data_dir, &bot_guid)?;
        run_migrations(&conn)?;

        // Fetch episode row.
        let row = conn
            .query_row(
                "SELECT episode_id, timestamp, created_at, content_text, episode_type, \
                 salience_score, last_recalled_at, recall_count, source, \
                 content_embedding_id, metadata \
                 FROM episodes WHERE episode_id = ?1",
                params![episode_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,       // episode_id
                        row.get::<_, i64>(1)?,       // timestamp
                        row.get::<_, i64>(2)?,       // created_at
                        row.get::<_, String>(3)?,    // content_text
                        row.get::<_, String>(4)?,    // episode_type
                        row.get::<_, f64>(5)?,       // salience_score
                        row.get::<_, Option<i64>>(6)?,  // last_recalled_at
                        row.get::<_, i64>(7)?,       // recall_count
                        row.get::<_, String>(8)?,    // source
                        row.get::<_, Option<i64>>(9)?,  // content_embedding_id
                        row.get::<_, Option<String>>(10)?,  // metadata
                    ))
                },
            )
            .optional()
            .map_err(AppError::Db)?;

        let (
            ep_id,
            ts,
            created_at,
            content_text,
            episode_type,
            salience_score,
            last_recalled_at,
            recall_count,
            source,
            content_embedding_id,
            metadata_raw,
        ) = row.ok_or(AppError::NotFound("episode_not_found"))?;

        // Fetch linked entities.
        let mut stmt = conn
            .prepare(
                "SELECT e.entity_id, e.entity_kind, e.entity_key, e.display_name, ee.role \
                 FROM episode_entities AS ee \
                 JOIN entities AS e ON e.entity_id = ee.entity_id \
                 WHERE ee.episode_id = ?1",
            )
            .map_err(AppError::Db)?;

        let entities: Vec<EntityOut> = stmt
            .query_map(params![ep_id], |row| {
                Ok(EntityOut {
                    entity_id: row.get(0)?,
                    entity_kind: row.get(1)?,
                    entity_key: row.get(2)?,
                    display_name: row.get(3)?,
                    role: row.get(4)?,
                })
            })
            .map_err(AppError::Db)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::Db)?;

        // Parse metadata JSON; treat parse failure as null (matches Python behavior).
        let metadata: Option<JsonValue> = metadata_raw.and_then(|s| {
            if s.is_empty() {
                None
            } else {
                serde_json::from_str(&s).ok()
            }
        });

        Ok(EpisodeReadResponse {
            episode_id: ep_id,
            timestamp: ts,
            created_at,
            content_text,
            episode_type,
            salience_score,
            last_recalled_at,
            recall_count,
            source,
            embedding_generated: content_embedding_id.is_some(),
            metadata,
            entities,
        })
    })
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("spawn_blocking join error: {e}")))?;

    let resp = result?;
    Ok((StatusCode::OK, Json(resp)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tempfile::TempDir;
    use tower::ServiceExt;

    use crate::app::build_router;
    use crate::state::{AppState, EmbedConfig};

    fn make_state(data_dir: &TempDir) -> AppState {
        AppState {
            data_dir: data_dir.path().to_path_buf(),
            embed: EmbedConfig {
                // Embed endpoint not called by read — use an unreachable URL.
                url: "http://127.0.0.1:19999".to_string(),
                model: "test-model".to_string(),
                api_key: String::new(),
            },
        }
    }

    /// Seed a minimal episode directly via SQL (avoids depending on the write handler).
    fn seed_episode(data_dir: &TempDir, bot_guid: &str) -> i64 {
        use crate::db::{migrate::run_migrations, open_bot_db};
        let conn = open_bot_db(data_dir.path(), bot_guid).unwrap();
        run_migrations(&conn).unwrap();
        conn.execute_batch("BEGIN").unwrap();
        let episode_id: i64 = conn
            .query_row(
                "INSERT INTO episodes \
                 (timestamp, content_text, episode_type, salience_score, source, metadata) \
                 VALUES (1700000000000, 'Read test content', 'chat', 0.75, 'self', '{\"k\":\"v\"}') \
                 RETURNING episode_id",
                [],
                |r| r.get(0),
            )
            .unwrap();
        // Add an entity.
        let entity_id: i64 = conn
            .query_row(
                "INSERT INTO entities (entity_kind, entity_key, display_name, last_seen_at) \
                 VALUES ('player', 'p1', 'Thrall', 1700000000000) RETURNING entity_id",
                [],
                |r| r.get(0),
            )
            .unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO episode_entities (episode_id, entity_id, role) VALUES (?1, ?2, 'subject')",
            [episode_id, entity_id],
        )
        .unwrap();
        conn.execute_batch("COMMIT").unwrap();
        episode_id
    }

    // ── 200 with full episode + entities ──────────────────────────────────────

    #[tokio::test]
    async fn read_returns_200_with_full_episode_and_entities() {
        let dir = TempDir::new().unwrap();
        let episode_id = seed_episode(&dir, "botread");
        let state = make_state(&dir);
        let app = build_router(state);

        let req = Request::builder()
            .method("GET")
            .uri(format!("/v1/memory/botread/episodes/{episode_id}"))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let bytes = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(parsed["episode_id"], episode_id);
        assert_eq!(parsed["content_text"], "Read test content");
        assert_eq!(parsed["episode_type"], "chat");
        assert_eq!(parsed["salience_score"], 0.75);
        assert_eq!(parsed["recall_count"], 0);
        assert!(parsed["last_recalled_at"].is_null(), "last_recalled_at must be null before any recall");
        assert_eq!(parsed["embedding_generated"], false);
        assert_eq!(parsed["metadata"]["k"], "v");

        let entities = parsed["entities"].as_array().unwrap();
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0]["display_name"], "Thrall");
        assert_eq!(entities[0]["role"], "subject");
        assert_eq!(entities[0]["entity_kind"], "player");
        assert_eq!(entities[0]["entity_key"], "p1");
    }

    // ── No last_recalled_at side-effect ───────────────────────────────────────

    #[tokio::test]
    async fn read_does_not_bump_last_recalled_at() {
        let dir = TempDir::new().unwrap();
        let episode_id = seed_episode(&dir, "botnoside");
        let state = make_state(&dir);
        let app = build_router(state);

        // Call read twice.
        for _ in 0..2 {
            let req = Request::builder()
                .method("GET")
                .uri(format!("/v1/memory/botnoside/episodes/{episode_id}"))
                .body(Body::empty())
                .unwrap();
            let resp = app.clone().oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
        }

        // Confirm last_recalled_at is still NULL and recall_count still 0.
        use rusqlite::Connection;
        let db_path = dir.path().join("botnoside").join("memory.sqlite");
        let conn = Connection::open(&db_path).unwrap();
        let (lra, rc): (Option<i64>, i64) = conn
            .query_row(
                "SELECT last_recalled_at, recall_count FROM episodes WHERE episode_id = ?1",
                [episode_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert!(lra.is_none(), "last_recalled_at must not be set by read");
        assert_eq!(rc, 0, "recall_count must not be incremented by read");
    }

    // ── 404 episode_not_found ─────────────────────────────────────────────────

    #[tokio::test]
    async fn read_returns_404_for_missing_episode() {
        let dir = TempDir::new().unwrap();
        let state = make_state(&dir);
        let app = build_router(state);

        let req = Request::builder()
            .method("GET")
            .uri("/v1/memory/botmissing/episodes/999999")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let bytes = axum::body::to_bytes(resp.into_body(), 256).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed["detail"], "episode_not_found");
    }
}
