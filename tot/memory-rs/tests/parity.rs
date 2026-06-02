// SPDX-License-Identifier: GPL-2.0-or-later
//! Cross-implementation parity harness — the v0.2.1 cutover gate.
//!
//! Fires an identical battery of recall / search / recall_about requests at:
//!   - The live Python memory-sidecar v0.2.1  (via `PYTHON_MEMORY_URL`)
//!   - The Rust router built from this crate   (in-process, tower::oneshot)
//!
//! Both sides call the **same** deterministic embed stub (`EMBED_STUB_URL`),
//! so identical query text → identical query vector → scores differ by at most
//! floating-point rounding (EPS = 1e-6).
//!
//! # Write isolation
//!
//! `recall` and `recall_about` bump `last_recalled_ts` in `memories`.  To
//! prevent file-lock contention between the two readers, the Rust side works on
//! a private copy of the snapshot (`PARITY_DB_PATH` + ".rust").  The Python
//! instance uses its own copy (the operator sets it up before running 7.2).
//! Because `recency_basis` defaults to `Created`, `last_recalled_ts` bumps do
//! NOT affect scores — isolation is still required to avoid SQLite write-lock
//! errors when both sides open the same file simultaneously.
//!
//! # Test population
//!
//! Derived at runtime from the snapshot: the top-N bots by memory count.
//! No bot_ids are hardcoded.
//!
//! # Env vars (all required at runtime; none needed to compile or list)
//!
//! | Variable             | Purpose                                               |
//! |----------------------|-------------------------------------------------------|
//! | `PARITY_DB_PATH`     | Path to a single-file SQLite snapshot (`db.sqlite`)   |
//! | `PYTHON_MEMORY_URL`  | Base URL of a live Python v0.2.1 instance (no trailing `/`) |
//! | `EMBED_STUB_URL`     | Base URL of the deterministic embed stub              |
//!
//! # Running (operator)
//!
//! ```sh
//! # 1. Copy snapshot so each side has its own writable copy:
//! cp /opt/containers/memory/db.sqlite /tmp/parity-snap.sqlite
//! cp /tmp/parity-snap.sqlite /tmp/parity-snap.sqlite.rust  # Rust copy
//! cp /tmp/parity-snap.sqlite /tmp/parity-snap.sqlite.py   # Python copy
//!
//! # 2. Start the deterministic embed stub:
//! cargo run -p memory-rs --bin embed-stub &
//! export EMBED_STUB_URL=http://127.0.0.1:<PORT>
//!
//! # 3. Start the Python parity instance (pointed at its own copy + stub):
//! MEM_DB_PATH=/tmp/parity-snap.sqlite.py \
//! MEM_EMBED_ENDPOINT=$EMBED_STUB_URL     \
//! python -m memory_sidecar --port 8091 &
//! export PYTHON_MEMORY_URL=http://127.0.0.1:8091
//!
//! # 4. Set the path to the snapshot (Rust side will derive its copy path):
//! export PARITY_DB_PATH=/tmp/parity-snap.sqlite
//!
//! # 5. Run the gate:
//! CARGO_BUILD_JOBS=4 cargo test -p memory-rs --test parity \
//!   -- --ignored --nocapture
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use serde_json::{json, Value};
use tower::ServiceExt; // oneshot

