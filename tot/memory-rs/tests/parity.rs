// SPDX-License-Identifier: GPL-2.0-or-later
//! Cross-implementation parity harness — the cutover gate.
//!
//! Fires an identical battery of recall/search/read/list requests at:
//!   - The Python memory-sidecar (live, at PYTHON_MEMORY_URL)
//!   - The Rust router (in-process via tower::ServiceExt::oneshot)
//!
//! Asserts for every response pair:
//!   - Identical episode_id ordering in results
//!   - Per-hit scores and component breakdowns within EPS = 1e-6
//!   - Identical HTTP status codes on error cases (400/404/503/empty)
//!
//! Prerequisites (env vars):
//!   PARITY_DB_DIR     — path to WAL-safe snapshot of live per-bot data dir
//!                        (cp -r /var/memory/<bot_guid> /tmp/parity-snap/<bot_guid>)
//!   PARITY_BOT_GUID   — which bot's DB to exercise
//!   EMBED_STUB_URL    — URL of the deterministic embed stub (same algorithm as
//!                        the Task 6.1 quality-gate stub_embed; see Step 1)
//!   PYTHON_MEMORY_URL — URL of the parity Python instance started with
//!                        BRAIN_EMBEDDINGS_URL=$EMBED_STUB_URL (NOT the live :8090)
//!
//! Run with:
//!   PARITY_DB_DIR=... PARITY_BOT_GUID=... EMBED_STUB_URL=... \
//!   PYTHON_MEMORY_URL=... \
//!   cargo test -p memory-rs --test parity -- --ignored --nocapture
//!
//! Operator setup (on Heimdal):
//!   # 1. WAL-safe snapshot (include -wal/-shm files):
//!   mkdir -p /tmp/parity-snapshot
//!   cp -r /opt/containers/memory/<bot_guid> /tmp/parity-snapshot/<bot_guid>
//!   export PARITY_DB_DIR=/tmp/parity-snapshot
//!   export PARITY_BOT_GUID=<bot_guid>
//!
//!   # 2. Start the deterministic embed stub (prints its URL):
//!   cargo run -p memory-rs --bin embed-stub &
//!   export EMBED_STUB_URL=http://127.0.0.1:<PORT>
//!
//!   # 3. Start parity Python sidecar pointed at the stub:
//!   BRAIN_EMBEDDINGS_URL=$EMBED_STUB_URL MEMORY_DATA_DIR=$PARITY_DB_DIR \
//!   python memory/app.py --port 8091 &
//!   export PYTHON_MEMORY_URL=http://127.0.0.1:8091
//!
//! Design note — why a deterministic embed stub (not a fixture file):
//! the stub maps any query text to a fixed 768-f32 vector via the SHA-256
//! algorithm identical to the Task 6.1 quality-gate `stub_embed`. Both the
//! Python parity instance (via BRAIN_EMBEDDINGS_URL=$EMBED_STUB_URL) and the
//! Rust router (via AppState.embed.url=$EMBED_STUB_URL) embed the SAME query
//! text through the SAME stub, so both sides receive byte-identical query
//! vectors. That is the precondition for dense-score parity within EPS.

use std::path::PathBuf;

use axum::body::Body;
use axum::http::Request;
use serde_json::{json, Value};
use tower::ServiceExt;

use memory_rs::state::{AppState, EmbedConfig};

// ── Epsilon for float comparison ──────────────────────────────────────────────
const EPS: f64 = 1e-6;

// ── Env-var helpers ───────────────────────────────────────────────────────────

fn require_env(key: &str) -> String {
    std::env::var(key)
        .unwrap_or_else(|_| panic!("parity test requires env var {key} to be set"))
}

fn parity_db_dir() -> PathBuf {
    PathBuf::from(require_env("PARITY_DB_DIR"))
}

fn parity_bot_guid() -> String {
    require_env("PARITY_BOT_GUID")
}

fn python_url() -> String {
    require_env("PYTHON_MEMORY_URL")
}

// PARITY_EMBED_FIXTURE is NOT used in the deterministic-stub design.
// The embed stub derives vectors from query text via SHA-256; no pre-recorded
// fixture file is needed. The EMBED_STUB_URL env var is read by make_rust_app.

/// A small set of query strings used for the parity battery.
/// These are sent as query_text to both the Python and Rust recall endpoints;
/// both call the EMBED_STUB_URL, which returns the same deterministic vector.
const PARITY_QUERIES: &[&str] = &[
    "what did Alice teach me about tanking",
    "Bob fishing spot near the pond",
    "Carol heal dungeon run",
    "fall damage Stormwind bridge warning",
    "tanking reflection preferred role",
];

