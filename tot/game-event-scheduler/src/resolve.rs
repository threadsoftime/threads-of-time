//! Holiday-event time resolution and dynamic-date generation.
//!
//! This module ports two C++ functions from `GameEventMgr.cpp`:
//!
//! - **`set_holiday_event_time`** (lines 1916–2006): computes `Start`, `Length`,
//!   and `Occurence` for a holiday-backed game event, given a synthetic
//!   `HolidaysEntry` (populated from the DBC) and the server reference time
//!   `resolve_ref`.  Does NOT set `End`; End is sourced from `game_event.end_time`.
//!
//! - **`generate_dynamic_dates`** (lines 1072–1150, Step 1): fills `Date[]` of a
//!   `HolidaysEntry` from the calculator rules (Darkmoon Faire special path or
//!   per-year `get_packed_holiday_date`).
//!
//! - **`apply_start_time_override`** (lines 1152–1190, Step 2): overrides
//!   `Date[0]` from `game_event.start_time` when its year ∈ [gen_year, 2030].
//!
//! Two public entry points:
//!
//! - **`resolve_periodic_event`**: for events with `holiday == 0`, directly
//!   copies start/end/occurence/length/state from the DB row.
//! - **`resolve_holiday_event`**: for holiday-backed events, calls
//!   `set_holiday_event_time` for Start/Length/Occurence; End from `row.end_time`.
//!
//! # Reference-parameterised design
//!
//! All resolution is parameterised on an explicit `resolve_ref` (the server
//! `GameTime` at load time, analogous to `curTime = GameTime::GetGameTime().count()`
//! in C++).  This makes the resolved event differ predictably from a reference
//! snapshot — the key property needed by Task 13 (shadow diff).
//!
//! # Confirmed C++ constants (verified 2026-05-30 against AC source)
//! - `SECOND` = 1                   (Common.h line 46)
//! - `MINUTE` = 60                  (Common.h line 47)
//! - `HOUR`   = 3 600               (Common.h line 48)
//! - `DAY`    = 86 400              (Common.h line 49)
//! - `WEEK`   = 604 800             (Common.h line 50)
//! - `YEAR`   = 31 536 000          (Common.h line 52: `DAY * 365`)
//! - `MAX_HOLIDAY_DATES`      = 26  (DBCStructure.h line 1159)
//! - `MAX_HOLIDAY_DURATIONS`  = 10  (DBCStructure.h line 1158)
//!
//! # End-time sourcing (CRITICAL)
//!
//! `SetHolidayEventTime` in C++ sets ONLY `Start`, `Length`, `Occurence`.
//! `End` is loaded from the `game_event` DB column `end_time` in `LoadGameEvents`
//! (a separate code path).  This Rust port follows the same split:
//! `resolve_holiday_event` takes the full `GameEventInput` row and uses
//! `row.end_time` for `End`.
//!
//! # DST caveat (Task 7 confirmed UTC — no DST)
//!
//! `set_holiday_event_time` and `unix_to_civil` both use a fixed `tz_offset_secs`.
//! Task 7 confirmed `server_tz_offset_secs = 0` (UTC) — no DST adjustment required.
//! The `// TODO(Task 7 confirmed UTC): DST` markers are kept as documentation breadcrumbs
//! for any future server that does observe DST.

use serde::{Deserialize, Deserializer};

use crate::holiday::{
    find_start_time_for_stage, get_darkmoon_faire_dates, get_packed_holiday_date,
    holiday_rules, HolidayCalculationType,
};
use crate::packed::{civil_to_unix, unix_to_civil, CivilDate};
use crate::schedule::{GameEventState, ResolvedEvent};

// ── C++ time constants (Common.h) ─────────────────────────────────────────────

const MINUTE: i64 = 60;
const HOUR: i64 = 3_600;
const WEEK: i64 = 604_800;
const YEAR: i64 = 31_536_000; // DAY * 365

/// `MAX_HOLIDAY_DATES` from DBCStructure.h line 1159.
pub const MAX_HOLIDAY_DATES: usize = 26;
/// `MAX_HOLIDAY_DURATIONS` from DBCStructure.h line 1158.
pub const MAX_HOLIDAY_DURATIONS: usize = 10;

// ── Input types (Task 11 will populate these from obs.game_events) ────────────

/// Row from the `game_event` DB table, as sourced by `obs.game_events`.
///
/// Task 11 will deserialise a live harness dump into this struct.
///
/// ## Null timestamps
///
/// Both `start_time` and `end_time` are nullable in the DB (holiday rows and some
/// periodic rows such as 97/98 have `end_time: null`).  They are carried as
/// `Option<i64>` to preserve the null/non-null distinction for callers.
///
/// **Null semantics (matching C++ `LoadGameEvents` / `GameEventMgr.cpp`):**
/// - `start_time = None` → effective start = 0  (C++ `Get<uint64>()` on null → 0;
///   no null-guard in `LoadGameEvents` line 357).  Holiday events have their Start
///   overwritten by `SetHolidayEventTime` anyway.
/// - `end_time = None` → effective end = `resolve_reference_unixtime + 63_072_000`
///   (C++ `LoadGameEvents` lines 359-360: `if (end_time IS NULL) endtime = curTime + 63072000`
///   where `63072000 = 730 * 86400` = exactly 2 years).
///
/// The caller (shadow.rs `compute_report`) applies these rules via
/// `effective_start` / `effective_end` before passing concrete values to the
/// resolvers.
#[derive(Debug, Clone)]
pub struct GameEventInput {
    pub entry: u16,
    /// `game_event.start_time` as Unix timestamp (seconds), or `None` if NULL in DB.
    pub start_time: Option<i64>,
    /// `game_event.end_time` as Unix timestamp (seconds), or `None` if NULL in DB.
    pub end_time: Option<i64>,
    /// `game_event.occurence` in minutes.
    pub occurence: i64,
    /// `game_event.length` in minutes.
    pub length: i64,
    /// `game_event.holiday` (holiday ID, 0 = no holiday).
    pub holiday: u32,
    /// `game_event.holidayStage` (1-indexed stage within the holiday; 0 = ignore).
    pub holiday_stage: u8,
    /// State machine state.
    pub state: GameEventState,
    /// Next scheduled start for world-phase events (Unix seconds).
    pub next_start: i64,
}

/// Deserialize helper: map an integer 0/1 to bool.
///
/// The `obs.game_events` harness payload sends `"looping": 0` or `"looping": 1`
/// (matching the DBC integer field) rather than a JSON bool.  This function lets
/// serde map it to Rust's `bool`.
fn deserialize_int_as_bool<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    let v = u8::deserialize(d)?;
    Ok(v != 0)
}

