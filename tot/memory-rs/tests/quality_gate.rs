// SPDX-License-Identifier: GPL-2.0-or-later
//! Quality gate for hybrid recall.
//!
//! Ports `tests/eval/test_quality_gate.py` to Rust. Asserts the §12.2 thresholds:
//!   recall@5    >= 0.80
//!   precision@5 >= 0.70
//!   p95 latency <= 100 ms
//!
//! Run explicitly (slow — corpus seed + 50 recalls):
//!   cargo test -p memory-rs --test quality_gate -- --ignored
//!
//! Design: embeddings are deterministic SHA-256 stubs — identical text always
//! maps to the same 768-dim L2-normalized f32 vector. No live embed endpoint
//! is required. The gate therefore exercises the combinatorial retrieval logic
//! (BM25 + entity hard-filter + decay + salience) without semantic signal.
//! The Python gate uses the same stub (see `_stub_embed` in test_quality_gate.py).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use rusqlite::Connection;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use memory_rs::db::migrate::run_migrations;
use memory_rs::db::open_bot_db;
use memory_rs::retrieval::bm25::bm25_search;
use memory_rs::retrieval::decay::half_life_for;
use memory_rs::retrieval::dense::dense_search;
use memory_rs::retrieval::entity::entity_filter;
use memory_rs::retrieval::hybrid::{
    ComponentScores, HybridScorer, DEFAULT_ALPHA, DEFAULT_BETA, DEFAULT_DELTA, DEFAULT_GAMMA,
};
use memory_rs::retrieval::rerank::max_normalize;
use memory_rs::EMBEDDING_DIM;

// ── §12.2 thresholds (matches Python gate) ────────────────────────────────────
const RECALL_AT_5_THRESHOLD: f64 = 0.80;
const PRECISION_AT_5_THRESHOLD: f64 = 0.70;
const P95_LATENCY_MS_THRESHOLD: f64 = 100.0;

// Fixed epoch ms used for decay calculations. Matches EVAL_NOW in the Python
// gate: `datetime(2026, 5, 27, 0, 0, 0, tzinfo=timezone.utc)`.
//
// NOTE: the plan's draft comment mis-stated the arithmetic (it claimed
// 2026-05-27 == 1_748_304_000_000, but that epoch is actually *2025*-05-27, a
// full year early — which would push every corpus episode into the future and
// silently flatten the decay term to a uniform 1.0). The correct epoch-ms for
// 2026-05-27T00:00:00Z is 1_779_840_000_000, which is exactly the newest
// timestamp in `synthetic_episodes.jsonl` (the corpus was generated with this
// as FIXED_NOW). Using it keeps decay meaningful and matches the Python gate.
const EVAL_NOW_MS: i64 = 1_779_840_000_000;

// ── Deterministic stub embedder ───────────────────────────────────────────────

/// SHA-256 → 768-dim L2-normalized f32 vector.
///
/// Mirrors `_stub_embed` from `test_quality_gate.py` exactly:
///   1. Hash text with SHA-256 (32 bytes).
///   2. Repeat digest to fill EMBEDDING_DIM (768 / 32 = 24 reps).
///   3. Map each byte b → (b as f64 - 128.0) / 128.0.
///   4. L2-normalize.
///
/// Same text → same vector across all runs (no randomness).
fn stub_embed(text: &str) -> Vec<f32> {
    let digest = Sha256::digest(text.as_bytes());
    let h: &[u8] = &digest;
    let repeated: Vec<u8> = h.iter().copied().cycle().take(EMBEDDING_DIM).collect();

    let mut vec: Vec<f64> = repeated.iter().map(|&b| (b as f64 - 128.0) / 128.0).collect();
    let norm = vec.iter().map(|v| v * v).sum::<f64>().sqrt();
    if norm > 0.0 {
        vec.iter_mut().for_each(|v| *v /= norm);
    }
    vec.iter().map(|&v| v as f32).collect()
}

// ── FTS5 query sanitizer ──────────────────────────────────────────────────────