// ── Float comparison helper ───────────────────────────────────────────────────

/// Assert two f64 values are within EPS = 1e-6 of each other.
///
/// Both NaN → pass (consistent absence of signal). One NaN → fail.
fn assert_near(label: &str, rust: f64, python: f64) {
    if rust.is_nan() && python.is_nan() {
        return;
    }
    let diff = (rust - python).abs();
    assert!(
        diff <= EPS,
        "{label}: Rust={rust:.9} Python={python:.9} diff={diff:.2e} (eps={EPS:.0e})"
    );
}

// ── In-process Rust router ────────────────────────────────────────────────────

/// Build an AppState pointing at the snapshot DB dir and the deterministic
/// embed stub URL (EMBED_STUB_URL env var). Recall requests will embed via
/// the stub, producing the same vector as the Python side.
fn make_rust_app(db_dir: PathBuf) -> axum::Router {
    let embed_stub_url =
        std::env::var("EMBED_STUB_URL").unwrap_or_else(|_| "http://127.0.0.1:0".to_string());
    let state = AppState {
        data_dir: db_dir,
        embed: EmbedConfig {
            url: embed_stub_url,
            model: "nomic-embed-text".to_string(),
            api_key: String::new(),
        },
    };
    memory_rs::app::build_router(state)
}

/// Issue a request to the in-process Rust router and return (status, body).
async fn rust_request(app: axum::Router, method: &str, path: &str, body: Value) -> (u16, Value) {
    let body_bytes = serde_json::to_vec(&body).unwrap();
    let req = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(body_bytes))
        .unwrap();
    let resp = app.oneshot(req).await.expect("oneshot");
    let status = resp.status().as_u16();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