/// Synthetic representation of a `HolidaysEntry` DBC record.
///
/// Field semantics:
/// - `holiday_id`: numeric WoW `HolidayIds` value (from `SharedDefines.h`). Present in the
///   harness payload; needed by shadow diff CHECK 1 to map to a `HolidayRule`.
/// - `date[i]`: packed WoW date (`uint32`, same layout as `HolidaysEntry::Date[]`).
/// - `duration[i]`: stage duration in **hours** (matches `HolidaysEntry::Duration[]`).
/// - `calendar_filter_type`: −1=Yearly, 0=Weekly, 1=Defined-dates, 2=Looping.
/// - `looping`: mirrors `HolidaysEntry::Looping` (0/1 in DBC, bool here).
/// - `region`: `HolidaysEntry::Region`.
///
/// Lengths must be ≤ `MAX_HOLIDAY_DATES` / `MAX_HOLIDAY_DURATIONS` respectively.
///
/// The harness payload sends `looping` as an integer (0 or 1); the custom
/// `deserialize_with` maps it to `bool`.  The `date` and `duration` fields are fixed-
/// length arrays in the C++ DBC but arrive as JSON arrays; serde collects them
/// directly.
#[derive(Debug, Clone, Deserialize)]
pub struct HolidaysEntry {
    /// WoW holiday ID. Used by shadow diff CHECK 1 to look up the matching `HolidayRule`.
    pub holiday_id: u32,
    pub date: Vec<u32>,
    /// Stage durations in **hours**.
    pub duration: Vec<u32>,
    pub calendar_filter_type: i32,
    #[serde(deserialize_with = "deserialize_int_as_bool")]
    pub looping: bool,
    pub region: u32,
}

// `unix_to_civil` was moved to `packed.rs` (P.S. nit, Task 11) — calendar primitive
// belongs next to `CivilDate`. It is imported via `crate::packed::unix_to_civil`.

// ── set_holiday_event_time ────────────────────────────────────────────────────

/// Port of `GameEventMgr::SetHolidayEventTime` (C++ lines 1916–2006).
///
/// Fills `out_start`, `out_length`, `out_occurence` from the holiday DBC data.
/// Does NOT touch `End`.
///
/// Returns `false` if `holiday_stage == 0` (skip, per C++ line 1918) or if
/// `Date[0]` or `Duration[0]` are zero (invalid definitions, per C++ line 1923).
///
/// `resolve_ref` = server game time at load (= `curTime` in C++).
/// `tz_offset_secs` = server TZ offset in seconds (UTC → 0). // TODO(Task 7 confirmed UTC): DST
pub fn set_holiday_event_time(
    holiday_stage: u8,
    holiday: &HolidaysEntry,
    resolve_ref: i64,
    tz_offset_secs: i32,
) -> Option<(i64 /*start*/, i64 /*length_min*/, i64 /*occurence_min*/)> {
    // C++ line 1918: if (!event.HolidayStage) return
    if holiday_stage == 0 {
        return None;
    }

    // C++ line 1923: if (!holiday->Date[0] || !holiday->Duration[0])
    let date0 = holiday.date.first().copied().unwrap_or(0);
    let dur0 = holiday.duration.first().copied().unwrap_or(0);
    if date0 == 0 || dur0 == 0 {
        return None;
    }

    let stage_index = (holiday_stage - 1) as usize;

    // C++ line 1930: event.Length = holiday->Duration[stageIndex] * HOUR / MINUTE
    let stage_dur = holiday.duration.get(stage_index).copied().unwrap_or(0);
    let length_min: i64 = (stage_dur as i64) * HOUR / MINUTE;

    // C++ lines 1932-1936: stageOffset = Σ Duration[0..stageIndex] * HOUR
    let mut stage_offset: i64 = 0;
    for i in 0..stage_index {
        let dur = holiday.duration.get(i).copied().unwrap_or(0);
        stage_offset += (dur as i64) * HOUR;
    }

    // C++ lines 1938-1950: CalendarFilterType switch
    let mut occurence_min: i64 = 0; // will be set below

    match holiday.calendar_filter_type {
        -1 => {
            // Yearly
            occurence_min = YEAR / MINUTE;
        }
        0 => {
            // Weekly
            occurence_min = WEEK / MINUTE;
        }
        1 => {
            // Defined dates only (Darkmoon Faire) — occurence unchanged (0 here; caller keeps row value)
        }
        2 => {
            // Only used for looping events (Call to Arms) — occurence unchanged until Looping check
        }
        _ => {
            // Unknown filter type — leave as 0, matching C++ fall-through
        }
    }

    // C++ lines 1952-1959: if (holiday->Looping) { Occurence = Σ Duration[i] * HOUR / MINUTE }
    if holiday.looping {
        occurence_min = 0;
        for i in 0..MAX_HOLIDAY_DURATIONS {
            let dur = holiday.duration.get(i).copied().unwrap_or(0);
            if dur == 0 {
                break;
            }
            occurence_min += (dur as i64) * HOUR / MINUTE;
        }
    }

    // C++ line 1961: singleDate = ((holiday->Date[0] >> 24) & 0x1F) == 31
    let single_date = ((date0 >> 24) & 0x1F) == 31;

    let start: i64 = if !single_date {
        // C++ lines 1965-1973: FindStartTimeForStage path.
        // C++ line 1969: `if (start) event.Start = start` — C++ only overwrites Start when
        // FindStartTimeForStage returns non-zero.  When it returns 0 (all populated holiday
        // dates are fully in the past relative to curTime), C++ keeps the existing event.Start,
        // which is the `game_event.start_time` column value loaded in LoadGameEvents (line 357).
        // We return 0 here as a sentinel; resolve_holiday_event detects it and falls back to
        // row.start_time, mirroring the C++ "keep existing" behaviour.
        find_start_time_for_stage(
            &holiday.date,
            stage_offset,
            length_min as u32,
            resolve_ref,
            tz_offset_secs,
        )
    } else {
        // C++ lines 1975-2005: singleDate year-boundary.
        //
        // The C++ loops `for (i < MAX_HOLIDAY_DATES && holiday->Date[i])` but always
        // `break`s on the first non-zero date.  We use the first non-zero date directly,
        // which is semantically identical and avoids a never-loop lint.
        let found_start: i64 = if let Some(&date_packed) = holiday.date.iter().find(|&&d| d != 0) {
            // C++ line 1979: tm timeInfo = Acore::Time::TimeBreakdown(curTime)
            // = localtime of curTime (with tz).
            let cur_civil = unix_to_civil(resolve_ref, tz_offset_secs); // TODO(Task 7 confirmed UTC): DST

            // C++ line 1980: timeInfo.tm_year -= 1  (try last year first)
            let last_year = cur_civil.year - 1;

            // C++ lines 1982-1986: extract month/day/hour/min from packed date
            let mon0 = ((date_packed >> 20) & 0xF) as i32;
            let mday = (((date_packed >> 14) & 0x3F) + 1) as i32;
            let hour = ((date_packed >> 6) & 0x1F) as i32;
            let min_field = (date_packed & 0x3F) as i32;

            // C++ line 1987: timeInfo.tm_isdst = -1 (no explicit DST)
            // We use civil_to_unix with fixed offset, which ignores DST.

            // Try last year
            let last_year_civil = CivilDate {
                year: last_year,
                mon0,
                mday,
                hour,
                min: min_field,
                wday: 0,
            };
            // C++ line 1990: startTime = mktime(&timeInfo)
            let start_time = civil_to_unix(&last_year_civil, tz_offset_secs); // TODO(Task 7 confirmed UTC): DST

            // C++ line 1991: if (curTime < startTime + stageOffset + event.Length * MINUTE)
            if resolve_ref < start_time + stage_offset + length_min * MINUTE {
                // C++ line 1993: event.Start = startTime + stageOffset
                start_time + stage_offset
            } else {
                // C++ lines 1998-2003: this-year fallback
                // tmCopy.tm_year = year (this year from curTime)
                let this_year = cur_civil.year;
                let this_year_civil = CivilDate {
                    year: this_year,
                    mon0,
                    mday,
                    hour,
                    min: min_field,
                    wday: 0,
                };
                // C++ line 2002: event.Start = mktime(&tmCopy) + stageOffset
                civil_to_unix(&this_year_civil, tz_offset_secs) + stage_offset // TODO(Task 7 confirmed UTC): DST
            }
        } else {
            0 // no non-zero date found
        };
        found_start
    };

    Some((start, length_min, occurence_min))
}

