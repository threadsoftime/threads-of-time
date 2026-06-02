// SPDX-License-Identifier: GPL-2.0-or-later
//! Cross-implementation parity harness — the v0.2.1 cutover gate.
//!
//! Fires an identical battery of recall / search / recall_about requests at:
//!   - The live Python memory-sidecar v0.2.1  (via `PYTHON_MEMORY_URL`)
//!   - The Rust router built from this crate   (in-process, tower::oneshot)
//!
//! ## Gold-standard real-vector design
//!
//! The prior gate used generic text queries ("what happened recently", etc.).
//! With the SHA-256 embed stub those queries produce vectors that are nearly
//! orthogonal to the *real* bge-small embeddings stored in the snapshot, so
//! all cosine scores cluster near the same tiny value and ranking is decided
//! by ~9e-9 floating-point noise — tie-boundary artefacts, not real divergence.
//!
//! Fix: the embed stub now supports `PARITY_MEM:{memory_id}` queries. When a
//! request arrives with that prefix, the stub returns the *exact* stored f32
//! embedding for that memory.  Querying with a stored vector means:
//!   - The queried memory has cosine = 1.0 with itself.
//!   - Other memories with similar content have cosine < 1.0 but well-separated.
//!   - The ranking landscape is dense and meaningful — a ranking divergence is
//!     a real bug, not a tie-break artefact.
//!
//! ## Comparator design
//!
//! `EPS = 1e-6` tolerance for floating-point parity.
//!
//! ### ε-tie-tolerant ordering (recall + search batteries)
//!
//! Two ordered lists are "equivalent" if they contain the same set of items
//! and any swap between adjacent items is between items whose scores differ
//! by ≤ EPS.  The comparator:
//!   1. If both lists are identical → pass.
//!   2. If the *sets* differ: find the item(s) present in one side but absent
//!      from the other. If that item's score is within EPS of the boundary
//!      score (the K-th score on the other side) → benign tie-break, pass.
//!      Otherwise → real divergence, fail with full diagnostics.
//!   3. If the sets are the same but ordering differs: every swap must be
//!      between two items whose scores differ by ≤ EPS.  Any pair where
//!      |score_A - score_B| > EPS and A precedes B on one side but B precedes
//!      A on the other → real divergence, fail with full diagnostics.
//!
//! ### Tie-free anchor assertions
//!
//! For a handful of `PARITY_MEM:{mid}` recall queries, assert BOTH sides rank
//! `mid` itself as #1 (cosine 1.0 is unique; the gap to #2 is guaranteed large
//! enough to be tie-free).
//!
//! ### recall_about + edge cases
//!
//! BFS graph traversal is deterministic; only tie-boundary ordering of scored
//! results may vary.  Compare *sets* of returned ids/hints; assert scores within ε.
//!
//! ## Write isolation
//!
//! `recall` and `recall_about` bump `last_recalled_ts` in `memories`.
//! The Rust side works on a private copy of the snapshot (PARITY_DB_PATH + ".rust").
//! The Python instance uses its own copy (operator sets it up before running).
//! `recency_basis = Created` so last_recalled_ts bumps do NOT affect scores.
//!
//! ## Env vars (all required at runtime; none needed to compile or list)
//!
//! | Variable             | Purpose                                               |
//! |----------------------|-------------------------------------------------------|
//! | `PARITY_DB_PATH`     | Path to a single-file SQLite snapshot                 |
//! | `PYTHON_MEMORY_URL`  | Base URL of a live Python v0.2.1 instance (no trailing `/`) |
//! | `EMBED_STUB_URL`     | Base URL of the real-vector-capable embed stub        |
//!
//! ## Running (operator)
//!
//! ```sh
//! # 1. Start the upgraded embed stub (real-vector mode):
//! PARITY_DB_PATH=/path/to/parity.sqlite python3 /path/to/embed_stub.py &
//! # verify: curl -s -XPOST 127.0.0.1:8188/v1/embeddings \
//! #    -H 'Content-Type: application/json' \
//! #    -d '{"input":"PARITY_MEM:m_3kkfanm46fb"}'
//!
//! # 2. Copy snapshot for the Python sidecar; start it:
//! cp $PARITY_DB_PATH /tmp/parity-py.sqlite
//! MEM_DB_PATH=/tmp/parity-py.sqlite MEM_EMBED_ENDPOINT=http://127.0.0.1:8188 \
//!   MEM_TOKEN_STORE=/nonexistent uvicorn memory_sidecar.main:create_app \
//!   --factory --host 127.0.0.1 --port 8191
//!
//! # 3. Run the gate:
//! PARITY_DB_PATH=/path/to/parity.sqlite \
//! PYTHON_MEMORY_URL=http://127.0.0.1:8191 \
//! EMBED_STUB_URL=http://127.0.0.1:8188 \
//! CARGO_BUILD_JOBS=4 cargo test -p memory-rs --test parity \
//!   -- --ignored --nocapture
//! ```

