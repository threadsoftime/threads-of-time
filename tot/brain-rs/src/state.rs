//! SQLite state store — faithful Rust port of `brain_sidecar/state.py`.
//!
//! # Design
//!
//! * Single `Mutex<Connection>` serialises all writes, mirroring the Python
//!   `threading.Lock` that guards non-atomic SELECT+INSERT sequences (e.g.
//!   `append_decision`).
//! * WAL mode + `foreign_keys=ON` set in `open()`, matching Python's pragmas.
//! * Three migration files are embedded at compile time via `include_str!`;
//!   `migrate()` applies them in sorted version order, skipping already-applied
//!   ones — idempotent on a live DB already at the latest schema_version.
//! * Ring-buffer depth (`_RING_DEPTH = 20`): after each INSERT into
//!   `decisions_recent`, rows with `seq <= next_seq - 20` are pruned, exactly
//!   mirroring the Python prune logic.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};

use crate::models::{Decision, PersonalityCard};

// ---------------------------------------------------------------------------
// Compile-time embedded migration SQL
// ---------------------------------------------------------------------------

/// 0001_living_bots.sql — creates living_bots, decisions_recent,
/// schema_version; idempotent (all IF NOT EXISTS + INSERT OR IGNORE).
const MIG_0001: &str = include_str!("../migrations/0001_living_bots.sql");

/// 0002_add_last_event_id.sql — ALTER TABLE + INSERT OR IGNORE INTO schema_version.
const MIG_0002: &str = include_str!("../migrations/0002_add_last_event_id.sql");

/// 0004_subset_tier.sql — ALTER TABLE (tier, hysteresis, pin) + CREATE INDEX.
/// Note: there is no 0003 migration.
const MIG_0004: &str = include_str!("../migrations/0004_subset_tier.sql");

/// Ordered list of `(file_version, sql)` pairs.  Version 1 is always applied
/// (safe because it uses `IF NOT EXISTS` / `INSERT OR IGNORE` everywhere);
/// versions > 1 are skipped if `schema_version` already records them.
const MIGRATIONS: &[(i64, &str)] = &[
    (1, MIG_0001),
    (2, MIG_0002),
    (4, MIG_0004),
];

/// Ring-buffer depth: keep at most this many `decisions_recent` rows per bot.
const RING_DEPTH: i64 = 20;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A row from `living_bots` with the personality seed deserialised.
/// Mirrors the Python `LivingBotRow` dataclass in `state.py`.
#[derive(Debug, Clone)]
pub struct LivingBotRow {
    pub bot_guid: i64,
    pub enrolled_at: i64,
    pub last_seen: Option<i64>,
    pub status: String,
    pub personality_seed: PersonalityCard,
}

// ---------------------------------------------------------------------------
// StateStore
// ---------------------------------------------------------------------------

/// Thin sync wrapper over a `rusqlite::Connection`.
///
/// Mirrors `brain_sidecar.state.StateStore`.  All async callers should use
/// `spawn_blocking` to avoid blocking the executor thread.
pub struct StateStore {
    conn: Mutex<Connection>,
}

