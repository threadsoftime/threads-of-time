// src/routes/update.rs
//! PATCH /v1/memory/:bot_guid/episodes/:episode_id
//!
//! Parity with `tot_memory/routes/update.py` (design subspec §10.6). Mutable
//! fields:
//!  - `content_text`   — triggers a synchronous re-embedding (replaces the
//!    `embeddings_vec` row). The FTS5 `episodes_au` trigger keeps BM25 in sync.
//!  - `salience_score` — overrides the server-computed salience (no re-embed).
//!  - `metadata`       — replaces the existing JSON blob entirely (compact JSON).
//!
//! Re-embed is **loud**: unlike the write path's graceful degradation, an
//! embed failure here returns 503 (the caller explicitly asked for a re-embed;
//! silently keeping the stale vector would be a worse outcome). The embed
//! round-trip happens on the async side BEFORE the DB is opened.
//!
//! Errors:
//!  - 400 `no_fields_to_update` — all three optional fields are null.
//!  - 404 `episode_not_found`   — the episode_id is absent.
//!  - 503 `embedding_service_unavailable` — content_text supplied but embed failed.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::db::{migrate::run_migrations, open_bot_db};
use crate::embeddings::EmbeddingsClient;
use crate::error::AppError;
use crate::state::AppState;

// ── Request / response types ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct UpdateRequest {
    pub content_text: Option<String>,
    pub salience_score: Option<f64>,
    pub metadata: Option<JsonValue>,
}

#[derive(Debug, Serialize)]
pub struct UpdateResponse {
    pub episode_id: i64,
    pub updated_fields: Vec<String>,
    pub reembedded: bool,
}

// ── Handler ───────────────────────────────────────────────────────────────────

