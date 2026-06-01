//! Typed input structs and serde deserializers for live harness data.
//!
//! Two sources of data:
//!
//! 1. **`obs.game_events`** response → [`GroundTruth`] / [`GroundTruthEvent`].
//!    This is the C++ RESOLVED view: holiday math already applied by the server.
//!    Used as the DIFF TARGET in Task 13.
//!
//! 2. **`obs.query_db game_event_all`** response → a list of [`SqlGameEventRow`]
//!    which maps to [`GameEventInput`] (the RAW compute inputs to our Rust resolver).
//!    The SQL rows use camelCase keys (`eventEntry`, `holidayStage`); `start_time`/
//!    `end_time` may be null → treated as 0.
//!
//! # Key mapping decisions (faithful to REAL live payloads, 2026-05-31)
//!
//! - `state` int 0..5 → [`GameEventState`] via `TryFrom<u8>`.
//! - `looping` int 0/1 → `HolidaysEntry.looping: bool` via custom deserializer
//!   (lives in `resolve.rs` on the `HolidaysEntry` definition).
//! - `calendar_filter_type` can be negative (i32) — serialized as a signed int.
//! - `date` array: exactly 26 u32 elements (`MAX_HOLIDAY_DATES`).
//! - `duration` array: exactly 10 u32 elements (`MAX_HOLIDAY_DURATIONS`).
//! - `eventEntry` / `holidayStage` camelCase → snake-case Rust fields via
//!   `#[serde(rename = "...")]`.
//! - SQL `start_time` / `end_time` nullable → `Option<i64>` → default to 0.

use serde::{Deserialize, Deserializer};

use crate::resolve::{GameEventInput, HolidaysEntry};
use crate::schedule::{GameEventState, ResolvedEvent};

// ── GameEventState integer deserializer ──────────────────────────────────────

/// Deserialize an integer 0..5 into [`GameEventState`].
///
/// The harness payload sends `"state": 0` (not a string); this function maps it.
/// Unknown values are a hard deserialization failure — the C++ state enum is closed
/// at 0..5 and any unrecognized value signals a schema mismatch that must not be
/// silently ignored.
fn deserialize_state<'de, D: Deserializer<'de>>(d: D) -> Result<GameEventState, D::Error> {
    let v = u8::deserialize(d)?;
    GameEventState::try_from(v).map_err(|unknown| {
        serde::de::Error::custom(format!("unknown GameEventState integer: {unknown}"))
    })
}

// ── GroundTruth (obs.game_events result) ─────────────────────────────────────

/// A single event entry from the C++ server's resolved event table.
///
/// This carries the values AFTER C++ has run `SetHolidayEventTime` and
/// `LoadHolidayDates` — i.e., the holiday-math-resolved start/length/occurence.
/// Used as the DIFF TARGET in Task 13 (compare our Rust output against this).
///
/// Field units: `occurence` and `length` are in **minutes** (matching
/// `game_event` DB semantics and `ResolvedEvent`).
#[derive(Debug, Clone, Deserialize)]
pub struct GroundTruthEvent {
    pub entry: u16,
    /// Resolved start (Unix seconds).
    pub start: i64,
    /// End time from `game_event.end_time` (Unix seconds).
    pub end: i64,
    /// Recurrence period in **minutes**.
    pub occurence: i64,
    /// Active window in **minutes**.
    pub length: i64,
    /// Holiday ID (0 = no holiday).
    pub holiday: u32,
    /// Holiday stage (1-indexed; 0 = no stage).
    pub holiday_stage: u8,
    /// Whether the event is currently active on the server.
    pub is_active: bool,
    /// Next start for world-phase events (Unix seconds; 0 = not applicable).
    pub next_start: i64,
    /// State machine state, deserialized from an integer 0..5.
    #[serde(deserialize_with = "deserialize_state")]
    pub state: GameEventState,
}

