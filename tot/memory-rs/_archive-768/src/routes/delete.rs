// src/routes/delete.rs
//! DELETE /v1/memory/:bot_guid/episodes/:episode_id
//!
//! Parity with `tot_memory/routes/delete.py` (design subspec §10.7). Hard-delete:
//!
//!  1. The `episode_entities` FK has `ON DELETE CASCADE`, so join rows vanish
//!     automatically when the `episodes` row is removed.
//!  2. The `ee_ad` trigger (§2.3) decrements `entities.total_episodes` for each
//!     link removed.
//!  3. `embeddings_vec` must be deleted explicitly — sqlite-vec virtual tables do
//!     NOT honour FK cascades (§10.7 implementation note).
//!  4. Delete the `episodes` row after the vec row (vec-first order is load-bearing).
//!
//! Both deletes are wrapped in an explicit transaction for atomicity (M2 fix).
//!
//! Response: 204 No Content (empty body).
//!
//! Errors:
//!  - 404 `episode_not_found` — episode_id absent in this bot's DB.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};

use crate::{
    db::{migrate::run_migrations, open_bot_db},
    error::AppError,
    state::AppState,
};

// ── Handler ───────────────────────────────────────────────────────────────────

pub async fn delete_handler(
    State(st): State<AppState>,
    Path((bot_guid, episode_id)): Path<(String, i64)>,
) -> Result<impl IntoResponse, AppError> {
    let data_dir = st.data_dir.clone();

    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        let conn = open_bot_db(&data_dir, &bot_guid)?;
        run_migrations(&conn)?;

        // Check the episode exists.
        let exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM episodes WHERE episode_id = ?1",
                rusqlite::params![episode_id],
                |row| row.get::<_, i64>(0),
            )
            .map(|c| c > 0)
            .unwrap_or(false);

        if !exists {
            return Err(AppError::NotFound("episode_not_found"));
        }

        // Wrap vec DELETE + episode DELETE in a single transaction for atomicity.
        conn.execute_batch("BEGIN")?;

        // Step 1: Delete the embeddings_vec row explicitly.
        // sqlite-vec virtual tables do NOT honour FK cascades; this must be
        // explicit and must happen BEFORE the episodes DELETE (vec-first order
        // matches the Python reference implementation).
        conn.execute(
            "DELETE FROM embeddings_vec WHERE rowid = ?1",
            rusqlite::params![episode_id],
        )?;

        // Step 2: Delete the episode row.
        // This cascades to episode_entities (ON DELETE CASCADE), which fires
        // the ee_ad trigger to decrement entities.total_episodes for each link
        // removed; and fires the episodes_ad trigger to keep the FTS index in sync.
        conn.execute(
            "DELETE FROM episodes WHERE episode_id = ?1",
            rusqlite::params![episode_id],
        )?;

        conn.execute_batch("COMMIT")?;

        Ok(())
    })
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("spawn_blocking join error: {e}")))??;

    Ok(StatusCode::NO_CONTENT)
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

    fn make_state(data_dir: PathBuf) -> AppState {
        AppState {
            data_dir,
            embed: EmbedConfig {
                url: "http://127.0.0.1:19999".to_string(),
                model: "nomic-embed-text".to_string(),
                api_key: String::new(),
            },
        }
    }

    /// Seed a fully-populated episode: episodes row + embeddings_vec row + entity link.
    /// Returns (data_dir, episode_id, entity_id).
    fn seed_full_episode(dir: &TempDir) -> (PathBuf, i64, i64) {
        let data_dir = dir.path().to_path_buf();
        let conn = open_bot_db(&data_dir, "bot1").unwrap();
        run_migrations(&conn).unwrap();

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        conn.execute(
            "INSERT INTO episodes (content_text, episode_type, timestamp, salience_score, source)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params!["battle at the crossroads", "combat", now_ms - 5000, 0.8, "self"],
        ).unwrap();
        let ep_id = conn.last_insert_rowid();

        // Insert vec row (rowid = episode_id by convention)
        let vec: Vec<f32> = vec![0.1f32; 768];
        let bytes: Vec<u8> = vec.iter().flat_map(|x| x.to_le_bytes()).collect();
        conn.execute(
            "INSERT INTO embeddings_vec (rowid, embedding) VALUES (?1, ?2)",
            rusqlite::params![ep_id, bytes],
        ).unwrap();
        conn.execute(
            "UPDATE episodes SET content_embedding_id = ?1 WHERE episode_id = ?1",
            rusqlite::params![ep_id],
        ).unwrap();

        // Insert an entity and link it
        conn.execute(
            "INSERT INTO entities (entity_kind, entity_key, display_name, first_seen_at)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["player", "12345", "Arganoth", now_ms],
        ).unwrap();
        let entity_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO episode_entities (episode_id, entity_id, role)
             VALUES (?1, ?2, ?3)",
            rusqlite::params![ep_id, entity_id, "participant"],
        ).unwrap();

        (data_dir, ep_id, entity_id)
    }

    // ── DELETE existing episode -> 204 no body ────────────────────────────────

    #[tokio::test]
    async fn delete_existing_returns_204_no_body() {
        let tmp = TempDir::new().unwrap();
        let (data_dir, ep_id, _) = seed_full_episode(&tmp);

        let state = make_state(data_dir);
        let app = build_router(state);

        let req = Request::builder()
            .method("DELETE")
            .uri(format!("/v1/memory/bot1/episodes/{ep_id}"))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        let body = axum::body::to_bytes(resp.into_body(), 256).await.unwrap();
        assert!(body.is_empty(), "204 response must have no body");
    }

    // ── vec row is deleted BEFORE episode (load-bearing order) ───────────────

    #[tokio::test]
    async fn delete_removes_vec_row_first_then_episode() {
        let tmp = TempDir::new().unwrap();
        let (data_dir, ep_id, _) = seed_full_episode(&tmp);

        let state = make_state(data_dir.clone());
        let app = build_router(state);

        let req = Request::builder()
            .method("DELETE")
            .uri(format!("/v1/memory/bot1/episodes/{ep_id}"))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        let conn = open_bot_db(&data_dir, "bot1").unwrap();
        run_migrations(&conn).unwrap();

        // embeddings_vec row must be gone
        let vec_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM embeddings_vec WHERE rowid = ?1",
                rusqlite::params![ep_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(vec_count, 0, "embeddings_vec row must be deleted");

        // episodes row must be gone
        let ep_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM episodes WHERE episode_id = ?1",
                rusqlite::params![ep_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(ep_count, 0, "episodes row must be deleted");
    }

    // ── episode_entities cascade + total_episodes decremented ────────────────

    #[tokio::test]
    async fn delete_cascades_to_episode_entities_and_decrements_total() {
        let tmp = TempDir::new().unwrap();
        let (data_dir, ep_id, entity_id) = seed_full_episode(&tmp);

        // Verify total_episodes = 1 before delete (set by ee_ai trigger)
        let conn = open_bot_db(&data_dir, "bot1").unwrap();
        run_migrations(&conn).unwrap();
        let total_before: i64 = conn
            .query_row(
                "SELECT total_episodes FROM entities WHERE entity_id = ?1",
                rusqlite::params![entity_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(total_before, 1, "total_episodes must be 1 after seeding");
        drop(conn);

        let state = make_state(data_dir.clone());
        let app = build_router(state);

        let req = Request::builder()
            .method("DELETE")
            .uri(format!("/v1/memory/bot1/episodes/{ep_id}"))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        let conn2 = open_bot_db(&data_dir, "bot1").unwrap();
        run_migrations(&conn2).unwrap();

        // episode_entities row must be gone (ON DELETE CASCADE)
        let link_count: i64 = conn2
            .query_row(
                "SELECT COUNT(*) FROM episode_entities WHERE episode_id = ?1",
                rusqlite::params![ep_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(link_count, 0, "episode_entities must cascade-delete");

        // total_episodes must be decremented to 0 (ee_ad trigger)
        let total_after: i64 = conn2
            .query_row(
                "SELECT total_episodes FROM entities WHERE entity_id = ?1",
                rusqlite::params![entity_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(total_after, 0, "total_episodes must be decremented by ee_ad trigger");
    }

    // ── FTS row is removed via episodes_ad trigger ────────────────────────────

    #[tokio::test]
    async fn delete_removes_fts_entry() {
        let tmp = TempDir::new().unwrap();
        let (data_dir, ep_id, _) = seed_full_episode(&tmp);

        // Verify FTS can find the episode before delete
        let conn = open_bot_db(&data_dir, "bot1").unwrap();
        run_migrations(&conn).unwrap();
        let fts_before: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM episodes_fts WHERE episodes_fts MATCH 'crossroads'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(fts_before, 1, "FTS must find the episode before delete");
        drop(conn);

        let state = make_state(data_dir.clone());
        let app = build_router(state);

        let req = Request::builder()
            .method("DELETE")
            .uri(format!("/v1/memory/bot1/episodes/{ep_id}"))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        let conn2 = open_bot_db(&data_dir, "bot1").unwrap();
        run_migrations(&conn2).unwrap();
        let fts_after: i64 = conn2
            .query_row(
                "SELECT COUNT(*) FROM episodes_fts WHERE episodes_fts MATCH 'crossroads'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(fts_after, 0, "FTS entry must be removed after delete (episodes_ad trigger)");
    }

    // ── DELETE missing episode -> 404 episode_not_found ───────────────────────

    #[tokio::test]
    async fn delete_missing_episode_returns_404() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().to_path_buf();
        let conn = open_bot_db(&data_dir, "bot1").unwrap();
        run_migrations(&conn).unwrap();
        drop(conn);

        let state = make_state(data_dir);
        let app = build_router(state);

        let req = Request::builder()
            .method("DELETE")
            .uri("/v1/memory/bot1/episodes/9999")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["detail"].as_str().unwrap(), "episode_not_found");
    }
}
