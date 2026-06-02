//! Idempotent migration runner.
//!
//! Embeds the 5 SQL files at compile time via `include_str!`. Tracks applied
//! versions in `_meta_migrations`. Mirrors the Python `runner.py` exactly.

use rusqlite::Connection;

/// All migrations in ascending version order.
///
/// Each entry is `(version, filename, sql)`. The version is the integer prefix
/// of the filename (000 → 0, 001 → 1, …). Idempotency: versions already in
/// `_meta_migrations` are skipped.
const MIGRATIONS: &[(i64, &str, &str)] = &[
    (
        0,
        "000_meta.sql",
        include_str!("../../migrations/000_meta.sql"),
    ),
    (
        1,
        "001_episodes.sql",
        include_str!("../../migrations/001_episodes.sql"),
    ),
    (
        2,
        "002_entities.sql",
        include_str!("../../migrations/002_entities.sql"),
    ),
    (
        3,
        "003_embeddings_vec.sql",
        include_str!("../../migrations/003_embeddings_vec.sql"),
    ),
    (
        4,
        "004_episodes_fts.sql",
        include_str!("../../migrations/004_episodes_fts.sql"),
    ),
];

/// Apply all unapplied migrations in ascending version order.
///
/// Idempotent: a version already recorded in `_meta_migrations` is skipped.
/// Mirrors `runner.py::run_migrations` exactly.
pub fn run_migrations(conn: &Connection) -> rusqlite::Result<()> {
    // Create the bookkeeping table if it doesn't exist.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _meta_migrations (
            version    INTEGER PRIMARY KEY,
            applied_at TEXT    NOT NULL DEFAULT CURRENT_TIMESTAMP,
            filename   TEXT    NOT NULL
        );",
    )?;

    // Collect already-applied versions.
    let applied: std::collections::HashSet<i64> = {
        let mut stmt = conn.prepare("SELECT version FROM _meta_migrations")?;
        let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };

    for &(version, filename, sql) in MIGRATIONS {
        if applied.contains(&version) {
            continue;
        }
        // executescript commits any open transaction before running; each
        // .sql file is self-contained DDL so this matches Python's
        // conn.executescript(sql) + conn.commit() behaviour.
        conn.execute_batch(sql)?;
        conn.execute(
            "INSERT INTO _meta_migrations (version, filename) VALUES (?1, ?2)",
            rusqlite::params![version, filename],
        )?;
    }

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    /// Register the vec0 auto-extension once for all tests in this process.
    ///
    /// Delegates to `crate::db::register_vec0()` which is guarded by a
    /// `std::sync::Once` and uses the typed fn-pointer directly (no transmute).
    fn ensure_vec0() {
        crate::db::register_vec0();
    }

    /// Open a temporary on-disk connection (vec0 requires an actual file).
    fn temp_conn() -> (tempfile::NamedTempFile, Connection) {
        ensure_vec0();
        let f = tempfile::NamedTempFile::new().expect("tempfile");
        let conn = Connection::open(f.path()).expect("open");
        (f, conn)
    }

    fn table_exists(conn: &Connection, name: &str) -> bool {
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type IN ('table','view') AND name = ?1",
            rusqlite::params![name],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
            > 0
    }

    fn virtual_table_exists(conn: &Connection, name: &str) -> bool {
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            rusqlite::params![name],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
            > 0
    }

    fn applied_versions(conn: &Connection) -> Vec<i64> {
        let mut stmt = conn
            .prepare("SELECT version FROM _meta_migrations ORDER BY version")
            .unwrap();
        stmt.query_map([], |row| row.get::<_, i64>(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    }

    /// After run_migrations on a fresh DB, all expected tables exist and
    /// _meta_migrations records versions 0..=4.
    #[test]
    fn fresh_db_has_all_tables_and_versions() {
        let (_f, conn) = temp_conn();
        run_migrations(&conn).expect("run_migrations should succeed on fresh DB");

        // Core relational tables
        assert!(table_exists(&conn, "episodes"), "episodes table missing");
        assert!(table_exists(&conn, "entities"), "entities table missing");
        assert!(table_exists(&conn, "episode_entities"), "episode_entities table missing");
        assert!(table_exists(&conn, "_meta_migrations"), "_meta_migrations table missing");

        // Virtual tables (vec0 + FTS5)
        assert!(
            virtual_table_exists(&conn, "embeddings_vec"),
            "embeddings_vec virtual table missing"
        );
        assert!(
            virtual_table_exists(&conn, "episodes_fts"),
            "episodes_fts virtual table missing"
        );

        // All 5 migration versions recorded
        let versions = applied_versions(&conn);
        assert_eq!(versions, vec![0, 1, 2, 3, 4], "expected versions 0..=4, got {versions:?}");
    }

    /// Running migrations twice on the same connection must be a no-op (no error,
    /// no duplicate rows in _meta_migrations).
    #[test]
    fn idempotent_run_twice_no_error_no_dup() {
        let (_f, conn) = temp_conn();
        run_migrations(&conn).expect("first run");
        run_migrations(&conn).expect("second run must not error");

        let versions = applied_versions(&conn);
        assert_eq!(
            versions,
            vec![0, 1, 2, 3, 4],
            "second run must not insert duplicate rows; got {versions:?}"
        );
    }

    /// Versions already in _meta_migrations before the runner is called are skipped.
    /// This simulates a live DB that has migrations 0..=2 applied; the runner
    /// should only apply 3 and 4.
    #[test]
    fn partial_db_only_applies_missing_versions() {
        let (_f, conn) = temp_conn();

        // Manually apply migrations 0, 1, 2 and record them.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS _meta_migrations (
                version    INTEGER PRIMARY KEY,
                applied_at TEXT    NOT NULL DEFAULT CURRENT_TIMESTAMP,
                filename   TEXT    NOT NULL
            );",
        )
        .unwrap();
        conn.execute_batch(include_str!("../../migrations/000_meta.sql"))
            .unwrap();
        conn.execute(
            "INSERT INTO _meta_migrations (version, filename) VALUES (0, '000_meta.sql')",
            [],
        )
        .unwrap();
        conn.execute_batch(include_str!("../../migrations/001_episodes.sql"))
            .unwrap();
        conn.execute(
            "INSERT INTO _meta_migrations (version, filename) VALUES (1, '001_episodes.sql')",
            [],
        )
        .unwrap();
        conn.execute_batch(include_str!("../../migrations/002_entities.sql"))
            .unwrap();
        conn.execute(
            "INSERT INTO _meta_migrations (version, filename) VALUES (2, '002_entities.sql')",
            [],
        )
        .unwrap();

        // Now run the full runner — must apply only 3 and 4.
        run_migrations(&conn).expect("run_migrations on partial DB");

        let versions = applied_versions(&conn);
        assert_eq!(versions, vec![0, 1, 2, 3, 4]);
        assert!(virtual_table_exists(&conn, "embeddings_vec"));
        assert!(virtual_table_exists(&conn, "episodes_fts"));
    }
}