/// Full result of `obs.game_events` (the `"result"` field after unwrapping `ok`).
///
/// Contains the server's resolved view of all game events, the active-event list,
/// the sHolidaysStore dump, and timing metadata.
#[derive(Debug, Clone, Deserialize)]
pub struct GroundTruth {
    /// All 181 game events, with server-resolved start/length/occurence.
    pub events: Vec<GroundTruthEvent>,
    /// Currently active event entry IDs (the server's live active set).
    pub active_event_list: Vec<u16>,
    /// Full sHolidaysStore dump: 24 holiday entries.
    pub holidays: Vec<HolidaysEntry>,
    /// Server game time at the moment the snapshot was taken (Unix seconds).
    pub server_gametime: i64,
    /// Reference time used by the server's `SetHolidayEventTime` (Unix seconds).
    pub resolve_reference_unixtime: i64,
    /// Server timezone offset from UTC in seconds (0 = UTC).
    pub server_tz_offset_secs: i32,
}

// ── Mapping: GroundTruthEvent → ResolvedEvent ─────────────────────────────────

impl From<&GroundTruthEvent> for ResolvedEvent {
    /// Convert a [`GroundTruthEvent`] (server ground-truth) into a [`ResolvedEvent`]
    /// for comparison with our Rust-resolved output in Task 13.
    ///
    /// `prerequisites` is always empty because the ground-truth payload does not
    /// carry the prerequisite list; `is_active` is carried in `GroundTruthEvent`
    /// and must be compared separately.
    fn from(e: &GroundTruthEvent) -> ResolvedEvent {
        ResolvedEvent {
            entry: e.entry,
            start: e.start,
            end: e.end,
            occurence: e.occurence,
            length: e.length,
            state: e.state,
            prerequisites: vec![],
            next_start: e.next_start,
        }
    }
}

// ── SQL row → GameEventInput ──────────────────────────────────────────────────

/// A raw row from `game_event` as returned by `obs.query_db game_event_all`.
///
/// The DB uses camelCase for some columns (`eventEntry`, `holidayStage`);
/// `start_time` and `end_time` may be null (events without a start/end time).
/// This struct is an intermediate deserialization target; convert to
/// [`GameEventInput`] via `Into<GameEventInput>`.
///
/// Note: the AC fork does NOT have `state` or `nextstart` columns in `game_event`.
/// `GameEventInput.state` is always set to `Normal`; `next_start` to 0.
#[derive(Debug, Clone, Deserialize)]
pub struct SqlGameEventRow {
    #[serde(rename = "eventEntry")]
    pub event_entry: u16,
    /// Nullable — some events have no explicit start time.
    pub start_time: Option<i64>,
    /// Nullable — some events have no explicit end time.
    pub end_time: Option<i64>,
    pub occurence: i64,
    pub length: i64,
    pub holiday: u32,
    #[serde(rename = "holidayStage")]
    pub holiday_stage: u8,
}

impl From<SqlGameEventRow> for GameEventInput {
    /// Map a SQL row to a [`GameEventInput`], preserving null timestamps as
    /// `None` so that callers can apply the C++ null-semantics rules:
    /// - `start_time = None` → effective start = 0  (no null-guard in C++ LoadGameEvents)
    /// - `end_time = None`   → effective end = resolve_ref + 63_072_000  (C++ lines 359-360)
    ///
    /// `state` defaults to `Normal` / `next_start` to 0 (this fork lacks
    /// those columns in `game_event`).
    fn from(row: SqlGameEventRow) -> GameEventInput {
        GameEventInput {
            entry: row.event_entry,
            start_time: row.start_time,
            end_time: row.end_time,
            occurence: row.occurence,
            length: row.length,
            holiday: row.holiday,
            holiday_stage: row.holiday_stage,
            state: GameEventState::Normal,
            next_start: 0,
        }
    }
}

// ── Wrapper types for harness response unwrapping ────────────────────────────

/// Wrapper for `{ "ok": true, "result": <GroundTruth> }`.
///
/// Unwrap with `.result` after deserializing.
#[derive(Debug, Deserialize)]
pub struct GameEventsResponse {
    pub ok: bool,
    pub result: GroundTruth,
}

