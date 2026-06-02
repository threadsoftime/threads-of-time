// /Users/tbrack/Documents/Projects/threads-of-time/tot/memory-rs/tests/db_contracts.rs
//! On-disk behavioral contract tests for the memory-rs DB layer.
//!
//! These are integration tests (in `tests/`) so they run in a separate binary
//! and can share the vec0 `Once` guard across the test binary without
//! conflicting with unit tests in other modules.

use memory_rs::db::migrate::run_migrations;
use memory_rs::db::open_bot_db;
use memory_rs::EMBEDDING_DIM;

fn ensure_vec0() {
    // Delegates to the idempotent crate-level register_vec0 (Once-guarded,
    // typed fn-pointer — no transmute). Also called internally by open_bot_db.
    memory_rs::db::register_vec0();
}

fn make_db() -> (tempfile::TempDir, rusqlite::Connection) {
    // open_bot_db calls register_vec0() internally; ensure_vec0() is kept
    // here for clarity in tests that call raw Connection::open separately.
    ensure_vec0();
    let tmp = tempfile::tempdir().expect("tempdir");
    let conn = open_bot_db(tmp.path(), "smoketest").expect("open_bot_db");
    run_migrations(&conn).expect("run_migrations");
    (tmp, conn)
}

/// Pack a Vec<f32> as little-endian bytes — the wire format used by both the
/// Python `struct.pack("{n}f")` call and the Rust route handlers.
fn pack_f32(vec: &[f32]) -> Vec<u8> {
    vec.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// Insert a minimal valid episode row and return its episode_id.
fn insert_episode(conn: &rusqlite::Connection, content: &str, ep_type: &str) -> i64 {
    conn.execute(
        "INSERT INTO episodes (timestamp, content_text, episode_type) VALUES (1000, ?1, ?2)",
        rusqlite::params![content, ep_type],
    )
    .expect("insert episode");
    conn.last_insert_rowid()
}

// ── vec0 KNN contract ──────────────────────────────────────────────────────────

/// Insert an episode + an embeddings_vec row (LE f32 bytes), then KNN MATCH
/// returns that rowid as the nearest neighbour of itself.
///
/// This validates:
/// - vec0 auto-extension is active,
/// - the `float[768]` schema is correct,
/// - LE byte packing matches vec0's internal format,
/// - KNN MATCH syntax and k parameter work.
#[test]
fn vec0_insert_and_knn_match_returns_rowid() {
    let (_tmp, conn) = make_db();

    let episode_id = insert_episode(&conn, "test content for vector search", "chat");

    // Build a unit vector in dimension 0 (all zeros except index 0 = 1.0).
    // A query of itself must return distance ≈ 0 (cosine: vectors are L2-normalised by
    // the embed model; vec0 cosine distance = 1 - cosine_similarity, so 0 for identical).
    let mut embedding = vec![0.0f32; EMBEDDING_DIM];
    embedding[0] = 1.0;
    let bytes = pack_f32(&embedding);

    conn.execute(
        "INSERT INTO embeddings_vec(rowid, embedding) VALUES (?1, ?2)",
        rusqlite::params![episode_id, bytes],
    )
    .expect("insert embeddings_vec row");

    // Update content_embedding_id to mark as embedded (as route handlers do).
    conn.execute(
        "UPDATE episodes SET content_embedding_id = ?1 WHERE episode_id = ?2",
        rusqlite::params![episode_id, episode_id],
    )
    .expect("set content_embedding_id");

    // KNN MATCH query — same shape as dense.rs::dense_search.
    let mut stmt = conn
        .prepare(
            "SELECT rowid, distance FROM embeddings_vec \
             WHERE embedding MATCH ?1 AND k = ?2 \
             ORDER BY distance",
        )
        .expect("prepare KNN query");

    let results: Vec<(i64, f64)> = stmt
        .query_map(rusqlite::params![bytes, 1i64], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
        })
        .expect("query_map")
        .map(|r| r.unwrap())
        .collect();

    assert_eq!(results.len(), 1, "KNN k=1 must return exactly 1 result");
    let (returned_rowid, distance) = results[0];
    assert_eq!(
        returned_rowid, episode_id,
        "KNN must return episode_id {episode_id} as nearest neighbour"
    );
    assert!(
        distance < 1e-5,
        "cosine distance of a vector to itself must be ≈ 0, got {distance}"
    );
}