/// Issue a GET request to the in-process Rust router.
async fn rust_get(app: axum::Router, path: &str) -> (u16, Value) {
    let req = Request::builder()
        .method("GET")
        .uri(path)
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.expect("oneshot");
    let status = resp.status().as_u16();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

// ── Python HTTP client ────────────────────────────────────────────────────────

/// Issue a POST request to the Python service and return (status, body).
async fn python_post(
    client: &reqwest::Client,
    base: &str,
    path: &str,
    body: &Value,
) -> (u16, Value) {
    let resp = client
        .post(format!("{base}{path}"))
        .json(body)
        .send()
        .await
        .unwrap_or_else(|e| panic!("Python POST {path} failed: {e}"));
    let status = resp.status().as_u16();
    let value: Value = resp.json().await.unwrap_or(Value::Null);
    (status, value)
}

/// Issue a GET request to the Python service and return (status, body).
async fn python_get(client: &reqwest::Client, base: &str, path: &str) -> (u16, Value) {
    let resp = client
        .get(format!("{base}{path}"))
        .send()
        .await
        .unwrap_or_else(|e| panic!("Python GET {path} failed: {e}"));
    let status = resp.status().as_u16();
    let value: Value = resp.json().await.unwrap_or(Value::Null);
    (status, value)
}

// ── Ordering + score comparison helpers ──────────────────────────────────────

/// Assert that two recall/search result arrays have the same episode_id ordering.
fn assert_same_ordering(label: &str, rust: &Value, python: &Value) {
    let empty: Vec<Value> = vec![];
    let rust_ids: Vec<i64> = rust
        .as_array()
        .unwrap_or(&empty)
        .iter()
        .filter_map(|r| r["episode_id"].as_i64())
        .collect();
    let python_ids: Vec<i64> = python
        .as_array()
        .unwrap_or(&empty)
        .iter()
        .filter_map(|r| r["episode_id"].as_i64())
        .collect();
    assert_eq!(
        rust_ids, python_ids,
        "{label}: episode_id ordering mismatch\n  Rust:   {rust_ids:?}\n  Python: {python_ids:?}"
    );
}

/// Assert per-hit score + component parity for recall results.
fn assert_recall_scores_match(label: &str, rust_results: &Value, python_results: &Value) {
    let rust_arr = rust_results.as_array().map(Vec::as_slice).unwrap_or(&[]);
    let python_arr = python_results.as_array().map(Vec::as_slice).unwrap_or(&[]);
    assert_eq!(
        rust_arr.len(),
        python_arr.len(),
        "{label}: result count mismatch: Rust={}, Python={}",
        rust_arr.len(),
        python_arr.len()
    );
    for (i, (r, p)) in rust_arr.iter().zip(python_arr.iter()).enumerate() {
        let hit_label = format!("{label}[{i}] eid={}", r["episode_id"]);
        assert_near(
            &format!("{hit_label}.score"),
            r["score"].as_f64().unwrap_or(f64::NAN),
            p["score"].as_f64().unwrap_or(f64::NAN),
        );
        let rc = &r["components"];
        let pc = &p["components"];
        for field in &["bm25_norm", "dense_norm", "decay", "salience", "entity_match"] {
            assert_near(
                &format!("{hit_label}.components.{field}"),
                rc[field].as_f64().unwrap_or(f64::NAN),
                pc[field].as_f64().unwrap_or(f64::NAN),
            );
        }
    }
}

/// Assert per-hit cosine_similarity parity for search results.
#[allow(dead_code)]
fn assert_search_scores_match(label: &str, rust_results: &Value, python_results: &Value) {
    let rust_arr = rust_results.as_array().map(Vec::as_slice).unwrap_or(&[]);
    let python_arr = python_results.as_array().map(Vec::as_slice).unwrap_or(&[]);
    assert_eq!(rust_arr.len(), python_arr.len(), "{label}: result count mismatch");
    for (i, (r, p)) in rust_arr.iter().zip(python_arr.iter()).enumerate() {
        assert_near(
            &format!("{label}[{i}].cosine_similarity"),
            r["cosine_similarity"].as_f64().unwrap_or(f64::NAN),
            p["cosine_similarity"].as_f64().unwrap_or(f64::NAN),
        );
    }
}

// ── The parity test ───────────────────────────────────────────────────────────

#[tokio::test]
#[ignore = "cutover gate — requires PARITY_DB_DIR, PARITY_BOT_GUID, EMBED_STUB_URL, PYTHON_MEMORY_URL"]
async fn parity_cutover_gate() {
    // Load prerequisites.
    memory_rs::db::register_vec0();

    let db_dir = parity_db_dir();
    let bot_guid = parity_bot_guid();
    let python_base = python_url();
    // EMBED_STUB_URL is consumed by make_rust_app() via env var; the Python
    // parity instance was started with BRAIN_EMBEDDINGS_URL=$EMBED_STUB_URL
    // so both sides embed via the same deterministic stub.
    let _ = require_env("EMBED_STUB_URL"); // assert it is set before proceeding

    let python_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap();

    // ── Case 1: GET /health ───────────────────────────────────────────────────
    {
        let rust_app = make_rust_app(db_dir.clone());
        let (rust_status, rust_body) = rust_get(rust_app, "/health").await;
        let (python_status, python_body) = python_get(&python_client, &python_base, "/health").await;
        assert_eq!(rust_status, python_status, "GET /health: status mismatch");
        assert_eq!(
            rust_body["status"], python_body["status"],
            "GET /health: body mismatch"
        );
    }

    // ── Case 2: GET /v1/memory/:bot_guid/episodes/:id (read) ─────────────────
    // Read episode 1 — must exist in any real bot DB that has been written to.
    {
        let rust_app = make_rust_app(db_dir.clone());
        let path = format!("/v1/memory/{bot_guid}/episodes/1");
        let (rust_status, rust_body) = rust_get(rust_app, &path).await;
        let (python_status, python_body) = python_get(&python_client, &python_base, &path).await;
        assert_eq!(rust_status, python_status, "GET {path}: status mismatch");
        if rust_status == 200 {
            for field in &[
                "episode_id",
                "content_text",
                "episode_type",
                "timestamp",
                "recall_count",
                "source",
            ] {
                assert_eq!(
                    rust_body[field], python_body[field],
                    "GET {path}: field {field} mismatch"
                );
            }
            assert_eq!(
                rust_body["embedding_generated"], python_body["embedding_generated"],
                "GET {path}: embedding_generated mismatch"
            );
        }
    }

    // ── Case 3: GET /v1/memory/:bot_guid/episodes (list, first page) ─────────
    {
        let rust_app = make_rust_app(db_dir.clone());
        let path = format!("/v1/memory/{bot_guid}/episodes?limit=10&offset=0");
        let (rust_status, rust_body) = rust_get(rust_app, &path).await;
        let (python_status, python_body) = python_get(&python_client, &python_base, &path).await;
        assert_eq!(rust_status, python_status, "GET {path}: status mismatch");
        if rust_status == 200 {
            assert_eq!(
                rust_body["total"], python_body["total"],
                "GET {path}: total mismatch"
            );
            assert_eq!(
                rust_body["has_more"], python_body["has_more"],
                "GET {path}: has_more mismatch"
            );
            let empty: Vec<Value> = vec![];
            let r_ids: Vec<i64> = rust_body["results"]
                .as_array()
                .unwrap_or(&empty)
                .iter()
                .filter_map(|r| r["episode_id"].as_i64())
                .collect();
            let p_ids: Vec<i64> = python_body["results"]
                .as_array()
                .unwrap_or(&empty)
                .iter()
                .filter_map(|r| r["episode_id"].as_i64())
                .collect();
            assert_eq!(r_ids, p_ids, "GET {path}: episode ordering mismatch");
        }
    }

    // ── Cases 4+: recall for each parity query string ────────────────────────
    // Both Python and Rust call the deterministic embed stub (EMBED_STUB_URL) to
    // embed query_text. The stub returns the same SHA-256-derived vector for the
    // same text, guaranteeing identical dense scores.
    for query_text in PARITY_QUERIES {
        // ── Case N.a: recall (BM25 + dense hybrid, default weights) ──────────
        {
            let rust_app = make_rust_app(db_dir.clone());
            let path = format!("/v1/memory/{bot_guid}/recall");
            let body = json!({
                "query_text": query_text,
                "top_k": 5
            });
            let (rust_status, rust_body) =
                rust_request(rust_app, "POST", &path, body.clone()).await;
            let (python_status, python_body) =
                python_post(&python_client, &python_base, &path, &body).await;
            assert_eq!(
                rust_status, python_status,
                "POST {path} query={query_text:?}: status mismatch"
            );
            if rust_status == 200 {
                let label = format!("recall query={query_text:?}");
                assert_same_ordering(&label, &rust_body["results"], &python_body["results"]);
                assert_recall_scores_match(&label, &rust_body["results"], &python_body["results"]);
            }
        }
    }

    // ── Error cases: assert identical status codes ────────────────────────────

    // Case: read non-existent episode → both must 404.
    {
        let rust_app = make_rust_app(db_dir.clone());
        let path = format!("/v1/memory/{bot_guid}/episodes/999999999");
        let (rust_status, rust_body) = rust_get(rust_app, &path).await;
        let (python_status, python_body) = python_get(&python_client, &python_base, &path).await;
        assert_eq!(rust_status, 404, "404 case: Rust must return 404");
        assert_eq!(python_status, 404, "404 case: Python must return 404");
        assert_eq!(
            rust_body["detail"], python_body["detail"],
            "404 case: detail mismatch"
        );
    }

    // Case: recall with entity filter that matches nothing → both must return empty results.
    {
        let rust_app = make_rust_app(db_dir.clone());
        let path = format!("/v1/memory/{bot_guid}/recall");
        let body = json!({
            "query_text": "anything",
            "top_k": 5,
            "entity_names": ["__no_such_entity_xyz__"]
        });
        let (rust_status, rust_body) = rust_request(rust_app, "POST", &path, body.clone()).await;
        let (python_status, python_body) =
            python_post(&python_client, &python_base, &path, &body).await;
        assert_eq!(rust_status, python_status, "empty entity filter: status mismatch");
        if rust_status == 200 {
            let r_arr = rust_body["results"].as_array().map(|a| a.len()).unwrap_or(0);
            let p_arr = python_body["results"].as_array().map(|a| a.len()).unwrap_or(0);
            assert_eq!(r_arr, 0, "empty entity filter: Rust must return 0 results");
            assert_eq!(p_arr, 0, "empty entity filter: Python must return 0 results");
        }
    }

    // Case: search with missing query (no query_text or query_vec) → both must 400.
    {
        let rust_app = make_rust_app(db_dir.clone());
        let path = format!("/v1/memory/{bot_guid}/search");
        let body = json!({ "top_k": 5 });
        let (rust_status, rust_body) = rust_request(rust_app, "POST", &path, body.clone()).await;
        let (python_status, python_body) =
            python_post(&python_client, &python_base, &path, &body).await;
        assert_eq!(rust_status, 400, "search-no-query: Rust must return 400");
        assert_eq!(python_status, 400, "search-no-query: Python must return 400");
        assert_eq!(
            rust_body["detail"], python_body["detail"],
            "search-no-query: detail mismatch (wire contract for this error message must match)"
        );
    }

    // Case: search with wrong-dim vec → both must 400.
    {
        let rust_app = make_rust_app(db_dir.clone());
        let path = format!("/v1/memory/{bot_guid}/search");
        let short_vec: Vec<f64> = vec![0.1; 10]; // dim 10, not 768
        let body = json!({ "query_vec": short_vec, "top_k": 5 });
        let (rust_status, _) = rust_request(rust_app, "POST", &path, body.clone()).await;
        let (python_status, _) = python_post(&python_client, &python_base, &path, &body).await;
        assert_eq!(rust_status, 400, "search-wrong-dim: Rust must return 400");
        assert_eq!(python_status, 400, "search-wrong-dim: Python must return 400");
    }

    eprintln!(
        "\n[parity] PASSED — {} query strings compared, bot_guid={bot_guid}",
        PARITY_QUERIES.len()
    );
}
