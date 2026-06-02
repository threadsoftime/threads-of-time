//! Entity upsert helper — ports `helpers.py::upsert_entity`.
//!
//! ```python
//! def upsert_entity(conn, bot_id, name, type_hint=None) -> int:
//!     lower = name.lower()
//!     cur = conn.execute(
//!         "SELECT id, type FROM entities WHERE bot_id=? AND name_lower=?",
//!         (bot_id, lower),
//!     )
//!     row = cur.fetchone()
//!     if row:
//!         eid, existing_type = row
//!         if type_hint is not None and existing_type is None:
//!             conn.execute("UPDATE entities SET type=? WHERE id=?", (type_hint, eid))
//!         return eid
//!     cur = conn.execute(
//!         "INSERT INTO entities (bot_id, name_lower, display_name, type) VALUES (?, ?, ?, ?)",
//!         (bot_id, lower, name, type_hint),
//!     )
//!     return cur.lastrowid
//! ```

use rusqlite::Connection;

/// Insert or fetch an entity row; optionally set `type` when the row is new or
/// the existing row has `type IS NULL`.
///
/// Returns the `entity_id` (INTEGER PRIMARY KEY rowid).
///
/// Matching `helpers.py`:
/// - Lookup key: `(bot_id, name_lower)` where `name_lower = name.to_lowercase()`.
/// - `display_name` is set to `name` (original casing) on insert.
/// - `type_hint`: honored only when (a) the entity is new OR (b) the existing
///   row has `type IS NULL`. Existing non-null types are never overwritten.
pub fn upsert_entity(
    conn: &Connection,
    bot_id: &str,
    name: &str,
    type_hint: Option<&str>,
) -> rusqlite::Result<i64> {
    let name_lower = name.to_lowercase();

    // Attempt lookup first (fast path for existing entities).
    let existing = conn.query_row(
        "SELECT id, type FROM entities WHERE bot_id = ?1 AND name_lower = ?2",
        rusqlite::params![bot_id, name_lower],
        |row| {
            let id: i64 = row.get(0)?;
            let entity_type: Option<String> = row.get(1)?;
            Ok((id, entity_type))
        },
    );

    match existing {
        Ok((eid, existing_type)) => {
            // Entity exists — optionally set type if the hint is present and
            // the existing type is NULL (mirrors Python's `if type_hint is not
            // None and existing_type is None`).
            if type_hint.is_some() && existing_type.is_none() {
                conn.execute(
                    "UPDATE entities SET type = ?1 WHERE id = ?2",
                    rusqlite::params![type_hint, eid],
                )?;
            }
            Ok(eid)
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            // Entity does not exist — insert a new row.
            conn.execute(
                "INSERT INTO entities (bot_id, name_lower, display_name, type) \
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![bot_id, name_lower, name, type_hint],
            )?;
            // For an INSERT on a table with an INTEGER PRIMARY KEY, the rowid
            // equals the primary key value.
            Ok(conn.last_insert_rowid())
        }
        Err(e) => Err(e),
    }
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

    /// Inserting a new entity returns a positive rowid.
    #[test]
    fn upsert_new_entity_returns_rowid() {
        let conn = open_migrated();
        let id = upsert_entity(&conn, "bot1", "Alice", None).expect("upsert");
        assert!(id > 0, "rowid must be > 0, got {id}");
    }

    /// Upserting the same (bot_id, name_lower) twice returns the same rowid.
    #[test]
    fn upsert_same_name_twice_returns_same_id() {
        let conn = open_migrated();
        let id1 = upsert_entity(&conn, "bot1", "Alice", None).expect("first upsert");
        let id2 = upsert_entity(&conn, "bot1", "Alice", None).expect("second upsert");
        assert_eq!(id1, id2, "same (bot_id, name) must return the same entity_id");
    }

    /// Case-insensitive: "Alice" and "alice" resolve to the same entity.
    #[test]
    fn upsert_case_insensitive_match() {
        let conn = open_migrated();
        let id1 = upsert_entity(&conn, "bot1", "Alice", None).expect("first upsert");
        let id2 = upsert_entity(&conn, "bot1", "alice", None).expect("lowercase upsert");
        assert_eq!(
            id1, id2,
            "\"Alice\" and \"alice\" must resolve to the same entity_id"
        );
    }

    /// Different bots can have entities with the same name — they get distinct rowids.
    #[test]
    fn upsert_different_bots_are_distinct() {
        let conn = open_migrated();
        let id1 = upsert_entity(&conn, "bot1", "Alice", None).expect("bot1 upsert");
        let id2 = upsert_entity(&conn, "bot2", "Alice", None).expect("bot2 upsert");
        assert_ne!(id1, id2, "different bots must produce separate entity rows");
    }

    /// display_name preserves the original casing on insert.
    #[test]
    fn upsert_stores_display_name_with_original_casing() {
        let conn = open_migrated();
        upsert_entity(&conn, "bot1", "Alice", None).expect("upsert");

        let display: String = conn
            .query_row(
                "SELECT display_name FROM entities WHERE bot_id='bot1' AND name_lower='alice'",
                [],
                |row| row.get(0),
            )
            .expect("SELECT display_name");
        assert_eq!(display, "Alice", "display_name must preserve original casing");
    }

    /// type_hint is stored on a new entity.
    #[test]
    fn upsert_stores_type_hint_on_new_entity() {
        let conn = open_migrated();
        upsert_entity(&conn, "bot1", "Stormwind", Some("location")).expect("upsert");

        let entity_type: Option<String> = conn
            .query_row(
                "SELECT type FROM entities WHERE bot_id='bot1' AND name_lower='stormwind'",
                [],
                |row| row.get(0),
            )
            .expect("SELECT type");
        assert_eq!(entity_type.as_deref(), Some("location"));
    }

    /// type_hint updates an existing NULL-type entity.
    #[test]
    fn upsert_updates_null_type_with_hint() {
        let conn = open_migrated();
        // Insert without type.
        let id = upsert_entity(&conn, "bot1", "Arthas", None).expect("insert");
        // Second call with type_hint — should update because existing type is NULL.
        let id2 = upsert_entity(&conn, "bot1", "Arthas", Some("character")).expect("update hint");
        assert_eq!(id, id2, "same entity_id");

        let entity_type: Option<String> = conn
            .query_row(
                "SELECT type FROM entities WHERE id = ?1",
                rusqlite::params![id],
                |row| row.get(0),
            )
            .expect("SELECT type");
        assert_eq!(entity_type.as_deref(), Some("character"));
    }

    /// type_hint does NOT overwrite an existing non-null type.
    #[test]
    fn upsert_does_not_overwrite_existing_type() {
        let conn = open_migrated();
        upsert_entity(&conn, "bot1", "Arthas", Some("character")).expect("insert with type");
        upsert_entity(&conn, "bot1", "Arthas", Some("npc")).expect("second call with different hint");

        let entity_type: Option<String> = conn
            .query_row(
                "SELECT type FROM entities WHERE bot_id='bot1' AND name_lower='arthas'",
                [],
                |row| row.get(0),
            )
            .expect("SELECT type");
        assert_eq!(
            entity_type.as_deref(),
            Some("character"),
            "existing non-null type must not be overwritten"
        );
    }
}
