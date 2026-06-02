// SPDX-License-Identifier: GPL-2.0-or-later
//! Entity hard-filter for hybrid recall (design subspec §6.7).
//!
//! OR-logic across supplied names; exact match on `entities.display_name`.
//! Returns the set of matching `episode_id` values.

use std::collections::HashSet;

use rusqlite::Connection;

/// Return the set of `episode_id` values that reference any entity whose
/// `display_name` is in `entity_names`.
///
/// * Empty `entity_names` → immediately returns an empty set (caller's
///   signal to skip the filter entirely).
/// * Names not present in the `entities` table → contribute nothing (no error).
pub fn entity_filter(
    conn: &Connection,
    entity_names: &[String],
) -> rusqlite::Result<HashSet<i64>> {
    if entity_names.is_empty() {
        return Ok(HashSet::new());
    }

    // Build `IN (?, ?, ...)` dynamically — rusqlite does not support binding
    // a Rust slice directly into an IN clause.
    let placeholders = entity_names
        .iter()
        .enumerate()
        .map(|(i, _)| format!("?{}", i + 1))
        .collect::<Vec<_>>()
        .join(", ");

    let sql = format!(
        "SELECT DISTINCT ee.episode_id \
         FROM episode_entities AS ee \
         JOIN entities AS e ON e.entity_id = ee.entity_id \
         WHERE e.display_name IN ({placeholders})"
    );

    let mut stmt = conn.prepare(&sql)?;
    let params: Vec<&dyn rusqlite::ToSql> = entity_names
        .iter()
        .map(|n| n as &dyn rusqlite::ToSql)
        .collect();

    let ids = stmt
        .query_map(params.as_slice(), |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<HashSet<_>>>()?;
    Ok(ids)
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

    fn insert_episode(conn: &Connection, text: &str) -> i64 {
        conn.execute(
            "INSERT INTO episodes (timestamp, content_text, episode_type, salience_score) \
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![1_748_395_200_000_i64, text, "social", 0.5_f64],
        )
        .expect("insert_episode");
        conn.last_insert_rowid()
    }

    fn insert_entity(conn: &Connection, kind: &str, key: &str, display: &str) -> i64 {
        conn.execute(
            "INSERT INTO entities (entity_kind, entity_key, display_name) VALUES (?1, ?2, ?3)",
            rusqlite::params![kind, key, display],
        )
        .expect("insert_entity");
        conn.last_insert_rowid()
    }

    fn link(conn: &Connection, episode_id: i64, entity_id: i64, role: &str) {
        conn.execute(
            "INSERT INTO episode_entities (episode_id, entity_id, role) VALUES (?1, ?2, ?3)",
            rusqlite::params![episode_id, entity_id, role],
        )
        .expect("link");
    }

    fn names(ns: &[&str]) -> Vec<String> {
        ns.iter().map(|s| s.to_string()).collect()
    }

    /// Empty input immediately returns an empty set (no DB round-trip needed).
    #[test]
    fn entity_filter_empty_input_returns_empty() {
        let (_dir, conn) = setup();
        let result = entity_filter(&conn, &[]).expect("entity_filter");
        assert!(result.is_empty());
    }

    /// Name not present in the entities table → empty set, no error.
    #[test]
    fn entity_filter_unknown_name_returns_empty() {
        let (_dir, conn) = setup();
        let result = entity_filter(&conn, &names(&["NoSuchEntity"])).expect("entity_filter");
        assert!(result.is_empty());
    }

    /// OR-logic: supplying one name returns only episodes that reference it.
    #[test]
    fn entity_filter_single_name_exact_match() {
        let (_dir, conn) = setup();
        let e1 = insert_episode(&conn, "Alice and Bob raided");
        let e2 = insert_episode(&conn, "Bob alone");
        let e3 = insert_episode(&conn, "no entities");
        let alice = insert_entity(&conn, "player", "100", "Alice");
        let bob   = insert_entity(&conn, "player", "200", "Bob");
        link(&conn, e1, alice, "participant");
        link(&conn, e1, bob,   "participant");
        link(&conn, e2, bob,   "participant");
        conn.execute_batch("COMMIT").ok();

        let ids = entity_filter(&conn, &names(&["Alice"])).expect("entity_filter");
        assert_eq!(ids, HashSet::from([e1]));

        let ids = entity_filter(&conn, &names(&["Bob"])).expect("entity_filter");
        assert_eq!(ids, HashSet::from([e1, e2]));

        // e3 has no entities — must never appear.
        let ids = entity_filter(&conn, &names(&["Alice", "Bob"])).expect("entity_filter");
        assert!(ids.contains(&e1));
        assert!(ids.contains(&e2));
        assert!(!ids.contains(&e3), "e3 has no entities");
    }

    /// OR-logic: two names → union of their episode sets.
    #[test]
    fn entity_filter_or_logic_across_names() {
        let (_dir, conn) = setup();
        let e1 = insert_episode(&conn, "only Alice");
        let e2 = insert_episode(&conn, "only Carol");
        let alice = insert_entity(&conn, "player", "100", "Alice");
        let carol = insert_entity(&conn, "player", "300", "Carol");
        link(&conn, e1, alice, "subject");
        link(&conn, e2, carol, "subject");
        conn.execute_batch("COMMIT").ok();

        let ids = entity_filter(&conn, &names(&["Alice", "Carol"])).expect("entity_filter");
        assert_eq!(ids, HashSet::from([e1, e2]));
    }

    /// A single episode linked to one entity in two roles appears in the result exactly once.
    #[test]
    fn entity_filter_deduplicates_multi_role_links() {
        let (_dir, conn) = setup();
        let e = insert_episode(&conn, "Carol fights Carol");
        let carol = insert_entity(&conn, "player", "300", "Carol");
        link(&conn, e, carol, "subject");
        link(&conn, e, carol, "target");
        conn.execute_batch("COMMIT").ok();

        let ids = entity_filter(&conn, &names(&["Carol"])).expect("entity_filter");
        assert_eq!(ids, HashSet::from([e]));
        assert_eq!(ids.len(), 1, "duplicate links must not inflate the set");
    }

    /// Exact name match only — partial substring is not matched.
    #[test]
    fn entity_filter_exact_name_not_substring() {
        let (_dir, conn) = setup();
        let e = insert_episode(&conn, "full name episode");
        let ent = insert_entity(&conn, "player", "400", "Alicia");
        link(&conn, e, ent, "participant");
        conn.execute_batch("COMMIT").ok();

        // "Alice" does not match "Alicia"
        let ids = entity_filter(&conn, &names(&["Alice"])).expect("entity_filter");
        assert!(ids.is_empty(), "partial match should not be returned");
    }
}
