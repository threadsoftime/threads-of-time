//! Schema-version migration runner — Rust port of `memory_sidecar/migrations.py`.
//!
//! # Semantics
//!
//! A migration is a single `.sql` file named `NNNN_description.sql` where
//! `NNNN` is a zero-padded integer.  Migrations are applied in ascending
//! version order.  Each migration file is executed via [`rusqlite::Connection::execute_batch`]
//! (equivalent to Python's `conn.executescript`), then a row is inserted into
//! `schema_version` and the transaction is committed.  If the INSERT fails the
//! insert-only transaction is rolled back; the DDL in the migration file has
//! already been implicitly committed by SQLite (DDL is auto-commit in SQLite,
//! matching Python `executescript` semantics).
//!
//! # Idempotency
//!
//! Calling [`run`] on a fully-migrated database is a no-op: `MAX(version)` is
//! already equal to the highest migration file number, so `list_pending` returns
//! an empty set.

use rusqlite::Connection;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const SCHEMA_VERSION_DDL: &str = "
CREATE TABLE IF NOT EXISTS schema_version (
    version     INTEGER PRIMARY KEY,
    applied_ts  INTEGER NOT NULL,
    description TEXT NOT NULL
)";

/// Ensure the `schema_version` tracking table exists.
fn ensure_schema_version_table(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(SCHEMA_VERSION_DDL)
}

/// Return the highest applied schema version (0 if no migrations have been
/// applied yet), mirroring `current_version()` in `migrations.py`.
fn current_version(conn: &Connection) -> rusqlite::Result<i64> {
    ensure_schema_version_table(conn)?;
    let v: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |row| row.get(0),
        )?;
    Ok(v)
}

/// Parse `NNNN_description.sql` → `(version, description)`.
/// Returns `None` if the filename does not match the expected pattern.
///
/// Mirrors `_parse_migration_filename` in `migrations.py`.
fn parse_migration_filename(name: &str) -> Option<(i64, String)> {
    let name = name.strip_suffix(".sql")?;
    let (prefix, description) = name.split_once('_')?;
    let version: i64 = prefix.parse().ok()?;
    Some((version, description.to_owned()))
}

/// Return `(version, description, path)` tuples for all migrations with
/// `version > current`, sorted ascending by version.
///
/// Mirrors `list_pending_migrations` in `migrations.py`.
fn list_pending(
    dir: &Path,
    current: i64,
) -> std::io::Result<Vec<(i64, String, std::path::PathBuf)>> {
    if !dir.is_dir() {
        return Ok(vec![]);
    }

    let mut entries: Vec<(i64, String, std::path::PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        if let Some((version, description)) = parse_migration_filename(&name) {
            if version > current {
                entries.push((version, description, entry.path()));
            }
        }
    }
    entries.sort_by_key(|(v, _, _)| *v);
    Ok(entries)
}

/// Unix-seconds timestamp — used for `applied_ts` column.
fn now_unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Apply all pending migrations to `conn` from the `.sql` files in `dir`.
///
/// Returns the number of migrations applied (0 if already up-to-date).
///
/// # Errors
///
/// Returns a [`MigrateError`] if a migration file cannot be read or its SQL
/// cannot be executed.  In that case the `schema_version` insert is rolled
/// back; already-executed DDL in that migration file is NOT rolled back
/// (SQLite DDL is implicitly committed — matches Python `executescript`
/// semantics).
pub fn run(conn: &Connection, dir: &Path) -> Result<usize, MigrateError> {
    let current = current_version(conn)?;
    let pending = list_pending(dir, current)?;
    let mut applied = 0usize;

    for (version, description, path) in pending {
        let sql = std::fs::read_to_string(&path).map_err(|e| {
            MigrateError::IoMsg(format!(
                "could not read migration {}: {e}",
                path.display()
            ))
        })?;

        // Execute the migration SQL — equivalent to Python's `conn.executescript(sql)`.
        // DDL in SQLite auto-commits; `execute_batch` runs all statements in the string.
        conn.execute_batch(&sql)?;

        // Record the application in schema_version.  Use a transaction so the
        // INSERT can be rolled back if it fails (e.g. duplicate version key
        // detected at the DB level).
        conn.execute(
            "INSERT INTO schema_version (version, applied_ts, description) VALUES (?1, ?2, ?3)",
            rusqlite::params![version, now_unix_secs(), description],
        )?;

        applied += 1;
    }

    Ok(applied)
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors from [`run`].
#[derive(Debug, thiserror::Error)]
pub enum MigrateError {
    #[error("SQLite error: {0}")]
    Rusqlite(#[from] rusqlite::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Wraps a string-formatted I/O error from path-level operations (e.g.
    /// `read_to_string` where we already have a formatted message).
    #[error("{0}")]
    IoMsg(String),
}

// Infallible conversions for the string-based I/O variant used in `run`.
impl From<String> for MigrateError {
    fn from(s: String) -> Self {
        MigrateError::IoMsg(s)
    }
}