/// Lower-case alphanumeric token extraction joined by FTS5 OR.
///
/// Mirrors `_sanitize_fts_query` from the Python gate. Strips apostrophes and
/// punctuation so FTS5 does not error on names like "Mor'Ladim".
fn sanitize_fts_query(text: &str) -> String {
    let tokens: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
        .collect();
    tokens.join(" OR ")
}

// ── Fixture types ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct EpisodeFixture {
    timestamp: i64,
    content_text: String,
    episode_type: String,
    salience_score: f64,
    #[serde(default)]
    source: String,
    #[serde(default)]
    metadata: Value,
    #[serde(default)]
    entities: Vec<EntityFixture>,
}

#[derive(Debug, Deserialize)]
struct EntityFixture {
    entity_kind: String,
    entity_key: String,
    display_name: String,
    #[serde(default)]
    role: String,
}

#[derive(Debug, Deserialize)]
struct QueryFixture {
    query_text: String,
    expected_episode_ids: Vec<i64>,
    #[serde(default)]
    top_k: Option<usize>,
    #[serde(default)]
    entity_filter: Option<Vec<String>>,
    #[serde(default)]
    episode_types: Option<Vec<String>>,
}

// ── Fixture loaders ───────────────────────────────────────────────────────────

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests")
}

fn load_corpus() -> Vec<EpisodeFixture> {
    let path = fixture_dir().join("fixtures/synthetic_episodes.jsonl");
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read corpus fixture {}: {}", path.display(), e));
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            serde_json::from_str(l).unwrap_or_else(|e| panic!("bad corpus line: {e}\nline: {l}"))
        })
        .collect()
}

fn load_queries() -> Vec<QueryFixture> {
    let path = fixture_dir().join("queries/recall_set.jsonl");
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read query fixture {}: {}", path.display(), e));
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            serde_json::from_str(l).unwrap_or_else(|e| panic!("bad query line: {e}\nline: {l}"))
        })
        .collect()
}

// ── DB seeder (raw INSERTs, bypasses HTTP write path) ─────────────────────────

/// Insert all 200 episodes + entities + stub embeddings into a fresh per-bot DB.
///
/// SQLite AUTOINCREMENT means episode_id == 1-indexed line number from JSONL.
/// The query fixture relies on this invariant; we assert it after seeding.
fn seed_db(conn: &Connection, eps: &[EpisodeFixture]) {
    run_migrations(conn).expect("run_migrations");

    let mut entity_cache: HashMap<(String, String), i64> = HashMap::new();

    for ep in eps {
        let metadata_json = serde_json::to_string(&ep.metadata).unwrap_or_else(|_| "{}".to_string());
        let source = if ep.source.is_empty() { "self" } else { &ep.source };

        conn.execute(
            "INSERT INTO episodes \
             (timestamp, content_text, episode_type, salience_score, source, metadata) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                ep.timestamp,
                ep.content_text,
                ep.episode_type,
                ep.salience_score,
                source,
                metadata_json,
            ],
        )
        .expect("INSERT episode");
        let episode_id = conn.last_insert_rowid();

        // Stub embedding: pack f32 LE bytes.
        let vec = stub_embed(&ep.content_text);
        let bytes: Vec<u8> = vec.iter().flat_map(|v| v.to_le_bytes()).collect();
        conn.execute(
            "INSERT INTO embeddings_vec (rowid, embedding) VALUES (?1, ?2)",
            rusqlite::params![episode_id, bytes],
        )
        .expect("INSERT embeddings_vec");
        conn.execute(
            "UPDATE episodes SET content_embedding_id = ?1 WHERE episode_id = ?1",
            rusqlite::params![episode_id],
        )
        .expect("UPDATE content_embedding_id");

        // Entities
        for ent in &ep.entities {
            let key = (ent.entity_kind.clone(), ent.entity_key.clone());
            let entity_id = if let Some(&cached) = entity_cache.get(&key) {
                cached
            } else {
                let existing: Option<i64> = conn
                    .query_row(
                        "SELECT entity_id FROM entities \
                         WHERE entity_kind = ?1 AND entity_key = ?2",
                        rusqlite::params![ent.entity_kind, ent.entity_key],
                        |r| r.get(0),
                    )
                    .ok();
                if let Some(eid) = existing {
                    entity_cache.insert(key, eid);
                    eid
                } else {
                    conn.execute(
                        "INSERT INTO entities \
                         (entity_kind, entity_key, display_name, last_seen_at) \
                         VALUES (?1, ?2, ?3, ?4)",
                        rusqlite::params![
                            ent.entity_kind,
                            ent.entity_key,
                            ent.display_name,
                            ep.timestamp,
                        ],
                    )
                    .expect("INSERT entity");
                    let eid = conn.last_insert_rowid();
                    entity_cache.insert(key, eid);
                    eid
                }
            };

            let role = if ent.role.is_empty() {
                "participant"
            } else {
                &ent.role
            };
            conn.execute(
                "INSERT OR IGNORE INTO episode_entities (episode_id, entity_id, role) \
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![episode_id, entity_id, role],
            )
            .expect("INSERT episode_entities");
        }
    }
}

