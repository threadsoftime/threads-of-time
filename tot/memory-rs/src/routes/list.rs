// src/routes/list.rs
//! GET /v1/memory/:bot_guid/episodes
//!
//! Paginated, filtered episode list per spec §5.4.
//! Query params: episode_type?, entity_name?, after?, before?,
//!               limit (1–200, default 50), offset (>=0, default 0).
//! ORDER BY episodes.timestamp DESC, episodes.episode_id DESC.
//! total = COUNT(DISTINCT episodes.episode_id) over the filtered set.
//! has_more = (offset + len(results)) < total.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use rusqlite::params_from_iter;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::db::{migrate::run_migrations, open_bot_db};
use crate::error::AppError;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub episode_type: Option<String>,
    pub entity_name: Option<String>,
    pub after: Option<i64>,
    pub before: Option<i64>,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    50
}

#[derive(Debug, Serialize)]
pub struct ListEpisodeRow {
    pub episode_id: i64,
    pub timestamp: i64,
    pub content_text: String,
    pub episode_type: String,
    pub salience_score: f64,
    pub source: String,
    pub embedding_generated: bool,
    pub metadata: Option<JsonValue>,
}

#[derive(Debug, Serialize)]
pub struct ListEpisodesResponse {
    pub results: Vec<ListEpisodeRow>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
    pub has_more: bool,
}