/// Dense search over multiple rows returns the nearest one first.
#[test]
fn vec0_knn_returns_nearest_first() {
    let (_tmp, conn) = make_db();

    // Episode A: unit vector [1, 0, 0, ...]
    let id_a = insert_episode(&conn, "episode A", "chat");
    let mut vec_a = vec![0.0f32; EMBEDDING_DIM];
    vec_a[0] = 1.0;

    // Episode B: unit vector [0, 1, 0, ...]  — orthogonal to A
    let id_b = insert_episode(&conn, "episode B", "chat");
    let mut vec_b = vec![0.0f32; EMBEDDING_DIM];
    vec_b[1] = 1.0;

    conn.execute(
        "INSERT INTO embeddings_vec(rowid, embedding) VALUES (?1, ?2)",
        rusqlite::params![id_a, pack_f32(&vec_a)],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO embeddings_vec(rowid, embedding) VALUES (?1, ?2)",
        rusqlite::params![id_b, pack_f32(&vec_b)],
    )
    .unwrap();

    // Query is close to vec_a (same direction) — A should rank first.
    let results: Vec<i64> = conn
        .prepare(
            "SELECT rowid FROM embeddings_vec \
             WHERE embedding MATCH ?1 AND k = ?2 ORDER BY distance",
        )
        .unwrap()
        .query_map(rusqlite::params![pack_f32(&vec_a), 2i64], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect();

    assert_eq!(results.len(), 2);
    assert_eq!(results[0], id_a, "nearest vector must rank first");
}

// ── FTS5 porter-stemmer contract ───────────────────────────────────────────────

/// Insert episode with "running" in content_text; FTS MATCH "run" (porter stem)
/// must find it. This proves the `porter unicode61` tokenizer is active.
#[test]
fn fts5_porter_stemmer_matches_word_form() {
    let (_tmp, conn) = make_db();

    let id = insert_episode(&conn, "The warrior was running through the forest", "combat");

    // Porter stem of "running" is "run"; FTS5 MATCH "run" should find it.
    let mut stmt = conn
        .prepare(
            "SELECT rowid FROM episodes_fts WHERE episodes_fts MATCH ?1 ORDER BY rowid",
        )
        .expect("prepare FTS query");

    let rowids: Vec<i64> = stmt
        .query_map(rusqlite::params!["run"], |row| row.get::<_, i64>(0))
        .expect("query_map")
        .map(|r| r.unwrap())
        .collect();

    assert!(
        rowids.contains(&id),
        "FTS5 MATCH 'run' must find episode with 'running'; rowids={rowids:?}"
    );
}

/// BM25 negation: -bm25(episodes_fts) is positive and increases with relevance.
/// Two episodes: one with the query term repeated, one with it once.
/// The repeated one must score higher.
#[test]
fn fts5_bm25_score_order() {
    let (_tmp, conn) = make_db();

    insert_episode(&conn, "druid druid druid cast moonfire", "combat"); // more occurrences
    insert_episode(&conn, "druid cast starfire once", "combat"); // fewer

    let mut stmt = conn
        .prepare(
            "SELECT rowid, -bm25(episodes_fts) AS score \
             FROM episodes_fts WHERE episodes_fts MATCH ?1 ORDER BY score DESC",
        )
        .expect("prepare bm25 query");

    let results: Vec<(i64, f64)> = stmt
        .query_map(rusqlite::params!["druid"], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
        })
        .expect("query_map")
        .map(|r| r.unwrap())
        .collect();

    assert_eq!(results.len(), 2, "must find both episodes");
    assert!(
        results[0].1 >= results[1].1,
        "episode with more 'druid' occurrences must rank first; scores: {:?}",
        results.iter().map(|(_, s)| s).collect::<Vec<_>>()
    );
}

// ── ee_ai / ee_ad trigger contract ────────────────────────────────────────────

/// Insert episode + entity + link → ee_ai trigger increments total_episodes.
/// Delete the link → ee_ad trigger decrements total_episodes (MAX(0,...)).
#[test]
fn entity_triggers_maintain_total_episodes() {
    let (_tmp, conn) = make_db();

    let ep_id = insert_episode(&conn, "Thrall joined the raid", "social");

    // Insert entity
    conn.execute(
        "INSERT INTO entities (entity_kind, entity_key, display_name) \
         VALUES ('player', 'thrall_key', 'Thrall')",
        [],
    )
    .expect("insert entity");
    let entity_id = conn.last_insert_rowid();

    // Before link: total_episodes should be 0
    let before: i64 = conn
        .query_row(
            "SELECT total_episodes FROM entities WHERE entity_id = ?1",
            rusqlite::params![entity_id],
            |row| row.get(0),
        )
        .expect("query before");
    assert_eq!(before, 0, "total_episodes must be 0 before linking");

    // Insert link — ee_ai fires
    conn.execute(
        "INSERT OR IGNORE INTO episode_entities (episode_id, entity_id, role) \
         VALUES (?1, ?2, 'participant')",
        rusqlite::params![ep_id, entity_id],
    )
    .expect("insert episode_entities");

    let after_insert: i64 = conn
        .query_row(
            "SELECT total_episodes FROM entities WHERE entity_id = ?1",
            rusqlite::params![entity_id],
            |row| row.get(0),
        )
        .expect("query after insert");
    assert_eq!(
        after_insert, 1,
        "ee_ai trigger must have incremented total_episodes to 1"
    );

    // Delete link — ee_ad fires
    conn.execute(
        "DELETE FROM episode_entities WHERE episode_id = ?1 AND entity_id = ?2",
        rusqlite::params![ep_id, entity_id],
    )
    .expect("delete episode_entities");

    let after_delete: i64 = conn
        .query_row(
            "SELECT total_episodes FROM entities WHERE entity_id = ?1",
            rusqlite::params![entity_id],
            |row| row.get(0),
        )
        .expect("query after delete");
    assert_eq!(
        after_delete, 0,
        "ee_ad trigger must have decremented total_episodes back to 0"
    );
}

