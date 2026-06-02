//! Database helpers: monolithic connection management and vec0 auto-extension.
//!
//! # Extension registration
//!
//! `register_vec0()` MUST be called exactly once at process startup, before any
//! [`rusqlite::Connection`] is opened. It registers `sqlite3_vec_init` as a
//! SQLite auto-extension.
//! `register_vec0()` is idempotent — it uses a `std::sync::Once` guard so
//! it is safe to call from every `open_db` invocation (production and
//! tests). The underlying `sqlite3_auto_extension` call only fires on the
//! first call per process.
//!
//! # Safety contract
//!
//! `sqlite3_auto_extension` takes the typed fn-pointer directly. The
//! `extern "C"` declaration matches the sqlite-vec 0.1.6 ABI exactly; no
//! transmute is required or used.

pub mod entities;
pub mod migrate;

use std::ffi::c_int;
use std::path::Path;
use std::sync::Once;

// Re-export rusqlite ffi types for use in the extern declaration.
use rusqlite::ffi::{
    sqlite3, sqlite3_api_routines, sqlite3_auto_extension,
};

extern "C" {
    /// Entry point of the statically-linked sqlite-vec 0.1.6 amalgamation.
    /// Compiled by `build.rs` via the `cc` crate from `vendor/sqlite-vec/sqlite-vec.c`.
    /// Signature matches the sqlite-vec 0.1.6 header declaration exactly:
    ///   int sqlite3_vec_init(sqlite3*, char**, const sqlite3_api_routines*);
    fn sqlite3_vec_init(
        db: *mut sqlite3,
        pz_err_msg: *mut *mut std::os::raw::c_char,
        p_api: *const sqlite3_api_routines,
    ) -> c_int;
}

static REGISTER_ONCE: Once = Once::new();

/// Register the `vec0` virtual-table module as a SQLite auto-extension.
///
/// Idempotent: safe to call from every `open_bot_db` invocation (production
/// and tests). The actual `sqlite3_auto_extension` call fires only once per
/// process, guarded by a `std::sync::Once`.
///
/// # Safety
///
/// Passes `sqlite3_vec_init` typed fn-pointer directly to
/// `sqlite3_auto_extension`; no transmute. The `extern "C"` declaration
/// matches the sqlite-vec 0.1.6 ABI exactly.
pub fn register_vec0() {
    REGISTER_ONCE.call_once(|| {
        unsafe {
            sqlite3_auto_extension(Some(sqlite3_vec_init));
        }
    });
}

/// Open (or create) the monolithic SQLite database at `path`.
///
/// Calls `register_vec0()` (idempotent) to guarantee the `vec0` virtual-table
/// module is available on the returned connection.  Sets the pragmas that
/// match the live Python `memory_sidecar/db.py`:
///   - `journal_mode = WAL`    — concurrent readers, single writer, no fsync on every write
///   - `synchronous  = NORMAL` — flush on checkpoint only (matches Python default)
///   - `foreign_keys = ON`     — enforce FK constraints (Rust adds this; Python omits it)
///
/// The caller is responsible for running [`migrate::run`] after opening if the
/// DB is new or may be at an older schema version.
pub fn open_db(path: &Path) -> rusqlite::Result<rusqlite::Connection> {
    register_vec0();
    let conn = rusqlite::Connection::open(path)?;
    // Use PRAGMA … = … form for journal_mode so WAL handshake is performed.
    // Use execute_batch for the remaining two; order matters: WAL first.
    conn.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;",
    )?;
    // Unconditional idempotent index guard — mirrors Python `run_legacy_migrations`
    // which always executes `CREATE INDEX IF NOT EXISTS idx_memories_bot`.
    //
    // The versioned Rust migration runner (migrate::run) skips migration 0001 on
    // existing snapshots where schema_version=1 is already recorded, so the index
    // may not be present on DBs created before this index was added. Without the
    // index, SQLite uses the TEXT PRIMARY KEY B-tree for `WHERE bot_id=? AND id IN
    // (...)`, returning TEXT-sorted order; with it, SQLite switches to an index
    // scan on bot_id returning rowid order — exactly what Python sees after it runs
    // its legacy migration on startup. Both orders must agree for MMR tie-break parity.
    //
    // Guard: only run if the `memories` table already exists (fresh in-memory and
    // not-yet-migrated connections call open_db before migrations are applied).
    let memories_exists: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='memories'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0) > 0;
    if memories_exists {
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_memories_bot ON memories(bot_id);",
        )?;
    }
    Ok(conn)
}

