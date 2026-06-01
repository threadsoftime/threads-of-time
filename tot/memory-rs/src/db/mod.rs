//! Database helpers: per-bot connection management and vec0 auto-extension.
//!
//! # Extension registration
//!
//! `register_vec0()` MUST be called exactly once at process startup, before any
//! [`rusqlite::Connection`] is opened. It registers `sqlite3_vec_init` as a
//! `register_vec0()` is idempotent — it uses a `std::sync::Once` guard so
//! it is safe to call from every `open_bot_db` invocation (production and
//! tests). The underlying `sqlite3_auto_extension` call only fires on the
//! first call per process.
//!
//! # Safety contract
//!
//! `sqlite3_auto_extension` takes the typed fn-pointer directly. The
//! `extern "C"` declaration matches the sqlite-vec 0.1.6 ABI exactly; no
//! transmute is required or used.

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

/// Open (or create) the per-bot SQLite database.
///
/// Creates `data_dir/<bot_guid>/` if it does not exist, then opens
/// `data_dir/<bot_guid>/memory.sqlite` in WAL mode with foreign keys on.
/// Calls `register_vec0()` (idempotent) at the top to guarantee the `vec0`
/// module is available on the returned connection — whether called from main,
/// a route handler, or a test that opens a DB directly.
pub fn open_bot_db(data_dir: &Path, bot_guid: &str) -> rusqlite::Result<rusqlite::Connection> {
    register_vec0();
    let bot_dir = data_dir.join(bot_guid);
    std::fs::create_dir_all(&bot_dir).map_err(|e| {
        rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ffi::ErrorCode::CannotOpen,
                extended_code: 0,
            },
            Some(format!("create_dir_all {}: {e}", bot_dir.display())),
        )
    })?;

    let db_path = bot_dir.join("memory.sqlite");
    let conn = rusqlite::Connection::open(&db_path)?;

    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;

    Ok(conn)
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
        let bytes: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();

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

    /// open_bot_db creates the directory tree and the database file.
    #[test]
    fn creates_directory_and_file() {
        register_vec0();
        let tmp = tempfile::tempdir().expect("tempdir");
        let data_dir = tmp.path();
        let bot_guid = "bot_12345";

        let _conn = open_bot_db(data_dir, bot_guid).expect("open_bot_db");

        let db_path = data_dir.join(bot_guid).join("memory.sqlite");
        assert!(
            db_path.exists(),
            "memory.sqlite must exist at {db_path:?}"
        );
    }

    /// open_bot_db sets journal_mode=WAL on the connection.
    #[test]
    fn pragma_journal_mode_wal() {
        register_vec0();
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open_bot_db(tmp.path(), "bot_wal_test").expect("open_bot_db");

        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .expect("PRAGMA journal_mode");
        assert_eq!(
            mode, "wal",
            "journal_mode must be 'wal', got '{mode}'"
        );
    }

    /// open_bot_db enables foreign-key enforcement.
    #[test]
    fn pragma_foreign_keys_on() {
        register_vec0();
        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open_bot_db(tmp.path(), "bot_fk_test").expect("open_bot_db");

        let fk: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .expect("PRAGMA foreign_keys");
        assert_eq!(fk, 1, "foreign_keys must be ON (1), got {fk}");
    }

    /// Calling open_bot_db twice on the same bot_guid is idempotent
    /// (the directory and file already exist — no error).
    #[test]
    fn idempotent_second_open() {
        register_vec0();
        let tmp = tempfile::tempdir().expect("tempdir");
        let data_dir = tmp.path();
        open_bot_db(data_dir, "bot_idem").expect("first open");
        open_bot_db(data_dir, "bot_idem").expect("second open must not error");
    }

    /// open_bot_db creates the directory and opens a WAL-mode connection.
    #[test]
    fn open_bot_db_creates_dir_and_sets_wal() {
        register_vec0();

        let tmp = tempfile::tempdir().expect("tempdir");
        let conn = open_bot_db(tmp.path(), "bot_1234")
            .expect("open_bot_db must succeed");

        // WAL mode: PRAGMA journal_mode returns "wal".
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .expect("pragma");
        assert_eq!(mode, "wal", "journal_mode must be WAL");

        // Foreign keys: PRAGMA foreign_keys returns 1.
        let fk: i32 = conn
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .expect("pragma");
        assert_eq!(fk, 1, "foreign_keys must be ON");

        // DB file must exist at the expected path.
        assert!(tmp.path().join("bot_1234").join("memory.sqlite").exists());
    }
}