pub async fn update_handler(
    State(st): State<AppState>,
    Path((bot_guid, episode_id)): Path<(String, i64)>,
    Json(body): Json<UpdateRequest>,
) -> Result<(StatusCode, Json<UpdateResponse>), AppError> {
    // Validate: at least one field must be present.
    if body.content_text.is_none() && body.salience_score.is_none() && body.metadata.is_none() {
        return Err(AppError::BadRequest("no_fields_to_update".to_string()));
    }

    // Validate content_text length if present (1..=4000 chars by code point).
    if let Some(ref ct) = body.content_text {
        if ct.is_empty() || ct.chars().count() > 4000 {
            return Err(AppError::BadRequest(
                "content_text must be 1–4000 characters".to_string(),
            ));
        }
    }

    // Validate salience_score range if present.
    if let Some(s) = body.salience_score {
        if !(0.0..=1.0).contains(&s) {
            return Err(AppError::BadRequest(
                "salience_score must be 0.0–1.0".to_string(),
            ));
        }
    }

    // Re-embed BEFORE opening the DB (loud — 503 on fail). Only when
    // content_text is present; mirrors update.py's network-before-DB ordering.
    let new_vec: Option<Vec<f32>> = if let Some(ref text) = body.content_text {
        let client = EmbeddingsClient::new(&st.embed.url, &st.embed.model, &st.embed.api_key);
        match client.embed(text).await {
            Ok(v) => Some(v),
            Err(e) => {
                tracing::warn!(
                    bot_guid = %bot_guid,
                    episode_id = episode_id,
                    error = %e,
                    "re-embed during update failed"
                );
                return Err(AppError::EmbeddingUnavailable);
            }
        }
    } else {
        None
    };

    let reembedded = new_vec.is_some();
    let content_text = body.content_text.clone();
    let salience_score = body.salience_score;
    // Serialize metadata as compact JSON (serde_json::Value::to_string has no
    // extra whitespace — equivalent to Python's separators=(",", ":")).
    let metadata_json: Option<String> = body.metadata.as_ref().map(|m| m.to_string());
    let data_dir = st.data_dir.clone();

    let resp = tokio::task::spawn_blocking(move || -> Result<UpdateResponse, AppError> {
        let conn = open_bot_db(&data_dir, &bot_guid)?;
        run_migrations(&conn)?;

        // Begin explicit transaction for atomicity: the episodes UPDATE + the
        // vec DELETE/INSERT + the content_embedding_id UPDATE must all commit or
        // all roll back (mirrors write.rs).
        conn.execute_batch("BEGIN")?;

        // Check episode exists.
        let exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM episodes WHERE episode_id = ?1",
                rusqlite::params![episode_id],
                |row| row.get::<_, i64>(0),
            )
            .map(|c| c > 0)
            .unwrap_or(false);

        if !exists {
            conn.execute_batch("ROLLBACK").ok();
            return Err(AppError::NotFound("episode_not_found"));
        }

        let mut sets: Vec<String> = Vec::new();
        let mut updated_fields: Vec<String> = Vec::new();

        if content_text.is_some() {
            sets.push("content_text = ?".to_string());
            updated_fields.push("content_text".to_string());
        }
        if salience_score.is_some() {
            sets.push("salience_score = ?".to_string());
            updated_fields.push("salience_score".to_string());
        }
        if metadata_json.is_some() {
            sets.push("metadata = ?".to_string());
            updated_fields.push("metadata".to_string());
        }

        // Build the UPDATE statement with positional params in the same order.
        let set_clause = sets.join(", ");
        let sql = format!("UPDATE episodes SET {set_clause} WHERE episode_id = ?");

        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(ref ct) = content_text {
            params.push(Box::new(ct.clone()));
        }
        if let Some(s) = salience_score {
            params.push(Box::new(s));
        }
        if let Some(ref m) = metadata_json {
            params.push(Box::new(m.clone()));
        }
        params.push(Box::new(episode_id));

        conn.execute(
            &sql,
            rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
        )?;

        // Replace the embeddings_vec row if re-embedded. vec0 virtual tables
        // don't support UPSERT, so DELETE + INSERT is the standard idiom.
        if let Some(ref vec_data) = new_vec {
            let bytes: Vec<u8> = vec_data.iter().flat_map(|x| x.to_le_bytes()).collect();
            conn.execute(
                "DELETE FROM embeddings_vec WHERE rowid = ?1",
                rusqlite::params![episode_id],
            )?;
            conn.execute(
                "INSERT INTO embeddings_vec (rowid, embedding) VALUES (?1, ?2)",
                rusqlite::params![episode_id, bytes],
            )?;
            conn.execute(
                "UPDATE episodes SET content_embedding_id = ?1 WHERE episode_id = ?1",
                rusqlite::params![episode_id],
            )?;
        }

        conn.execute_batch("COMMIT")?;

        Ok(UpdateResponse {
            episode_id,
            updated_fields,
            reembedded,
        })
    })
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("spawn_blocking join error: {e}")))??;

    Ok((StatusCode::OK, Json(resp)))
}

#[cfg(test)]
mod tests {
    use crate::app::build_router;
    use crate::db::{migrate::run_migrations, open_bot_db};
    use crate::state::{AppState, EmbedConfig};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use serde_json::Value;
    use std::path::PathBuf;
    use tempfile::TempDir;
    use tower::ServiceExt;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn seed_episode(dir: &TempDir) -> (PathBuf, i64) {
        let data_dir = dir.path().to_path_buf();
        let conn = open_bot_db(&data_dir, "bot1").unwrap();
        run_migrations(&conn).unwrap();

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        conn.execute(
            "INSERT INTO episodes (content_text, episode_type, timestamp, salience_score, source, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                "original content",
                "chat",
                now_ms - 1000,
                0.5,
                "self",
                r#"{"key":"oldval"}"#
            ],
        )
        .unwrap();
        let ep_id = conn.last_insert_rowid();