/// The `result` object returned by `obs.query_db`.
///
/// The real live wire shape is `{ "row_count": <int>, "rows": [ ... ] }`.
/// We model only `rows`; serde ignores unknown fields by default, so
/// `row_count` is silently skipped.
#[derive(Debug, Deserialize)]
pub struct QueryDbResult {
    pub rows: Vec<SqlGameEventRow>,
}

/// Wrapper for `{ "ok": true, "result": { "row_count": <int>, "rows": [ ... ] } }`.
#[derive(Debug, Deserialize)]
pub struct SqlGameEventResponse {
    pub ok: bool,
    pub result: QueryDbResult,
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolve::{MAX_HOLIDAY_DATES, MAX_HOLIDAY_DURATIONS};

    // ── Sample JSON fixtures (from REAL live payloads, 2026-05-31) ────────────

    /// Minimal obs.game_events result fixture (2 events, 1 holiday).
    /// Taken verbatim from the live Heimdal response; field names, types, and
    /// null-handling are exact.
    const GAME_EVENTS_JSON: &str = r#"
    {
      "ok": true,
      "result": {
        "events": [
          {"entry":1,"start":1782000000,"end":1843316906,"occurence":525600,"length":20160,
           "holiday":341,"holiday_stage":1,"is_active":false,"next_start":0,"state":0},
          {"entry":16,"start":1779033600,"end":1843316906,"occurence":10080,"length":4320,
           "holiday":62,"holiday_stage":1,"is_active":true,"next_start":0,"state":0}
        ],
        "active_event_list": [16,20,29,30,42],
        "holidays": [
          {"holiday_id":62,"calendar_filter_type":-1,"looping":0,"region":1,
           "date":[425781248,442560512,459325440,476106752,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],
           "duration":[18,0,0,0,0,0,0,0,0,0]}
        ],
        "server_gametime": 1780245388,
        "resolve_reference_unixtime": 1780244906,
        "server_tz_offset_secs": 0
      }
    }"#;

    /// Sample SQL row fixture (obs.query_db game_event_all).
    /// Uses camelCase keys as the DB returns them. `start_time` is null for
    /// event 1 (confirmed from live data: first row has `"start_time": null`).
    ///
    /// The REAL live wire shape (2026-05-31 verified) is:
    /// `{ "ok": true, "result": { "row_count": <int>, "rows": [ ... ] } }`
    const SQL_ROWS_JSON: &str = r#"
    {
      "ok": true,
      "result": {
        "row_count": 2,
        "rows": [
          {"eventEntry":1,"start_time":null,"end_time":1843316906,"occurence":525600,"length":20160,
           "holiday":341,"holidayStage":1,"description":"Midsummer Fire Festival","world_event":0,"announce":2},
          {"eventEntry":16,"start_time":1779033600,"end_time":1843316906,"occurence":10080,"length":4320,
           "holiday":62,"holidayStage":1,"description":"Fireworks Spectacular","world_event":0,"announce":2}
        ]
      }
    }"#;

    // ── GroundTruth deserialization ───────────────────────────────────────────

    #[test]
    fn ground_truth_deserializes_event_count() {
        let resp: GameEventsResponse = serde_json::from_str(GAME_EVENTS_JSON)
            .expect("should parse GameEventsResponse");
        assert!(resp.ok);
        assert_eq!(resp.result.events.len(), 2, "should have 2 events");
    }

    #[test]
    fn ground_truth_event1_fields() {
        let resp: GameEventsResponse = serde_json::from_str(GAME_EVENTS_JSON).unwrap();
        let ev = &resp.result.events[0];
        assert_eq!(ev.entry, 1);
        assert_eq!(ev.start, 1782000000);
        assert_eq!(ev.end, 1843316906);
        assert_eq!(ev.occurence, 525600);
        assert_eq!(ev.length, 20160);
        assert_eq!(ev.holiday, 341);
        assert_eq!(ev.holiday_stage, 1);
        assert!(!ev.is_active, "is_active should be false");
        assert_eq!(ev.next_start, 0);
    }