// ── generate_dynamic_dates ────────────────────────────────────────────────────

/// Port of `GameEventMgr::LoadHolidayDates` Step 1 (C++ lines 1072–1150).
///
/// Fills `entry.date` (and potentially `entry.duration[0]` for Darkmoon)
/// from the calculator rules, relative to `gen_year`.
///
/// - **Darkmoon Faire** (`DARKMOON_FAIRE`): calls `get_darkmoon_faire_dates`
///   with `start_year = gen_year − 1`, 4 years, appending up to
///   `MAX_HOLIDAY_DATES` dates.  Also sets `Duration[0] = 168` if unset.
/// - **All others**: iterates `yearOffset ∈ [−1, 2]`, skips years > 2030.
///   `dateId = (yearOffset + 1) as u8`.
///   `Date[dateId] = get_packed_holiday_date(holiday_id, gen_year + yearOffset)`.
///
/// `holiday_id` is used to look up the rule; if no rule is found, `date` is
/// left empty (the C++ LOG_INFO path — no panic).
pub fn generate_dynamic_dates(
    holiday_id: u32,
    gen_year: i32,
    entry: &mut HolidaysEntry,
) {
    // Find the rule for this holiday_id.
    let rule = holiday_rules().iter().find(|r| r.holiday_id == holiday_id);
    let rule = match rule {
        Some(r) => r,
        None => return, // C++ line 1094: LOG_INFO + continue
    };

    if rule.calc_type == HolidayCalculationType::DarkmoonFaire {
        // C++ lines 1099-1122: Darkmoon Faire special path
        let location_offset = rule.month;
        let dates = get_darkmoon_faire_dates(location_offset, gen_year - 1, 4, rule.offset);

        // C++ lines 1104-1112: fill Date[dateId++] up to MAX_HOLIDAY_DATES.
        // Intentional divergence from C++: C++ overwrites Date[dateId] in-place starting
        // at 0 on a pre-allocated array; here we clear + push, which is semantically
        // equivalent because the indices are always filled from 0 upward in order.
        entry.date.clear();
        for packed in dates.into_iter().take(MAX_HOLIDAY_DATES) {
            entry.date.push(packed);
        }

        // C++ lines 1115-1116: if (!entry->Duration[0]) entry->Duration[0] = 168
        if entry.duration.is_empty() || entry.duration[0] == 0 {
            if entry.duration.is_empty() {
                entry.duration.push(168);
            } else {
                entry.duration[0] = 168;
            }
        }
    } else {
        // C++ lines 1126-1145: per-year generation
        // Ensure date vec is large enough (indices 0..3 = dateId for yearOffset −1..+2)
        // dateId = (yearOffset + 1) as u8 → indices 0..=3
        while entry.date.len() < MAX_HOLIDAY_DATES {
            entry.date.push(0);
        }

        for year_offset in -1i32..=2 {
            let year = gen_year + year_offset;
            if year > 2030 {
                break;
            }
            let date_id = (year_offset + 1) as usize; // 0, 1, 2, 3
            if date_id >= MAX_HOLIDAY_DATES {
                break;
            }
            let packed = get_packed_holiday_date(rule.holiday_id, year);
            entry.date[date_id] = packed;
        }
    }
}

// ── apply_start_time_override ─────────────────────────────────────────────────

/// Port of `GameEventMgr::LoadHolidayDates` Step 2 (C++ lines 1152–1190).
///
/// Overrides `date[0]` from `game_event_start_time_unix` IF its year is in
/// `[gen_year, 2030]`.
///
/// Packs `(yearOffset<<24)|(month<<20)|(day<<14)|(weekday<<11)` — NO hour/min,
/// exactly matching C++ line 1186.
///
/// **Upstream AC note:** this implementation faithfully mirrors upstream AzerothCore —
/// `game_event.start_time` is the override source (C++ LoadHolidayDates line 1170,
/// `fields[1].Get<uint64>()`).  This is NOT a fork divergence; Task 7 live data confirmed.
pub fn apply_start_time_override(
    date: &mut Vec<u32>,
    game_event_start_time_unix: i64,
    gen_year: i32,
    tz_offset_secs: i32,
) {
    // C++ line 1170: time_t startTime = fields[1].Get<uint64>()
    if game_event_start_time_unix == 0 {
        return;
    }

    // C++ line 1174: Acore::Time::TimeBreakdown(startTime)
    let time_info = unix_to_civil(game_event_start_time_unix, tz_offset_secs); // TODO(Task 7 confirmed UTC): DST

    // C++ line 1176: int year = timeInfo.tm_year + 1900
    let year = time_info.year;

    // C++ line 1178: if (year < currentYear || year > 2030) continue
    if year < gen_year || year > 2030 {
        return;
    }

    // C++ line 1182: uint32_t yearOffset = year - 2000
    let year_offset = (year - 2000) as u32;
    // C++ line 1183: uint32_t month = timeInfo.tm_mon  (0-indexed)
    let month = time_info.mon0 as u32;
    // C++ line 1184: uint32_t day = timeInfo.tm_mday - 1  (0-indexed)
    let day = (time_info.mday - 1) as u32;
    // C++ line 1185: uint32_t weekday = timeInfo.tm_wday
    let weekday = time_info.wday as u32;

    // C++ line 1186: (yearOffset<<24)|(month<<20)|(day<<14)|(weekday<<11)
    // NO hour/min bits — intentional, matching C++ exactly.
    let packed = (year_offset << 24) | (month << 20) | (day << 14) | (weekday << 11);

    // Override Date[0]
    if date.is_empty() {
        date.push(packed);
    } else {
        date[0] = packed;
    }
}