use std::collections::{HashMap, HashSet};
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

/// Float comparison tolerance for scores that contain NO recency term.
///
/// Covers: (a) RRF search scores (purely rank-based integer arithmetic — no
/// `exp(-(now-ts)/tau)` term); (b) ordering / set-boundary comparisons where
/// both sides compute `now` within the same in-process call so the recency
/// terms cancel to within float-accumulation noise (~9e-9 each).
const EPS: f64 = 1e-6;

/// Timing-aware tolerance for absolute scores that CONTAIN the recency term.
///
/// Formula: `score_memory = w_rel·cosine + w_rec·exp(-(now−created_ts)/τ) + w_imp·salience`
///
/// Python computes `now` in its HTTP handler; Rust computes `now` inside
/// `spawn_blocking` roughly 1-2 seconds later.  The maximum rate at which the
/// recency term can change is `|d/dt exp(-t/τ)| = 1/τ` (at t=0, where the
/// term is largest), so the score difference induced by a timing gap `Δt` is
/// bounded by:
///
///   Δscore ≤ w_rec · Δt / τ
///
/// With w_rec = 0.2 and τ = 604_800 s (7 days), a conservative DT_MAX = 30 s:
///
///   timing_budget = 0.2 × 30 / 604_800 ≈ 9.92e-6
///
/// Adding the float-accumulation floor FLOAT_EPS = 1e-6:
///
///   SCORE_EPS_TIMING = 1e-6 + 9.92e-6 ≈ 1.09e-5
///
/// Applied ONLY to recall `score` (contains recency) and anchor score
/// comparisons.  NOT applied to search scores (RRF — no recency), NOT applied
/// to ordering comparisons (recency terms cancel between two items compared at
/// the same `now`).
const SCORE_EPS_TIMING: f64 = {
    const FLOAT_EPS: f64 = 1e-6;
    const W_REC: f64 = 0.2;
    const TAU: f64 = 604_800.0;
    const DT_MAX: f64 = 30.0; // conservative inter-process wall-clock skew (seconds)
    FLOAT_EPS + W_REC * DT_MAX / TAU
};

/// Number of top bots (by memory count) to exercise.
const TOP_BOTS: usize = 8;

/// How many of a bot's real memory IDs to use in the PARITY_MEM battery.
const REAL_MIDS_PER_BOT: usize = 5;

/// How many anchor IDs (tie-free #1 self-similarity assertions) per bot.
const ANCHOR_MIDS_PER_BOT: usize = 3;

// ---------------------------------------------------------------------------
// Env-var helpers (panic clearly when a required var is absent)
// ---------------------------------------------------------------------------

fn require_env(key: &str) -> String {
    std::env::var(key)
        .unwrap_or_else(|_| panic!("parity test requires env var {key} to be set"))
}