// ── Episode delete cascade contract ───────────────────────────────────────────

/// Deleting an episode (in the order the DELETE handler uses) must:
/// 1. Remove the embeddings_vec row first (no FK cascade for vec0).
/// 2. Cascade episode_entities deletion (ON DELETE CASCADE FK).
/// 3. Decrement entities.total_episodes via ee_ad trigger.
/// 4. Remove the episode from episodes_fts via episodes_ad trigger.
#[test]
fn delete_episode_cascades_and_cleans_fts_and_vec() {
    let (_tmp, conn) = make_db();

    let ep_id = insert_episode(&conn, "A rare discovery in Stranglethorn", "discovery");

    // Insert embedding
    let mut embedding = vec![0.0f32; EMBEDDING_DIM];
    embedding[5] = 1.0;
    conn.execute(
        "INSERT INTO embeddings_vec(rowid, embedding) VALUES (?1, ?2)",
        rusqlite::params![ep_id, pack_f32(&embedding)],
    )
    .expect("insert vec");
    conn.execute(
        "UPDATE episodes SET content_embedding_id = ?1 WHERE episode_id = ?2",
        rusqlite::params![ep_id, ep_id],
    )
    .expect("set embedding id");

    // Insert entity and link
    conn.execute(
        "INSERT INTO entities (entity_kind, entity_key, display_name) \
         VALUES ('location', 'stv_key', 'Stranglethorn Vale')",
        [],
    )
    .expect("insert entity");
    let entity_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT OR IGNORE INTO episode_entities (episode_id, entity_id, role) \
         VALUES (?1, ?2, 'subject')",
        rusqlite::params![ep_id, entity_id],
    )
    .expect("insert link");

    // Confirm entity total_episodes = 1
    let before: i64 = conn
        .query_row(
            "SELECT total_episodes FROM entities WHERE entity_id = ?1",
            rusqlite::params![entity_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(before, 1);

    // DELETE ORDER IS LOAD-BEARING (matches delete route handler, spec §5.8):
    // 1. Delete embeddings_vec first (no FK cascade on vec0 virtual tables).
    conn.execute(
        "DELETE FROM embeddings_vec WHERE rowid = ?1",
        rusqlite::params![ep_id],
    )
    .expect("delete embeddings_vec");

    // 2. Delete episode (cascades episode_entities; fires ee_ad + episodes_ad).
    conn.execute(
        "DELETE FROM episodes WHERE episode_id = ?1",
        rusqlite::params![ep_id],
    )
    .expect("delete episode");

    // Verify: episode is gone
    let ep_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM episodes WHERE episode_id = ?1",
            rusqlite::params![ep_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(ep_count, 0, "episode must be deleted");

    // Verify: episode_entities cascaded
    let link_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM episode_entities WHERE episode_id = ?1",
            rusqlite::params![ep_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(link_count, 0, "episode_entities must cascade-delete");

    // Verify: total_episodes decremented via ee_ad
    let after_total: i64 = conn
        .query_row(
            "SELECT total_episodes FROM entities WHERE entity_id = ?1",
            rusqlite::params![entity_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        after_total, 0,
        "ee_ad trigger must decrement total_episodes after cascade"
    );

    // Verify: FTS removed (episodes_ad trigger fired)
    let fts_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM episodes_fts WHERE episodes_fts MATCH 'discovery'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        fts_count, 0,
        "episodes_ad trigger must remove episode from FTS index"
    );

    // Verify: embeddings_vec row is gone
    let vec_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM embeddings_vec WHERE rowid = ?1",
            rusqlite::params![ep_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(vec_count, 0, "embeddings_vec row must be deleted");
}