pub async fn list_episodes(
    State(st): State<AppState>,
    Path(bot_guid): Path<String>,
    Query(q): Query<ListQuery>,
) -> Result<(StatusCode, Json<ListEpisodesResponse>), AppError> {
    // Validate limit and offset in-handler.
    if !(1..=200).contains(&q.limit) {
        return Err(AppError::BadRequest("limit must be 1–200".to_string()));
    }
    if q.offset < 0 {
        return Err(AppError::BadRequest("offset must be >= 0".to_string()));
    }

    let data_dir = st.data_dir.clone();

    let result = tokio::task::spawn_blocking(move || -> Result<ListEpisodesResponse, AppError> {
        let conn = open_bot_db(&data_dir, &bot_guid)?;
        run_migrations(&conn)?;

        // Build WHERE clauses and positional params dynamically (mirrors Python list_eps.py).
        // rusqlite positional params are ?1, ?2, ... but params_from_iter uses ? placeholders.
        // We build with ? and collect values in order, then append limit/offset.
        let mut clauses: Vec<&'static str> = Vec::new();
        let mut joins = String::new();

        // We use owned String for dynamic values.
        // Using a Vec<Box<dyn rusqlite::ToSql>> would require boxing; instead we build
        // the SQL and collect values as serde_json::Value equivalents isn't ergonomic.
        // Cleanest approach for rusqlite: use String placeholders and Vec<rusqlite::types::Value>.

        let mut params: Vec<rusqlite::types::Value> = Vec::new();

        if q.entity_name.is_some() {
            joins.push_str(
                " JOIN episode_entities AS ee ON ee.episode_id = episodes.episode_id \
                 JOIN entities AS e ON e.entity_id = ee.entity_id",
            );
            clauses.push("e.display_name = ?");
            params.push(rusqlite::types::Value::Text(
                q.entity_name.clone().unwrap(),
            ));
        }
        if let Some(ref et) = q.episode_type {
            clauses.push("episodes.episode_type = ?");
            params.push(rusqlite::types::Value::Text(et.clone()));
        }
        if let Some(a) = q.after {
            clauses.push("episodes.timestamp >= ?");
            params.push(rusqlite::types::Value::Integer(a));
        }
        if let Some(b) = q.before {
            clauses.push("episodes.timestamp <= ?");
            params.push(rusqlite::types::Value::Integer(b));
        }

        let where_clause = if clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", clauses.join(" AND "))
        };

        // COUNT(DISTINCT ...) for accurate total when entity join may duplicate rows.
        let count_sql = format!(
            "SELECT COUNT(DISTINCT episodes.episode_id) FROM episodes{joins} {where_clause}",
            joins = joins,
            where_clause = where_clause,
        );
        let total: i64 = conn
            .query_row(
                &count_sql,
                params_from_iter(params.iter()),
                |r| r.get(0),
            )
            .map_err(AppError::Db)?;

        // SELECT with DISTINCT to avoid duplicate rows when entity join fires.
        let select_sql = format!(
            "SELECT DISTINCT episodes.episode_id, episodes.timestamp, episodes.content_text, \
             episodes.episode_type, episodes.salience_score, episodes.source, \
             episodes.content_embedding_id, episodes.metadata \
             FROM episodes{joins} {where_clause} \
             ORDER BY episodes.timestamp DESC, episodes.episode_id DESC \
             LIMIT ? OFFSET ?",
            joins = joins,
            where_clause = where_clause,
        );

        // Append limit and offset to params for the SELECT.
        let mut select_params = params.clone();
        select_params.push(rusqlite::types::Value::Integer(q.limit));
        select_params.push(rusqlite::types::Value::Integer(q.offset));

        let mut stmt = conn.prepare(&select_sql).map_err(AppError::Db)?;
        let rows: Vec<ListEpisodeRow> = stmt
            .query_map(params_from_iter(select_params.iter()), |row| {
                let metadata_raw: Option<String> = row.get(7)?;
                Ok((
                    row.get::<_, i64>(0)?,      // episode_id
                    row.get::<_, i64>(1)?,      // timestamp
                    row.get::<_, String>(2)?,   // content_text
                    row.get::<_, String>(3)?,   // episode_type
                    row.get::<_, f64>(4)?,      // salience_score
                    row.get::<_, String>(5)?,   // source
                    row.get::<_, Option<i64>>(6)?,  // content_embedding_id
                    metadata_raw,
                ))
            })
            .map_err(AppError::Db)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::Db)?
            .into_iter()
            .map(
                |(episode_id, timestamp, content_text, episode_type, salience_score, source, eid, metadata_raw)| {
                    let metadata: Option<JsonValue> = metadata_raw.and_then(|s| {
                        if s.is_empty() {
                            None
                        } else {
                            serde_json::from_str(&s).ok()
                        }
                    });
                    ListEpisodeRow {
                        episode_id,
                        timestamp,
                        content_text,
                        episode_type,
                        salience_score,
                        source,
                        embedding_generated: eid.is_some(),
                        metadata,
                    }
                },
            )
            .collect();

        let has_more = (q.offset + rows.len() as i64) < total;

        Ok(ListEpisodesResponse {
            results: rows,
            total,
            limit: q.limit,
            offset: q.offset,
            has_more,
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
    use crate::db::{migrate::run_migrations, open_bot_db};
    use crate::state::{AppState, EmbedConfig};

    fn make_state(data_dir: &TempDir) -> AppState {
        AppState {
            data_dir: data_dir.path().to_path_buf(),
            embed: EmbedConfig {
                url: "http://127.0.0.1:19999".to_string(),
                model: "test-model".to_string(),
                api_key: String::new(),
            },
        }
    }

    /// Insert N episodes for bot_guid and return their episode_ids (ascending insertion order).
    fn seed_episodes(data_dir: &TempDir, bot_guid: &str, specs: &[(&str, i64, &str)]) -> Vec<i64> {
        // specs: (content_text, timestamp, episode_type)
        let conn = open_bot_db(data_dir.path(), bot_guid).unwrap();
        run_migrations(&conn).unwrap();
        let mut ids = Vec::new();
        for (text, ts, ep_type) in specs {
            let id: i64 = conn
                .query_row(
                    "INSERT INTO episodes (timestamp, content_text, episode_type, salience_score, source) \
                     VALUES (?1, ?2, ?3, 0.5, 'self') RETURNING episode_id",
                    rusqlite::params![ts, text, ep_type],
                    |r| r.get(0),
                )
                .unwrap();
            ids.push(id);
        }
        ids
    }

    /// Add an entity link to an episode.
    fn link_entity(data_dir: &TempDir, bot_guid: &str, episode_id: i64, display_name: &str) {
        let conn = open_bot_db(data_dir.path(), bot_guid).unwrap();
        run_migrations(&conn).unwrap();
        let entity_id: i64 = conn
            .query_row(
                "INSERT OR IGNORE INTO entities (entity_kind, entity_key, display_name, last_seen_at) \
                 VALUES ('npc', ?1, ?1, 1700000000000) RETURNING entity_id",
                [display_name],
                |r| r.get(0),
            )
            .unwrap_or_else(|_| {
                conn.query_row(
                    "SELECT entity_id FROM entities WHERE entity_key = ?1",
                    [display_name],
                    |r| r.get(0),
                )
                .unwrap()
            });
        conn.execute(
            "INSERT OR IGNORE INTO episode_entities (episode_id, entity_id, role) VALUES (?1, ?2, 'participant')",
            [episode_id, entity_id],
        )
        .unwrap();
    }

    // ── Ordering: timestamp DESC then episode_id DESC ─────────────────────────

    #[tokio::test]
    async fn list_orders_by_timestamp_desc_then_episode_id_desc() {
        let dir = TempDir::new().unwrap();
        // Three episodes: two share the same timestamp; one is older.
        // Same-timestamp episodes must come out ordered by episode_id DESC.
        let specs = &[
            ("oldest event", 1_000_000_000i64, "chat"),
            ("same ts A", 2_000_000_000i64, "chat"),
            ("same ts B", 2_000_000_000i64, "combat"),
        ];
        let ids = seed_episodes(&dir, "botord", specs);
        let state = make_state(&dir);
        let app = build_router(state);

        let req = Request::builder()
            .method("GET")
            .uri("/v1/memory/botord/episodes")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let bytes = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let results = parsed["results"].as_array().unwrap();

        // Expected order: ids[2] (same ts, higher id), ids[1] (same ts, lower id), ids[0] (oldest).
        assert_eq!(results[0]["episode_id"], ids[2]);
        assert_eq!(results[1]["episode_id"], ids[1]);
        assert_eq!(results[2]["episode_id"], ids[0]);
    }

    // ── episode_type filter ───────────────────────────────────────────────────

    #[tokio::test]
    async fn list_filters_by_episode_type() {
        let dir = TempDir::new().unwrap();
        let specs = &[
            ("chat event 1", 1_000i64, "chat"),
            ("combat event", 2_000i64, "combat"),
            ("chat event 2", 3_000i64, "chat"),
        ];
        seed_episodes(&dir, "bottype", specs);
        let state = make_state(&dir);
        let app = build_router(state);

        let req = Request::builder()
            .method("GET")
            .uri("/v1/memory/bottype/episodes?episode_type=chat")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(parsed["total"], 2);
        let results = parsed["results"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        for row in results {
            assert_eq!(row["episode_type"], "chat");
        }
    }

    // ── entity_name join filter ───────────────────────────────────────────────

    #[tokio::test]
    async fn list_filters_by_entity_name() {
        let dir = TempDir::new().unwrap();
        let specs = &[
            ("episode linked to Sylvanas", 1_000i64, "social"),
            ("unrelated episode", 2_000i64, "chat"),
            ("another linked to Sylvanas", 3_000i64, "quest"),
        ];
        let ids = seed_episodes(&dir, "botent", specs);
        link_entity(&dir, "botent", ids[0], "Sylvanas");
        link_entity(&dir, "botent", ids[2], "Sylvanas");
        let state = make_state(&dir);
        let app = build_router(state);

        let req = Request::builder()
            .method("GET")
            .uri("/v1/memory/botent/episodes?entity_name=Sylvanas")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(parsed["total"], 2, "entity_name filter must return 2 linked episodes");
        let results = parsed["results"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        // Must NOT include the unrelated episode.
        for row in results {
            assert_ne!(row["episode_id"], ids[1]);
        }
    }

    // ── after / before timestamp filters ─────────────────────────────────────

    #[tokio::test]
    async fn list_filters_by_after_and_before() {
        let dir = TempDir::new().unwrap();
        let specs = &[
            ("early",  1_000i64, "chat"),
            ("middle", 5_000i64, "chat"),
            ("late",  10_000i64, "chat"),
        ];
        let ids = seed_episodes(&dir, "botts", specs);
        let state = make_state(&dir);
        let app = build_router(state);

        // after=2000 before=9000 → only middle.
        let req = Request::builder()
            .method("GET")
            .uri("/v1/memory/botts/episodes?after=2000&before=9000")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(parsed["total"], 1);
        let results = parsed["results"].as_array().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["episode_id"], ids[1]);
    }

    // ── limit / offset / has_more ─────────────────────────────────────────────

    #[tokio::test]
    async fn list_pagination_limit_offset_has_more() {
        let dir = TempDir::new().unwrap();
        // Seed 5 episodes.
        let specs: Vec<(&str, i64, &str)> = (0..5)
            .map(|i| ("text", (i as i64) * 1000 + 1, "chat"))
            .collect();
        let specs_refs: Vec<(&str, i64, &str)> = specs.iter().map(|(t, ts, et)| (*t, *ts, *et)).collect();
        seed_episodes(&dir, "botpage", &specs_refs);
        let state = make_state(&dir);
        let app = build_router(state);

        // First page: limit=2 offset=0 → 2 results, has_more=true, total=5.
        let req = Request::builder()
            .method("GET")
            .uri("/v1/memory/botpage/episodes?limit=2&offset=0")
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
        let p1: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(p1["total"], 5);
        assert_eq!(p1["limit"], 2);
        assert_eq!(p1["offset"], 0);
        assert_eq!(p1["has_more"], true);
        assert_eq!(p1["results"].as_array().unwrap().len(), 2);

        // Last page: limit=2 offset=4 → 1 result, has_more=false.
        let req2 = Request::builder()
            .method("GET")
            .uri("/v1/memory/botpage/episodes?limit=2&offset=4")
            .body(Body::empty())
            .unwrap();
        let resp2 = app.oneshot(req2).await.unwrap();
        assert_eq!(resp2.status(), StatusCode::OK);
        let bytes2 = axum::body::to_bytes(resp2.into_body(), 8192).await.unwrap();
        let p2: serde_json::Value = serde_json::from_slice(&bytes2).unwrap();
        assert_eq!(p2["total"], 5);
        assert_eq!(p2["has_more"], false);
        assert_eq!(p2["results"].as_array().unwrap().len(), 1);
    }

    // ── Empty bot DB → empty results ─────────────────────────────────────────

    #[tokio::test]
    async fn list_empty_db_returns_empty_results() {
        let dir = TempDir::new().unwrap();
        let state = make_state(&dir);
        let app = build_router(state);

        let req = Request::builder()
            .method("GET")
            .uri("/v1/memory/botempty/episodes")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(parsed["total"], 0);
        assert_eq!(parsed["has_more"], false);
        assert_eq!(parsed["results"].as_array().unwrap().len(), 0);
    }

    // ── COUNT(DISTINCT) — no duplicate rows from entity join ──────────────────

    #[tokio::test]
    async fn list_entity_join_no_duplicates_in_total() {
        let dir = TempDir::new().unwrap();
        let specs = &[("shared episode", 1_000i64, "social")];
        let ids = seed_episodes(&dir, "botdup", specs);
        // Link two different entities to the same episode.
        link_entity(&dir, "botdup", ids[0], "Jaina");
        link_entity(&dir, "botdup", ids[0], "Varian");

        let state = make_state(&dir);
        let app = build_router(state);

        // No entity_name filter — should see exactly 1 row, total=1.
        let req = Request::builder()
            .method("GET")
            .uri("/v1/memory/botdup/episodes")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(parsed["total"], 1, "COUNT(DISTINCT) must not double-count via entity join");
        assert_eq!(parsed["results"].as_array().unwrap().len(), 1);
    }
}