/// Encode a slice of `f32` values as a little-endian byte blob.
///
/// Parity with Python `helpers.py`:
/// ```python
/// def embedding_to_blob(vec: np.ndarray) -> bytes:
///     return vec.astype(np.float32).tobytes()
/// ```
/// `numpy.ndarray.tobytes()` uses the array's memory layout; float32 arrays on
/// x86/ARM are always little-endian.  `f32::to_le_bytes()` produces the same
/// 4-byte representation.
///
/// This is the canonical storage encoding for `vec_memories.embedding`.
pub fn pack_f32_le(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for &x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

/// Decode a little-endian byte blob back into a `Vec<f32>`.
///
/// Inverse of [`pack_f32_le`].  `b.len()` must be a multiple of 4; trailing
/// bytes (if `b.len() % 4 != 0`) are silently ignored (same as `numpy.frombuffer`
/// behaviour when the buffer is already aligned).
pub fn unpack_f32_le(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|chunk| {
            let arr: [u8; 4] = chunk.try_into().expect("chunk_exact guarantees 4 bytes");
            f32::from_le_bytes(arr)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Prove that vec0 is linked and auto-registered: CREATE VIRTUAL TABLE must
    /// succeed on an in-memory connection opened after register_vec0().
    #[test]
    fn vec0_create_virtual_table_smoke() {
        // Idempotent: safe to call in any test that needs vec0.
        register_vec0();

        let conn = rusqlite::Connection::open_in_memory().expect("in-memory db");

        // If vec0 is not registered this will return an error like
        // "no such module: vec0".
        conn.execute_batch(
            "CREATE VIRTUAL TABLE t USING vec0(embedding float[768]);",
        )
        .expect("CREATE VIRTUAL TABLE ... USING vec0 must succeed after register_vec0()");
    }

    /// Prove that we can insert a vector and run a KNN query against it.
    #[test]
    fn vec0_insert_and_knn_query() {
        register_vec0();

        let conn = rusqlite::Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch(
            "CREATE VIRTUAL TABLE vecs USING vec0(embedding float[768]);",
        )
        .expect("create table");

        // Pack 768 f32 values as little-endian bytes.
        let v: Vec<f32> = (0..768).map(|i| i as f32 / 768.0).collect();
        let bytes = pack_f32_le(&v);

        conn.execute(
            "INSERT INTO vecs(rowid, embedding) VALUES (1, ?1)",
            rusqlite::params![bytes],
        )
        .expect("insert vector");

        // KNN query: must return the one row we inserted with distance ≥ 0.
        let distance: f64 = conn
            .query_row(
                "SELECT distance FROM vecs WHERE embedding MATCH ?1 AND k = 1 ORDER BY distance",
                rusqlite::params![bytes],
                |row| row.get(0),
            )
            .expect("KNN query");

        // cosine distance from a vector to itself is 0.0 (within float precision).
        assert!(
            distance.abs() < 1e-5,
            "cosine distance of a vector to itself must be ~0.0, got {distance}"
        );
    }

    /// open_db creates the database file at the given path.
    #[test]
    fn creates_db_file() {
        let tmp = tempfile::NamedTempFile::new().expect("tempfile");
        let _conn = open_db(tmp.path()).expect("open_db");
        assert!(tmp.path().exists(), "DB file must exist at given path");
    }

    /// open_db sets journal_mode=WAL on the connection.
    #[test]
    fn pragma_journal_mode_wal() {
        let tmp = tempfile::NamedTempFile::new().expect("tempfile");
        let conn = open_db(tmp.path()).expect("open_db");

        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .expect("PRAGMA journal_mode");
        assert_eq!(mode, "wal", "journal_mode must be 'wal', got '{mode}'");
    }

    /// open_db enables foreign-key enforcement.
    #[test]
    fn pragma_foreign_keys_on() {
        let tmp = tempfile::NamedTempFile::new().expect("tempfile");
        let conn = open_db(tmp.path()).expect("open_db");

        let fk: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .expect("PRAGMA foreign_keys");
        assert_eq!(fk, 1, "foreign_keys must be ON (1), got {fk}");
    }

    /// Calling open_db twice on the same path is idempotent (file already
    /// exists — no error).
    #[test]
    fn idempotent_second_open() {
        let tmp = tempfile::NamedTempFile::new().expect("tempfile");
        open_db(tmp.path()).expect("first open");
        open_db(tmp.path()).expect("second open must not error");
    }

    /// pack_f32_le / unpack_f32_le round-trip: exact f32 bit-pattern identity.
    #[test]
    fn pack_unpack_round_trip() {
        let v: Vec<f32> = (0..384).map(|i| i as f32 * 0.001).collect();
        let blob = pack_f32_le(&v);
        assert_eq!(blob.len(), 384 * 4);
        let decoded = unpack_f32_le(&blob);
        assert_eq!(decoded.len(), v.len());
        for (i, (&orig, &dec)) in v.iter().zip(decoded.iter()).enumerate() {
            assert_eq!(
                orig.to_bits(), dec.to_bits(),
                "bit mismatch at index {i}"
            );
        }
    }

    /// pack_f32_le(1.0) == [0x00, 0x00, 0x80, 0x3F] — numpy parity.
    #[test]
    fn pack_f32_le_one_point_zero_numpy_parity() {
        let blob = pack_f32_le(&[1.0_f32]);
        assert_eq!(blob, &[0x00u8, 0x00, 0x80, 0x3F]);
    }
}
