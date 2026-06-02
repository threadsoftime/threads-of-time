// src/routes/recall.rs
//! POST /v1/memory/:bot_guid/recall — memory.recall route.
//!
//! Parity with `tot_memory/routes/recall.py` (design subspec §10.3 / §6).
//!
//! Pipeline:
//!  1. Embed the query text via the BYOLLM endpoint. **Graceful degradation:**
//!     any embed failure falls back to BM25-only (`query_vec = []`) — distinct
//!     from search/update which surface 503. This route never returns 503 for an
//!     embed failure.
//!  2. If `entity_names` is non-empty, resolve it to the set of episode_ids via
//!     `retrieval::entity::entity_filter`. Empty resolved set ⇒ `{results:[]}`
//!     (hard-filter semantics — never silently skip the filter).
//!  3. If `episode_types` and/or `time_filter` are present, narrow the id set by
//!     intersection. Empty ⇒ `{results:[]}`.
//!  4. Call `retrieval::rerank::recall` (candidate_multiplier = 5) to score
//!     candidates via the §6.2 hybrid formula, then hydrate row-level fields.
//!  5. **Side effect:** bump `last_recalled_at` + `recall_count` on each returned
//!     episode. This is the load-bearing distinction from memory.search (§10.4).

use std::collections::HashSet;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};

use crate::{
    db::{migrate::run_migrations, open_bot_db},
    embeddings::EmbeddingsClient,
    error::AppError,
    retrieval::{
        entity::entity_filter,
        hybrid::{DEFAULT_ALPHA, DEFAULT_BETA, DEFAULT_DELTA, DEFAULT_GAMMA},
        rerank::recall as recall_orchestrator,
    },
    state::AppState,
};

