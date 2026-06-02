// SPDX-License-Identifier: GPL-2.0-or-later
//! BM25 keyword retrieval over the `episodes_fts` FTS5 virtual table.
//!
//! SQLite's `bm25()` returns lower-is-better (negative rank). We negate so
//! higher scores map to more relevant results, matching the hybrid scorer.

use rusqlite::Connection;

/// A single BM25 hit.
#[derive(Debug, Clone, PartialEq)]
pub struct Bm25Hit {
    pub episode_id: i64,
    /// Negated FTS5 bm25() — higher = more relevant.
    pub bm25_score: f64,
}

/// Return up to `top_k` episodes whose `content_text` matches `query`.
///
/// Passes `query` straight to FTS5 `MATCH`. Negates `bm25()` so higher score
/// means better match. Returns `vec![]` on empty results; callers are
/// responsible for sending non-empty queries.
pub fn bm25_search(
    conn: &Connection,
    query: &str,
    top_k: i64,
) -> rusqlite::Result<Vec<Bm25Hit>> {
    let mut stmt = conn.prepare_cached(
        "SELECT rowid, -bm25(episodes_fts) AS score \
         FROM episodes_fts \
         WHERE episodes_fts MATCH ?1 \
         ORDER BY score DESC \
         LIMIT ?2",
    )?;
    let hits = stmt
        .query_map(rusqlite::params![query, top_k], |row| {
            Ok(Bm25Hit {
                episode_id: row.get::<_, i64>(0)?,
                bm25_score: row.get::<_, f64>(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{migrate::run_migrations, open_bot_db};
    use tempfile::TempDir;

    fn setup() -> (TempDir, Connection) {
        let dir = TempDir::new().expect("tempdir");
        let conn = open_bot_db(dir.path(), "test-bot").expect("open_bot_db");
        run_migrations(&conn).expect("run_migrations");
        (dir, conn)
    }

    fn seed(conn: &Connection, episodes: &[(i64, &str, &str, f64)]) {
        for &(ts, text, etype, salience) in episodes {
            conn.execute(
                "INSERT INTO episodes (timestamp, content_text, episode_type, salience_score) \
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![ts, text, etype, salience],
            )
            .expect("seed insert");
        }
        conn.execute_batch("COMMIT; BEGIN").ok();
        // FTS triggers fire on INSERT so just commit.
        conn.execute_batch("COMMIT").ok();
    }

    /// BM25 matches episodes containing the query token; unmatched episodes
    /// are not returned.
    #[test]
    fn bm25_returns_matching_episodes_only() {
        let (_dir, conn) = setup();
        seed(
            &conn,
            &[
                (1_748_395_200_000, "Alice taught me how to AoE pull", "social", 0.7),
                (1_748_395_500_000, "I died to a boss", "combat", 0.5),
                (1_748_395_800_000, "Alice and I cleared a dungeon", "social", 0.8),
            ],
        );
        let results = bm25_search(&conn, "Alice", 10).expect("bm25_search");
        let ids: std::collections::HashSet<i64> = results.iter().map(|r| r.episode_id).collect();
        assert!(ids.contains(&1), "episode 1 should match");
        assert!(ids.contains(&3), "episode 3 should match");
        assert!(!ids.contains(&2), "episode 2 should NOT match");
    }

    /// Negated bm25() means all returned scores are positive.
    #[test]
    fn bm25_scores_are_positive_after_negation() {
        let (_dir, conn) = setup();
        seed(
            &conn,
            &[
                (1_748_395_200_000, "Alice Alice Alice tanks BFD", "social", 0.7),
                (1_748_395_500_000, "Alice mentioned tanks once", "social", 0.5),
            ],
        );
        let results = bm25_search(&conn, "Alice tanks", 10).expect("bm25_search");
        assert_eq!(results.len(), 2);
        for r in &results {
            assert!(r.bm25_score > 0.0, "score should be positive, got {}", r.bm25_score);
        }
    }

    /// More occurrences of the query token → higher BM25 score (ordering test).
    #[test]
    fn bm25_higher_frequency_ranks_first() {
        let (_dir, conn) = setup();
        seed(
            &conn,
            &[
                (1_748_395_200_000, "Alice Alice Alice tanks BFD", "social", 0.7),
                (1_748_395_500_000, "Alice mentioned tanks once", "social", 0.5),
            ],
        );
        let results = bm25_search(&conn, "Alice tanks", 10).expect("bm25_search");
        assert_eq!(results.len(), 2);
        // Results are already ORDER BY score DESC; first result must be episode 1.
        assert_eq!(results[0].episode_id, 1, "high-frequency episode should rank first");
    }

    /// Empty database → empty result list, no error.
    #[test]
    fn bm25_empty_db_returns_empty() {
        let (_dir, conn) = setup();
        let results = bm25_search(&conn, "anything", 5).expect("bm25_search");
        assert!(results.is_empty());
    }

    /// `top_k` limits the number of returned results.
    #[test]
    fn bm25_respects_top_k_limit() {
        let (_dir, conn) = setup();
        seed(
            &conn,
            &[
                (1_748_395_200_000, "Alice was here episode one", "social", 0.5),
                (1_748_395_300_000, "Alice was here episode two", "social", 0.5),
                (1_748_395_400_000, "Alice was here episode three", "social", 0.5),
            ],
        );
        let results = bm25_search(&conn, "Alice", 2).expect("bm25_search");
        assert!(results.len() <= 2);
    }
}