// ── Recall orchestrator (mirrors rerank.recall exactly) ───────────────────────

/// Run the hybrid recall pipeline against `conn` for one query.
///
/// This mirrors `retrieval.rerank.recall` without involving the HTTP layer.
/// Uses the same `candidate_multiplier = 5` the Python gate uses.
///
/// NOTE: duplicates the recall pipeline inline (no HTTP layer). If
/// retrieval::rerank::recall changes, update this to match.
fn run_recall(
    conn: &Connection,
    query_text: &str,
    query_vec: &[f32],
    now_ms: i64,
    top_k: usize,
    entity_filter_ids: Option<&HashSet<i64>>,
) -> Vec<(i64, f64, ComponentScores)> {
    let candidate_k = (top_k as i64 * 5).max(1);

    let fts = sanitize_fts_query(query_text);
    let bm25_raw = if !fts.is_empty() {
        bm25_search(conn, &fts, candidate_k).unwrap_or_default()
    } else {
        vec![]
    };
    let dense_raw = if !query_vec.is_empty() {
        dense_search(conn, query_vec, candidate_k).unwrap_or_default()
    } else {
        vec![]
    };

    let bm25_map: HashMap<i64, f64> = bm25_raw.iter().map(|h| (h.episode_id, h.bm25_score)).collect();
    let dense_map: HashMap<i64, f64> = dense_raw
        .iter()
        .map(|h| (h.episode_id, h.cosine_similarity))
        .collect();

    let bm25_norm = max_normalize(&bm25_map);
    let dense_norm = max_normalize(&dense_map);

    let mut candidate_ids: HashSet<i64> =
        bm25_norm.keys().chain(dense_norm.keys()).copied().collect();
    if let Some(filter) = entity_filter_ids {
        candidate_ids = candidate_ids.intersection(filter).copied().collect();
    }
    if candidate_ids.is_empty() {
        return vec![];
    }

    let placeholders: String = candidate_ids
        .iter()
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT episode_id, timestamp, salience_score, episode_type \
         FROM episodes WHERE episode_id IN ({})",
        placeholders
    );
    let params: Vec<rusqlite::types::Value> = candidate_ids
        .iter()
        .map(|&id| rusqlite::types::Value::Integer(id))
        .collect();
    let mut stmt = conn.prepare(&sql).expect("prepare hydration");
    let rows: Vec<(i64, i64, f64, String)> = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .expect("query_map")
        .map(|r| r.expect("row"))
        .collect();

    let scorer = HybridScorer::new(DEFAULT_ALPHA, DEFAULT_BETA, DEFAULT_GAMMA, DEFAULT_DELTA);
    let mut results: Vec<(i64, f64, ComponentScores)> = rows
        .into_iter()
        .map(|(eid, ts_ms, salience, ep_type)| {
            let half_life = half_life_for(&ep_type);
            let age_hours = ((now_ms - ts_ms) as f64) / 3_600_000.0;
            let decay = if age_hours <= 0.0 {
                1.0
            } else {
                0.5_f64.powf(age_hours / half_life)
            };
            let entity_match = match entity_filter_ids {
                Some(f) if f.contains(&eid) => 1.0,
                _ => 0.0,
            };
            let components = ComponentScores {
                bm25_norm: *bm25_norm.get(&eid).unwrap_or(&0.0),
                dense_norm: *dense_norm.get(&eid).unwrap_or(&0.0),
                decay,
                salience,
                entity_match,
            };
            let score = scorer.score(&components);
            (eid, score, components)
        })
        .collect();

    // Total order: primary desc by score, secondary asc by episode_id — matches
    // the rerank.recall tie-break so equal-score episodes are deterministic.
    results.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    results.truncate(top_k);
    results
}

