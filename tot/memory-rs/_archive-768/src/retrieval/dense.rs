// SPDX-License-Identifier: GPL-2.0-or-later
//! Dense vector retrieval via sqlite-vec KNN over `embeddings_vec`.
//!
//! sqlite-vec vec0 returns the configured distance metric for the table's
//! vector column. For L2-normalized embeddings (nomic-embed-text outputs
//! unit-norm vectors), the configured distance is equivalent to cosine
//! distance: `distance = 1 - cosine_similarity`. We convert to similarity
//! via `cosine_similarity = 1.0 - distance` so higher = more relevant,
//! matching the hybrid scorer's convention (design subspec §6.3 / §6.5).
//! NOTE: this mapping is valid only for L2-normalized vectors; unnormalized
//! vectors would require a different conversion.

use rusqlite::Connection;

use crate::EMBEDDING_DIM;

/// A single dense-KNN hit.
#[derive(Debug, Clone, PartialEq)]
pub struct DenseHit {
    pub episode_id: i64,
    /// `1.0 - L2_distance` — higher = more relevant.
    pub cosine_similarity: f64,
}

/// Pack a `&[f32]` as little-endian bytes for sqlite-vec `MATCH`.
///
/// x86-64 is LE, so this is native on the deploy target.
fn pack_vec(vec: &[f32]) -> Vec<u8> {
    vec.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// Return the top-`top_k` episodes by L2 distance to `query_vec`.
///
/// Returns `vec![]` when `top_k <= 0` or the `embeddings_vec` table is empty.
/// Panics (debug) / returns error (release) if `query_vec.len() != EMBEDDING_DIM`.
pub fn dense_search(
    conn: &Connection,
    query_vec: &[f32],
    top_k: i64,
) -> rusqlite::Result<Vec<DenseHit>> {
    if top_k <= 0 {
        return Ok(vec![]);
    }
    debug_assert_eq!(
        query_vec.len(),
        EMBEDDING_DIM,
        "query_vec length {} != EMBEDDING_DIM {}",
        query_vec.len(),
        EMBEDDING_DIM
    );
    let query_bytes = pack_vec(query_vec);
    let mut stmt = conn.prepare_cached(
        "SELECT rowid, distance \
         FROM embeddings_vec \
         WHERE embedding MATCH ?1 AND k = ?2 \
         ORDER BY distance",
    )?;
    let hits = stmt
        .query_map(rusqlite::params![query_bytes, top_k], |row| {
            Ok(DenseHit {
                episode_id:        row.get::<_, i64>(0)?,
                cosine_similarity: 1.0 - row.get::<_, f64>(1)?,
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

    fn seed_vec(conn: &Connection, rowid: i64, vec: &[f32]) {
        let bytes = pack_vec(vec);
        conn.execute(
            "INSERT INTO embeddings_vec (rowid, embedding) VALUES (?1, ?2)",
            rusqlite::params![rowid, bytes],
        )
        .expect("seed_vec insert");
    }

    /// Build a 768-dim unit vector with 1.0 at index 0, rest 0.0.
    fn e1() -> Vec<f32> {
        let mut v = vec![0.0_f32; EMBEDDING_DIM];
        v[0] = 1.0;
        v
    }

    /// Build a 768-dim unit vector with 1.0 at index 1, rest 0.0 (orthogonal to e1).
    fn e2() -> Vec<f32> {
        let mut v = vec![0.0_f32; EMBEDDING_DIM];
        v[1] = 1.0;
        v
    }

    /// Query parallel to episode 1 → episode 1 ranks above orthogonal episode 2.
    #[test]
    fn dense_ranks_parallel_vector_above_orthogonal() {
        let (_dir, conn) = setup();
        seed_vec(&conn, 1, &e1());
        seed_vec(&conn, 2, &e2());
        conn.execute_batch("COMMIT").ok();

        let results = dense_search(&conn, &e1(), 2).expect("dense_search");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].episode_id, 1, "parallel episode should rank first");
        assert!(
            results[0].cosine_similarity > results[1].cosine_similarity,
            "parallel sim {} should exceed orthogonal sim {}",
            results[0].cosine_similarity,
            results[1].cosine_similarity
        );
    }

    /// Identical vectors → L2 distance 0 → cosine_similarity ≈ 1.0.
    #[test]
    fn dense_identical_vectors_give_similarity_one() {
        let (_dir, conn) = setup();
        let vec = e1();
        seed_vec(&conn, 1, &vec);
        conn.execute_batch("COMMIT").ok();

        let results = dense_search(&conn, &vec, 1).expect("dense_search");
        assert_eq!(results.len(), 1);
        let sim = results[0].cosine_similarity;
        assert!((sim - 1.0).abs() < 1e-5, "expected ~1.0, got {sim}");
    }

    /// Empty embeddings_vec table → empty result list, no error.
    #[test]
    fn dense_empty_table_returns_empty() {
        let (_dir, conn) = setup();
        let query = vec![0.5_f32; EMBEDDING_DIM];
        let results = dense_search(&conn, &query, 5).expect("dense_search");
        assert!(results.is_empty());
    }

    /// top_k <= 0 → immediately return empty vec (guard before any DB call).
    #[test]
    fn dense_top_k_zero_returns_empty() {
        let (_dir, conn) = setup();
        seed_vec(&conn, 1, &e1());
        conn.execute_batch("COMMIT").ok();
        let results = dense_search(&conn, &e1(), 0).expect("dense_search");
        assert!(results.is_empty(), "top_k=0 must return empty");
    }

    #[test]
    fn dense_top_k_negative_returns_empty() {
        let (_dir, conn) = setup();
        seed_vec(&conn, 1, &e1());
        conn.execute_batch("COMMIT").ok();
        let results = dense_search(&conn, &e1(), -5).expect("dense_search");
        assert!(results.is_empty(), "top_k<0 must return empty");
    }

    /// cosine_similarity = 1 - distance, so it is in a sensible range.
    #[test]
    fn dense_cosine_similarity_is_one_minus_distance() {
        let (_dir, conn) = setup();
        seed_vec(&conn, 1, &e1());
        seed_vec(&conn, 2, &e2());
        conn.execute_batch("COMMIT").ok();

        let results = dense_search(&conn, &e1(), 2).expect("dense_search");
        assert_eq!(results.len(), 2);
        // The parallel episode has distance ≈ 0 → similarity ≈ 1.0
        // The orthogonal episode has distance = sqrt(2) → similarity ≈ 1 - 1.414 ≈ -0.414
        // (negative is acceptable — max_normalize will clip it to 0)
        let sim_parallel = results.iter().find(|r| r.episode_id == 1).unwrap().cosine_similarity;
        let sim_ortho    = results.iter().find(|r| r.episode_id == 2).unwrap().cosine_similarity;
        assert!(sim_parallel > sim_ortho, "parallel should have higher similarity");
        assert!((sim_parallel - 1.0).abs() < 1e-4, "parallel sim ≈ 1.0, got {sim_parallel}");
    }

    /// LE f32 packing: a known 2-element vector packs to 8 bytes with correct byte order.
    #[test]
    fn dense_le_f32_packing() {
        // 1.0_f32 LE bytes = [0x00, 0x00, 0x80, 0x3F]
        // 2.0_f32 LE bytes = [0x00, 0x00, 0x00, 0x40]
        let v = [1.0_f32, 2.0_f32];
        let bytes = pack_vec(&v);
        assert_eq!(bytes.len(), 8);
        assert_eq!(&bytes[0..4], &1.0_f32.to_le_bytes());
        assert_eq!(&bytes[4..8], &2.0_f32.to_le_bytes());
    }
}
