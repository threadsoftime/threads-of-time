// SPDX-License-Identifier: GPL-2.0-or-later
//! Hybrid recall orchestrator — design subspec §6.1 / §6.4.
//!
//! Two-stage pipeline:
//! 1. Candidate generation: BM25 ∪ dense KNN, intersect entity filter.
//! 2. Hybrid scoring: per-episode decay + ComponentScores → HybridScorer → sort.

use std::collections::{HashMap, HashSet};

use rusqlite::Connection;

use super::{
    bm25::bm25_search,
    decay::{decay_weight, half_life_for},
    dense::dense_search,
    hybrid::{ComponentScores, HybridScorer},
};

/// Max-normalise a score map to `[0, 1]`.
///
/// * Empty input → empty map.
/// * `max ≤ 0` → all 0.0.
/// * Negative individual values are clipped to 0 before dividing.
pub fn max_normalize(scores: &HashMap<i64, f64>) -> HashMap<i64, f64> {
    if scores.is_empty() {
        return HashMap::new();
    }
    let max = scores.values().cloned().fold(f64::NEG_INFINITY, f64::max);
    if max <= 0.0 {
        return scores.keys().map(|&k| (k, 0.0)).collect();
    }
    scores
        .iter()
        .map(|(&k, &v)| (k, (v.max(0.0)) / max))
        .collect()
}

/// A single hybrid-scored episode with its component breakdown.
#[derive(Debug, Clone, PartialEq)]
pub struct RecallResult {
    pub episode_id:   i64,
    pub score:        f64,
    pub bm25_norm:    f64,
    pub dense_norm:   f64,
    pub decay:        f64,
    pub salience:     f64,
    pub entity_match: f64,
}