use memory_rs::{
    app::build_router,
    config::ScoringWeights,
    core::MemoryService,
    db::{self, migrate},
    embed_cache::EmbedCache,
    embeddings::EmbeddingsClient,
    pubsub::PubSub,
    state::AppState,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Float comparison tolerance.
const EPS: f64 = 1e-6;

/// Number of top bots (by memory count) to exercise.
const TOP_BOTS: usize = 8;

/// Generic recall / search query strings fired per bot.
const RECALL_QUERIES: &[&str] = &[
    "what happened recently",
    "group plans",
    "Stormwind",
    "the dungeon run",
    "trade",
];

/// Search query strings (same style; a separate constant for clarity).
const SEARCH_QUERIES: &[&str] = &[
    "what happened recently",
    "group plans",
    "Stormwind",
    "the dungeon run",
    "combat encounter",
];

// ---------------------------------------------------------------------------
// Env-var helpers (panic clearly when a required var is absent)
// ---------------------------------------------------------------------------

fn require_env(key: &str) -> String {
    std::env::var(key)
        .unwrap_or_else(|_| panic!("parity test requires env var {key} to be set"))
}

/// Path to the read-only snapshot file.
fn snapshot_path() -> PathBuf {
    PathBuf::from(require_env("PARITY_DB_PATH"))
}

/// Path where the Rust side's private writable copy lives.
fn rust_db_path(snapshot: &PathBuf) -> PathBuf {
    let mut p = snapshot.as_os_str().to_owned();
    p.push(".rust");
    PathBuf::from(p)
}

fn python_base_url() -> String {
    require_env("PYTHON_MEMORY_URL")
}

fn embed_stub_url() -> String {
    require_env("EMBED_STUB_URL")
}

// ---------------------------------------------------------------------------
// Snapshot helpers (rusqlite, read-only access to the ORIGINAL snapshot)
// ---------------------------------------------------------------------------

/// Return the top `n` bot_ids ordered by descending memory count.
fn pick_bots(snapshot: &PathBuf, n: usize) -> Vec<String> {
    let conn = rusqlite::Connection::open_with_flags(
        snapshot,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("open snapshot read-only");
    let mut stmt = conn
        .prepare(
            "SELECT bot_id, COUNT(*) AS cnt \
             FROM memories \
             GROUP BY bot_id \
             ORDER BY cnt DESC \
             LIMIT ?",
        )
        .expect("prepare pick_bots");
    stmt.query_map([n as i64], |row| row.get::<_, String>(0))
        .expect("query_map")
        .map(|r| r.expect("row"))
        .collect()
}

/// Return up to 3 entity name_lower values for `bot_id` from the snapshot.
/// Used to build the recall_about battery.
fn pick_entities(snapshot: &PathBuf, bot_id: &str) -> Vec<String> {
    let conn = rusqlite::Connection::open_with_flags(
        snapshot,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("open snapshot read-only for entities");
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT name_lower \
             FROM entities \
             WHERE bot_id = ? AND name_lower IS NOT NULL AND name_lower != '' \
             LIMIT 3",
        )
        .expect("prepare pick_entities");
    stmt.query_map([bot_id], |row| row.get::<_, String>(0))
        .expect("query_map entities")
        .map(|r| r.expect("entity row"))
        .collect()
}

// ---------------------------------------------------------------------------
// Rust in-process router
// ---------------------------------------------------------------------------

/// Build an `AppState` backed by `db_path`, using the deterministic embed stub.
fn make_rust_state(db_path: PathBuf, embed_url: &str) -> AppState {
    db::register_vec0();
    // Run migrations so the copy is fully initialised (the snapshot was already
    // migrated in production; this is a no-op for all existing versions).
    {
        let conn = db::open_db(&db_path).expect("open rust db copy");
        migrate::run(
            &conn,
            std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/migrations")),
        )
        .expect("migrations on rust copy");
    }
    let client = EmbeddingsClient::new(embed_url, "nomic-embed-text", "");
    let embed = Arc::new(EmbedCache::new(client));
    let pubsub = Arc::new(PubSub::new());
    let weights = ScoringWeights {
        w_rel: 0.5,
        w_rec: 0.2,
        w_imp: 0.3,
        tau_seconds: 604_800,
    };
    let svc = Arc::new(MemoryService::new(
        db_path,
        weights,
        2_000,
        embed,
        pubsub.clone(),
    ));
    AppState::for_test(svc, pubsub)
}

/// Issue a POST request via tower::oneshot and return `(status, body)`.
async fn rust_post(state: AppState, path: &str, body: &Value) -> (u16, Value) {
    let app = build_router(state);
    let bytes = serde_json::to_vec(body).expect("serialize request body");
    let req = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(bytes))
        .expect("build request");
    let resp = app.oneshot(req).await.expect("oneshot");
    let status = resp.status().as_u16();
    let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let value: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    (status, value)
}

// ---------------------------------------------------------------------------
// Python HTTP client
// ---------------------------------------------------------------------------

/// Issue a POST request to the Python service and return `(status, body)`.
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

// ---------------------------------------------------------------------------
// Float comparison
// ---------------------------------------------------------------------------

/// Assert two `f64` values are within `EPS`.
///
/// Both NaN → pass (consistent absence of signal).
/// One NaN, other not → fail.
///
/// On mismatch, prints `query`, `bot`, and the numeric diff so failures are
/// immediately debuggable.
fn assert_near(label: &str, rust: f64, python: f64) {
    if rust.is_nan() && python.is_nan() {
        return;
    }
    let diff = (rust - python).abs();
    assert!(
        diff <= EPS,
        "[parity] FLOAT MISMATCH — {label}\n  Rust   = {rust:.9}\n  Python = {python:.9}\n  diff   = {diff:.2e}  (eps = {EPS:.0e})"
    );
}

// ---------------------------------------------------------------------------
// Per-side result extraction helpers
// ---------------------------------------------------------------------------

/// Extract an ordered list of `memory_id` strings from a recall response.
///
/// Python shape: `{"memories": [{"memory_id": "...", ...}]}`
fn recall_ids(body: &Value) -> Vec<String> {
    body["memories"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|r| r["memory_id"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Extract an ordered list of `memory_id` strings from a search response.
///
/// Python shape: `{"items": [{"memory_id": "...", ...}]}`
fn search_ids(body: &Value) -> Vec<String> {
    body["items"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|r| r["memory_id"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Assert identical memory_id ordering across a recall response pair.
fn assert_recall_order(label: &str, rust_body: &Value, python_body: &Value) {
    let r_ids = recall_ids(rust_body);
    let p_ids = recall_ids(python_body);
    assert_eq!(
        r_ids, p_ids,
        "[parity] ORDERING MISMATCH ({label})\n  Rust   = {r_ids:?}\n  Python = {p_ids:?}"
    );
}

/// Assert per-hit score parity across a recall response pair.
fn assert_recall_scores(label: &str, rust_body: &Value, python_body: &Value) {
    let empty: Vec<Value> = vec![];
    let rust_arr = rust_body["memories"].as_array().unwrap_or(&empty);
    let python_arr = python_body["memories"].as_array().unwrap_or(&empty);
    assert_eq!(
        rust_arr.len(),
        python_arr.len(),
        "[parity] COUNT MISMATCH ({label}): Rust={}, Python={}",
        rust_arr.len(),
        python_arr.len()
    );
    for (i, (r, p)) in rust_arr.iter().zip(python_arr.iter()).enumerate() {
        let hit = format!("{label}[{i}] memory_id={}", r["memory_id"].as_str().unwrap_or("?"));
        assert_near(
            &format!("{hit}.score"),
            r["score"].as_f64().unwrap_or(f64::NAN),
            p["score"].as_f64().unwrap_or(f64::NAN),
        );
    }
}

/// Assert identical memory_id ordering and per-hit signals across a search
/// response pair.
///
/// RRF scores are compared within EPS.  `signals.bm25_rank`,
/// `dense_rank`, and `entity_rank` (nullable integers) must match exactly.
fn assert_search_parity(label: &str, rust_body: &Value, python_body: &Value) {
    let r_ids = search_ids(rust_body);
    let p_ids = search_ids(python_body);
    assert_eq!(
        r_ids, p_ids,
        "[parity] SEARCH ORDERING MISMATCH ({label})\n  Rust   = {r_ids:?}\n  Python = {p_ids:?}"
    );

    let empty: Vec<Value> = vec![];
    let rust_arr = rust_body["items"].as_array().unwrap_or(&empty);
    let python_arr = python_body["items"].as_array().unwrap_or(&empty);
    for (i, (r, p)) in rust_arr.iter().zip(python_arr.iter()).enumerate() {
        let hit = format!("{label}[{i}] memory_id={}", r["memory_id"].as_str().unwrap_or("?"));

        // RRF score parity.
        assert_near(
            &format!("{hit}.score"),
            r["score"].as_f64().unwrap_or(f64::NAN),
            p["score"].as_f64().unwrap_or(f64::NAN),
        );

        // signals.*_rank: nullable integer — must match exactly.
        let rs = &r["signals"];
        let ps = &p["signals"];
        for field in &["bm25_rank", "dense_rank", "entity_rank"] {
            assert_eq!(
                rs[field], ps[field],
                "[parity] SIGNALS MISMATCH — {hit}.signals.{field}\n  Rust   = {}\n  Python = {}",
                rs[field], ps[field]
            );
        }
    }
}

// ---------------------------------------------------------------------------
// THE PARITY GATE
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "cutover gate — requires PARITY_DB_PATH, PYTHON_MEMORY_URL, EMBED_STUB_URL"]
async fn parity_cutover_gate() {
    // ── 0. Read env vars ─────────────────────────────────────────────────────
    let snapshot = snapshot_path();
    let rust_db  = rust_db_path(&snapshot);
    let py_base  = python_base_url();
    let stub_url = embed_stub_url();

    // ── 1. Copy snapshot to Rust-private path ────────────────────────────────
    // Both sides perform writes (last_recalled_ts bumps); they must not share
    // a file to avoid SQLite BUSY/lock errors.
    std::fs::copy(&snapshot, &rust_db)
        .unwrap_or_else(|e| panic!("copy snapshot → {}: {e}", rust_db.display()));
    eprintln!("[parity] snapshot copied → {}", rust_db.display());

    // ── 2. Derive test population from the read-only snapshot ────────────────
    let bots = pick_bots(&snapshot, TOP_BOTS);
    assert!(
        !bots.is_empty(),
        "[parity] snapshot has no bot rows — is PARITY_DB_PATH correct?"
    );
    eprintln!("[parity] testing {} bots: {bots:?}", bots.len());

    // ── 3. Build Rust state (once for the whole run) ─────────────────────────
    let rust_state = make_rust_state(rust_db.clone(), &stub_url);

    // ── 4. Build reqwest client for the Python side ──────────────────────────
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("build reqwest client");

    let mut passed_cases: usize = 0;

    // ── 5. Battery ────────────────────────────────────────────────────────────
    for bot_id in &bots {
        eprintln!("[parity] === bot_id={bot_id} ===");

        // ── 5a. recall battery ────────────────────────────────────────────────
        for &q in RECALL_QUERIES {
            let body = json!({
                "bot_id": bot_id,
                "query": q,
                "top_k": 5
            });
            let label = format!("recall bot={bot_id} q={q:?}");

            let (r_status, r_body) =
                rust_post(rust_state.clone(), "/memory/recall", &body).await;
            let (p_status, p_body) =
                python_post(&http, &py_base, "/memory/recall", &body).await;

            assert_eq!(
                r_status, p_status,
                "[parity] STATUS MISMATCH — {label}\n  Rust={r_status}  Python={p_status}"
            );
            if r_status == 200 {
                assert_recall_order(&label, &r_body, &p_body);
                assert_recall_scores(&label, &r_body, &p_body);
            }
            passed_cases += 1;
        }

        // ── 5b. search battery ────────────────────────────────────────────────
        for &q in SEARCH_QUERIES {
            let body = json!({
                "bot_id": bot_id,
                "query": q,
                "top_k": 5
            });
            let label = format!("search bot={bot_id} q={q:?}");

            let (r_status, r_body) =
                rust_post(rust_state.clone(), "/memory/search", &body).await;
            let (p_status, p_body) =
                python_post(&http, &py_base, "/memory/search", &body).await;

            assert_eq!(
                r_status, p_status,
                "[parity] STATUS MISMATCH — {label}\n  Rust={r_status}  Python={p_status}"
            );
            if r_status == 200 {
                assert_search_parity(&label, &r_body, &p_body);
            }
            passed_cases += 1;
        }

        // ── 5c. recall_about battery ──────────────────────────────────────────
        let entities = pick_entities(&snapshot, bot_id);
        for entity in &entities {
            let body = json!({
                "bot_id": bot_id,
                "entity": entity,
                "max_hops": 2,
                "top_k": 3
            });
            let label = format!("recall_about bot={bot_id} entity={entity:?}");

            let (r_status, r_body) =
                rust_post(rust_state.clone(), "/memory/recall_about", &body).await;
            let (p_status, p_body) =
                python_post(&http, &py_base, "/memory/recall_about", &body).await;

            assert_eq!(
                r_status, p_status,
                "[parity] STATUS MISMATCH — {label}\n  Rust={r_status}  Python={p_status}"
            );
            if r_status == 200 {
                let r_hints = r_body["hints"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                let p_hints = p_body["hints"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                assert_eq!(
                    r_hints, p_hints,
                    "[parity] HINTS MISMATCH — {label}\n  Rust   = {r_hints:?}\n  Python = {p_hints:?}"
                );
            }
            passed_cases += 1;
        }
    }

    // ── 6. Edge cases ─────────────────────────────────────────────────────────

    // 6a. Recall on an unknown bot_id → both sides return empty memories list.
    {
        let body = json!({
            "bot_id": "__no_such_bot_parity_xyz__",
            "query": "anything",
            "top_k": 5
        });
        let (r_status, r_body) =
            rust_post(rust_state.clone(), "/memory/recall", &body).await;
        let (p_status, p_body) =
            python_post(&http, &py_base, "/memory/recall", &body).await;
        assert_eq!(r_status, p_status,
            "[parity] EDGE unknown_bot recall: status mismatch Rust={r_status} Python={p_status}");
        if r_status == 200 {
            assert_eq!(
                recall_ids(&r_body).len(), 0,
                "[parity] EDGE unknown_bot recall: Rust returned non-empty memories"
            );
            assert_eq!(
                recall_ids(&p_body).len(), 0,
                "[parity] EDGE unknown_bot recall: Python returned non-empty memories"
            );
        }
        passed_cases += 1;
    }

    // 6b. All-stopword query — both sides return identical (possibly empty) results.
    // The FTS5 stopword filter strips "the a to of"; dense scoring still fires
    // but may return nothing if the embed stub produces a degenerate vector.
    // The contract is: both sides behave identically, not that they return empty.
    if let Some(bot_id) = bots.first() {
        let body = json!({
            "bot_id": bot_id,
            "query": "the a to of",
            "top_k": 5
        });
        let label = format!("recall all-stopwords bot={bot_id}");

        let (r_status, r_body) =
            rust_post(rust_state.clone(), "/memory/recall", &body).await;
        let (p_status, p_body) =
            python_post(&http, &py_base, "/memory/recall", &body).await;

        assert_eq!(r_status, p_status,
            "[parity] EDGE {label}: status mismatch Rust={r_status} Python={p_status}");
        if r_status == 200 {
            assert_recall_order(&label, &r_body, &p_body);
            assert_recall_scores(&label, &r_body, &p_body);
        }
        passed_cases += 1;
    }

    // ── 7. Done ───────────────────────────────────────────────────────────────
    eprintln!(
        "\n[parity] PASSED — {passed_cases} cases across {} bots",
        bots.len()
    );

    // Clean up the Rust-private copy so re-runs start fresh.
    let _ = std::fs::remove_file(&rust_db);
}