    #[test]
    fn ground_truth_state_int_maps_to_enum() {
        let resp: GameEventsResponse = serde_json::from_str(GAME_EVENTS_JSON).unwrap();
        // Both test events have state=0 → Normal
        assert_eq!(resp.result.events[0].state, GameEventState::Normal);
        assert_eq!(resp.result.events[1].state, GameEventState::Normal);
    }

    #[test]
    fn ground_truth_active_event_list() {
        let resp: GameEventsResponse = serde_json::from_str(GAME_EVENTS_JSON).unwrap();
        assert_eq!(resp.result.active_event_list, vec![16u16, 20, 29, 30, 42]);
    }

    #[test]
    fn ground_truth_server_timestamps() {
        let resp: GameEventsResponse = serde_json::from_str(GAME_EVENTS_JSON).unwrap();
        assert_eq!(resp.result.server_gametime, 1780245388);
        assert_eq!(resp.result.resolve_reference_unixtime, 1780244906);
        assert_eq!(resp.result.server_tz_offset_secs, 0);
    }

    // ── HolidaysEntry deserialization ─────────────────────────────────────────

    #[test]
    fn holidays_entry_looping_int_to_bool() {
        let resp: GameEventsResponse = serde_json::from_str(GAME_EVENTS_JSON).unwrap();
        let h = &resp.result.holidays[0];
        // looping: 0 → false
        assert!(!h.looping, "looping int 0 should map to false");
    }

    #[test]
    fn holidays_entry_calendar_filter_type_negative() {
        let resp: GameEventsResponse = serde_json::from_str(GAME_EVENTS_JSON).unwrap();
        let h = &resp.result.holidays[0];
        // calendar_filter_type: -1 (yearly)
        assert_eq!(h.calendar_filter_type, -1);
    }

    #[test]
    fn holidays_entry_date_array_length() {
        let resp: GameEventsResponse = serde_json::from_str(GAME_EVENTS_JSON).unwrap();
        let h = &resp.result.holidays[0];
        // date has 26 elements (MAX_HOLIDAY_DATES)
        assert_eq!(h.date.len(), MAX_HOLIDAY_DATES, "date array should have {MAX_HOLIDAY_DATES} elements");
    }

    #[test]
    fn holidays_entry_duration_array_length() {
        let resp: GameEventsResponse = serde_json::from_str(GAME_EVENTS_JSON).unwrap();
        let h = &resp.result.holidays[0];
        // duration has 10 elements (MAX_HOLIDAY_DURATIONS)
        assert_eq!(
            h.duration.len(),
            MAX_HOLIDAY_DURATIONS,
            "duration array should have {MAX_HOLIDAY_DURATIONS} elements"
        );
    }

    #[test]
    fn holidays_entry_date_values() {
        let resp: GameEventsResponse = serde_json::from_str(GAME_EVENTS_JSON).unwrap();
        let h = &resp.result.holidays[0];
        // First 4 entries from the live payload
        assert_eq!(h.date[0], 425781248);
        assert_eq!(h.date[1], 442560512);
        assert_eq!(h.date[2], 459325440);
        assert_eq!(h.date[3], 476106752);
        // Remaining 22 entries are 0
        for i in 4..MAX_HOLIDAY_DATES {
            assert_eq!(h.date[i], 0, "date[{i}] should be 0");
        }
    }

    #[test]
    fn holidays_entry_region() {
        let resp: GameEventsResponse = serde_json::from_str(GAME_EVENTS_JSON).unwrap();
        let h = &resp.result.holidays[0];
        assert_eq!(h.region, 1);
    }

    // ── SQL row deserialization ───────────────────────────────────────────────

    #[test]
    fn sql_rows_deserializes_count() {
        let resp: SqlGameEventResponse = serde_json::from_str(SQL_ROWS_JSON)
            .expect("should parse SqlGameEventResponse");
        assert!(resp.ok);
        assert_eq!(resp.result.rows.len(), 2);
    }

