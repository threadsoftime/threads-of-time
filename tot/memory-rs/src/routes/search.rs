// src/routes/search.rs
//! POST /v1/memory/:bot_guid/search — memory.search route.
//!
//! Parity with `tot_memory/routes/search.py` (design subspec §10.4 / plan abbreviation).
//!
//! Pure nearest-neighbour over `embeddings_vec` — **no** BM25, **no** decay,
//! **no** salience boost, **no** entity filter, **no** `recall_count` /
//! `last_recalled_at` bump.
//!
//! Distinct from `memory.recall` (§10.3) in two ways:
//!  1. No hybrid scoring — dense-only.
//!  2. **Loud 503** on embed failure (query_text path): unlike recall's
//!     graceful BM25-only fallback, this route surfaces the embed error.
//!
//! Request: `{ query_text? | query_vec?, top_k? }` — exactly one of
//! `query_text` / `query_vec` is required. `query_vec` must be `EMBEDDING_DIM`
//! floats. `top_k` defaults to 10; range 1..=100.
//!
//! Response: `{ results: [{ episode_id, content_text, cosine_similarity }] }`

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};

use crate::{
    EMBEDDING_DIM,
    db::{migrate::run_migrations, open_bot_db},
    embeddings::EmbeddingsClient,
    error::AppError,
    retrieval::dense::dense_search,
    state::AppState,
};

// ── Request / response types ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SearchRequest {
    pub query_text: Option<String>,
    pub query_vec: Option<Vec<f32>>,
    pub top_k: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct SearchHit {
    pub episode_id: i64,
    pub content_text: String,
    pub cosine_similarity: f64,
}

#[derive(Debug, Serialize)]
pub struct SearchResponse {
    pub results: Vec<SearchHit>,
}

// ── Handler ───────────────────────────────────────────────────────────────────

