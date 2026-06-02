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

use std::collections::HashSet;

use rusqlite::Connection;

/// BFS over the `edges` table starting from `seed_entity_id`, following up to
/// `max_hops` hops in BOTH directions (src→dst and dst→src), scoped to
/// `bot_id` via the entity table.
///
/// Ports the BFS loop in `routes_memory.py::recall_about`:
/// ```python
/// visited = {seed_id}
/// frontier = {seed_id}
/// for _ in range(max(0, req.max_hops)):
///     if not frontier: break
///     # query: WHERE src_entity_id IN frontier OR dst_entity_id IN frontier
///     for s, d in rows:
///         for n in (s, d):
///             if n not in visited:
///                 next_frontier.add(n); visited.add(n)
///     frontier = next_frontier
/// ```
///
/// Returns all reachable entity_ids including `seed_entity_id`.
/// The edges table has no `bot_id` column; scoping is implicit through
/// `memory_entities` (the caller uses the returned ids against `m.bot_id`).
pub fn bfs_entities(
    conn: &Connection,
    seed_entity_id: i64,
    max_hops: usize,
) -> rusqlite::Result<Vec<i64>> {
    let mut visited: HashSet<i64> = HashSet::new();
    visited.insert(seed_entity_id);
    let mut frontier: HashSet<i64> = HashSet::new();
    frontier.insert(seed_entity_id);

    for _ in 0..max_hops {
        if frontier.is_empty() {
            break;
        }
        let frontier_vec: Vec<i64> = frontier.iter().copied().collect();
        // Build a dynamic placeholder string for the IN clause.
        // We need it twice (for src_entity_id and dst_entity_id).
        let placeholders = frontier_vec
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(",");
        let n = frontier_vec.len();
        // Offset for the second IN set: ?{n+1} … ?{2n}
        let placeholders2 = frontier_vec
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", n + i + 1))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT src_entity_id, dst_entity_id FROM edges \
             WHERE src_entity_id IN ({placeholders}) \
                OR dst_entity_id IN ({placeholders2})"
        );

        // Build the params list: frontier ids × 2.
        let mut params: Vec<rusqlite::types::Value> = frontier_vec
            .iter()
            .map(|&id| rusqlite::types::Value::Integer(id))
            .collect();
        params.extend(frontier_vec.iter().map(|&id| rusqlite::types::Value::Integer(id)));

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(params.iter()), |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let mut next_frontier: HashSet<i64> = HashSet::new();
        for (s, d) in rows {
            for &n_id in &[s, d] {
                if !visited.contains(&n_id) {
                    next_frontier.insert(n_id);
                    visited.insert(n_id);
                }
            }
        }
        frontier = next_frontier;
    }

    Ok(visited.into_iter().collect())
}

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

    // -----------------------------------------------------------------------
    // bfs_entities tests
    // -----------------------------------------------------------------------

    /// Helper: insert an edge between two entity_ids.
    fn insert_edge(conn: &rusqlite::Connection, src: i64, dst: i64) {
        conn.execute(
            "INSERT OR REPLACE INTO edges \
             (src_entity_id, rel, dst_entity_id, weight, last_seen_ts) \
             VALUES (?1, 'related', ?2, 1.0, 0)",
            rusqlite::params![src, dst],
        )
        .unwrap();
    }

    /// BFS-1: no edges → only the seed is returned.
    #[test]
    fn bfs_no_edges_returns_seed_only() {
        let conn = open_migrated();
        let seed = upsert_entity(&conn, "bot1", "Alice", None).unwrap();
        let result = bfs_entities(&conn, seed, 2).unwrap();
        assert_eq!(result, vec![seed], "no edges → only seed returned");
    }

    /// BFS-2: direct edge (1 hop).  max_hops=1 reaches the neighbour.
    #[test]
    fn bfs_one_hop_reaches_neighbour() {
        let conn = open_migrated();
        let alice = upsert_entity(&conn, "bot1", "Alice", None).unwrap();
        let bob = upsert_entity(&conn, "bot1", "Bob", None).unwrap();
        insert_edge(&conn, alice, bob);

        let mut result = bfs_entities(&conn, alice, 1).unwrap();
        result.sort();
        let mut expected = vec![alice, bob];
        expected.sort();
        assert_eq!(result, expected);
    }

    /// BFS-3: both directions — edge dst→src reached when starting from src.
    #[test]
    fn bfs_follows_reverse_direction() {
        let conn = open_migrated();
        let alice = upsert_entity(&conn, "bot1", "Alice", None).unwrap();
        let carol = upsert_entity(&conn, "bot1", "Carol", None).unwrap();
        // Edge: carol → alice (alice is the dst)
        insert_edge(&conn, carol, alice);

        // Starting from alice, should reach carol via reverse direction.
        let mut result = bfs_entities(&conn, alice, 1).unwrap();
        result.sort();
        let mut expected = vec![alice, carol];
        expected.sort();
        assert_eq!(result, expected, "reverse direction must be followed");
    }

    /// BFS-4: 2-hop chain A→B→C with max_hops=2 reaches all three.
    #[test]
    fn bfs_two_hop_chain() {
        let conn = open_migrated();
        let a = upsert_entity(&conn, "bot1", "A", None).unwrap();
        let b = upsert_entity(&conn, "bot1", "B", None).unwrap();
        let c = upsert_entity(&conn, "bot1", "C", None).unwrap();
        insert_edge(&conn, a, b);
        insert_edge(&conn, b, c);

        let mut result = bfs_entities(&conn, a, 2).unwrap();
        result.sort();
        let mut expected = vec![a, b, c];
        expected.sort();
        assert_eq!(result, expected, "2-hop chain from A must reach C");
    }

    /// BFS-5: 2-hop chain with max_hops=1 stops at B (does not reach C).
    #[test]
    fn bfs_max_hops_limits_reach() {
        let conn = open_migrated();
        let a = upsert_entity(&conn, "bot1", "A", None).unwrap();
        let b = upsert_entity(&conn, "bot1", "B", None).unwrap();
        let c = upsert_entity(&conn, "bot1", "C", None).unwrap();
        insert_edge(&conn, a, b);
        insert_edge(&conn, b, c);

        let mut result = bfs_entities(&conn, a, 1).unwrap();
        result.sort();
        let mut expected = vec![a, b];
        expected.sort();
        assert_eq!(result, expected, "max_hops=1 must not reach C");
        assert!(!result.contains(&c), "C must not be reachable with max_hops=1");
    }

    /// BFS-6: max_hops=0 → only the seed.
    #[test]
    fn bfs_zero_hops_returns_seed_only() {
        let conn = open_migrated();
        let a = upsert_entity(&conn, "bot1", "A", None).unwrap();
        let b = upsert_entity(&conn, "bot1", "B", None).unwrap();
        insert_edge(&conn, a, b);

        let result = bfs_entities(&conn, a, 0).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result.contains(&a));
    }
}