// ── Metric helpers ────────────────────────────────────────────────────────────

/// recall@k: fraction of expected that appears in top-k of retrieved.
///
/// Bounded denominator `min(|expected|, k)` matches `_recall_at_k` in the
/// Python gate (queries with broad expected sets still get a fair score).
fn recall_at_k(retrieved: &[i64], expected: &[i64], k: usize) -> f64 {
    if expected.is_empty() {
        return 1.0;
    }
    let top: HashSet<i64> = retrieved.iter().copied().take(k).collect();
    let hits = expected.iter().filter(|e| top.contains(e)).count();
    let denom = expected.len().min(k);
    hits as f64 / denom as f64
}

/// Bounded precision@k: hits / min(k, |expected|).
fn precision_at_k(retrieved: &[i64], expected: &[i64], k: usize) -> f64 {
    if k == 0 || expected.is_empty() {
        return 0.0;
    }
    let expected_set: HashSet<i64> = expected.iter().copied().collect();
    let hits = retrieved
        .iter()
        .take(k)
        .filter(|id| expected_set.contains(id))
        .count();
    let denom = k.min(expected.len());
    hits as f64 / denom as f64
}

/// Linear-interpolation percentile on a sorted copy.
fn percentile(values: &[f64], pct: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sv = values.to_vec();
    sv.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let k = (sv.len() as f64 - 1.0) * (pct / 100.0);
    let f = k as usize;
    let c = (f + 1).min(sv.len() - 1);
    if f == c {
        sv[f]
    } else {
        sv[f] + (sv[c] - sv[f]) * (k - f as f64)
    }
}

/// Return the `n` lowest-scoring queries as `(query_text, score)` for debug output.
fn worst_queries<'a>(
    queries: &'a [QueryFixture],
    scores: &[f64],
    n: usize,
) -> Vec<(&'a str, f64)> {
    let mut paired: Vec<(&str, f64)> = queries
        .iter()
        .zip(scores.iter())
        .map(|(q, &s)| (q.query_text.as_str(), s))
        .collect();
    paired.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    paired.truncate(n);
    paired
}

// ── The gate test ─────────────────────────────────────────────────────────────