impl StateStore {
    /// Open (or create) the SQLite database at `db_path`.
    ///
    /// Sets `journal_mode=WAL` and `foreign_keys=ON`, matching Python:
    /// ```python
    /// self._conn.execute("PRAGMA journal_mode=WAL")
    /// self._conn.execute("PRAGMA foreign_keys=ON")
    /// ```
    pub fn open(db_path: &str) -> Result<Self, rusqlite::Error> {
        if let Some(parent) = Path::new(db_path).parent() {
            // Ignore errors (e.g. parent is empty string ""); the open() call
            // below will surface any real I/O problem.
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    // -----------------------------------------------------------------------
    // Migrations
    // -----------------------------------------------------------------------

    /// Apply migrations in sorted version order, skipping already-applied ones.
    ///
    /// Mirrors `StateStore.migrate()` in `state.py`:
    /// * Migration 0001 is always executed via `execute_batch` (safe: uses
    ///   `IF NOT EXISTS` / `INSERT OR IGNORE` everywhere).
    /// * Migrations with `file_version > 1` are skipped when
    ///   `MAX(version) >= file_version` in `schema_version`.
    ///
    /// Calling `migrate()` on a fully-migrated DB is a clean no-op.
    pub fn migrate(&self) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        for &(file_version, sql) in MIGRATIONS {
            if file_version > 1 {
                // Check whether this migration has already been applied.
                let current: i64 = conn
                    .query_row(
                        "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                        [],
                        |row| row.get(0),
                    )
                    .unwrap_or(0);
                if current >= file_version {
                    continue; // already applied
                }
            }
            conn.execute_batch(sql)?;
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Bot lifecycle
    // -----------------------------------------------------------------------

    /// Enrol a new bot.  Mirrors Python `enroll()`.
    ///
    /// Returns `Err` with a descriptive message if the bot is already enrolled
    /// and not in `released` status — matching the Python `ValueError` guard.
    pub fn enroll(
        &self,
        bot_guid: i64,
        enrolled_at_ms: i64,
        personality_seed: &PersonalityCard,
    ) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        let existing: Option<i64> = conn
            .query_row(
                "SELECT bot_guid FROM living_bots WHERE bot_guid = ? AND status != 'released'",
                params![bot_guid],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|e: rusqlite::Error| e.to_string())?;
        if existing.is_some() {
            return Err(format!("bot_guid {bot_guid} already enrolled"));
        }
        let seed_json = serde_json::to_string(personality_seed)
            .map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR REPLACE INTO living_bots \
             (bot_guid, enrolled_at, last_seen, status, personality_seed_json) \
             VALUES (?, ?, ?, 'active', ?)",
            params![bot_guid, enrolled_at_ms, Option::<i64>::None, seed_json],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Update the `status` column for a bot.  Mirrors Python `set_status()`.
    pub fn set_status(&self, bot_guid: i64, status: &str) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE living_bots SET status = ? WHERE bot_guid = ?",
            params![status, bot_guid],
        )?;
        Ok(())
    }

    /// Fetch a single bot row.  Returns `None` if the `bot_guid` is not found.
    /// Mirrors Python `get_bot()`.
    pub fn get_bot(&self, bot_guid: i64) -> Result<Option<LivingBotRow>, String> {
        let conn = self.conn.lock().unwrap();
        let row: Option<(i64, i64, Option<i64>, String, String)> = conn
            .query_row(
                "SELECT bot_guid, enrolled_at, last_seen, status, personality_seed_json \
                 FROM living_bots WHERE bot_guid = ?",
                params![bot_guid],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(|e: rusqlite::Error| e.to_string())?;

        match row {
            None => Ok(None),
            Some((guid, enrolled_at, last_seen, status, seed_json)) => {
                let personality_seed: PersonalityCard =
                    serde_json::from_str(&seed_json).map_err(|e| e.to_string())?;
                Ok(Some(LivingBotRow {
                    bot_guid: guid,
                    enrolled_at,
                    last_seen,
                    status,
                    personality_seed,
                }))
            }
        }
    }

    /// Return all bots with `status = 'active'`.  Mirrors Python `list_active()`.
    pub fn list_active(&self) -> Result<Vec<LivingBotRow>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT bot_guid, enrolled_at, last_seen, status, personality_seed_json \
                 FROM living_bots WHERE status = 'active'",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(|e| e.to_string())?;

        let mut result = Vec::new();
        for row in rows {
            let (guid, enrolled_at, last_seen, status, seed_json) =
                row.map_err(|e| e.to_string())?;
            let personality_seed: PersonalityCard =
                serde_json::from_str(&seed_json).map_err(|e| e.to_string())?;
            result.push(LivingBotRow {
                bot_guid: guid,
                enrolled_at,
                last_seen,
                status,
                personality_seed,
            });
        }
        Ok(result)
    }

    // -----------------------------------------------------------------------
    // Decision ring buffer
    // -----------------------------------------------------------------------

    /// Append a decision for `bot_guid` and prune old rows to maintain the
    /// ring-buffer depth of `RING_DEPTH = 20`.
    ///
    /// Mirrors Python `append_decision()` exactly:
    /// ```python
    /// next_seq = COALESCE(MAX(seq), -1) + 1
    /// INSERT ...
    /// DELETE WHERE bot_guid = ? AND seq <= next_seq - _RING_DEPTH
    /// ```
    ///
    /// The `Mutex` lock serialises the SELECT+INSERT against concurrent callers.
    pub fn append_decision(
        &self,
        bot_guid: i64,
        ts_ms: i64,
        decision: &Decision,
    ) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        // Compute next monotonic seq (COALESCE(MAX(seq), -1) + 1).
        let next_seq: i64 = conn.query_row(
            "SELECT COALESCE(MAX(seq), -1) FROM decisions_recent WHERE bot_guid = ?",
            params![bot_guid],
            |row| row.get::<_, i64>(0),
        )? + 1;

        let decision_json =
            serde_json::to_string(decision).expect("Decision serialization is infallible");

        conn.execute(
            "INSERT INTO decisions_recent (bot_guid, seq, ts_ms, decision_json) \
             VALUES (?, ?, ?, ?)",
            params![bot_guid, next_seq, ts_ms, decision_json],
        )?;

        // Prune anything older than the newest RING_DEPTH rows.
        conn.execute(
            "DELETE FROM decisions_recent WHERE bot_guid = ? AND seq <= ?",
            params![bot_guid, next_seq - RING_DEPTH],
        )?;

        Ok(())
    }