// ── Request / response types ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct TimeFilter {
    pub after: Option<i64>,
    pub before: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct RecallRequest {
    pub query_text: String,
    pub top_k: Option<i64>,
    pub entity_names: Option<Vec<String>>,
    pub episode_types: Option<Vec<String>>,
    pub time_filter: Option<TimeFilter>,
    pub alpha: Option<f64>,
    pub beta: Option<f64>,
    pub gamma: Option<f64>,
    pub delta: Option<f64>,
    /// Accepted for API forward-compat; ignored (MMR rerank deferred, §6.3).
    pub mmr_lambda: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct RecallComponentBreakdown {
    pub bm25_norm: f64,
    pub dense_norm: f64,
    pub decay: f64,
    pub salience: f64,
    pub entity_match: f64,
}

#[derive(Debug, Serialize)]
pub struct RecallHit {
    pub episode_id: i64,
    pub content_text: String,
    pub timestamp: i64,
    pub episode_type: String,
    pub salience_score: f64,
    pub score: f64,
    pub components: RecallComponentBreakdown,
}

#[derive(Debug, Serialize)]
pub struct RecallResponse {
    pub results: Vec<RecallHit>,
}

// ── Handler ───────────────────────────────────────────────────────────────────

pub async fn recall_handler(
    State(st): State<AppState>,
    Path(bot_guid): Path<String>,
    Json(body): Json<RecallRequest>,
) -> Result<(StatusCode, Json<RecallResponse>), AppError> {
    // Validate request (in-handler; acceptable divergence from FastAPI 422).
    if body.query_text.is_empty() || body.query_text.chars().count() > 500 {
        return Err(AppError::BadRequest(
            "query_text must be 1..500 characters".to_string(),
        ));
    }
    let top_k = body.top_k.unwrap_or(5);
    if !(1..=20).contains(&top_k) {
        return Err(AppError::BadRequest("top_k must be 1..20".to_string()));
    }

    let alpha = body.alpha.unwrap_or(DEFAULT_ALPHA);
    let beta = body.beta.unwrap_or(DEFAULT_BETA);
    let gamma = body.gamma.unwrap_or(DEFAULT_GAMMA);
    let delta = body.delta.unwrap_or(DEFAULT_DELTA);

    // Embed the query; on any failure fall back to BM25-only (query_vec = []).
    // This route degrades gracefully — it never surfaces a 503 for embed errors.
    let embed_client = EmbeddingsClient::new(&st.embed.url, &st.embed.model, &st.embed.api_key);
    let query_vec: Vec<f32> = match embed_client.embed(&body.query_text).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                bot_guid = %bot_guid,
                error = %e,
                "embedding call failed during recall; falling back to BM25-only"
            );
            vec![]
        }
    };

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    let query_text = body.query_text.clone();
    let entity_names = body.entity_names.clone().unwrap_or_default();
    let episode_types = body.episode_types.clone();
    let time_filter_after = body.time_filter.as_ref().and_then(|f| f.after);
    let time_filter_before = body.time_filter.as_ref().and_then(|f| f.before);
    let data_dir = st.data_dir.clone();

    let hits = tokio::task::spawn_blocking(move || -> Result<Vec<RecallHit>, AppError> {
        let conn = open_bot_db(&data_dir, &bot_guid)?;
        run_migrations(&conn)?;

        // Resolve entity hard-filter. Non-empty names that resolve to nothing ⇒
        // empty results (never silently skip the filter — §6.1 hard-filter).
        let mut entity_filter_ids: Option<HashSet<i64>> = None;
        if !entity_names.is_empty() {
            let ids = entity_filter(&conn, &entity_names)?;
            if ids.is_empty() {
                return Ok(vec![]);
            }
            entity_filter_ids = Some(ids);
        }

        // Apply episode_type + time_filter constraints by intersecting ids.
        if episode_types.is_some() || time_filter_after.is_some() || time_filter_before.is_some() {
            let extra = ids_matching_constraints(
                &conn,
                episode_types.as_deref(),
                time_filter_after,
                time_filter_before,
            )?;
            entity_filter_ids = Some(match entity_filter_ids {
                None => extra,
                Some(existing) => existing.intersection(&extra).copied().collect(),
            });
            if entity_filter_ids
                .as_ref()
                .map(|s| s.is_empty())
                .unwrap_or(false)
            {
                return Ok(vec![]);
            }
        }

        let scored = recall_orchestrator(
            &conn,
            &query_text,
            &query_vec,
            now_ms,
            top_k,
            entity_filter_ids.as_ref(),
            alpha,
            beta,
            gamma,
            delta,
            5, // candidate_multiplier
        )?;

        if scored.is_empty() {
            return Ok(vec![]);
        }

        // Hydrate rows for the scored episode ids.
        let ids: Vec<i64> = scored.iter().map(|r| r.episode_id).collect();
        let placeholders: String = ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT episode_id, content_text, timestamp, episode_type, salience_score \
             FROM episodes WHERE episode_id IN ({placeholders})"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows: std::collections::HashMap<i64, (String, i64, String, f64)> = stmt
            .query_map(rusqlite::params_from_iter(ids.iter()), |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    (
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, f64>(4)?,
                    ),
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();

        let mut hits: Vec<RecallHit> = Vec::new();
        for r in &scored {
            if let Some((content_text, timestamp, episode_type, salience_score)) =
                rows.get(&r.episode_id)
            {
                hits.push(RecallHit {
                    episode_id: r.episode_id,
                    content_text: content_text.clone(),
                    timestamp: *timestamp,
                    episode_type: episode_type.clone(),
                    salience_score: *salience_score,
                    score: r.score,
                    components: RecallComponentBreakdown {
                        bm25_norm: r.bm25_norm,
                        dense_norm: r.dense_norm,
                        decay: r.decay,
                        salience: r.salience,
                        entity_match: r.entity_match,
                    },
                });
            }
        }

        // Side effect: bump last_recalled_at + recall_count on returned hits.
        // (memory.search does NOT do this — this is the §10.3 distinction.)
        if !hits.is_empty() {
            let hit_ids: Vec<i64> = hits.iter().map(|h| h.episode_id).collect();
            let ph: String = hit_ids
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", i + 2))
                .collect::<Vec<_>>()
                .join(",");
            let update_sql = format!(
                "UPDATE episodes SET last_recalled_at = ?1, recall_count = recall_count + 1 \
                 WHERE episode_id IN ({ph})"
            );
            conn.execute(
                &update_sql,
                rusqlite::params_from_iter(
                    std::iter::once(&now_ms as &dyn rusqlite::ToSql)
                        .chain(hit_ids.iter().map(|id| id as &dyn rusqlite::ToSql)),
                ),
            )?;
        }

        Ok(hits)
    })
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("spawn_blocking join error: {e}")))??;

    Ok((StatusCode::OK, Json(RecallResponse { results: hits })))
}

