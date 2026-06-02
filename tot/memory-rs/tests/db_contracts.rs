//! Integration-level contracts for the database layer.
//!
//! These tests operate on real SQLite connections (in-memory or tempfile) and
//! verify the observable behaviour of `db::migrate::run` without mocking any
//! internal seam — matching the testing discipline from Palmieri Ch 3.

use memory_rs::db;
use std::path::Path;

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// Open an in-memory SQLite connection with vec0 registered.
///
/// `register_vec0()` is idempotent (Once guard), so calling this from every
/// test is safe.  The first migration (0001) creates a `vec0` virtual table;
/// the extension MUST be registered before `migrate::run` is called.
fn open_mem_db() -> rusqlite::Connection {
    db::register_vec0();
    rusqlite::Connection::open_in_memory().expect("in-memory db")
}

// ---------------------------------------------------------------------------
// Task 1.2 — schema_version migration runner contract
// ---------------------------------------------------------------------------

/// The migrations directory is embedded at compile time so the tests are
/// hermetic and do not depend on the working-directory at runtime.
fn migrations_dir() -> &'static Path {
    // CARGO_MANIFEST_DIR is set by Cargo for integration tests; the
    // `migrations/` folder lives next to `Cargo.toml`.
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/migrations"))
}

/// After `run`, every migration file (versions 1..=5) must have exactly one
/// row in `schema_version` with the correct version number and a non-empty
/// description.
#[test]
fn migrate_run_populates_schema_version() {
    let conn = open_mem_db();
    db::migrate::run(&conn, migrations_dir()).expect("migrate::run must succeed");

    // Collect all (version, description) rows.
    let mut stmt = conn
        .prepare("SELECT version, description FROM schema_version ORDER BY version")
        .expect("prepare schema_version query");
    let rows: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("query_map")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect rows");

    assert_eq!(
        rows.len(),
        5,
        "expected 5 schema_version rows (one per migration), got {}: {rows:?}",
        rows.len()
    );

    // Versions must be 1, 2, 3, 4, 5 in order.
    let versions: Vec<i64> = rows.iter().map(|(v, _)| *v).collect();
    assert_eq!(
        versions,
        vec![1, 2, 3, 4, 5],
        "schema_version rows must be versions 1..=5 in order, got {versions:?}"
    );

    // Every row must have a non-empty description.
    for (version, desc) in &rows {
        assert!(
            !desc.is_empty(),
            "schema_version row for version {version} has empty description"
        );
    }
}

/// Descriptions parsed from filenames:
///   0001_v01_baseline.sql       → "v01_baseline"
///   0002_add_memory_columns.sql → "add_memory_columns"
///   0003_add_entity_type.sql    → "add_entity_type"
///   0004_create_goals.sql       → "create_goals"
///   0005_create_fts5.sql        → "create_fts5"
#[test]
fn migrate_run_records_correct_descriptions() {
    let conn = open_mem_db();
    db::migrate::run(&conn, migrations_dir()).expect("migrate::run");

    let mut stmt = conn
        .prepare("SELECT version, description FROM schema_version ORDER BY version")
        .expect("prepare");
    let rows: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("query_map")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect");

    let expected = vec![
        (1i64, "v01_baseline"),
        (2i64, "add_memory_columns"),
        (3i64, "add_entity_type"),
        (4i64, "create_goals"),
        (5i64, "create_fts5"),
    ];
    for ((version, desc), (exp_v, exp_d)) in rows.iter().zip(expected.iter()) {
        assert_eq!(version, exp_v, "version mismatch");
        assert_eq!(desc.as_str(), *exp_d, "description mismatch for version {version}");
    }
}

/// `applied_ts` must be a plausible Unix-seconds timestamp (> 0, not some
/// arbitrary constant).
#[test]
fn migrate_run_records_applied_ts() {
    let conn = open_mem_db();
    db::migrate::run(&conn, migrations_dir()).expect("migrate::run");

    let mut stmt = conn
        .prepare("SELECT version, applied_ts FROM schema_version ORDER BY version")
        .expect("prepare");
    let rows: Vec<(i64, i64)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("query_map")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect");

    for (version, ts) in &rows {
        assert!(
            *ts > 0,
            "applied_ts for version {version} must be > 0, got {ts}"
        );
    }
}

/// Calling `run` a second time on the same connection must be idempotent:
/// no error, no new rows in `schema_version`, no duplicate rows.
#[test]
fn migrate_run_is_idempotent() {
    let conn = open_mem_db();
    db::migrate::run(&conn, migrations_dir()).expect("first run");
    db::migrate::run(&conn, migrations_dir()).expect("second run must not error");

    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM schema_version",
            [],
            |row| row.get(0),
        )
        .expect("count schema_version");

    assert_eq!(
        count, 5,
        "after two runs, schema_version must still have exactly 5 rows, got {count}"
    );
}

/// After migration, core tables required by the v0.2.1 contract must exist.
#[test]
fn migrate_run_creates_expected_tables() {
    let conn = open_mem_db();
    db::migrate::run(&conn, migrations_dir()).expect("migrate::run");

    let expected_tables = [
        "bots",
        "entities",
        "edges",
        "memories",
        "memory_entities",
        "goals",
        "memories_fts",
        "vec_memories",
    ];
    for table in &expected_tables {
        let exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type IN ('table','shadow') AND name = ?1",
                rusqlite::params![table],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n > 0)
            .unwrap_or_else(|_| {
                // sqlite_master may not surface virtual-table shadows; try a
                // simpler existence check.
                conn.execute_batch(&format!("SELECT 1 FROM \"{table}\" LIMIT 0"))
                    .is_ok()
            });
        assert!(
            exists,
            "table '{table}' must exist after running all migrations"
        );
    }
}

/// Partial migration: if only migrations up to version 2 have been applied,
/// `run` must pick up versions 3, 4, 5 without re-applying 1 or 2.
#[test]
fn migrate_run_continues_from_current_version() {
    let conn = open_mem_db();

    // Manually apply migrations 1 and 2 only (without schema_version).
    // We seed schema_version directly to simulate a partially-migrated DB.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version     INTEGER PRIMARY KEY,
            applied_ts  INTEGER NOT NULL,
            description TEXT NOT NULL
        )",
    )
    .expect("create schema_version");

    // Apply migration 1 manually so the tables exist (needed for migration 2
    // which does ALTER TABLE on `memories`).
    let sql1 = std::fs::read_to_string(migrations_dir().join("0001_v01_baseline.sql"))
        .expect("read 0001");
    conn.execute_batch(&sql1).expect("apply 0001 manually");

    let sql2 = std::fs::read_to_string(migrations_dir().join("0002_add_memory_columns.sql"))
        .expect("read 0002");
    conn.execute_batch(&sql2).expect("apply 0002 manually");

    conn.execute(
        "INSERT INTO schema_version (version, applied_ts, description) VALUES (1, 1000000, 'v01_baseline'), (2, 1000001, 'add_memory_columns')",
        [],
    )
    .expect("seed schema_version");

    // Now run the full migration set — must apply only 3, 4, 5.
    db::migrate::run(&conn, migrations_dir()).expect("migrate::run from v2");

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM schema_version", [], |row| row.get(0))
        .expect("count");
    assert_eq!(
        count, 5,
        "after partial seed + run, must have exactly 5 rows, got {count}"
    );

    let max_version: i64 = conn
        .query_row(
            "SELECT MAX(version) FROM schema_version",
            [],
            |row| row.get(0),
        )
        .expect("max version");
    assert_eq!(max_version, 5, "max applied version must be 5");
}