/// Hybrid recall: BM25 ∪ dense → entity filter → score → top-K.
///
/// # Arguments
/// * `conn`                — bot-scoped rusqlite connection with migrations applied.
/// * `query_text`          — query for BM25; empty string → BM25 step skipped.
/// * `query_vec`           — query embedding; empty slice → dense step skipped.
/// * `now_ms`              — current wall-clock as unix epoch milliseconds.
/// * `top_k`               — number of results to return after final ranking.
/// * `entity_filter_ids`   — when `Some`, hard-filter: only these episode ids are scored.
/// * `alpha,beta,gamma,delta` — hybrid scorer weights (use `DEFAULT_*` for defaults).
/// * `candidate_multiplier` — stage-1 over-fetch factor (default 5).
#[allow(clippy::too_many_arguments)]
pub fn recall(
    conn:               &Connection,
    query_text:         &str,
    query_vec:          &[f32],
    now_ms:             i64,
    top_k:              i64,
    entity_filter_ids:  Option<&HashSet<i64>>,
    alpha:              f64,
    beta:               f64,
    gamma:              f64,
    delta:              f64,
    candidate_multiplier: i64,
) -> rusqlite::Result<Vec<RecallResult>> {
    let candidate_k = (top_k * candidate_multiplier).max(1);

    // ── Stage 1: candidate generation ───────────────────────────────────────
    let bm25_map: HashMap<i64, f64> = if !query_text.is_empty() {
        bm25_search(conn, query_text, candidate_k)?
            .into_iter()
            .map(|h| (h.episode_id, h.bm25_score))
            .collect()
    } else {
        HashMap::new()
    };

    let dense_map: HashMap<i64, f64> = if !query_vec.is_empty() {
        dense_search(conn, query_vec, candidate_k)?
            .into_iter()
            .map(|h| (h.episode_id, h.cosine_similarity))
            .collect()
    } else {
        HashMap::new()
    };

    let bm25_norm  = max_normalize(&bm25_map);
    let dense_norm = max_normalize(&dense_map);

    let mut candidate_ids: HashSet<i64> = bm25_norm.keys().chain(dense_norm.keys()).cloned().collect();

    // ── Entity filter intersection ───────────────────────────────────────────
    if let Some(filter) = entity_filter_ids {
        candidate_ids = candidate_ids.intersection(filter).cloned().collect();
    }

    if candidate_ids.is_empty() {
        return Ok(vec![]);
    }

    // ── Stage 2: hydrate + score ─────────────────────────────────────────────
    let placeholders: Vec<String> = (1..=candidate_ids.len())
        .map(|i| format!("?{i}"))
        .collect();
    let sql = format!(
        "SELECT episode_id, timestamp, salience_score, episode_type \
         FROM episodes WHERE episode_id IN ({})",
        placeholders.join(", ")
    );

    let ids_vec: Vec<i64> = candidate_ids.into_iter().collect();
    let params: Vec<&dyn rusqlite::ToSql> =
        ids_vec.iter().map(|id| id as &dyn rusqlite::ToSql).collect();

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params.as_slice(), |row| {
            Ok((
                row.get::<_, i64>(0)?,   // episode_id
                row.get::<_, i64>(1)?,   // timestamp ms
                row.get::<_, f64>(2)?,   // salience_score
                row.get::<_, String>(3)?, // episode_type
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let scorer = HybridScorer::new(alpha, beta, gamma, delta);
    let mut results: Vec<RecallResult> = rows
        .into_iter()
        .map(|(eid, ts_ms, salience, etype)| {
            let hl    = half_life_for(&etype);
            let decay = decay_weight(ts_ms, now_ms, hl);
            let entity_match = match entity_filter_ids {
                Some(filter) if filter.contains(&eid) => 1.0,
                _ => 0.0,
            };
            let c = ComponentScores {
                bm25_norm:    *bm25_norm.get(&eid).unwrap_or(&0.0),
                dense_norm:   *dense_norm.get(&eid).unwrap_or(&0.0),
                decay,
                salience,
                entity_match,
            };
            RecallResult {
                episode_id:   eid,
                score:        scorer.score(&c),
                bm25_norm:    c.bm25_norm,
                dense_norm:   c.dense_norm,
                decay:        c.decay,
                salience:     c.salience,
                entity_match: c.entity_match,
            }
        })
        .collect();

    // Total order: primary desc by score, secondary asc by episode_id.
    // The episode_id tiebreak disambiguates floating-point equal scores (rare
    // in practice; deterministic in tests and parity comparisons).
    // The Python rerank.recall must use the same key: `(-score, episode_id)`.
    results.sort_by(|a, b| {
        b.score.partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.episode_id.cmp(&b.episode_id))
    });
    results.truncate(top_k as usize);
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{migrate::run_migrations, open_bot_db};
    use crate::retrieval::hybrid::{DEFAULT_ALPHA, DEFAULT_BETA, DEFAULT_DELTA, DEFAULT_GAMMA};
    use crate::EMBEDDING_DIM;
    use tempfile::TempDir;

    fn setup() -> (TempDir, Connection) {
        let dir = TempDir::new().expect("tempdir");
        let conn = open_bot_db(dir.path(), "test-bot").expect("open_bot_db");
        run_migrations(&conn).expect("run_migrations");
        (dir, conn)
    }

    fn ts_ms(year: i32, month: u32, day: u32, hour: u32) -> i64 {
        // Simple deterministic timestamp builder in ms.
        // Approximation: days since 1970-01-01 using year/month/day offsets is
        // fine here — we only need relative ordering, not calendar precision.
        let days_since_epoch =
            (year as i64 - 1970) * 365 + (month as i64 - 1) * 30 + day as i64;
        (days_since_epoch * 86_400 + hour as i64 * 3_600) * 1_000
    }

    fn now_ms() -> i64 {
        ts_ms(2026, 5, 27, 13)
    }

    fn e1() -> Vec<f32> {
        let mut v = vec![0.0_f32; EMBEDDING_DIM];
        v[0] = 1.0;
        v
    }
    fn e2() -> Vec<f32> {
        let mut v = vec![0.0_f32; EMBEDDING_DIM];
        v[1] = 1.0;
        v
    }

    fn seed_full(
        conn:     &Connection,
        ts:       i64,
        text:     &str,
        etype:    &str,
        salience: f64,
        vec:      &[f32],
    ) -> i64 {
        conn.execute(
            "INSERT INTO episodes (timestamp, content_text, episode_type, salience_score) \
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![ts, text, etype, salience],
        )
        .expect("insert episode");
        let eid = conn.last_insert_rowid();
        let bytes: Vec<u8> = vec.iter().flat_map(|x| x.to_le_bytes()).collect();
        conn.execute(
            "INSERT INTO embeddings_vec (rowid, embedding) VALUES (?1, ?2)",
            rusqlite::params![eid, bytes],
        )
        .expect("insert vec");
        conn.execute(
            "UPDATE episodes SET content_embedding_id = ?1 WHERE episode_id = ?1",
            rusqlite::params![eid],
        )
        .expect("update embedding id");
        eid
    }

    fn add_entity(conn: &Connection, episode_id: i64, display_name: &str) {
        conn.execute(
            "INSERT INTO entities (entity_kind, entity_key, display_name) VALUES (?1, ?2, ?3)",
            rusqlite::params!["player", display_name.to_lowercase(), display_name],
        )
        .expect("insert entity");
        let entity_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO episode_entities (episode_id, entity_id, role) VALUES (?1, ?2, ?3)",
            rusqlite::params![episode_id, entity_id, "subject"],
        )
        .expect("link entity");
    }

    fn default_recall(conn: &Connection, text: &str, vec: &[f32], top_k: i64) -> Vec<RecallResult> {
        recall(
            conn, text, vec, now_ms(), top_k, None,
            DEFAULT_ALPHA, DEFAULT_BETA, DEFAULT_GAMMA, DEFAULT_DELTA, 5,
        )
        .expect("recall")
    }

    /// Fresh + salient + keyword-matching episode outranks stale + low-salience + unmatched.
    #[test]
    fn recall_ranks_fresh_salient_keyword_match_first() {
        let (_dir, conn) = setup();
        let winner = seed_full(&conn, ts_ms(2026, 5, 27, 12),
            "Alice and I cleared BFD", "social", 0.9, &e1());
        let loser  = seed_full(&conn, ts_ms(2026, 4, 1, 12),
            "I died alone", "combat", 0.1, &e2());
        conn.execute_batch("COMMIT").ok();

        let results = default_recall(&conn, "Alice", &e1(), 2);
        assert!(!results.is_empty());
        assert_eq!(results[0].episode_id, winner,
            "fresh salient winner should rank first");
        if results.len() > 1 {
            let loser_result = results.iter().find(|r| r.episode_id == loser);
            if let Some(l) = loser_result {
                assert!(results[0].score > l.score);
            }
        }
    }

    /// top_k=1 returns at most one result even with many candidates.
    #[test]
    fn recall_respects_top_k() {
        let (_dir, conn) = setup();
        for i in 0..5 {
            seed_full(&conn, ts_ms(2026, 5, 27, 10 + i),
                &format!("Alice helped me episode {i}"), "social", 0.5, &e1());
        }
        conn.execute_batch("COMMIT").ok();
        let results = default_recall(&conn, "Alice", &e1(), 1);
        assert_eq!(results.len(), 1, "top_k=1 must return exactly 1 result");
    }

    /// No BM25 hits and empty embeddings table → empty vec.
    #[test]
    fn recall_returns_empty_when_no_candidates_match() {
        let (_dir, conn) = setup();
        let zero_vec = vec![0.0_f32; EMBEDDING_DIM];
        let results = default_recall(&conn, "nothingmatcheshere", &zero_vec, 5);
        assert!(results.is_empty());
    }

    /// entity_filter_ids hard-filters: episodes not in the set are never scored.
    #[test]
    fn recall_entity_filter_drops_unmatched_episodes() {
        let (_dir, conn) = setup();
        let alice_ep = seed_full(&conn, ts_ms(2026, 5, 27, 12),
            "Alice helped me", "social", 0.7, &e1());
        add_entity(&conn, alice_ep, "Alice");
        let bob_ep = seed_full(&conn, ts_ms(2026, 5, 27, 12),
            "Bob helped me", "social", 0.7, &e1());
        add_entity(&conn, bob_ep, "Bob");
        conn.execute_batch("COMMIT").ok();

        let filter = HashSet::from([alice_ep]);
        let results = recall(
            &conn, "helped", &e1(), now_ms(), 5, Some(&filter),
            DEFAULT_ALPHA, DEFAULT_BETA, DEFAULT_GAMMA, DEFAULT_DELTA, 5,
        )
        .expect("recall");

        let ids: HashSet<i64> = results.iter().map(|r| r.episode_id).collect();
        assert!(ids.contains(&alice_ep), "alice_ep must be in results");
        assert!(!ids.contains(&bob_ep), "bob_ep must be excluded by filter");
    }

    /// Entity filter that matches nothing → empty result (hard filter semantics).
    #[test]
    fn recall_empty_entity_filter_intersection_returns_empty() {
        let (_dir, conn) = setup();
        let _ep = seed_full(&conn, ts_ms(2026, 5, 27, 12),
            "Alice helped me", "social", 0.7, &e1());
        conn.execute_batch("COMMIT").ok();

        // Filter to episode id 9999 which doesn't exist.
        let filter = HashSet::from([9999_i64]);
        let results = recall(
            &conn, "Alice", &e1(), now_ms(), 5, Some(&filter),
            DEFAULT_ALPHA, DEFAULT_BETA, DEFAULT_GAMMA, DEFAULT_DELTA, 5,
        )
        .expect("recall");
        assert!(results.is_empty(), "no intersection → must return empty");
    }

    /// BM25-only path (empty query_vec) still returns results.
    #[test]
    fn recall_bm25_only_path() {
        let (_dir, conn) = setup();
        seed_full(&conn, ts_ms(2026, 5, 27, 12),
            "Alice tanked the boss", "combat", 0.6, &e1());
        conn.execute_batch("COMMIT").ok();

        let results = recall(
            &conn, "Alice", &[], now_ms(), 5, None,
            DEFAULT_ALPHA, DEFAULT_BETA, DEFAULT_GAMMA, DEFAULT_DELTA, 5,
        )
        .expect("recall");
        assert!(!results.is_empty(), "BM25-only path must return results");
        assert_eq!(results[0].dense_norm, 0.0, "dense_norm must be 0 on BM25-only path");
    }

    /// Dense-only path (empty query_text) still returns results.
    #[test]
    fn recall_dense_only_path() {
        let (_dir, conn) = setup();
        seed_full(&conn, ts_ms(2026, 5, 27, 12),
            "some episode text", "social", 0.5, &e1());
        conn.execute_batch("COMMIT").ok();

        let results = recall(
            &conn, "", &e1(), now_ms(), 5, None,
            DEFAULT_ALPHA, DEFAULT_BETA, DEFAULT_GAMMA, DEFAULT_DELTA, 5,
        )
        .expect("recall");
        assert!(!results.is_empty(), "dense-only path must return results");
        assert_eq!(results[0].bm25_norm, 0.0, "bm25_norm must be 0 on dense-only path");
    }

    /// candidate_multiplier=5: candidate_k = top_k * 5.
    /// Smoke test: with 3 episodes and top_k=1, multiplier=5 fetches up to 5 candidates
    /// but returns only 1.
    #[test]
    fn recall_candidate_multiplier_controls_over_fetch() {
        let (_dir, conn) = setup();
        for i in 0..3_i32 {
            seed_full(&conn, ts_ms(2026, 5, 27, 10 + i as u32),
                &format!("Alice episode {i}"), "social", 0.5, &e1());
        }
        conn.execute_batch("COMMIT").ok();

        let results = recall(
            &conn, "Alice", &e1(), now_ms(), 1, None,
            DEFAULT_ALPHA, DEFAULT_BETA, DEFAULT_GAMMA, DEFAULT_DELTA, 5,
        )
        .expect("recall");
        assert_eq!(results.len(), 1);
    }

    /// Result components are within expected ranges and score field matches formula.
    #[test]
    fn recall_result_exposes_score_breakdown() {
        let (_dir, conn) = setup();
        seed_full(&conn, ts_ms(2026, 5, 27, 12),
            "Alice tanked", "social", 0.7, &e1());
        conn.execute_batch("COMMIT").ok();

        let results = default_recall(&conn, "Alice", &e1(), 1);
        assert_eq!(results.len(), 1);
        let r = &results[0];
        assert!((0.0..=1.0).contains(&r.bm25_norm),   "bm25_norm out of range");
        assert!((0.0..=1.0).contains(&r.dense_norm),  "dense_norm out of range");
        assert!((0.0..=1.0).contains(&r.decay),       "decay out of range");
        assert!((0.0..=1.0).contains(&r.salience),    "salience out of range");
        assert!(r.entity_match == 0.0 || r.entity_match == 1.0, "entity_match must be 0 or 1");

        // Verify score matches formula (with entity_match=0 since no filter was applied).
        let scorer = crate::retrieval::hybrid::HybridScorer::default();
        let expected = scorer.score(&ComponentScores {
            bm25_norm:    r.bm25_norm,
            dense_norm:   r.dense_norm,
            decay:        r.decay,
            salience:     r.salience,
            entity_match: r.entity_match,
        });
        assert!((r.score - expected).abs() < 1e-9,
            "score={} does not match formula expected={}", r.score, expected);
    }

    /// Results are sorted descending by score.
    #[test]
    fn recall_results_sorted_descending() {
        let (_dir, conn) = setup();
        // seed several episodes with clearly different salience so ordering is deterministic.
        seed_full(&conn, ts_ms(2026, 5, 27, 12), "Alice fought hard", "social", 0.9, &e1());
        seed_full(&conn, ts_ms(2026, 5, 27, 11), "Alice mentioned it", "social", 0.3, &e1());
        seed_full(&conn, ts_ms(2026, 5, 27, 10), "Alice was there", "social", 0.5, &e1());
        conn.execute_batch("COMMIT").ok();

        let results = default_recall(&conn, "Alice", &e1(), 3);
        assert!(results.len() > 1);
        for window in results.windows(2) {
            assert!(
                window[0].score >= window[1].score,
                "results must be sorted descending: {} < {}",
                window[0].score, window[1].score
            );
        }
    }

    /// Union of BM25 and dense candidates: an episode that matches BM25 but has no
    /// embedding still appears in results (bm25_norm > 0, dense_norm = 0).
    #[test]
    fn recall_union_includes_bm25_only_candidate() {
        let (_dir, conn) = setup();
        // Episode with text match but no embedding.
        conn.execute(
            "INSERT INTO episodes (timestamp, content_text, episode_type, salience_score) \
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![ts_ms(2026, 5, 27, 12), "Alice solo dungeon", "social", 0.5_f64],
        )
        .expect("insert");
        conn.execute_batch("COMMIT").ok();

        // Dense query: empty slice → skip dense, rely on BM25 only.
        let results = recall(
            &conn, "Alice", &[], now_ms(), 5, None,
            DEFAULT_ALPHA, DEFAULT_BETA, DEFAULT_GAMMA, DEFAULT_DELTA, 5,
        )
        .expect("recall");
        assert!(!results.is_empty(), "BM25-only candidate must be returned");
        assert!(results[0].bm25_norm > 0.0, "bm25_norm should be > 0");
        assert_eq!(results[0].dense_norm, 0.0, "dense_norm should be 0");
    }
}