    #[test]
    fn sql_row_camelcase_event_entry() {
        let resp: SqlGameEventResponse = serde_json::from_str(SQL_ROWS_JSON).unwrap();
        assert_eq!(resp.result.rows[0].event_entry, 1, "eventEntry should map to event_entry");
        assert_eq!(resp.result.rows[1].event_entry, 16);
    }

    #[test]
    fn sql_row_camelcase_holiday_stage() {
        let resp: SqlGameEventResponse = serde_json::from_str(SQL_ROWS_JSON).unwrap();
        assert_eq!(resp.result.rows[0].holiday_stage, 1, "holidayStage should map to holiday_stage");
    }

    #[test]
    fn sql_row_null_start_time_is_option_none() {
        let resp: SqlGameEventResponse = serde_json::from_str(SQL_ROWS_JSON).unwrap();
        // event 1 has null start_time
        assert!(
            resp.result.rows[0].start_time.is_none(),
            "null start_time should deserialize as None"
        );
    }

    #[test]
    fn sql_row_non_null_start_time() {
        let resp: SqlGameEventResponse = serde_json::from_str(SQL_ROWS_JSON).unwrap();
        assert_eq!(resp.result.rows[1].start_time, Some(1779033600));
    }

    // ── SqlGameEventRow → GameEventInput mapping ──────────────────────────────

    #[test]
    fn sql_row_to_game_event_input_null_start_preserved_as_none() {
        let resp: SqlGameEventResponse = serde_json::from_str(SQL_ROWS_JSON).unwrap();
        let row = resp.result.rows[0].clone();
        let input: GameEventInput = row.into();
        assert_eq!(
            input.start_time, None,
            "null start_time must be preserved as None in GameEventInput (caller applies effective_start)"
        );
    }

    #[test]
    fn sql_row_to_game_event_input_fields() {
        let resp: SqlGameEventResponse = serde_json::from_str(SQL_ROWS_JSON).unwrap();
        let row = resp.result.rows[1].clone();
        let input: GameEventInput = row.into();
        assert_eq!(input.entry, 16);
        assert_eq!(input.start_time, Some(1779033600));
        assert_eq!(input.end_time, Some(1843316906));
        assert_eq!(input.occurence, 10080);
        assert_eq!(input.length, 4320);
        assert_eq!(input.holiday, 62);
        assert_eq!(input.holiday_stage, 1);
    }

    #[test]
    fn sql_row_to_game_event_input_state_is_normal() {
        let resp: SqlGameEventResponse = serde_json::from_str(SQL_ROWS_JSON).unwrap();
        let input: GameEventInput = resp.result.rows[0].clone().into();
        assert_eq!(
            input.state,
            GameEventState::Normal,
            "SQL rows have no state column → always Normal"
        );
    }

    #[test]
    fn sql_row_to_game_event_input_next_start_is_zero() {
        let resp: SqlGameEventResponse = serde_json::from_str(SQL_ROWS_JSON).unwrap();
        let input: GameEventInput = resp.result.rows[0].clone().into();
        assert_eq!(input.next_start, 0, "SQL rows have no next_start column → always 0");
    }

    // ── Both start_time and end_time null (holiday rows from live data) ────────