// ── Public entry points ────────────────────────────────────────────────────────

/// Compute the effective start time from a possibly-null DB value.
///
/// C++ `LoadGameEvents` line 357: `event.Start = fields[1].Get<uint64>()`.
/// `Get<uint64>()` on a NULL field returns 0 — no null-guard.  Holiday events
/// have Start overwritten by `SetHolidayEventTime` anyway; for periodic events
/// with null start, 0 persists.
///
/// `63_072_000 = 730 * 86_400` (exactly 2 years, matching C++ lines 359-360).
pub fn effective_start(raw: Option<i64>) -> i64 {
    raw.unwrap_or(0)
}

/// Compute the effective end time from a possibly-null DB value.
///
/// C++ `LoadGameEvents` lines 359-360:
/// ```cpp
/// if (end_time IS NULL)
///     endtime = GameTime::GetGameTime().count() + 63072000;  // +2 years
/// ```
/// `63_072_000 = 730 * 86_400` (exactly 2 years — do not approximate).
pub fn effective_end(raw: Option<i64>, resolve_ref: i64) -> i64 {
    match raw {
        Some(t) => t,
        None => resolve_ref + 63_072_000,
    }
}

/// Resolve a periodic (non-holiday) game event from its DB row.
///
/// Takes pre-materialized `start` and `end` (callers apply `effective_start` /
/// `effective_end` before calling this function).  No holiday math is performed.
/// This covers `holiday == 0` events.
pub fn resolve_periodic_event(row: &GameEventInput, start: i64, end: i64) -> ResolvedEvent {
    ResolvedEvent {
        entry: row.entry,
        start,
        end,
        occurence: row.occurence,
        length: row.length,
        state: row.state,
        prerequisites: vec![],
        next_start: row.next_start,
    }
}