pub async fn search_handler(
    State(st): State<AppState>,
    Path(bot_guid): Path<String>,
    Json(body): Json<SearchRequest>,
) -> Result<(StatusCode, Json<SearchResponse>), AppError> {
    // Validate: exactly one of query_text or query_vec must be present.
    if body.query_text.is_none() && body.query_vec.is_none() {
        return Err(AppError::BadRequest(
            "one of query_text or query_vec is required".to_string(),
        ));
    }

    let top_k = body.top_k.unwrap_or(10);
    if top_k < 1 || top_k > 100 {
        return Err(AppError::BadRequest("top_k must be 1..100".to_string()));
    }

    // Validate query_text length if present.
    if let Some(ref qt) = body.query_text {
        if qt.is_empty() || qt.chars().count() > 500 {
            return Err(AppError::BadRequest(
                "query_text must be 1..500 characters".to_string(),
            ));
        }
    }

    // Resolve the query vector (either pre-supplied or embedded).
    let query_vec: Vec<f32> = if let Some(provided_vec) = body.query_vec {
        if provided_vec.len() != EMBEDDING_DIM {
            return Err(AppError::BadRequest(format!(
                "query_vec dimension {} does not match EMBEDDING_DIM {EMBEDDING_DIM}",
                provided_vec.len()
            )));
        }
        provided_vec
    } else {
        // query_text is Some here (validated above).
        let text = body.query_text.unwrap();
        let client = EmbeddingsClient::new(&st.embed.url, &st.embed.model, &st.embed.api_key);
        match client.embed(&text).await {
            Ok(v) => v,
            Err(e) => {
                // LOUD — unlike recall's graceful BM25-only fallback, search
                // surfaces the error as 503 (§10.4 distinction).
                tracing::warn!(bot_guid = %bot_guid, error = %e, "embed failed during search");
                return Err(AppError::EmbeddingUnavailable);
            }
        }
    };

    let data_dir = st.data_dir.clone();

    let hits = tokio::task::spawn_blocking(move || -> Result<Vec<SearchHit>, AppError> {
        let conn = open_bot_db(&data_dir, &bot_guid)?;
        run_migrations(&conn)?;

        let dense_hits = dense_search(&conn, &query_vec, top_k)?;
        if dense_hits.is_empty() {
            return Ok(vec![]);
        }

        // Hydrate content_text for returned episode ids.
        let ids: Vec<i64> = dense_hits.iter().map(|h| h.episode_id).collect();
        let placeholders: String = ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT episode_id, content_text FROM episodes WHERE episode_id IN ({placeholders})"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows: std::collections::HashMap<i64, String> = stmt
            .query_map(rusqlite::params_from_iter(ids.iter()), |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .filter_map(|r| r.ok())
            .collect();

        // Build output in dense_hits order (already sorted by cosine_similarity desc).
        // No side effects — recall_count / last_recalled_at are NOT touched.
        let out: Vec<SearchHit> = dense_hits
            .iter()
            .filter_map(|h| {
                rows.get(&h.episode_id).map(|content_text| SearchHit {
                    episode_id: h.episode_id,
                    content_text: content_text.clone(),
                    cosine_similarity: h.cosine_similarity,
                })
            })
            .collect();
        Ok(out)
    })
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("spawn_blocking join error: {e}")))??;

    Ok((StatusCode::OK, Json(SearchResponse { results: hits })))
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

    fn fake_embed_response() -> Value {
        let vec: Vec<f64> = (0..768).map(|i| (i as f64) / 768.0).collect();
        serde_json::json!({ "data": [{ "embedding": vec }] })
    }

    fn seed_db_with_embedding(dir: &TempDir) -> (PathBuf, i64) {
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
            rusqlite::params!["warrior charges into battle", "combat", now_ms - 1000, 0.7, "self"],
        )
        .unwrap();
        let ep_id = conn.last_insert_rowid();

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

    // ── query_text path: embed + dense search ─────────────────────────────────

    #[tokio::test]
    async fn search_query_text_returns_200_with_results() {
        let tmp = TempDir::new().unwrap();
        let (data_dir, ep_id) = seed_db_with_embedding(&tmp);

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
            .uri("/v1/memory/bot1/search")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({ "query_text": "warrior charges", "top_k": 5 }).to_string(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 16384).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        let results = v["results"].as_array().unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0]["episode_id"].as_i64().unwrap(), ep_id);
        assert!(results[0]["cosine_similarity"].as_f64().is_some());
        assert!(results[0]["content_text"].as_str().is_some());
    }

    // ── query_vec path: caller-provided pre-computed vector ───────────────────

    #[tokio::test]
    async fn search_query_vec_returns_200_no_embed_call() {
        let tmp = TempDir::new().unwrap();
        let (data_dir, ep_id) = seed_db_with_embedding(&tmp);

        // No mock server — embed endpoint must NOT be called
        let state = make_state(
            data_dir,
            "http://127.0.0.1:19999".to_string(), // unreachable port
        );
        let app = build_router(state);

        let vec: Vec<f64> = (0..768_usize).map(|i| (i as f64) / 768.0).collect();

        let req = Request::builder()
            .method("POST")
            .uri("/v1/memory/bot1/search")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({ "query_vec": vec, "top_k": 5 }).to_string(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 16384).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        let results = v["results"].as_array().unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0]["episode_id"].as_i64().unwrap(), ep_id);
    }

    // ── neither query_text nor query_vec -> 400 ───────────────────────────────

    #[tokio::test]
    async fn search_neither_text_nor_vec_returns_400() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().to_path_buf();
        let state = make_state(data_dir, "http://127.0.0.1:19999".to_string());
        let app = build_router(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/memory/bot1/search")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({ "top_k": 5 }).to_string()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            v["detail"].as_str().unwrap(),
            "one of query_text or query_vec is required"
        );
    }

    // ── query_vec with wrong dimension -> 400 ────────────────────────────────

    #[tokio::test]
    async fn search_wrong_dim_returns_400() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().to_path_buf();
        let state = make_state(data_dir, "http://127.0.0.1:19999".to_string());
        let app = build_router(state);

        let bad_vec: Vec<f64> = vec![0.1; 100]; // wrong dim

        let req = Request::builder()
            .method("POST")
            .uri("/v1/memory/bot1/search")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({ "query_vec": bad_vec, "top_k": 5 }).to_string(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // ── embed fail on query_text -> 503 (loud, unlike recall) ────────────────

    #[tokio::test]
    async fn search_embed_fail_returns_503() {
        let tmp = TempDir::new().unwrap();
        let (data_dir, _) = seed_db_with_embedding(&tmp);

        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/embeddings"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&mock)
            .await;

        let state = make_state(data_dir, mock.uri());
        let app = build_router(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/memory/bot1/search")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({ "query_text": "warrior charges", "top_k": 5 }).to_string(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["detail"].as_str().unwrap(), "embedding_service_unavailable");
    }

    // ── search has NO side effects: recall_count must not change ─────────────

    #[tokio::test]
    async fn search_no_side_effects_recall_count_unchanged() {
        let tmp = TempDir::new().unwrap();
        let (data_dir, ep_id) = seed_db_with_embedding(&tmp);

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
            .uri("/v1/memory/bot1/search")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({ "query_text": "warrior charges", "top_k": 5 }).to_string(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Check recall_count is still 0
        let conn = open_bot_db(&data_dir, "bot1").unwrap();
        run_migrations(&conn).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT recall_count FROM episodes WHERE episode_id = ?1",
                rusqlite::params![ep_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "search must not bump recall_count");
    }

    // ── dense-only ordering: closest vector ranks first ───────────────────────

    #[tokio::test]
    async fn search_dense_orders_by_cosine_similarity_desc() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().to_path_buf();
        let conn = open_bot_db(&data_dir, "bot1").unwrap();
        run_migrations(&conn).unwrap();

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        // Two episodes with known vectors; query = vec_close matches ep1
        let vec_close: Vec<f32> = vec![1.0f32; 768]; // all-ones
        let vec_far: Vec<f32> = vec![-1.0f32; 768]; // all-neg-ones

        // Normalize both for cosine (vec0 computes cosine distance on L2-normalized vecs)
        let norm = (768.0f32).sqrt();
        let vec_close_norm: Vec<f32> = vec_close.iter().map(|x| x / norm).collect();
        let vec_far_norm: Vec<f32> = vec_far.iter().map(|x| x / norm).collect();

        for (i, vec) in [&vec_close_norm, &vec_far_norm].iter().enumerate() {
            conn.execute(
                "INSERT INTO episodes (content_text, episode_type, timestamp, salience_score, source)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    format!("episode {i}"),
                    "chat",
                    now_ms - (i as i64 * 1000),
                    0.5,
                    "self"
                ],
            )
            .unwrap();
            let ep_id = conn.last_insert_rowid();
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
        }

        // Query with vec_close_norm: ep1 should rank first
        let state = make_state(
            data_dir,
            "http://127.0.0.1:19999".to_string(), // no embed needed
        );
        let app = build_router(state);

        let query_vec: Vec<f64> = vec_close_norm.iter().map(|x| *x as f64).collect();

        let req = Request::builder()
            .method("POST")
            .uri("/v1/memory/bot1/search")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({ "query_vec": query_vec, "top_k": 2 }).to_string(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 16384).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        let results = v["results"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        // First result must have higher cosine_similarity
        let sim0 = results[0]["cosine_similarity"].as_f64().unwrap();
        let sim1 = results[1]["cosine_similarity"].as_f64().unwrap();
        assert!(
            sim0 >= sim1,
            "results must be ordered by cosine_similarity desc: {sim0} >= {sim1}"
        );
    }
}