#[test]
#[ignore = "slow eval — run with: cargo test -p memory-rs --test quality_gate -- --ignored"]
fn quality_gate() {
    // register vec0 auto-extension once before opening any connection
    memory_rs::db::register_vec0();

    let corpus = load_corpus();
    assert_eq!(
        corpus.len(),
        200,
        "corpus size drift: expected 200, got {}",
        corpus.len()
    );
    let queries = load_queries();
    assert!(queries.len() >= 50, "query set too small: {}", queries.len());

    // Seed into a temp dir.
    let tmp = tempfile::tempdir().expect("tempdir");
    let conn = open_bot_db(tmp.path(), "eval-bot").expect("open_bot_db");

    seed_db(&conn, &corpus);

    // Invariant: SQLite AUTOINCREMENT episode_id == 1-indexed line number.
    let max_id: i64 = conn
        .query_row("SELECT MAX(episode_id) FROM episodes", [], |r| r.get(0))
        .expect("MAX(episode_id)");
    assert_eq!(
        max_id, 200,
        "episode_id != line number invariant: max={}",
        max_id
    );

    let mut per_query_recall: Vec<f64> = Vec::with_capacity(queries.len());
    let mut per_query_precision: Vec<f64> = Vec::with_capacity(queries.len());
    let mut per_query_latency_ms: Vec<f64> = Vec::with_capacity(queries.len());

    for q in &queries {
        let top_k = q.top_k.unwrap_or(5);
        let expected = &q.expected_episode_ids;
        let entity_names = q.entity_filter.as_deref().unwrap_or(&[]);

        let query_vec = stub_embed(&q.query_text);

        // Resolve entity filter.
        let entity_filter_ids: Option<HashSet<i64>> = if !entity_names.is_empty() {
            let ids = entity_filter(&conn, entity_names).expect("entity_filter");
            assert!(
                !ids.is_empty(),
                "entity_filter {:?} returned empty set for query {:?}; corpus drift?",
                entity_names,
                q.query_text
            );
            Some(ids)
        } else {
            None
        };

        // Apply episode_types narrowing (mirrors the Python gate).
        let effective_filter: Option<HashSet<i64>> = if let Some(types) = &q.episode_types {
            let placeholders: String = types.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT episode_id FROM episodes WHERE episode_type IN ({})",
                placeholders
            );
            let params: Vec<rusqlite::types::Value> = types
                .iter()
                .map(|t| rusqlite::types::Value::Text(t.clone()))
                .collect();
            let mut stmt = conn.prepare(&sql).expect("prepare type filter");
            let type_ids: HashSet<i64> = stmt
                .query_map(rusqlite::params_from_iter(params.iter()), |r| r.get(0))
                .expect("query_map type filter")
                .map(|r| r.expect("row"))
                .collect();
            Some(match entity_filter_ids {
                None => type_ids,
                Some(ef) => ef.intersection(&type_ids).copied().collect(),
            })
        } else {
            entity_filter_ids
        };

        let t0 = Instant::now();
        let results = run_recall(
            &conn,
            &q.query_text,
            &query_vec,
            EVAL_NOW_MS,
            top_k,
            effective_filter.as_ref(),
        );
        let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;

        let retrieved_ids: Vec<i64> = results.iter().map(|(id, _, _)| *id).collect();
        per_query_recall.push(recall_at_k(&retrieved_ids, expected, 5));
        per_query_precision.push(precision_at_k(&retrieved_ids, expected, 5));
        per_query_latency_ms.push(elapsed_ms);
    }

    conn.close().ok();

    let mean_recall = per_query_recall.iter().copied().sum::<f64>() / per_query_recall.len() as f64;
    let mean_precision =
        per_query_precision.iter().copied().sum::<f64>() / per_query_precision.len() as f64;
    let p95_latency = percentile(&per_query_latency_ms, 95.0);
    let p50_latency = percentile(&per_query_latency_ms, 50.0);

    eprintln!(
        "\n[eval] recall@5={:.3} precision@5={:.3} p50_ms={:.2} p95_ms={:.2}",
        mean_recall, mean_precision, p50_latency, p95_latency
    );

    assert!(
        mean_recall >= RECALL_AT_5_THRESHOLD,
        "recall@5 {:.3} < threshold {:.3}; worst queries: {:?}",
        mean_recall,
        RECALL_AT_5_THRESHOLD,
        worst_queries(&queries, &per_query_recall, 5)
    );
    assert!(
        mean_precision >= PRECISION_AT_5_THRESHOLD,
        "precision@5 {:.3} < threshold {:.3}; worst queries: {:?}",
        mean_precision,
        PRECISION_AT_5_THRESHOLD,
        worst_queries(&queries, &per_query_precision, 5)
    );
    assert!(
        p95_latency <= P95_LATENCY_MS_THRESHOLD,
        "p95 latency {:.2}ms > threshold {:.2}ms",
        p95_latency,
        P95_LATENCY_MS_THRESHOLD
    );
}