/// Resolve a holiday-backed game event.
///
/// - `Start`, `Length`, `Occurence`: from `set_holiday_event_time`.
/// - `End`: from the pre-materialized `end` argument (callers apply `effective_end`).
/// - `state`, `next_start`: from `row`.
///
/// If `set_holiday_event_time` returns `None` (invalid holiday or stage==0),
/// the original row values are used unchanged (safe fallback matching the C++
/// "return early" behaviour that leaves the event data unmodified).
pub fn resolve_holiday_event(
    row: &GameEventInput,
    start: i64,
    end: i64,
    holiday: &HolidaysEntry,
    resolve_ref: i64,
    tz_offset_secs: i32,
) -> ResolvedEvent {
    match set_holiday_event_time(row.holiday_stage, holiday, resolve_ref, tz_offset_secs) {
        Some((holiday_start, length_min, occurence_min)) => ResolvedEvent {
            entry: row.entry,
            // C++ `if (start) event.Start = start` in SetHolidayEventTime line 1969: when
            // FindStartTimeForStage returns 0 (all holiday dates are in the past), C++ keeps
            // the existing event.Start (= game_event.start_time from LoadGameEvents line 357).
            // A genuine resolved start is ~1.7e9 (a ~2026 unixtime), so 0 is a safe sentinel.
            start: if holiday_start != 0 { holiday_start } else { start },
            end, // End comes from game_event.end_time (effective_end applied by caller), NOT holiday math
            occurence: if occurence_min == 0 {
                // filter_type==1 or filter_type==2 and !looping: keep row value
                row.occurence
            } else {
                occurence_min
            },
            length: length_min,
            state: row.state,
            prerequisites: vec![],
            next_start: row.next_start,
        },
        None => {
            // Holiday stage == 0 or invalid holiday — return row values unchanged.
            ResolvedEvent {
                entry: row.entry,
                start,
                end,
                occurence: row.occurence,
                length: row.length,
                state: row.state,
                prerequisites: vec![],
                next_start: row.next_start,
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::holiday::get_packed_holiday_date;
    use crate::packed::{normalize_date, pack_date};
    use crate::schedule::{is_active, GameEventState};

    // ── Helpers ──────────────────────────────────────────────────────────────

    /// Build a minimal `GameEventInput` with Normal state for testing.
    /// `start` and `end` are `Some(i64)` — use `None` to test null-timestamp paths.
    fn make_row(entry: u16, start: i64, end: i64, occ: i64, len: i64) -> GameEventInput {
        GameEventInput {
            entry,
            start_time: Some(start),
            end_time: Some(end),
            occurence: occ,
            length: len,
            holiday: 0,
            holiday_stage: 0,
            state: GameEventState::Normal,
            next_start: 0,
        }
    }

    /// Build a `HolidaysEntry` with the given filter / looping / dates / durations.
    /// `holiday_id = 0` is a sentinel for tests that don't exercise the id.
    fn make_holiday(
        dates: Vec<u32>,
        durations: Vec<u32>,
        filter: i32,
        looping: bool,
    ) -> HolidaysEntry {
        HolidaysEntry {
            holiday_id: 0,
            date: dates,
            duration: durations,
            calendar_filter_type: filter,
            looping,
            region: 0,
        }
    }

    // ── unix_to_civil round-trip ─────────────────────────────────────────────

    #[test]
    fn unix_to_civil_epoch() {
        // Unix 0 = 1970-01-01 00:00:00 UTC
        let d = unix_to_civil(0, 0);
        assert_eq!((d.year, d.mon0 + 1, d.mday, d.hour, d.min), (1970, 1, 1, 0, 0));
    }

    #[test]
    fn unix_to_civil_known_date() {
        // 2026-05-30 12:34:00 UTC = ?
        // Using civil_to_unix as oracle: build the civil date, get unix, then convert back.
        let civil = CivilDate { year: 2026, mon0: 4, mday: 30, hour: 12, min: 34, wday: 0 };
        let unix = civil_to_unix(&civil, 0);
        let back = unix_to_civil(unix, 0);
        assert_eq!(back.year, 2026);
        assert_eq!(back.mon0, 4); // May
        assert_eq!(back.mday, 30);
        assert_eq!(back.hour, 12);
        assert_eq!(back.min, 34);
    }

    #[test]
    fn unix_to_civil_roundtrip_property() {
        // civil_to_unix then unix_to_civil must be identity for a range of dates.
        let test_dates: &[(i32, i32, i32)] = &[
            (2000, 1, 1),
            (2010, 6, 15),
            (2020, 2, 29), // leap
            (2025, 10, 18), // Hallow's End
            (2030, 12, 31),
        ];
        for &(y, m, d) in test_dates {
            let mut civil = CivilDate { year: y, mon0: m - 1, mday: d, hour: 0, min: 0, wday: 0 };
            normalize_date(&mut civil);
            let unix = civil_to_unix(&civil, 0);
            let back = unix_to_civil(unix, 0);
            assert_eq!(
                (back.year, back.mon0 + 1, back.mday),
                (y, m, d),
                "roundtrip failed for {y}-{m:02}-{d:02}"
            );
        }
    }

    // ── effective_start / effective_end ──────────────────────────────────────

    #[test]
    fn effective_start_some_passes_through() {
        assert_eq!(effective_start(Some(1_000_000)), 1_000_000);
    }

    #[test]
    fn effective_start_none_is_zero() {
        assert_eq!(effective_start(None), 0);
    }

    #[test]
    fn effective_end_some_passes_through() {
        assert_eq!(effective_end(Some(9_999_999), 1_780_000_000), 9_999_999);
    }

    #[test]
    fn effective_end_none_is_resolve_ref_plus_63072000() {
        // 63_072_000 = 730 * 86_400 (exactly 2 years — C++ LoadGameEvents lines 359-360)
        let resolve_ref = 1_780_000_000i64;
        let expected = resolve_ref + 63_072_000;
        assert_eq!(effective_end(None, resolve_ref), expected);
        // Verify the constant is exactly 730 * 86_400
        assert_eq!(63_072_000i64, 730 * 86_400, "63_072_000 must equal 730*86400 exactly");
    }

    #[test]
    fn effective_end_none_two_separate_resolve_refs() {
        // The default depends on resolve_ref — verify two different refs give two different ends
        let r1 = 1_780_000_000i64;
        let r2 = 1_790_000_000i64;
        assert_eq!(effective_end(None, r1), r1 + 63_072_000);
        assert_eq!(effective_end(None, r2), r2 + 63_072_000);
        assert_ne!(effective_end(None, r1), effective_end(None, r2));
    }

    // ── resolve_periodic_event ────────────────────────────────────────────────

    #[test]
    fn periodic_event_round_trips_row_columns() {
        let row = make_row(7, 1_000_000, 2_000_000, 10080, 60);
        let s = effective_start(row.start_time);
        let e = effective_end(row.end_time, 1_780_000_000);
        let resolved = resolve_periodic_event(&row, s, e);
        assert_eq!(resolved.entry, 7);
        assert_eq!(resolved.start, 1_000_000);
        assert_eq!(resolved.end, 2_000_000);
        assert_eq!(resolved.occurence, 10080);
        assert_eq!(resolved.length, 60);
        assert_eq!(resolved.state, GameEventState::Normal);
        assert_eq!(resolved.next_start, 0);
    }

    #[test]
    fn periodic_event_null_end_time_gets_default() {
        // A row with end_time=None should resolve to resolve_ref + 63_072_000
        let mut row = make_row(97, 0, 0, 525_600, 20_160);
        row.start_time = None;
        row.end_time = None;
        let resolve_ref = 1_780_244_906i64;
        let s = effective_start(row.start_time);
        let e = effective_end(row.end_time, resolve_ref);
        let resolved = resolve_periodic_event(&row, s, e);
        assert_eq!(resolved.start, 0, "null start → effective start = 0");
        assert_eq!(
            resolved.end,
            resolve_ref + 63_072_000,
            "null end → resolve_ref + 63_072_000"
        );
    }

    // ── set_holiday_event_time: CalendarFilterType = -1 (Yearly) ─────────────

    #[test]
    fn yearly_holiday_occurence_is_year_over_minute() {
        // filter=-1 → occurence = YEAR/MINUTE = 31536000/60 = 525600
        // Build a packed date for a future event so resolve_ref is before it.
        // Use Hallow's End 2025 (Oct 18): year_offset=25, mon0=9, mday=18, wday=6(Sat)
        let mut d = CivilDate { year: 2025, mon0: 9, mday: 18, hour: 0, min: 0, wday: 0 };
        normalize_date(&mut d);
        let packed = pack_date(&d);

        let holiday = make_holiday(vec![packed], vec![336], -1, false); // 336h = 14 days
        // resolve_ref = 2025-10-01 (before the event)
        let ref_civil = CivilDate { year: 2025, mon0: 9, mday: 1, hour: 0, min: 0, wday: 0 };
        let resolve_ref = civil_to_unix(&ref_civil, 0);

        let result = set_holiday_event_time(1, &holiday, resolve_ref, 0);
        let (_, _, occurence_min) = result.expect("should resolve");
        assert_eq!(occurence_min, YEAR / MINUTE, "yearly: occurence = YEAR/MINUTE");
    }

    // ── set_holiday_event_time: CalendarFilterType = 0 (Weekly) ──────────────

    #[test]
    fn weekly_holiday_occurence_is_week_over_minute() {
        // filter=0 → occurence = WEEK/MINUTE = 604800/60 = 10080
        let mut d = CivilDate { year: 2025, mon0: 9, mday: 18, hour: 0, min: 0, wday: 0 };
        normalize_date(&mut d);
        let packed = pack_date(&d);

        let holiday = make_holiday(vec![packed], vec![72], 0, false); // 72h = 3 days
        let ref_civil = CivilDate { year: 2025, mon0: 9, mday: 1, hour: 0, min: 0, wday: 0 };
        let resolve_ref = civil_to_unix(&ref_civil, 0);

        let result = set_holiday_event_time(1, &holiday, resolve_ref, 0);
        let (_, length_min, occurence_min) = result.expect("should resolve");
        assert_eq!(occurence_min, WEEK / MINUTE, "weekly: occurence = WEEK/MINUTE");
        assert_eq!(length_min, 72 * HOUR / MINUTE, "weekly: length = duration*HOUR/MINUTE");
    }

    // ── set_holiday_event_time: filter=1 (Defined dates, e.g. Darkmoon) ──────

    #[test]
    fn darkmoon_style_filter1_occurence_unchanged_uses_find_start() {
        // filter=1 → occurence stays 0 (caller keeps row value).
        // Darkmoon: Duration[0]=168 (7 days). Use a future packed date.
        // Use Darkmoon Elwynn 2026-03-01 (first Sunday of March 2026).
        let holiday_id = 374u32; // Darkmoon Elwynn
        let packed = get_packed_holiday_date(holiday_id, 2026); // first Sunday of March 2026

        // resolve_ref = 2026-02-15 (before the event)
        let ref_civil = CivilDate { year: 2026, mon0: 1, mday: 15, hour: 0, min: 0, wday: 0 };
        let resolve_ref = civil_to_unix(&ref_civil, 0);

        let holiday = make_holiday(vec![packed, 0], vec![168], 1, false);
        let result = set_holiday_event_time(1, &holiday, resolve_ref, 0);
        let (start, length_min, occurence_min) = result.expect("should resolve");

        // occurence == 0: caller keeps row value (filter_type==1 is "defined dates", no math)
        assert_eq!(occurence_min, 0, "filter=1: occurence unchanged (0)");
        assert_eq!(length_min, 168 * HOUR / MINUTE, "filter=1: length = 168*HOUR/MINUTE");
        // start should be > resolve_ref (we found a future event)
        assert!(start > resolve_ref, "start={start} must be > resolve_ref={resolve_ref}");
    }

    // ── set_holiday_event_time: Looping (Σ Duration) ──────────────────────────

    #[test]
    fn looping_holiday_occurence_is_sum_of_durations() {
        // Looping=true, Duration=[24, 48, 0, ...] → occurence = (24+48)*HOUR/MINUTE = 4320
        let mut d = CivilDate { year: 2025, mon0: 9, mday: 18, hour: 0, min: 0, wday: 0 };
        normalize_date(&mut d);
        let packed = pack_date(&d);

        let holiday = make_holiday(vec![packed], vec![24, 48], 2, true);
        let ref_civil = CivilDate { year: 2025, mon0: 9, mday: 1, hour: 0, min: 0, wday: 0 };
        let resolve_ref = civil_to_unix(&ref_civil, 0);

        let result = set_holiday_event_time(1, &holiday, resolve_ref, 0);
        let (_, _, occurence_min) = result.expect("should resolve");
        let expected = (24i64 + 48) * HOUR / MINUTE;
        assert_eq!(occurence_min, expected, "looping: occurence = sum durations * HOUR/MINUTE");
    }

    // ── set_holiday_event_time: holiday_stage == 0 returns None ──────────────

    #[test]
    fn stage_zero_returns_none() {
        let d = CivilDate { year: 2025, mon0: 9, mday: 18, ..Default::default() };
        let packed = pack_date(&d);
        let holiday = make_holiday(vec![packed], vec![168], -1, false);
        assert!(
            set_holiday_event_time(0, &holiday, 0, 0).is_none(),
            "stage==0 must return None"
        );
    }

    // ── set_holiday_event_time: invalid holiday (empty dates) returns None ────

    #[test]
    fn invalid_holiday_empty_dates_returns_none() {
        let holiday = make_holiday(vec![], vec![], -1, false);
        assert!(
            set_holiday_event_time(1, &holiday, 0, 0).is_none(),
            "empty dates must return None"
        );
    }

    // ── generate_dynamic_dates: Hallow's End (fixed, holiday_id=324) ─────────

    #[test]
    fn generate_dynamic_dates_fixed_holiday_hallowsend() {
        // Hallow's End = holiday_id 324, FixedDate Oct 18.
        // generate for gen_year=2026: dateId 0=2025, 1=2026, 2=2027, 3=2028
        let mut entry = HolidaysEntry {
            holiday_id: 324,
            date: vec![],
            duration: vec![336], // 14 days
            calendar_filter_type: -1,
            looping: false,
            region: 0,
        };
        generate_dynamic_dates(324, 2026, &mut entry);

        // dateId = (yearOffset+1): yearOffset=-1→id=0(year=2025), -1+2=id1(2026), etc.
        for year_offset in -1i32..=2 {
            let year = 2026 + year_offset;
            if year > 2030 {
                break;
            }
            let date_id = (year_offset + 1) as usize;
            let expected = get_packed_holiday_date(324, year);
            assert_ne!(expected, 0, "packed date for year {year} should be non-zero");
            assert_eq!(
                entry.date[date_id], expected,
                "Hallow's End Date[{date_id}] for year {year}"
            );
        }
    }

    // ── generate_dynamic_dates: Darkmoon Elwynn (holiday_id=374) ─────────────

    #[test]
    fn generate_dynamic_dates_darkmoon_fills_dates_and_duration() {
        let mut entry = HolidaysEntry {
            holiday_id: 374,
            date: vec![],
            duration: vec![0], // unset → should be set to 168
            calendar_filter_type: 1,
            looping: false,
            region: 0,
        };
        generate_dynamic_dates(374, 2026, &mut entry);

        // Duration[0] should be set to 168 (was 0)
        assert_eq!(entry.duration[0], 168, "Darkmoon duration should be 168");

        // Should have multiple entries (4 years × 4 months each = up to 16)
        assert!(!entry.date.is_empty(), "Darkmoon dates should be populated");

        // All non-zero entries should be valid packed dates
        for &d in &entry.date {
            if d != 0 {
                // year_offset should be reasonable: bits [24-28] => year = 2000 + offset
                let year_offset = (d >> 24) & 0x1F;
                assert!(year_offset <= 30, "year_offset {year_offset} <= 30 (<=2030)");
            }
        }
    }

    // ── generate_dynamic_dates: unknown holiday_id → no-op ────────────────────

    #[test]
    fn generate_dynamic_dates_unknown_id_is_noop() {
        let mut entry = HolidaysEntry {
            holiday_id: 99999,
            date: vec![],
            duration: vec![],
            calendar_filter_type: -1,
            looping: false,
            region: 0,
        };
        generate_dynamic_dates(99999, 2026, &mut entry);
        assert!(entry.date.is_empty(), "unknown holiday_id: dates should remain empty");
    }

    // ── apply_start_time_override ─────────────────────────────────────────────

    #[test]
    fn override_in_range_sets_date0() {
        // game_event.start_time = 2026-10-18 (year=2026 >= gen_year=2026, <=2030)
        let oct18_2026 = civil_to_unix(
            &CivilDate { year: 2026, mon0: 9, mday: 18, hour: 0, min: 0, wday: 0 },
            0,
        );
        let mut date = vec![0u32; 4];
        apply_start_time_override(&mut date, oct18_2026, 2026, 0);

        // Verify date[0] was set
        assert_ne!(date[0], 0, "date[0] should be overridden");

        // Verify the packed fields (yearOffset=26, mon0=9=Oct, day=17=0-indexed 18)
        let year_offset = (date[0] >> 24) & 0x1F;
        let month = (date[0] >> 20) & 0xF;
        let day_0idx = (date[0] >> 14) & 0x3F;
        assert_eq!(year_offset, 26, "year_offset should be 2026-2000=26");
        assert_eq!(month, 9, "month should be 9 (October 0-indexed)");
        assert_eq!(day_0idx, 17, "day (0-indexed) should be 17 (18-1)");

        // No hour/min bits — per C++ line 1186 exactly
        let hour_bits = (date[0] >> 6) & 0x1F;
        let min_bits = date[0] & 0x3F;
        assert_eq!(hour_bits, 0, "override must NOT set hour bits (C++ line 1186)");
        assert_eq!(min_bits, 0, "override must NOT set min bits (C++ line 1186)");
    }

    #[test]
    fn override_out_of_range_year_before_gen_year_ignored() {
        // start_time year = 2025 < gen_year = 2026 → ignored
        let past = civil_to_unix(
            &CivilDate { year: 2025, mon0: 9, mday: 18, hour: 0, min: 0, wday: 0 },
            0,
        );
        let mut date = vec![0xDEADBEEFu32]; // sentinel
        apply_start_time_override(&mut date, past, 2026, 0);
        assert_eq!(date[0], 0xDEADBEEF, "past year should be ignored — date[0] unchanged");
    }

    #[test]
    fn override_out_of_range_year_after_2030_ignored() {
        // start_time year = 2031 > 2030 → ignored
        let future = civil_to_unix(
            &CivilDate { year: 2031, mon0: 9, mday: 18, hour: 0, min: 0, wday: 0 },
            0,
        );
        let mut date = vec![0xDEADBEEFu32];
        apply_start_time_override(&mut date, future, 2026, 0);
        assert_eq!(date[0], 0xDEADBEEF, "year > 2030 should be ignored — date[0] unchanged");
    }

    #[test]
    fn override_zero_unix_time_ignored() {
        let mut date = vec![0xDEADBEEFu32];
        apply_start_time_override(&mut date, 0, 2026, 0);
        assert_eq!(date[0], 0xDEADBEEF, "zero unix time should be ignored");
    }

    // ── resolve_holiday_event: End from row ───────────────────────────────────

    #[test]
    fn holiday_event_end_comes_from_row() {
        // Weekly holiday, stage=1
        let mut d = CivilDate { year: 2025, mon0: 9, mday: 18, hour: 0, min: 0, wday: 0 };
        normalize_date(&mut d);
        let packed = pack_date(&d);
        let holiday = make_holiday(vec![packed], vec![72], 0, false);

        let row = GameEventInput {
            entry: 5,
            start_time: Some(0),
            end_time: Some(9_999_999), // this should survive into resolved.end
            occurence: 10080,
            length: 0,
            holiday: 42,
            holiday_stage: 1,
            state: GameEventState::Normal,
            next_start: 0,
        };
        let ref_civil = CivilDate { year: 2025, mon0: 9, mday: 1, hour: 0, min: 0, wday: 0 };
        let resolve_ref = civil_to_unix(&ref_civil, 0);

        let s = effective_start(row.start_time);
        let e = effective_end(row.end_time, resolve_ref);
        let resolved = resolve_holiday_event(&row, s, e, &holiday, resolve_ref, 0);
        assert_eq!(resolved.end, 9_999_999, "End must come from row.end_time, not holiday math");
        assert_eq!(resolved.occurence, WEEK / MINUTE, "Weekly: occurence = WEEK/MINUTE");
        assert_eq!(resolved.length, 72 * HOUR / MINUTE);
    }

    #[test]
    fn holiday_event_null_end_time_gets_resolve_ref_plus_two_years() {
        // A holiday row with end_time=None → effective end = resolve_ref + 63_072_000
        let mut d = CivilDate { year: 2025, mon0: 9, mday: 18, hour: 0, min: 0, wday: 0 };
        normalize_date(&mut d);
        let packed = pack_date(&d);
        let holiday = make_holiday(vec![packed], vec![72], 0, false);

        let row = GameEventInput {
            entry: 1,
            start_time: None,  // null start
            end_time: None,    // null end — should get resolve_ref + 63_072_000
            occurence: 10080,
            length: 0,
            holiday: 341,
            holiday_stage: 1,
            state: GameEventState::Normal,
            next_start: 0,
        };
        let ref_civil = CivilDate { year: 2026, mon0: 4, mday: 31, hour: 0, min: 0, wday: 0 };
        let resolve_ref = civil_to_unix(&ref_civil, 0);

        let s = effective_start(row.start_time);
        let e = effective_end(row.end_time, resolve_ref);

        assert_eq!(e, resolve_ref + 63_072_000, "null end_time → resolve_ref + 63_072_000");

        let resolved = resolve_holiday_event(&row, s, e, &holiday, resolve_ref, 0);
        assert_eq!(
            resolved.end,
            resolve_ref + 63_072_000,
            "resolved end must be resolve_ref + 63_072_000 for null end_time"
        );
    }

    // ── forward-date probes: is_active window ─────────────────────────────────

    #[test]
    fn forward_date_probes_active_window() {
        // Build a resolved weekly event: start at T, length=72h, occurence=WEEK.
        // Choose a known start time so we can probe precisely.
        // T = 2026-06-01 00:00:00 UTC
        let t_civil = CivilDate { year: 2026, mon0: 5, mday: 1, hour: 0, min: 0, wday: 0 };
        let t_start = civil_to_unix(&t_civil, 0);
        let length_min = 72i64 * HOUR / MINUTE; // 72h → 4320 min
        let occurence_min = WEEK / MINUTE; // 10080 min

        // End: T + 30 days (well beyond one window — this is just the overall event range)
        let t_end = t_start + 30 * 24 * 3600;

        let event = ResolvedEvent {
            entry: 1,
            start: t_start,
            end: t_end,
            occurence: occurence_min,
            length: length_min,
            state: GameEventState::Normal,
            prerequisites: vec![],
            next_start: 0,
        };

        // window-open: one second after start (strict < on start)
        let window_open = t_start + 1;
        assert!(
            is_active(&event, window_open, |_| None),
            "window-open: should be active at t_start+1"
        );

        // mid-window: 36h in (half of 72h window)
        let mid_window = t_start + 36 * 3600;
        assert!(
            is_active(&event, mid_window, |_| None),
            "mid-window: should be active at t_start+36h"
        );

        // window-close: exactly at start + length (out of window — strict <)
        let window_close = t_start + length_min * MINUTE;
        assert!(
            !is_active(&event, window_close, |_| None),
            "window-close: should be inactive at exactly t_start+length"
        );

        // next occurrence: t_start + WEEK + 1
        let next_occ = t_start + WEEK + 1;
        assert!(
            is_active(&event, next_occ, |_| None),
            "next-occurrence: should be active at t_start+WEEK+1"
        );
    }

    #[test]
    fn forward_date_probes_between_occurrences_inactive() {
        let t_civil = CivilDate { year: 2026, mon0: 5, mday: 1, hour: 0, min: 0, wday: 0 };
        let t_start = civil_to_unix(&t_civil, 0);
        let length_min = 72i64 * HOUR / MINUTE; // 72h
        let occurence_min = WEEK / MINUTE; // 1 week

        let t_end = t_start + 60 * 24 * 3600;

        let event = ResolvedEvent {
            entry: 2,
            start: t_start,
            end: t_end,
            occurence: occurence_min,
            length: length_min,
            state: GameEventState::Normal,
            prerequisites: vec![],
            next_start: 0,
        };

        // Gap: between end-of-window-1 (t_start + 72h) and start-of-window-2 (t_start + WEEK)
        let gap_instant = t_start + 4 * 24 * 3600; // 4 days in (after 72h window, before 1wk)
        assert!(
            !is_active(&event, gap_instant, |_| None),
            "gap: should be inactive between occurrences"
        );
    }

    // ── singleDate path: year-boundary ────────────────────────────────────────

    #[test]
    fn single_date_path_picks_last_year_when_before_window_end() {
        // A singleDate holiday has year_offset=31 in Date[0] bits [24-28].
        // We simulate a fixed-year holiday (year_offset=31 marker).
        // Build a packed date with year_offset=31, month=9(Oct), day=17(0-idx=18).
        // Stage: length=336h (14 days), stage_offset=0, filter=-1.
        //
        // "last year" scenario: resolve_ref is in October 2025, BEFORE window_end.
        // We expect Start = mktime(last_year=2024 Oct 18) + 0 = 2024-10-18.

        // Encode singleDate marker: year_offset = 31
        let year_offset: u32 = 31;
        let mon0: u32 = 9;        // October
        let day0: u32 = 17;       // mday-1 = 18-1
        let weekday: u32 = 0;     // placeholder
        let packed = (year_offset << 24) | (mon0 << 20) | (day0 << 14) | (weekday << 11);

        // Duration[0]=336h=14 days; filter=-1 (yearly)
        let holiday = make_holiday(vec![packed], vec![336], -1, false);

        // resolve_ref = 2025-10-25 (within the 14-day window starting 2024-10-18)
        // "last year" = 2024 since we try cur_year-1 first.
        // startTime(last_year=2024, Oct 18) = 2024-10-18 UTC
        // Window: [2024-10-18 .. 2024-10-18 + 14*24*3600 = 2024-11-01)
        // resolve_ref 2025-10-25 is NOT in that window — let's pick an earlier ref.
        // Actually use resolve_ref = 2024-10-20 (within the 2024 window).
        let ref_civil = CivilDate { year: 2024, mon0: 9, mday: 20, hour: 0, min: 0, wday: 0 };
        let resolve_ref = civil_to_unix(&ref_civil, 0);

        // Expected start: last year (cur=2024 → last_year = 2023), Oct 18
        // Wait — cur_civil.year = 2024, so last_year = 2023.
        // startTime = 2023-10-18 UTC
        // check: resolve_ref(2024-10-20) < startTime(2023-10-18) + 0 + 336*HOUR?
        // 2023-10-18 + 14 days = 2023-11-01; 2024-10-20 is NOT < 2023-11-01.
        // So C++ falls through to the "else" branch: this_year = 2024, Start = 2024-10-18.
        let expected_civil = CivilDate { year: 2024, mon0: 9, mday: 18, hour: 0, min: 0, wday: 0 };
        let expected_start = civil_to_unix(&expected_civil, 0);

        let result = set_holiday_event_time(1, &holiday, resolve_ref, 0);
        let (start, _, _) = result.expect("should resolve");
        assert_eq!(start, expected_start, "singleDate: should pick this_year=2024 for Oct 18");
    }

    #[test]
    fn single_date_path_last_year_when_within_window() {
        // If resolve_ref is early enough that last-year's window includes it,
        // Start = last_year's date + stageOffset.
        // Use filter=-1, Duration[0]=8760h (one year, unrealistically large to test the branch).
        // Actually use 8760 hours = 365 days = entire year window.
        // cur_year = 2026, last_year = 2025.
        // startTime(2025-10-18) + 365*24*3600 = 2026-10-18
        // resolve_ref = 2026-01-01 < 2026-10-18 → branch taken → Start = 2025-10-18.

        let year_offset: u32 = 31; // singleDate marker
        let mon0: u32 = 9;         // October
        let day0: u32 = 17;        // 0-indexed mday=18
        let weekday: u32 = 5;      // Saturday
        let packed = (year_offset << 24) | (mon0 << 20) | (day0 << 14) | (weekday << 11);

        let holiday = make_holiday(vec![packed], vec![8760], -1, false); // 8760h = 365 days

        // resolve_ref = 2026-01-01
        let ref_civil = CivilDate { year: 2026, mon0: 0, mday: 1, hour: 0, min: 0, wday: 0 };
        let resolve_ref = civil_to_unix(&ref_civil, 0);

        // Expected: last_year=2025, Start = 2025-10-18 UTC
        let expected_civil = CivilDate { year: 2025, mon0: 9, mday: 18, hour: 0, min: 0, wday: 0 };
        let expected_start = civil_to_unix(&expected_civil, 0);

        let result = set_holiday_event_time(1, &holiday, resolve_ref, 0);
        let (start, _, _) = result.expect("should resolve");
        assert_eq!(
            start, expected_start,
            "singleDate: should pick last_year=2025 when within window"
        );
    }

    // ── Regression: !single_date, FindStartTimeForStage returns 0 → keep row.start_time ──

    #[test]
    fn non_single_date_all_past_retains_row_start_time() {
        // Regression for C++ `if (start) event.Start = start` (GameEventMgr.cpp line 1969).
        //
        // Setup: holiday with single_date=false (year_offset != 31), filter=-1 (yearly).
        // All populated Date[] entries resolve to instants fully in the past relative to
        // resolve_ref, so find_start_time_for_stage returns 0.
        //
        // The resolved event's start must equal row.start_time, NOT 0.
        //
        // Build a packed date with year_offset=20 (year 2020) — well in the past.
        // resolve_ref = 2026-05-30 (far past 2020).
        let year_offset: u32 = 20; // year 2020 — NOT the singleDate sentinel (31)
        let mon0: u32 = 0;         // January
        let day0: u32 = 0;         // 1st (0-indexed)
        let weekday: u32 = 0;
        let packed_past = (year_offset << 24) | (mon0 << 20) | (day0 << 14) | (weekday << 11);
        // year_offset=20 → bits[24-28] = 20 ≠ 31 → single_date = false.

        // Duration[0]=168h (7 days), filter=-1 (yearly).
        let holiday = make_holiday(vec![packed_past, 0], vec![168], -1, false);

        // resolve_ref = 2026-05-30 (way after 2020-01-01 + 7 days)
        let ref_civil = CivilDate { year: 2026, mon0: 4, mday: 30, hour: 0, min: 0, wday: 0 };
        let resolve_ref = civil_to_unix(&ref_civil, 0);

        // row.start_time is set to a well-known sentinel value that is NOT 0.
        let sentinel_start: i64 = 1_700_000_000; // 2023-11-14 — arbitrary known unixtime

        let row = GameEventInput {
            entry: 99,
            start_time: Some(sentinel_start),
            end_time: Some(9_999_999_999),
            occurence: 525_600, // YEAR/MINUTE
            length: 168 * 60,   // 168h in minutes
            holiday: 99,
            holiday_stage: 1,
            state: GameEventState::Normal,
            next_start: 0,
        };

        let s = effective_start(row.start_time);
        let e = effective_end(row.end_time, resolve_ref);
        let resolved = resolve_holiday_event(&row, s, e, &holiday, resolve_ref, 0);

        // C++ `if (start) event.Start = start`: since FindStartTimeForStage returned 0
        // (no qualifying date), the C++ code keeps game_event.start_time.
        // We must NOT propagate 0 — the resolved start must equal row.start_time.
        assert_eq!(
            resolved.start, sentinel_start,
            "!single_date + all-past dates: resolved.start must equal row.start_time ({sentinel_start}), not 0"
        );
    }
}
