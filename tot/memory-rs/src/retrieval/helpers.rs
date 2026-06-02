//! Per-bot eviction helper — ports `helpers.py::evict_if_over_cap`.
//!
//! ```python
//! def evict_if_over_cap(conn, bot_id, cap, now, w: ScoringWeights) -> int:
//!     cur = conn.execute(
//!         "SELECT id, salience, created_ts FROM memories WHERE bot_id=?", (bot_id,)
//!     )
//!     rows = cur.fetchall()
//!     if len(rows) <= cap:
//!         return 0
//!
//!     def score(row) -> float:
//!         age = max(0, now - row[2])
//!         return row[1] * math.exp(-age / w.tau_seconds)
//!
//!     rows_sorted = sorted(rows, key=score)
//!     n_evict = len(rows) - cap
//!     to_evict = [r[0] for r in rows_sorted[:n_evict]]
//!     for mid in to_evict:
//!         conn.execute("DELETE FROM memories WHERE id=?", (mid,))
//!         conn.execute("DELETE FROM vec_memories WHERE memory_id=?", (mid,))
//!         conn.execute("DELETE FROM memory_entities WHERE memory_id=?", (mid,))
//!     return n_evict
//! ```
//!
//! Scoring: `salience * exp(-max(0, now - created_ts) / tau_seconds)`.
//! No w_rel / w_rec / w_imp weights — eviction uses salience+recency only.
//! Rows are sorted ascending by score; the `n_evict` lowest are deleted.
//!
//! Must be called INSIDE the write transaction, BEFORE commit.  The
//! just-inserted row counts toward the cap.

use rusqlite::Connection;
use std::time::{SystemTime, UNIX_EPOCH};