    #[test]
    fn sql_row_both_times_null_preserved_as_none() {
        // The real live first row (eventEntry:1 Midsummer Fire Festival) has BOTH
        // start_time: null AND end_time: null (confirmed from live Heimdal 2026-05-31).
        // Both must be preserved as None so that callers can apply C++ null semantics:
        //   start=None → effective_start(None) = 0
        //   end=None   → effective_end(None, resolve_ref) = resolve_ref + 63_072_000
        let json = r#"{
          "ok": true,
          "result": {
            "row_count": 1,
            "rows": [
              {"eventEntry":1,"start_time":null,"end_time":null,"occurence":525600,"length":20160,
               "holiday":341,"holidayStage":1,"description":"Midsummer Fire Festival","world_event":0,"announce":2}
            ]
          }
        }"#;
        let resp: SqlGameEventResponse = serde_json::from_str(json)
            .expect("should parse row with both timestamps null");
        let row = resp.result.rows[0].clone();
        assert!(row.start_time.is_none(), "start_time should be None");
        assert!(row.end_time.is_none(), "end_time should be None");
        let input: GameEventInput = row.into();
        // GameEventInput now preserves None so callers can apply the C++ null-semantics rules.
        assert_eq!(input.start_time, None, "null start_time → None in GameEventInput");
        assert_eq!(input.end_time, None, "null end_time → None in GameEventInput");
    }

    // ── GroundTruthEvent → ResolvedEvent mapping ──────────────────────────────

    #[test]
    fn ground_truth_event_to_resolved_event() {
        let resp: GameEventsResponse = serde_json::from_str(GAME_EVENTS_JSON).unwrap();
        let gt_ev = &resp.result.events[0];
        let resolved = ResolvedEvent::from(gt_ev);
        assert_eq!(resolved.entry, gt_ev.entry);
        assert_eq!(resolved.start, gt_ev.start);
        assert_eq!(resolved.end, gt_ev.end);
        assert_eq!(resolved.occurence, gt_ev.occurence);
        assert_eq!(resolved.length, gt_ev.length);
        assert_eq!(resolved.state, gt_ev.state);
        assert_eq!(resolved.next_start, gt_ev.next_start);
        assert!(resolved.prerequisites.is_empty(), "prerequisites always empty from ground truth");
    }

    // ── GameEventState TryFrom<u8> ────────────────────────────────────────────

    #[test]
    fn game_event_state_try_from_all_valid_values() {
        assert_eq!(GameEventState::try_from(0u8), Ok(GameEventState::Normal));
        assert_eq!(GameEventState::try_from(1u8), Ok(GameEventState::WorldInactive));
        assert_eq!(GameEventState::try_from(2u8), Ok(GameEventState::WorldConditions));
        assert_eq!(GameEventState::try_from(3u8), Ok(GameEventState::WorldNextphase));
        assert_eq!(GameEventState::try_from(4u8), Ok(GameEventState::WorldFinished));
        assert_eq!(GameEventState::try_from(5u8), Ok(GameEventState::Internal));
    }

    #[test]
    fn game_event_state_try_from_invalid_returns_err() {
        assert!(GameEventState::try_from(6u8).is_err());
        assert!(GameEventState::try_from(255u8).is_err());
    }

    // ── Looping = 1 maps to true ──────────────────────────────────────────────

    #[test]
    fn holidays_entry_looping_one_maps_to_true() {
        let json = r#"{
          "ok": true,
          "result": {
            "events": [],
            "active_event_list": [],
            "holidays": [
              {"holiday_id":99,"calendar_filter_type":2,"looping":1,"region":0,
               "date":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],
               "duration":[24,48,0,0,0,0,0,0,0,0]}
            ],
            "server_gametime": 1000,
            "resolve_reference_unixtime": 1000,
            "server_tz_offset_secs": 0
          }
        }"#;
        let resp: GameEventsResponse = serde_json::from_str(json).unwrap();
        assert!(
            resp.result.holidays[0].looping,
            "looping int 1 should map to true"
        );
    }

    // ── holiday_id field in HolidaysEntry is deserialized ────────────────────────

    // The obs.game_events payload includes "holiday_id" in each holidays entry.
    // HolidaysEntry now carries the holiday_id field (needed by shadow diff CHECK 1
    // to map holidays to HolidayRules).
    #[test]
    fn holidays_entry_deserializes_holiday_id() {
        let resp: GameEventsResponse = serde_json::from_str(GAME_EVENTS_JSON).unwrap();
        assert_eq!(resp.result.holidays.len(), 1, "should deserialize one holiday entry");
        assert_eq!(
            resp.result.holidays[0].holiday_id,
            62,
            "holiday_id should be deserialized from payload"
        );
    }
}