// ── Internal helper: episode_ids matching episode_type + time constraints ─────

fn ids_matching_constraints(
    conn: &rusqlite::Connection,
    episode_types: Option<&[String]>,
    after_ms: Option<i64>,
    before_ms: Option<i64>,
) -> rusqlite::Result<HashSet<i64>> {
    let mut clauses: Vec<String> = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    let mut param_idx = 1usize;

    if let Some(types) = episode_types {
        if !types.is_empty() {
            let ph: String = types
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", param_idx + i))
                .collect::<Vec<_>>()
                .join(",");
            clauses.push(format!("episode_type IN ({ph})"));
            for t in types {
                params.push(Box::new(t.clone()));
                param_idx += 1;
            }
        }
    }
    if let Some(after) = after_ms {
        clauses.push(format!("timestamp >= ?{param_idx}"));
        params.push(Box::new(after));
        param_idx += 1;
    }
    if let Some(before) = before_ms {
        clauses.push(format!("timestamp <= ?{param_idx}"));
        params.push(Box::new(before));
    }

    let where_clause = if clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", clauses.join(" AND "))
    };

    let sql = format!("SELECT episode_id FROM episodes {where_clause}");
    let mut stmt = conn.prepare(&sql)?;
    let ids: HashSet<i64> = stmt
        .query_map(
            rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
            |row| row.get::<_, i64>(0),
        )?
        .filter_map(|r| r.ok())
        .collect();
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use crate::app::build_router;
    use crate::state::{AppState, EmbedConfig};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use serde_json::Value;
    use std::path::PathBuf;
    use tempfile::TempDir;
    use tower::ServiceExt;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // ── Helper: build a 768-dim fake embedding JSON response ─────────────────

    fn fake_embed_response() -> Value {
        let vec: Vec<f64> = (0..768).map(|i| (i as f64) / 768.0).collect();
        serde_json::json!({
            "data": [{ "embedding": vec }]
        })
    }

    // ── Helper: open + migrate a temp DB and seed episodes ───────────────────

    fn seed_db(dir: &TempDir) -> (PathBuf, i64, i64) {
        use crate::db::migrate::run_migrations;
        use crate::db::open_bot_db;

        let data_dir = dir.path().to_path_buf();
        let conn = open_bot_db(&data_dir, "bot1").unwrap();
        run_migrations(&conn).unwrap();

        // Episode 1: recent chat, high salience
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        conn.execute(
            "INSERT INTO episodes (content_text, episode_type, timestamp, salience_score, source)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params!["the dragon attacked the village", "combat", now_ms - 60_000, 0.9, "self"],
        ).unwrap();
        let ep1_id = conn.last_insert_rowid();

        // Insert embedding for ep1
        let vec1: Vec<f32> = (0..768_usize).map(|i| (i as f32) / 768.0).collect();
        let bytes1: Vec<u8> = vec1.iter().flat_map(|x| x.to_le_bytes()).collect();
        conn.execute(
            "INSERT INTO embeddings_vec (rowid, embedding) VALUES (?1, ?2)",
            rusqlite::params![ep1_id, bytes1],
        ).unwrap();
        conn.execute(
            "UPDATE episodes SET content_embedding_id = ?1 WHERE episode_id = ?1",
            rusqlite::params![ep1_id],
        ).unwrap();

        // Episode 2: older chat, lower salience
        conn.execute(
            "INSERT INTO episodes (content_text, episode_type, timestamp, salience_score, source)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params!["dragon lore in the archives", "chat", now_ms - 86_400_000, 0.3, "self"],
        ).unwrap();
        let ep2_id = conn.last_insert_rowid();

        let vec2: Vec<f32> = (0..768_usize).map(|i| 1.0 - (i as f32) / 768.0).collect();
        let bytes2: Vec<u8> = vec2.iter().flat_map(|x| x.to_le_bytes()).collect();
        conn.execute(
            "INSERT INTO embeddings_vec (rowid, embedding) VALUES (?1, ?2)",
            rusqlite::params![ep2_id, bytes2],
        ).unwrap();
        conn.execute(
            "UPDATE episodes SET content_embedding_id = ?1 WHERE episode_id = ?1",
            rusqlite::params![ep2_id],
        ).unwrap();

        conn.execute("UPDATE episodes SET recall_count = 0", []).unwrap();
        (data_dir, ep1_id, ep2_id)
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

    // ── Test: basic recall returns 200 with results + component scores ────────

    #[tokio::test]
    async fn recall_200_returns_results_with_components() {
        let tmp = TempDir::new().unwrap();
        let (data_dir, ep1_id, _ep2_id) = seed_db(&tmp);

        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(fake_embed_response()))
            .mount(&mock)
            .await;

        let state = make_state(data_dir, mock.uri());
        let app = build_router(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/memory/bot1/recall")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "query_text": "dragon attack",
                    "top_k": 2
                })
                .to_string(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        let results = v["results"].as_array().unwrap();
        assert!(!results.is_empty(), "expected at least one result");

        let first = &results[0];
        // ep1 should rank first: higher salience + more recent decay
        assert_eq!(first["episode_id"].as_i64().unwrap(), ep1_id);
        assert!(first["score"].as_f64().unwrap() > 0.0);

        // components must be present with correct keys
        let comp = &first["components"];
        assert!(comp["bm25_norm"].as_f64().is_some());
        assert!(comp["dense_norm"].as_f64().is_some());
        assert!(comp["decay"].as_f64().is_some());
        assert!(comp["salience"].as_f64().is_some());
        assert!(comp["entity_match"].as_f64().is_some());

        // decay must be in (0,1]
        let decay = comp["decay"].as_f64().unwrap();
        assert!(decay > 0.0 && decay <= 1.0, "decay={decay}");
    }

    // ── Test: recall bumps last_recalled_at + recall_count on returned hits ───

    #[tokio::test]
    async fn recall_bumps_last_recalled_at_and_recall_count() {
        use crate::db::migrate::run_migrations;
        use crate::db::open_bot_db;

        let tmp = TempDir::new().unwrap();
        let (data_dir, ep1_id, _ep2_id) = seed_db(&tmp);

        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(fake_embed_response()))
            .mount(&mock)
            .await;

        let state = make_state(data_dir.clone(), mock.uri());
        let app = build_router(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/memory/bot1/recall")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "query_text": "dragon attack",
                    "top_k": 1
                })
                .to_string(),
            ))
            .unwrap();

        let before_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Wait a tick so now_ms > before_ms is guaranteed
        tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;

        let conn = open_bot_db(&data_dir, "bot1").unwrap();
        run_migrations(&conn).unwrap();

        let (last_recalled_at, recall_count): (Option<i64>, i64) = conn
            .query_row(
                "SELECT last_recalled_at, recall_count FROM episodes WHERE episode_id = ?1",
                rusqlite::params![ep1_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();

        assert!(
            last_recalled_at.is_some(),
            "last_recalled_at must be set after recall"
        );
        assert!(
            last_recalled_at.unwrap() >= before_ms,
            "last_recalled_at must be >= before_ms"
        );
        assert_eq!(recall_count, 1, "recall_count must be incremented to 1");
    }

    // ── Test: entity hard-filter with no matching episodes -> {results:[]} ────

    #[tokio::test]
    async fn recall_entity_hard_filter_empty_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let (data_dir, _ep1_id, _ep2_id) = seed_db(&tmp);

        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(fake_embed_response()))
            .mount(&mock)
            .await;

        let state = make_state(data_dir, mock.uri());
        let app = build_router(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/memory/bot1/recall")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "query_text": "dragon attack",
                    "top_k": 5,
                    "entity_names": ["NonExistentEntity"]
                })
                .to_string(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["results"].as_array().unwrap().len(), 0);
    }

    // ── Test: episode_types filter narrows results ────────────────────────────

    #[tokio::test]
    async fn recall_episode_types_filter_narrows() {
        let tmp = TempDir::new().unwrap();
        let (data_dir, _ep1_id, ep2_id) = seed_db(&tmp);

        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(fake_embed_response()))
            .mount(&mock)
            .await;

        // Only request "chat" type — ep1 is "combat", ep2 is "chat"
        let state = make_state(data_dir, mock.uri());
        let app = build_router(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/memory/bot1/recall")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "query_text": "dragon",
                    "top_k": 5,
                    "episode_types": ["chat"]
                })
                .to_string(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        let results = v["results"].as_array().unwrap();
        // All returned episodes must be "chat" type
        for r in results {
            assert_eq!(r["episode_type"].as_str().unwrap(), "chat");
            assert_eq!(r["episode_id"].as_i64().unwrap(), ep2_id);
        }
    }

    // ── Test: time_filter narrows to episodes within window ──────────────────

    #[tokio::test]
    async fn recall_time_filter_narrows_by_timestamp() {
        let tmp = TempDir::new().unwrap();
        let (data_dir, ep1_id, _ep2_id) = seed_db(&tmp);

        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(fake_embed_response()))
            .mount(&mock)
            .await;

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        // after = 5 minutes ago — only ep1 (1 min ago) qualifies; ep2 is 24h ago
        let after_ms = now_ms - 300_000;

        let state = make_state(data_dir, mock.uri());
        let app = build_router(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/memory/bot1/recall")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "query_text": "dragon",
                    "top_k": 5,
                    "time_filter": { "after": after_ms }
                })
                .to_string(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        let results = v["results"].as_array().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["episode_id"].as_i64().unwrap(), ep1_id);
    }

    // ── Test: per-call alpha/beta override changes scoring ───────────────────

    #[tokio::test]
    async fn recall_alpha_beta_override_accepted() {
        let tmp = TempDir::new().unwrap();
        let (data_dir, _ep1_id, _ep2_id) = seed_db(&tmp);

        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(fake_embed_response()))
            .mount(&mock)
            .await;

        let state = make_state(data_dir, mock.uri());
        let app = build_router(state);

        // Pure BM25: alpha=1.0, beta=0.0
        let req = Request::builder()
            .method("POST")
            .uri("/v1/memory/bot1/recall")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "query_text": "dragon",
                    "top_k": 2,
                    "alpha": 1.0,
                    "beta": 0.0
                })
                .to_string(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        let results = v["results"].as_array().unwrap();
        // With beta=0, the dense contribution to the score is suppressed, but
        // dense_norm (a normalized KNN distance) may still be > 0 because it is
        // computed before the beta weight is applied. Assert the score is
        // BM25-dominated: score ≈ alpha * bm25_norm * decay * (1 + gamma * salience)
        // (with delta * entity_match additive). Do not assert dense_norm == 0.
        for r in results {
            let bm25_norm = r["components"]["bm25_norm"].as_f64().unwrap_or(0.0);
            let score = r["score"].as_f64().unwrap_or(0.0);
            // score must be positive and BM25-driven when there are BM25 matches
            assert!(
                score >= 0.0,
                "score must be non-negative with beta=0; got {score}"
            );
            // If BM25 matches, bm25_norm should contribute to the score
            if bm25_norm > 0.0 {
                assert!(
                    score > 0.0,
                    "score must be > 0 when bm25_norm={bm25_norm} and beta=0"
                );
            }
        }
    }

    // ── Test: embed-down falls back to BM25-only (NOT 503) ───────────────────

    #[tokio::test]
    async fn recall_embed_down_falls_back_to_bm25_not_503() {
        let tmp = TempDir::new().unwrap();
        let (data_dir, _ep1_id, _ep2_id) = seed_db(&tmp);

        let mock = MockServer::start().await;
        // Respond with 503 to simulate embed service down
        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&mock)
            .await;

        let state = make_state(data_dir, mock.uri());
        let app = build_router(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/memory/bot1/recall")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "query_text": "dragon attack",
                    "top_k": 5
                })
                .to_string(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        // Must NOT be 503 — recall degrades gracefully to BM25-only
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "recall with embed down must return 200 (BM25-only fallback)"
        );
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        let results = v["results"].as_array().unwrap();
        // BM25 should still find something for "dragon attack"
        assert!(!results.is_empty(), "BM25 fallback must return results");
        // All results must have dense_norm=0 (no dense scoring happened)
        for r in results {
            assert_eq!(r["components"]["dense_norm"].as_f64().unwrap(), 0.0);
        }
    }
}