/// Current Unix time in seconds — convenience for callers that don't already
/// have `now` in scope.
pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Evict the lowest-scoring memories for `bot_id` until the count is at most
/// `cap`.  Returns the number of rows evicted (0 if already within cap).
///
/// Scoring formula: `salience * exp(-max(0, now - created_ts) / tau_seconds)`.
///
/// Deletes from `memories`, `vec_memories`, and `memory_entities`.
/// Must be called inside an open write transaction.
pub fn evict_if_over_cap(
    conn: &Connection,
    bot_id: &str,
    cap: usize,
    now: i64,
    tau_seconds: u64,
) -> rusqlite::Result<usize> {
    // Fetch all memories for this bot.
    let mut stmt = conn.prepare(
        "SELECT id, salience, created_ts FROM memories WHERE bot_id = ?1",
    )?;
    let rows: Vec<(String, f64, i64)> = stmt
        .query_map(rusqlite::params![bot_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?, row.get::<_, i64>(2)?))
        })?
        .collect::<rusqlite::Result<_>>()?;

    if rows.len() <= cap {
        return Ok(0);
    }

    let tau = tau_seconds as f64;

    // Score each row: salience * exp(-age / tau_seconds).
    // Identical to Python's `score` closure.
    let mut scored: Vec<(f64, String)> = rows
        .into_iter()
        .map(|(id, salience, created_ts)| {
            let age = (now - created_ts).max(0) as f64;
            let score = salience * (-age / tau).exp();
            (score, id)
        })
        .collect();

    // Sort ascending by score — lowest scores (oldest/least salient) first.
    scored.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    let n_evict = scored.len() - cap;
    let to_evict: Vec<String> = scored.into_iter().take(n_evict).map(|(_, id)| id).collect();

    for mid in &to_evict {
        conn.execute("DELETE FROM memories WHERE id = ?1", rusqlite::params![mid])?;
        conn.execute(
            "DELETE FROM vec_memories WHERE memory_id = ?1",
            rusqlite::params![mid],
        )?;
        conn.execute(
            "DELETE FROM memory_entities WHERE memory_id = ?1",
            rusqlite::params![mid],
        )?;
    }

    Ok(n_evict)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn open_migrated() -> rusqlite::Connection {
        db::register_vec0();
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory db");
        let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/migrations"));
        db::migrate::run(&conn, dir).expect("migrate::run");
        conn
    }

    /// Insert a memory row with the given parameters.  Returns the memory_id.
    fn insert_memory(
        conn: &Connection,
        bot_id: &str,
        id: &str,
        salience: f64,
        created_ts: i64,
    ) {
        conn.execute(
            "INSERT OR IGNORE INTO bots (bot_id, created_ts) VALUES (?1, ?2)",
            rusqlite::params![bot_id, created_ts],
        )
        .expect("insert bot");

        conn.execute(
            "INSERT INTO memories (id, bot_id, text, salience, created_ts, last_recalled_ts, memory_type) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?5, 'event')",
            rusqlite::params![id, bot_id, format!("text for {id}"), salience, created_ts],
        )
        .expect("insert memory");

        // Insert a vec_memories row (embedding as empty blob — eviction only touches rowid).
        let blob = db::pack_f32_le(&vec![0.0_f32; 384]);
        conn.execute(
            "INSERT INTO vec_memories (memory_id, bot_id, embedding) VALUES (?1, ?2, ?3)",
            rusqlite::params![id, bot_id, blob],
        )
        .expect("insert vec_memories");
    }

    /// Below or at cap: no eviction, returns 0.
    #[test]
    fn below_cap_returns_zero() {
        let conn = open_migrated();
        let now = 1_700_000_000i64;
        insert_memory(&conn, "bot1", "m_aaa", 0.9, now - 100);
        insert_memory(&conn, "bot1", "m_bbb", 0.8, now - 200);

        let evicted = evict_if_over_cap(&conn, "bot1", 5, now, 604800).expect("evict");
        assert_eq!(evicted, 0, "below cap must evict 0");

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM memories WHERE bot_id='bot1'", [], |r| r.get(0))
            .expect("count");
        assert_eq!(count, 2);
    }

    /// At exact cap: no eviction, returns 0.
    #[test]
    fn at_cap_returns_zero() {
        let conn = open_migrated();
        let now = 1_700_000_000i64;
        for i in 0..3 {
            insert_memory(&conn, "bot1", &format!("m_{i:011}"), 0.5, now - i * 100);
        }
        let evicted = evict_if_over_cap(&conn, "bot1", 3, now, 604800).expect("evict");
        assert_eq!(evicted, 0, "at cap must evict 0");
    }

    /// Insert cap+3 rows; exactly 3 lowest-scored must be removed.
    #[test]
    fn over_cap_evicts_lowest_scored() {
        let conn = open_migrated();
        let cap = 3usize;
        let now = 1_700_000_000i64;
        let tau = 604800u64;

        // Insert 6 rows with known saliences + ages so we can predict which get evicted.
        // Score = salience * exp(-age / tau)
        // We design 3 high-score rows and 3 low-score rows.
        //
        // High (will survive): salience=1.0, age=0      → score = 1.0
        //                      salience=0.9, age=0      → score = 0.9
        //                      salience=0.8, age=0      → score = 0.8
        // Low (will be evicted): salience=0.1, age=tau  → score = 0.1/e ≈ 0.037
        //                        salience=0.1, age=2*tau→ score = 0.1/e^2 ≈ 0.014
        //                        salience=0.05, age=tau → score ≈ 0.018
        let memories: Vec<(&str, f64, i64)> = vec![
            ("m_high000001", 1.0, now),
            ("m_high000002", 0.9, now),
            ("m_high000003", 0.8, now),
            ("m_low0000001", 0.1, now - tau as i64),
            ("m_low0000002", 0.1, now - 2 * tau as i64),
            ("m_low0000003", 0.05, now - tau as i64),
        ];
        for (id, sal, ts) in &memories {
            insert_memory(&conn, "bot1", id, *sal, *ts);
        }

        let evicted = evict_if_over_cap(&conn, "bot1", cap, now, tau).expect("evict");
        assert_eq!(evicted, 3, "must evict exactly cap+3 - cap = 3 rows");

        let remaining_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM memories WHERE bot_id='bot1'", [], |r| r.get(0))
            .expect("count memories");
        assert_eq!(remaining_count, 3, "must have exactly cap=3 rows remaining");

        let remaining_vec_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM vec_memories WHERE bot_id='bot1'", [], |r| r.get(0))
            .expect("count vec_memories");
        assert_eq!(remaining_vec_count, 3, "vec_memories must also have 3 rows");

        // High-score rows must survive.
        for id in ["m_high000001", "m_high000002", "m_high000003"] {
            let exists: bool = conn
                .query_row(
                    "SELECT COUNT(*) FROM memories WHERE id=?1",
                    rusqlite::params![id],
                    |r| r.get::<_, i64>(0),
                )
                .map(|n| n > 0)
                .expect("check survival");
            assert!(exists, "high-score memory {id} must survive eviction");
        }

        // Low-score rows must be evicted.
        for id in ["m_low0000001", "m_low0000002", "m_low0000003"] {
            let exists: bool = conn
                .query_row(
                    "SELECT COUNT(*) FROM memories WHERE id=?1",
                    rusqlite::params![id],
                    |r| r.get::<_, i64>(0),
                )
                .map(|n| n > 0)
                .expect("check eviction");
            assert!(!exists, "low-score memory {id} must be evicted");
        }
    }

    /// Eviction also cleans memory_entities rows.
    #[test]
    fn eviction_removes_memory_entities() {
        let conn = open_migrated();
        let now = 1_700_000_000i64;
        let cap = 1usize;
        let tau = 604800u64;

        // Insert entity and two memories, one of which links to the entity.
        conn.execute(
            "INSERT OR IGNORE INTO bots (bot_id, created_ts) VALUES ('botx', ?1)",
            rusqlite::params![now],
        )
        .expect("insert bot");
        conn.execute(
            "INSERT INTO entities (bot_id, name_lower, display_name) VALUES ('botx', 'alice', 'Alice')",
            [],
        )
        .expect("insert entity");
        let entity_id: i64 = conn
            .query_row("SELECT id FROM entities WHERE name_lower='alice'", [], |r| r.get(0))
            .expect("get entity_id");

        insert_memory(&conn, "botx", "m_keeper0001", 1.0, now);
        insert_memory(&conn, "botx", "m_evicted001", 0.01, now - 2 * tau as i64);

        // Link the evicted memory to the entity.
        conn.execute(
            "INSERT OR IGNORE INTO memory_entities (memory_id, entity_id) VALUES ('m_evicted001', ?1)",
            rusqlite::params![entity_id],
        )
        .expect("insert memory_entities");

        let evicted = evict_if_over_cap(&conn, "botx", cap, now, tau).expect("evict");
        assert_eq!(evicted, 1);

        let me_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memory_entities WHERE memory_id='m_evicted001'",
                [],
                |r| r.get(0),
            )
            .expect("count memory_entities");
        assert_eq!(me_count, 0, "memory_entities for evicted memory must be removed");
    }
}