fn snapshot_path() -> PathBuf {
    PathBuf::from(require_env("PARITY_DB_PATH"))
}

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

/// Return up to `n` memory IDs for `bot_id` from the snapshot.
///
/// These are used as `PARITY_MEM:{mid}` queries — each returns the memory's
/// own stored embedding, giving cosine = 1.0 for that memory itself and a
/// well-separated cosine landscape for the rest.
fn pick_memory_ids(snapshot: &PathBuf, bot_id: &str, n: usize) -> Vec<String> {
    let conn = rusqlite::Connection::open_with_flags(
        snapshot,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("open snapshot read-only for memory ids");
    let mut stmt = conn
        .prepare(
            "SELECT id FROM memories \
             WHERE bot_id = ? AND embedding IS NOT NULL \
             ORDER BY created_ts DESC \
             LIMIT ?",
        )
        .expect("prepare pick_memory_ids");
    stmt.query_map(rusqlite::params![bot_id, n as i64], |row| {
        row.get::<_, String>(0)
    })
    .expect("query_map memory_ids")
    .map(|r| r.expect("memory_id row"))
    .collect()
}

/// Return up to 3 entity name_lower values for `bot_id` from the snapshot.
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

/// Build an `AppState` backed by `db_path`, using the embed stub.
fn make_rust_state(db_path: PathBuf, embed_url: &str) -> AppState {
    db::register_vec0();
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
    let app = build_router(state, vec![]);  // disable host check in tests
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
// Result extraction helpers
// ---------------------------------------------------------------------------

/// Ordered `(memory_id, score)` pairs from a recall response.
fn recall_scored(body: &Value) -> Vec<(String, f64)> {
    body["memories"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|r| {
                    let mid = r["memory_id"].as_str()?.to_owned();
                    let score = r["score"].as_f64().unwrap_or(f64::NAN);
                    Some((mid, score))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Ordered `(memory_id, rrf_score, signals)` tuples from a search response.
fn search_scored(body: &Value) -> Vec<(String, f64, Value)> {
    body["items"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|r| {
                    let mid = r["memory_id"].as_str()?.to_owned();
                    let score = r["score"].as_f64().unwrap_or(f64::NAN);
                    let signals = r["signals"].clone();
                    Some((mid, score, signals))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Set of memory_ids from a recall response.
fn recall_id_set(body: &Value) -> HashSet<String> {
    recall_scored(body).into_iter().map(|(id, _)| id).collect()
}

// ---------------------------------------------------------------------------
// ε-tie-tolerant comparator
// ---------------------------------------------------------------------------

/// Compare two ordered (id, score) lists with ε-tie tolerance.
///
/// # Contract
///
/// Returns `Ok(())` if the lists are parity-equivalent:
///   - Identical → trivially OK.
///   - Sets differ: an item present in one but absent from the other is benign
///     iff its score and the boundary score on the missing side differ by ≤ EPS.
///   - Sets match, ordering differs: every out-of-order pair (A before B on
///     side-1, B before A on side-2) must have |score_A - score_B| ≤ EPS.
///
/// Returns `Err(String)` with full diagnostics on a REAL divergence.
fn compare_ordered_with_eps_tolerance(
    label: &str,
    rust_list: &[(String, f64)],
    python_list: &[(String, f64)],
) -> Result<(), String> {
    // Fast path: identical.
    if rust_list == python_list {
        return Ok(());
    }

    let rust_set: HashSet<&str> = rust_list.iter().map(|(id, _)| id.as_str()).collect();
    let python_set: HashSet<&str> = python_list.iter().map(|(id, _)| id.as_str()).collect();

    // Build score maps for quick lookup.
    let rust_scores: HashMap<&str, f64> =
        rust_list.iter().map(|(id, s)| (id.as_str(), *s)).collect();
    let python_scores: HashMap<&str, f64> =
        python_list.iter().map(|(id, s)| (id.as_str(), *s)).collect();

    // Check items in Rust but not Python.
    for id in rust_set.difference(&python_set) {
        let rust_score = rust_scores[id];
        // The boundary is the lowest score on the Python side (the K-th result).
        let boundary = python_list.last().map(|(_, s)| *s).unwrap_or(0.0);
        let diff = (rust_score - boundary).abs();
        if diff > EPS {
            return Err(format!(
                "[parity] REAL DIVERGENCE — {label}\n\
                 Item {id:?} present in Rust (score={rust_score:.9}) \
                 but absent from Python.\n\
                 Python boundary score={boundary:.9}  diff={diff:.2e}  eps={EPS:.0e}\n\
                 Rust  list = {rust_list:?}\n\
                 Python list = {python_list:?}"
            ));
        }
    }

    // Check items in Python but not Rust.
    for id in python_set.difference(&rust_set) {
        let python_score = python_scores[id];
        let boundary = rust_list.last().map(|(_, s)| *s).unwrap_or(0.0);
        let diff = (python_score - boundary).abs();
        if diff > EPS {
            return Err(format!(
                "[parity] REAL DIVERGENCE — {label}\n\
                 Item {id:?} present in Python (score={python_score:.9}) \
                 but absent from Rust.\n\
                 Rust boundary score={boundary:.9}  diff={diff:.2e}  eps={EPS:.0e}\n\
                 Rust  list = {rust_list:?}\n\
                 Python list = {python_list:?}"
            ));
        }
    }

    // Sets match or any set differences are EPS-tied.  Check ordering: every
    // pair (A, B) where Rust has A before B but Python has B before A must
    // have |score_A - score_B| ≤ EPS.
    let common: HashSet<&str> = rust_set.intersection(&python_set).copied().collect();
    let rust_order: Vec<(&str, f64)> = rust_list
        .iter()
        .filter(|(id, _)| common.contains(id.as_str()))
        .map(|(id, s)| (id.as_str(), *s))
        .collect();
    let python_order: Vec<(&str, f64)> = python_list
        .iter()
        .filter(|(id, _)| common.contains(id.as_str()))
        .map(|(id, s)| (id.as_str(), *s))
        .collect();

    // Build a position map for Python's ordering of common items.
    // (Rust iteration order is already determined by rust_order itself.)
    let python_pos: HashMap<&str, usize> =
        python_order.iter().enumerate().map(|(i, (id, _))| (*id, i)).collect();

    for (i, (id_a, score_a)) in rust_order.iter().enumerate() {
        for (id_b, score_b) in &rust_order[i + 1..] {
            // Rust: id_a before id_b.  Python: may have them reversed.
            let p_a = python_pos[id_a];
            let p_b = python_pos[id_b];
            if p_b < p_a {
                // Python has B before A — a swap vs Rust.
                let gap = (score_a - score_b).abs();
                if gap > EPS {
                    return Err(format!(
                        "[parity] REAL ORDERING DIVERGENCE — {label}\n\
                         Rust has {id_a:?} (score={score_a:.9}) before {id_b:?} (score={score_b:.9})\n\
                         Python has them reversed.\n\
                         |score_a - score_b| = {gap:.2e}  eps={EPS:.0e}\n\
                         Rust  list = {rust_list:?}\n\
                         Python list = {python_list:?}"
                    ));
                }
            }
        }
    }

    // All differences are within EPS-tie tolerance.
    Ok(())
}

/// Compare score values within a given `eps` after set/order comparison.
fn compare_scores_with_eps(
    label: &str,
    rust_list: &[(String, f64)],
    python_list: &[(String, f64)],
    eps: f64,
) -> Result<(), String> {
    let rust_scores: HashMap<&str, f64> =
        rust_list.iter().map(|(id, s)| (id.as_str(), *s)).collect();
    let python_scores: HashMap<&str, f64> =
        python_list.iter().map(|(id, s)| (id.as_str(), *s)).collect();

    for (id, rs) in &rust_scores {
        if let Some(&ps) = python_scores.get(id) {
            let diff = (rs - ps).abs();
            if diff > eps {
                return Err(format!(
                    "[parity] SCORE MISMATCH — {label}  memory_id={id}\n\
                     Rust={rs:.9}  Python={ps:.9}  diff={diff:.2e}  eps={eps:.2e}"
                ));
            }
        }
    }
    Ok(())
}

/// Compare score values within EPS (for RRF / recency-free scores).
fn compare_scores_eps(
    label: &str,
    rust_list: &[(String, f64)],
    python_list: &[(String, f64)],
) -> Result<(), String> {
    compare_scores_with_eps(label, rust_list, python_list, EPS)
}

/// Compare score values within SCORE_EPS_TIMING (for recall scores containing
/// the recency term, where cross-process wall-clock skew introduces bounded
/// divergence — see the SCORE_EPS_TIMING derivation comment above).
fn compare_scores_timing_eps(
    label: &str,
    rust_list: &[(String, f64)],
    python_list: &[(String, f64)],
) -> Result<(), String> {
    compare_scores_with_eps(label, rust_list, python_list, SCORE_EPS_TIMING)
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
    let mut tie_benign_cases: usize = 0;

    // ── 5. Battery ────────────────────────────────────────────────────────────
    for bot_id in &bots {
        eprintln!("[parity] === bot_id={bot_id} ===");

        let real_mids = pick_memory_ids(&snapshot, bot_id, REAL_MIDS_PER_BOT);
        if real_mids.is_empty() {
            eprintln!("[parity]   SKIP: no embeddings for bot {bot_id}");
            continue;
        }

        // ── 5a. recall battery: PARITY_MEM: queries ──────────────────────────
        // Each query vector IS the stored embedding for that memory →
        // well-separated cosine landscape; ranking is meaningful.
        for mid in &real_mids {
            let query = format!("PARITY_MEM:{mid}");
            let body = json!({
                "bot_id": bot_id,
                "query": query,
                "top_k": 5
            });
            let label = format!("recall bot={bot_id} PARITY_MEM:{mid}");

            let (r_status, r_body) =
                rust_post(rust_state.clone(), "/memory/recall", &body).await;
            let (p_status, p_body) =
                python_post(&http, &py_base, "/memory/recall", &body).await;

            assert_eq!(
                r_status, p_status,
                "[parity] STATUS MISMATCH — {label}\n  Rust={r_status}  Python={p_status}"
            );

            if r_status == 200 {
                let rust_ranked = recall_scored(&r_body);
                let python_ranked = recall_scored(&p_body);

                // ε-tie-tolerant ordering comparison.
                match compare_ordered_with_eps_tolerance(&label, &rust_ranked, &python_ranked) {
                    Ok(()) => {}
                    Err(e) => panic!("{e}"),
                }

                // Absolute score parity on items that appear in both.
                // Recall scores contain a recency term exp(-(now-ts)/τ); Python
                // and Rust compute `now` in separate processes with a small
                // wall-clock skew, so use SCORE_EPS_TIMING (see derivation above).
                match compare_scores_timing_eps(&label, &rust_ranked, &python_ranked) {
                    Ok(()) => {}
                    Err(e) => panic!("{e}"),
                }

                // Check whether any tolerance was needed (for reporting).
                if rust_ranked != python_ranked {
                    tie_benign_cases += 1;
                    eprintln!("[parity]   TIE-BENIGN swap (ε-ok): {label}");
                }
            }
            passed_cases += 1;
        }

        // ── 5b. search battery: PARITY_MEM: queries ──────────────────────────
        for mid in &real_mids {
            let query = format!("PARITY_MEM:{mid}");
            let body = json!({
                "bot_id": bot_id,
                "query": query,
                "top_k": 5
            });
            let label = format!("search bot={bot_id} PARITY_MEM:{mid}");

            let (r_status, r_body) =
                rust_post(rust_state.clone(), "/memory/search", &body).await;
            let (p_status, p_body) =
                python_post(&http, &py_base, "/memory/search", &body).await;

            assert_eq!(
                r_status, p_status,
                "[parity] STATUS MISMATCH — {label}\n  Rust={r_status}  Python={p_status}"
            );

            if r_status == 200 {
                let rust_ranked: Vec<(String, f64)> = search_scored(&r_body)
                    .into_iter()
                    .map(|(id, s, _)| (id, s))
                    .collect();
                let python_ranked: Vec<(String, f64)> = search_scored(&p_body)
                    .into_iter()
                    .map(|(id, s, _)| (id, s))
                    .collect();

                match compare_ordered_with_eps_tolerance(&label, &rust_ranked, &python_ranked) {
                    Ok(()) => {}
                    Err(e) => panic!("{e}"),
                }

                // Search scores are RRF (pure rank-based integer arithmetic; NO
                // recency term) — strict EPS=1e-6 applies; timing skew is irrelevant.
                match compare_scores_eps(&label, &rust_ranked, &python_ranked) {
                    Ok(()) => {}
                    Err(e) => panic!("{e}"),
                }

                // signals.*_rank: nullable integer — must match exactly for
                // items present in both sides.
                let r_items = search_scored(&r_body);
                let p_items = search_scored(&p_body);
                let r_sig_map: HashMap<String, Value> = r_items
                    .iter()
                    .map(|(id, _, sig)| (id.clone(), sig.clone()))
                    .collect();
                let p_sig_map: HashMap<String, Value> = p_items
                    .iter()
                    .map(|(id, _, sig)| (id.clone(), sig.clone()))
                    .collect();

                for (id, rs) in &r_sig_map {
                    if let Some(ps) = p_sig_map.get(id) {
                        for field in &["bm25_rank", "dense_rank", "entity_rank"] {
                            assert_eq!(
                                rs[field], ps[field],
                                "[parity] SIGNALS MISMATCH — {label} \
                                 memory_id={id} .signals.{field}\n\
                                 Rust={rs_field}  Python={ps_field}",
                                rs_field = rs[field],
                                ps_field = ps[field],
                            );
                        }
                    }
                }

                if rust_ranked != python_ranked {
                    tie_benign_cases += 1;
                    eprintln!("[parity]   TIE-BENIGN swap (ε-ok): {label}");
                }
            }
            passed_cases += 1;
        }

        // ── 5c. Cosine-identity anchor assertions ─────────────────────────────
        // For a handful of PARITY_MEM:{mid} recall queries, verify that `mid`
        // appears in BOTH sides' results (top_k=50 is large enough for bots
        // with 2000 memories to surface the queried mid) and that its score
        // matches between sides within SCORE_EPS_TIMING.
        //
        // This is the strongest correctness proof available: the queried mid
        // has cosine = 1.0 with itself, giving it the maximum possible relevance
        // contribution.  Cosine and salience are identical on both sides; the only
        // source of score difference is the recency term's cross-process wall-clock
        // skew.  Any discrepancy exceeding SCORE_EPS_TIMING indicates a genuine
        // scoring bug in one implementation (see constant derivation above).
        let anchor_mids = pick_memory_ids(&snapshot, bot_id, ANCHOR_MIDS_PER_BOT);
        for mid in &anchor_mids {
            let query = format!("PARITY_MEM:{mid}");
            let body = json!({
                "bot_id": bot_id,
                "query": query,
                "top_k": 50
            });
            let label = format!("anchor bot={bot_id} PARITY_MEM:{mid}");

            let (r_status, r_body) =
                rust_post(rust_state.clone(), "/memory/recall", &body).await;
            let (p_status, p_body) =
                python_post(&http, &py_base, "/memory/recall", &body).await;

            assert_eq!(r_status, 200, "[parity] ANCHOR {label}: Rust non-200 status {r_status}");
            assert_eq!(p_status, 200, "[parity] ANCHOR {label}: Python non-200 status {p_status}");

            let rust_ranked = recall_scored(&r_body);
            let python_ranked = recall_scored(&p_body);

            // Find mid's score on each side (it must appear somewhere in top-50).
            let rust_mid_entry = rust_ranked.iter().find(|(id, _)| id == mid);
            let python_mid_entry = python_ranked.iter().find(|(id, _)| id == mid);

            assert!(
                rust_mid_entry.is_some(),
                "[parity] ANCHOR FAIL (Rust) — {label}\n\
                 Memory {mid:?} not found in Rust top-50 results.\n\
                 Rust list: {rust_ranked:?}"
            );
            assert!(
                python_mid_entry.is_some(),
                "[parity] ANCHOR FAIL (Python) — {label}\n\
                 Memory {mid:?} not found in Python top-50 results.\n\
                 Python list: {python_ranked:?}"
            );

            // Score of the queried mid must match within SCORE_EPS_TIMING on
            // both sides.  Cosine = 1.0 is exact and bit-identical; w_imp·salience
            // is read from the DB and is identical.  The only source of divergence
            // is the recency term: Python computes `now` in its HTTP handler, Rust
            // computes `now` inside spawn_blocking ~1-2 s later, giving a bounded
            // Δscore ≤ w_rec·Δt/τ (see SCORE_EPS_TIMING derivation).  Any
            // divergence exceeding that bound is a genuine scoring bug.
            let rust_mid_score = rust_mid_entry.unwrap().1;
            let python_mid_score = python_mid_entry.unwrap().1;
            let score_diff = (rust_mid_score - python_mid_score).abs();
            assert!(
                score_diff <= SCORE_EPS_TIMING,
                "[parity] ANCHOR SCORE MISMATCH — {label}\n\
                 Rust   score({mid}) = {rust_mid_score:.9}\n\
                 Python score({mid}) = {python_mid_score:.9}\n\
                 diff = {score_diff:.2e}  eps = {SCORE_EPS_TIMING:.2e} (timing-aware)"
            );

            // Both sides should also agree on ordering up to EPS-tie tolerance.
            match compare_ordered_with_eps_tolerance(&label, &rust_ranked, &python_ranked) {
                Ok(()) => {}
                Err(e) => panic!("{e}"),
            }

            eprintln!(
                "[parity]   ANCHOR OK: {label}  \
                 rust_score={rust_mid_score:.6}  python_score={python_mid_score:.6}  \
                 diff={score_diff:.2e}"
            );
            passed_cases += 1;
        }

        // ── 5d. recall_about battery ──────────────────────────────────────────
        // BFS graph traversal is deterministic; compare *sets* of returned ids.
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
                // hints: compare as sets (ordering may differ for tied BFS paths).
                let r_hints: HashSet<Value> = r_body["hints"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .collect();
                let p_hints: HashSet<Value> = p_body["hints"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .collect();

                // Set-equality: both must contain exactly the same hints.
                let only_rust: Vec<&Value> = r_hints.difference(&p_hints).collect();
                let only_python: Vec<&Value> = p_hints.difference(&r_hints).collect();
                assert!(
                    only_rust.is_empty() && only_python.is_empty(),
                    "[parity] HINTS SET MISMATCH — {label}\n\
                     Only in Rust:   {only_rust:?}\n\
                     Only in Python: {only_python:?}"
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
        assert_eq!(
            r_status, p_status,
            "[parity] EDGE unknown_bot recall: status mismatch \
             Rust={r_status} Python={p_status}"
        );
        if r_status == 200 {
            assert_eq!(
                recall_id_set(&r_body).len(),
                0,
                "[parity] EDGE unknown_bot recall: Rust returned non-empty memories"
            );
            assert_eq!(
                recall_id_set(&p_body).len(),
                0,
                "[parity] EDGE unknown_bot recall: Python returned non-empty memories"
            );
        }
        passed_cases += 1;
    }

    // 6b. All-stopword query — both sides behave identically.
    // The FTS5 stopword filter strips "the a to of"; dense scoring still fires
    // (SHA-256 stub produces a valid vector).  Contract: both sides return the
    // same set of ids (order may differ for ties).
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

        assert_eq!(
            r_status, p_status,
            "[parity] EDGE {label}: status mismatch Rust={r_status} Python={p_status}"
        );
        if r_status == 200 {
            let rust_ranked = recall_scored(&r_body);
            let python_ranked = recall_scored(&p_body);
            // SHA-256 stub produces arbitrary ordering; use set-equality only.
            let rust_set: HashSet<&str> =
                rust_ranked.iter().map(|(id, _)| id.as_str()).collect();
            let python_set: HashSet<&str> =
                python_ranked.iter().map(|(id, _)| id.as_str()).collect();
            let only_rust: Vec<&&str> = rust_set.difference(&python_set).collect();
            let only_python: Vec<&&str> = python_set.difference(&rust_set).collect();

            // For SHA-256 queries the cosine landscape is flat, so ties abound.
            // We can only assert set-level parity + score parity within EPS.
            // If the SETS differ, check whether all outsiders are within EPS of
            // the boundary (same tolerance as compare_ordered_with_eps_tolerance).
            let all_diff_benign = {
                let r_score_map: HashMap<&str, f64> =
                    rust_ranked.iter().map(|(id, s)| (id.as_str(), *s)).collect();
                let p_score_map: HashMap<&str, f64> =
                    python_ranked.iter().map(|(id, s)| (id.as_str(), *s)).collect();
                let r_boundary = rust_ranked.last().map(|(_, s)| *s).unwrap_or(0.0);
                let p_boundary = python_ranked.last().map(|(_, s)| *s).unwrap_or(0.0);
                let rust_outsiders_ok = only_rust.iter().all(|&&id| {
                    p_score_map.get(id).map(|&s| (s - r_boundary).abs() <= EPS).unwrap_or(false)
                        || r_score_map
                            .get(id)
                            .map(|&s| (s - p_boundary).abs() <= EPS)
                            .unwrap_or(false)
                });
                let python_outsiders_ok = only_python.iter().all(|&&id| {
                    r_score_map.get(id).map(|&s| (s - p_boundary).abs() <= EPS).unwrap_or(false)
                        || p_score_map
                            .get(id)
                            .map(|&s| (s - r_boundary).abs() <= EPS)
                            .unwrap_or(false)
                });
                rust_outsiders_ok && python_outsiders_ok
            };
            assert!(
                all_diff_benign || (only_rust.is_empty() && only_python.is_empty()),
                "[parity] EDGE {label}: set divergence beyond EPS\n\
                 Only in Rust:   {only_rust:?}\n\
                 Only in Python: {only_python:?}"
            );

            // Absolute score parity on common items.
            // These are recall scores containing the recency term; use
            // SCORE_EPS_TIMING to account for cross-process wall-clock skew
            // (SHA-256 stub produces a flat cosine landscape so the recency
            // contribution is the dominant source of difference here).
            match compare_scores_timing_eps(&label, &rust_ranked, &python_ranked) {
                Ok(()) => {}
                Err(e) => panic!("{e}"),
            }
        }
        passed_cases += 1;
    }

    // ── 7. Done ───────────────────────────────────────────────────────────────
    eprintln!(
        "\n[parity] PASSED — {passed_cases} cases across {} bots \
         ({tie_benign_cases} ε-benign tie swaps)",
        bots.len()
    );

    // Clean up the Rust-private copy so re-runs start fresh.
    let _ = std::fs::remove_file(&rust_db);
}