    /// Return the `k` most-recent decisions for `bot_guid`, ordered newest-first.
    ///
    /// Mirrors Python `decisions_recent(*, bot_guid, k)`:
    /// ```sql
    /// SELECT decision_json FROM decisions_recent
    /// WHERE bot_guid = ? ORDER BY ts_ms DESC LIMIT ?
    /// ```
    pub fn decisions_recent(
        &self,
        bot_guid: i64,
        k: i64,
    ) -> Result<Vec<Decision>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT decision_json FROM decisions_recent \
                 WHERE bot_guid = ? ORDER BY ts_ms DESC LIMIT ?",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![bot_guid, k], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?;

        let mut result = Vec::new();
        for row in rows {
            let json = row.map_err(|e| e.to_string())?;
            let d: Decision = serde_json::from_str(&json).map_err(|e| e.to_string())?;
            result.push(d);
        }
        Ok(result)
    }

    // -----------------------------------------------------------------------
    // SSE reconnect cursor
    // -----------------------------------------------------------------------

    /// Return the last SSE `row_id` persisted for this bot (0 if unknown).
    /// Mirrors Python `read_last_event_id()`.
    pub fn read_last_event_id(&self, bot_guid: i64) -> Result<i64, rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        let row: Option<i64> = conn
            .query_row(
                "SELECT last_event_id FROM living_bots WHERE bot_guid = ?",
                params![bot_guid],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        Ok(row.unwrap_or(0))
    }

    /// Monotonic update — never regress to a lower value.
    ///
    /// Mirrors Python `write_last_event_id()`:
    /// ```sql
    /// UPDATE living_bots SET last_event_id = ?
    /// WHERE bot_guid = ? AND last_event_id < ?
    /// ```
    pub fn write_last_event_id(
        &self,
        bot_guid: i64,
        event_id: i64,
    ) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE living_bots SET last_event_id = ? \
             WHERE bot_guid = ? AND last_event_id < ?",
            params![event_id, bot_guid, event_id],
        )?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Subset-gating helpers
    // -----------------------------------------------------------------------

    /// Set the bot's tier (`"full"` | `"reduced"`).
    /// Returns `Err` for invalid tier values, mirroring Python's `ValueError`.
    pub fn set_tier(&self, bot_guid: i64, tier: &str) -> Result<(), String> {
        if tier != "full" && tier != "reduced" {
            return Err(format!("invalid tier {tier:?}"));
        }
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE living_bots SET tier = ? WHERE bot_guid = ?",
            params![tier, bot_guid],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Return the bot's tier.  Returns `"full"` if the bot is not found or the
    /// column is NULL (mirrors Python default).
    pub fn get_tier(&self, bot_guid: i64) -> Result<String, rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        let row: Option<Option<String>> = conn
            .query_row(
                "SELECT tier FROM living_bots WHERE bot_guid = ?",
                params![bot_guid],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?;
        Ok(match row {
            None => "full".to_string(),
            Some(None) => "full".to_string(),
            Some(Some(t)) => t,
        })
    }

    /// Return `(in_range_ticks, out_of_range_ticks)`.
    /// Mirrors Python `get_hysteresis()`.
    pub fn get_hysteresis(&self, bot_guid: i64) -> Result<(i64, i64), rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        let row: Option<(Option<i64>, Option<i64>)> = conn
            .query_row(
                "SELECT in_range_ticks, out_of_range_ticks FROM living_bots WHERE bot_guid = ?",
                params![bot_guid],
                |row| Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, Option<i64>>(1)?)),
            )
            .optional()?;
        Ok(match row {
            None => (0, 0),
            Some((in_t, out_t)) => (in_t.unwrap_or(0), out_t.unwrap_or(0)),
        })
    }

    /// Increment the appropriate hysteresis counter and reset the other.
    ///
    /// Mirrors Python `bump_hysteresis()`:
    /// * `in_range=true`  → `in_range_ticks += 1; out_of_range_ticks = 0`
    /// * `in_range=false` → `in_range_ticks = 0; out_of_range_ticks += 1`
    ///
    /// Returns the new `(in_range_ticks, out_of_range_ticks)`.
    /// The `Mutex` lock serialises the SELECT+UPDATE.
    pub fn bump_hysteresis(
        &self,
        bot_guid: i64,
        in_range: bool,
    ) -> Result<(i64, i64), rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        let row: Option<(Option<i64>, Option<i64>)> = conn
            .query_row(
                "SELECT in_range_ticks, out_of_range_ticks FROM living_bots WHERE bot_guid = ?",
                params![bot_guid],
                |row| Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, Option<i64>>(1)?)),
            )
            .optional()?;
        let (in_t, out_t) = match row {
            None => return Ok((0, 0)),
            Some((in_t, out_t)) => (in_t.unwrap_or(0), out_t.unwrap_or(0)),
        };
        let (new_in, new_out) = if in_range {
            (in_t + 1, 0i64)
        } else {
            (0i64, out_t + 1)
        };
        conn.execute(
            "UPDATE living_bots SET in_range_ticks = ?, out_of_range_ticks = ? \
             WHERE bot_guid = ?",
            params![new_in, new_out, bot_guid],
        )?;
        Ok((new_in, new_out))
    }

    /// Flip a bot back to `status='active'` and reset hysteresis counters.
    /// Mirrors Python `reactivate()`.
    pub fn reactivate(&self, bot_guid: i64) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE living_bots \
             SET status='active', in_range_ticks=0, out_of_range_ticks=0 \
             WHERE bot_guid = ?",
            params![bot_guid],
        )?;
        Ok(())
    }

    /// Ensure a bot is enrolled with `status='active'`, regardless of prior status.
    ///
    /// Used at startup to pin exec-roster bots into the brain:
    /// * If the bot already has a row (any status) → UPDATE to `status='active'`,
    ///   reset hysteresis counters, and preserve existing personality.
    ///   Equivalent to `reactivate()`, but tolerates a bot that was never inserted.
    /// * If no row exists → INSERT with `placeholder_seed` as the personality card
    ///   (the SubsetGate's `enroll_via_api` will overwrite this with a real personality
    ///   on the first proximity-driven enroll; for the exec-roster the LLM brain loop
    ///   still needs SOME row to exist before `list_active()` can return it).
    ///
    /// Idempotent: calling it N times has the same effect as calling it once.
    pub fn seed_active(
        &self,
        bot_guid: i64,
        enrolled_at_ms: i64,
        placeholder_seed: &PersonalityCard,
    ) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        // Check if a row already exists.
        let existing: Option<i64> = conn
            .query_row(
                "SELECT bot_guid FROM living_bots WHERE bot_guid = ?",
                params![bot_guid],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|e: rusqlite::Error| e.to_string())?;

        if existing.is_some() {
            // Row exists — reactivate in place (preserve existing personality_seed_json).
            conn.execute(
                "UPDATE living_bots \
                 SET status='active', in_range_ticks=0, out_of_range_ticks=0 \
                 WHERE bot_guid = ?",
                params![bot_guid],
            )
            .map_err(|e| e.to_string())?;
        } else {
            // No row — insert with placeholder personality.
            let seed_json = serde_json::to_string(placeholder_seed)
                .map_err(|e| e.to_string())?;
            conn.execute(
                "INSERT INTO living_bots \
                 (bot_guid, enrolled_at, last_seen, status, personality_seed_json) \
                 VALUES (?, ?, ?, 'active', ?)",
                params![bot_guid, enrolled_at_ms, Option::<i64>::None, seed_json],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Set the `pinned` flag for a bot.  Mirrors Python `set_pin()`.
    pub fn set_pin(&self, bot_guid: i64, pinned: bool) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE living_bots SET pinned = ? WHERE bot_guid = ?",
            params![if pinned { 1i64 } else { 0i64 }, bot_guid],
        )?;
        Ok(())
    }

    /// Return the list of `bot_guid`s with `pinned = 1`.
    /// Mirrors Python `list_pinned()`.
    pub fn list_pinned(&self) -> Result<Vec<i64>, rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT bot_guid FROM living_bots WHERE pinned = 1",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn temp_store() -> (StateStore, NamedTempFile) {
        let f = NamedTempFile::new().unwrap();
        let s = StateStore::open(f.path().to_str().unwrap()).unwrap();
        s.migrate().unwrap();
        (s, f)
    }

    fn test_card() -> PersonalityCard {
        PersonalityCard {
            name: "Kael".into(),
            race: "Elf".into(),
            class_: "Paladin".into(),
            backstory: "A warrior.".into(),
            talkativeness: 0.5,
            courage: 0.7,
            greed: 0.3,
            attitude_to_master: 0.0,
            party_invite_policy: "accept_from_known".into(),
            pvp_appetite: None,
            raid_appetite: None,
            completionist_streak: None,
            gold_motivation: None,
            profession_appetite: None,
        }
    }

    #[test]
    fn test_enroll_and_get_bot() {
        let (store, _f) = temp_store();
        let card = test_card();
        store.enroll(1001, 1_000_000, &card).unwrap();
        let row = store.get_bot(1001).unwrap().unwrap();
        assert_eq!(row.bot_guid, 1001);
        assert_eq!(row.status, "active");
        assert_eq!(row.personality_seed.name, "Kael");
    }

    #[test]
    fn test_enroll_duplicate_active_rejects() {
        let (store, _f) = temp_store();
        let card = test_card();
        store.enroll(1001, 1_000_000, &card).unwrap();
        let err = store.enroll(1001, 1_000_001, &card);
        assert!(err.is_err(), "double-enroll should fail");
    }

    #[test]
    fn test_append_decision_ring_buffer_depth_20() {
        let (store, _f) = temp_store();
        let card = test_card();
        store.enroll(1001, 0, &card).unwrap();
        let d = Decision {
            kind: crate::models::DecisionKind::NoOp,
            tool: None,
            args: None,
            confidence: 0.0,
            reasoning: "x".into(),
            wakeup_in_ms: None,
        };
        for _ in 0..25 {
            store.append_decision(1001, 0, &d).unwrap();
        }
        let recent = store.decisions_recent(1001, 25).unwrap();
        assert_eq!(recent.len(), 20, "ring buffer depth must be 20");
    }

    #[test]
    fn test_set_get_tier() {
        let (store, _f) = temp_store();
        let card = test_card();
        store.enroll(1001, 0, &card).unwrap();
        assert_eq!(store.get_tier(1001).unwrap(), "full");
        store.set_tier(1001, "reduced").unwrap();
        assert_eq!(store.get_tier(1001).unwrap(), "reduced");
    }

    #[test]
    fn test_bump_hysteresis() {
        let (store, _f) = temp_store();
        let card = test_card();
        store.enroll(1001, 0, &card).unwrap();
        let (in_t, out_t) = store.bump_hysteresis(1001, true).unwrap();
        assert_eq!(in_t, 1);
        assert_eq!(out_t, 0);
        let (in_t2, out_t2) = store.bump_hysteresis(1001, false).unwrap();
        assert_eq!(in_t2, 0);
        assert_eq!(out_t2, 1);
    }

    #[test]
    fn test_migrations_idempotent() {
        let (store, _f) = temp_store();
        // Calling migrate() a second time on the same DB must not fail.
        store.migrate().unwrap();
    }

    #[test]
    fn test_last_event_id_monotonic() {
        let (store, _f) = temp_store();
        let card = test_card();
        store.enroll(1001, 0, &card).unwrap();
        store.write_last_event_id(1001, 100).unwrap();
        assert_eq!(store.read_last_event_id(1001).unwrap(), 100);
        // Monotonic: writing a lower value must not regress.
        store.write_last_event_id(1001, 50).unwrap();
        assert_eq!(store.read_last_event_id(1001).unwrap(), 100);
    }

    #[test]
    fn test_decisions_recent_newest_first() {
        let (store, _f) = temp_store();
        let card = test_card();
        store.enroll(1001, 0, &card).unwrap();
        // Insert three decisions with distinct ts_ms values.
        for ts in [1000i64, 2000, 3000] {
            let d = Decision {
                kind: crate::models::DecisionKind::NoOp,
                tool: None,
                args: None,
                confidence: 0.0,
                reasoning: format!("ts={ts}"),
                wakeup_in_ms: None,
            };
            store.append_decision(1001, ts, &d).unwrap();
        }
        let recent = store.decisions_recent(1001, 3).unwrap();
        assert_eq!(recent.len(), 3);
        // Newest first: reasoning for ts=3000 must be first.
        assert_eq!(recent[0].reasoning, "ts=3000");
        assert_eq!(recent[2].reasoning, "ts=1000");
    }

    #[test]
    fn test_enroll_personality_round_trips_via_db() {
        let (store, _f) = temp_store();
        let card = test_card();
        store.enroll(1001, 42, &card).unwrap();
        let row = store.get_bot(1001).unwrap().unwrap();
        // Verify key fields survived the JSON round-trip through DB.
        assert_eq!(row.personality_seed.class_, "Paladin");
        assert_eq!(row.personality_seed.race, "Elf");
        assert_eq!(row.enrolled_at, 42);
        assert!(row.last_seen.is_none());
    }

    #[test]
    fn test_list_active() {
        let (store, _f) = temp_store();
        let card = test_card();
        store.enroll(1001, 0, &card).unwrap();
        store.enroll(1002, 0, &card).unwrap();
        store.set_status(1002, "released").unwrap();
        let active = store.list_active().unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].bot_guid, 1001);
    }

    #[test]
    fn test_reactivate() {
        let (store, _f) = temp_store();
        let card = test_card();
        store.enroll(1001, 0, &card).unwrap();
        store.set_status(1001, "released").unwrap();
        store.reactivate(1001).unwrap();
        let row = store.get_bot(1001).unwrap().unwrap();
        assert_eq!(row.status, "active");
    }

    #[test]
    fn test_set_pin_and_list_pinned() {
        let (store, _f) = temp_store();
        let card = test_card();
        store.enroll(1001, 0, &card).unwrap();
        store.enroll(1002, 0, &card).unwrap();
        store.set_pin(1001, true).unwrap();
        let pinned = store.list_pinned().unwrap();
        assert_eq!(pinned, vec![1001i64]);
        store.set_pin(1001, false).unwrap();
        let pinned2 = store.list_pinned().unwrap();
        assert!(pinned2.is_empty());
    }

    #[test]
    fn test_get_hysteresis_defaults_to_zero() {
        let (store, _f) = temp_store();
        let card = test_card();
        store.enroll(1001, 0, &card).unwrap();
        let (in_t, out_t) = store.get_hysteresis(1001).unwrap();
        assert_eq!(in_t, 0);
        assert_eq!(out_t, 0);
    }

    // ── exec-roster enrollment fix tests ──────────────────────────────────────

    /// Test 1 (state/seed): seeding an exec-roster guid that was previously
    /// `set_status("released")` makes it appear in `list_active()`.
    /// This is the fix for the "enrollment gap" where exec-roster bots were
    /// skipped by rehydration after a restart because their status was 'released'.
    #[test]
    fn test_seed_active_reactivates_released_bot() {
        let (store, _f) = temp_store();
        let card = test_card();

        // Enroll then release — simulates the state after a prior SubsetGate session.
        store.enroll(1001, 0, &card).unwrap();
        store.set_status(1001, "released").unwrap();

        // Verify it does NOT appear in list_active after release.
        let active_before = store.list_active().unwrap();
        assert!(active_before.is_empty(), "released bot must not be in list_active");

        // seed_active must flip it back to active.
        store.seed_active(1001, 0, &card).unwrap();

        let active_after = store.list_active().unwrap();
        assert_eq!(active_after.len(), 1, "seed_active must make bot appear in list_active");
        assert_eq!(active_after[0].bot_guid, 1001);
        assert_eq!(active_after[0].status, "active");
    }

    /// seed_active on a bot that has never been enrolled inserts a new active row.
    #[test]
    fn test_seed_active_inserts_when_no_row_exists() {
        let (store, _f) = temp_store();
        let card = test_card();

        // No prior enrollment.
        store.seed_active(9999, 42_000, &card).unwrap();

        let active = store.list_active().unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].bot_guid, 9999);
        assert_eq!(active[0].status, "active");
        assert_eq!(active[0].enrolled_at, 42_000);
    }

    /// seed_active is idempotent: calling it N times leaves exactly one active row.
    #[test]
    fn test_seed_active_is_idempotent() {
        let (store, _f) = temp_store();
        let card = test_card();

        store.seed_active(1001, 0, &card).unwrap();
        store.seed_active(1001, 0, &card).unwrap(); // second call must not error
        store.seed_active(1001, 0, &card).unwrap(); // third call

        let active = store.list_active().unwrap();
        assert_eq!(active.len(), 1, "idempotent: exactly one row");
    }

    /// seed_active preserves the existing personality when the bot already has a row.
    #[test]
    fn test_seed_active_preserves_existing_personality() {
        let (store, _f) = temp_store();
        let card = test_card(); // name = "Kael"

        store.enroll(1001, 0, &card).unwrap();
        store.set_status(1001, "released").unwrap();

        // Seed with a different placeholder card.
        let placeholder = PersonalityCard {
            name: "Placeholder".into(),
            race: "Gnome".into(),
            class_: "Mage".into(),
            backstory: "Unknown.".into(),
            talkativeness: 0.5,
            courage: 0.5,
            greed: 0.0,
            attitude_to_master: 0.0,
            party_invite_policy: "none".into(),
            pvp_appetite: None,
            raid_appetite: None,
            completionist_streak: None,
            gold_motivation: None,
            profession_appetite: None,
        };

        store.seed_active(1001, 0, &placeholder).unwrap();

        // Existing personality ("Kael") must be preserved — seed_active does an UPDATE,
        // not an INSERT OR REPLACE when a row exists.
        let row = store.get_bot(1001).unwrap().unwrap();
        assert_eq!(row.personality_seed.name, "Kael", "existing personality preserved");
        assert_eq!(row.status, "active");
    }
}