        // Insert embedding row.
        let vec: Vec<f32> = (0..768_usize).map(|i| (i as f32) / 768.0).collect();
        let bytes: Vec<u8> = vec.iter().flat_map(|x| x.to_le_bytes()).collect();
        conn.execute(
            "INSERT INTO embeddings_vec (rowid, embedding) VALUES (?1, ?2)",
            rusqlite::params![ep_id, bytes],
        )
        .unwrap();
        conn.execute(
            "UPDATE episodes SET content_embedding_id = ?1 WHERE episode_id = ?1",
            rusqlite::params![ep_id],
        )
        .unwrap();

        (data_dir, ep_id)
    }

    fn make_state(data_dir: PathBuf, embed_url: String) -> AppState {
        AppState {
            data_dir,
            embed: EmbedConfig {
                url: embed_url,
                model: "nomic-embed-text".to_string(),
                api_key: String::new(),
            },
        }
    }

    // ── salience-only update: no re-embed, reembedded=false ──────────────────

    #[tokio::test]
    async fn update_salience_only_no_reembed() {
        let tmp = TempDir::new().unwrap();
        let (data_dir, ep_id) = seed_episode(&tmp);

        let state = make_state(
            data_dir.clone(),
            "http://127.0.0.1:19999".to_string(), // unreachable — must not be called
        );
        let app = build_router(state);

        let req = Request::builder()
            .method("PATCH")
            .uri(format!("/v1/memory/bot1/episodes/{ep_id}"))
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({ "salience_score": 0.9 }).to_string(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["episode_id"].as_i64().unwrap(), ep_id);
        assert_eq!(v["reembedded"].as_bool().unwrap(), false);
        let updated_fields = v["updated_fields"].as_array().unwrap();
        assert!(updated_fields
            .iter()
            .any(|f| f.as_str().unwrap() == "salience_score"));
        assert!(!updated_fields
            .iter()
            .any(|f| f.as_str().unwrap() == "content_text"));

        // Verify DB.
        let conn = open_bot_db(&data_dir, "bot1").unwrap();
        run_migrations(&conn).unwrap();
        let salience: f64 = conn
            .query_row(
                "SELECT salience_score FROM episodes WHERE episode_id = ?1",
                rusqlite::params![ep_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            (salience - 0.9).abs() < 1e-9,
            "salience_score must be updated to 0.9"
        );
    }

    // ── metadata replace: compact JSON, no re-embed ───────────────────────────

    #[tokio::test]
    async fn update_metadata_replaces_and_no_reembed() {
        let tmp = TempDir::new().unwrap();
        let (data_dir, ep_id) = seed_episode(&tmp);

        let state = make_state(data_dir.clone(), "http://127.0.0.1:19999".to_string());
        let app = build_router(state);

        let req = Request::builder()
            .method("PATCH")
            .uri(format!("/v1/memory/bot1/episodes/{ep_id}"))
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({ "metadata": { "new_key": "new_val" } }).to_string(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["reembedded"].as_bool().unwrap(), false);
        let fields = v["updated_fields"].as_array().unwrap();
        assert!(fields.iter().any(|f| f.as_str().unwrap() == "metadata"));

        // Verify DB: metadata must be the new value (compact JSON).
        let conn = open_bot_db(&data_dir, "bot1").unwrap();
        run_migrations(&conn).unwrap();
        let stored_meta: Option<String> = conn
            .query_row(
                "SELECT metadata FROM episodes WHERE episode_id = ?1",
                rusqlite::params![ep_id],
                |row| row.get(0),
            )
            .unwrap();
        let stored_meta = stored_meta.unwrap();
        let parsed: Value = serde_json::from_str(&stored_meta).unwrap();
        assert_eq!(parsed["new_key"].as_str().unwrap(), "new_val");
        // Old key must be gone.
        assert!(
            parsed["key"].is_null(),
            "old metadata must be replaced, not merged"
        );
    }

    // ── content re-embed: reembedded=true, vec row replaced ──────────────────

    #[tokio::test]
    async fn update_content_reembeds_and_replaces_vec_row() {
        let tmp = TempDir::new().unwrap();
        let (data_dir, ep_id) = seed_episode(&tmp);

        let mock = MockServer::start().await;
        // Return a new embedding (768-element pattern, different from seed).
        let new_vec: Vec<f64> = vec![0.5 / (768.0f64).sqrt(); 768];
        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({ "data": [{ "embedding": new_vec }] }),
                ),
            )
            .mount(&mock)
            .await;

        let state = make_state(data_dir.clone(), mock.uri());
        let app = build_router(state);

        let req = Request::builder()
            .method("PATCH")
            .uri(format!("/v1/memory/bot1/episodes/{ep_id}"))
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({ "content_text": "updated content about the siege" })
                    .to_string(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["reembedded"].as_bool().unwrap(), true);
        let fields = v["updated_fields"].as_array().unwrap();
        assert!(fields.iter().any(|f| f.as_str().unwrap() == "content_text"));

        // Verify: content_text updated in DB.
        let conn = open_bot_db(&data_dir, "bot1").unwrap();
        run_migrations(&conn).unwrap();
        let content: String = conn
            .query_row(
                "SELECT content_text FROM episodes WHERE episode_id = ?1",
                rusqlite::params![ep_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(content, "updated content about the siege");

        // Verify: embeddings_vec row exists with new bytes.
        let vec_bytes: Vec<u8> = conn
            .query_row(
                "SELECT embedding FROM embeddings_vec WHERE rowid = ?1",
                rusqlite::params![ep_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(vec_bytes.len(), 768 * 4, "vec bytes must be 768 f32s");

        // content_embedding_id must still point to ep_id.
        let ceid: Option<i64> = conn
            .query_row(
                "SELECT content_embedding_id FROM episodes WHERE episode_id = ?1",
                rusqlite::params![ep_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(ceid, Some(ep_id));
    }

    // ── all-null body -> 400 no_fields_to_update ─────────────────────────────

    #[tokio::test]
    async fn update_all_null_returns_400_no_fields() {
        let tmp = TempDir::new().unwrap();
        let (data_dir, ep_id) = seed_episode(&tmp);

        let state = make_state(data_dir, "http://127.0.0.1:19999".to_string());
        let app = build_router(state);

        let req = Request::builder()
            .method("PATCH")
            .uri(format!("/v1/memory/bot1/episodes/{ep_id}"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({}).to_string()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["detail"].as_str().unwrap(), "no_fields_to_update");
    }

    // ── missing episode -> 404 episode_not_found ──────────────────────────────

    #[tokio::test]
    async fn update_missing_episode_returns_404() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().to_path_buf();
        // Open + migrate the DB so it exists, just has no episodes.
        let conn = open_bot_db(&data_dir, "bot1").unwrap();
        run_migrations(&conn).unwrap();
        drop(conn);

        let state = make_state(data_dir, "http://127.0.0.1:19999".to_string());
        let app = build_router(state);

        let req = Request::builder()
            .method("PATCH")
            .uri("/v1/memory/bot1/episodes/9999")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({ "salience_score": 0.8 }).to_string(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["detail"].as_str().unwrap(), "episode_not_found");
    }

    // ── content re-embed fails -> 503 (loud — not graceful) ──────────────────

    #[tokio::test]
    async fn update_content_embed_down_returns_503() {
        let tmp = TempDir::new().unwrap();
        let (data_dir, ep_id) = seed_episode(&tmp);

        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&mock)
            .await;

        let state = make_state(data_dir, mock.uri());
        let app = build_router(state);

        let req = Request::builder()
            .method("PATCH")
            .uri(format!("/v1/memory/bot1/episodes/{ep_id}"))
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({ "content_text": "new content" }).to_string(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["detail"].as_str().unwrap(), "embedding_service_unavailable");
    }
}
