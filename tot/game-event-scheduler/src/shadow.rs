//! Shadow diff engine — three-check comparison of Rust-computed schedule against
//! the C++ `obs.game_events` ground truth.
//!
//! `compute_report(gt, raw_events, rules)` is the single public entry point.
//!
//! # The three checks (and their anti-circularity contracts)
//!
//! ## CHECK 1 — DateMath (deterministic, non-circular)
//!
//! **Inputs:** `gt.holidays[i].holiday_id` (to look up a `HolidayRule`), and the
//! current server year derived from `gt.server_gametime`.
//!
//! **Rust computes:** `get_packed_holiday_date(holiday_id, year)` for
//! `gen_year − 1 ..= gen_year + 2` (matching the C++ per-year generation loop),
//! or the Darkmoon generate_dynamic_dates path (4-year span).
//!
//! **Comparison target:** `gt.holidays[i].date[]` (the C++ DBC dump — not the
//! resolved event start/end). This is the C++ ANSWER to the date calculation.
//!
//! **Non-circular:** Rust recomputes the date from the rule; the C++ dump is only
//! used as the expected value. No `gt.events[].start/end` is consulted.
//!
//! ## CHECK 2 — Resolution (reference-aware diagnostic)
//!
//! **Inputs (Rust):** `raw_events` rows (the RAW SQL `game_event` table dump);
//! for holiday events, the `HolidaysEntry` from `gt.holidays` is used AS AN INPUT
//! (it carries the DBC data: `date[]`, `duration[]`, `calendar_filter_type`,
//! `looping`, `region`) — these are the inputs to `SetHolidayEventTime`, NOT the answer.
//! `gt.resolve_reference_unixtime` and `gt.server_tz_offset_secs` are used for
//! the reference-parameterised holiday resolution.
//!
//! **Rust computes:** `resolve_periodic_event(row)` or
//! `resolve_holiday_event(row, holidays_entry, resolve_ref, tz_offset)`.
//!
//! **Comparison target:** `gt.events[entry].{start, end, occurence, length}`.
//!
//! **Anti-circularity:** `gt.events[].start/end` is NEVER read as a compute
//! input — it is the comparison target only. The `HolidaysEntry.date[]` data
//! from the C++ dump IS an input (it is the DBC data, not the answer).
//!
//! **Diagnostic status:** Holiday start/end mismatches are flagged
//! `[holiday-diagnostic]` — they are informational because the reference time
//! used by C++ at actual load time may differ from `resolve_reference_unixtime`.
//! Periodic start/end/occurence/length mismatches are exact failures.
//!
//! ## CHECK 3 — ActiveSet (PRIMARY exit gate, load-independent)
//!
//! **Inputs:** The Rust-resolved event (from CHECK 2) and `gt.server_gametime`.
//!
//! **Rust computes:** `is_active(rust_resolved, server_gametime, |_| None)`.
//!
//! **Comparison:** (a) `gt.events[entry].is_active`; (b) membership in
//! `gt.active_event_list`.
//!
//! **Primary gate:** Zero mismatches over the live set is the Inc-1 exit criterion.
//! This check is load-time-independent (it only uses the schedule math and
//! `server_gametime`, which is the same for both Rust and C++).

use serde::Serialize;

use crate::events::{GroundTruth, GroundTruthEvent};
use crate::holiday::{get_packed_holiday_date, HolidayCalculationType, HolidayRule};
use crate::resolve::{
    generate_dynamic_dates, resolve_holiday_event, resolve_periodic_event, GameEventInput,
    HolidaysEntry, MAX_HOLIDAY_DATES,
};
use crate::schedule::is_active;

// ── Public types ─────────────────────────────────────────────────────────────

/// Counts for one check — `matched` items agreed; `checked - matched` are mismatches.
#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    pub checked: usize,
    pub matched: usize,
}

impl CheckResult {
    fn new() -> Self {
        CheckResult { checked: 0, matched: 0 }
    }

    fn record(&mut self, matches: bool) {
        self.checked += 1;
        if matches {
            self.matched += 1;
        }
    }

    pub fn mismatches(&self) -> usize {
        self.checked - self.matched
    }
}

/// Which of the three checks produced a mismatch.
#[derive(Debug, Clone, Serialize)]
pub enum CheckKind {
    DateMath,
    Resolution,
    ActiveSet,
}

/// A single mismatch record.
#[derive(Debug, Clone, Serialize)]
pub struct Mismatch {
    /// The check that produced this mismatch.
    pub kind: CheckKind,
    /// Event entry (u16) or holiday ID (u32) as a string key.
    pub key: String,
    /// The field that differed (e.g., `"start"`, `"date[1]"`, `"is_active"`).
    pub field: String,
    /// Rust's computed value (as a string).
    pub rust: String,
    /// C++'s observed value (as a string).
    pub cpp: String,
}

/// Full shadow diff report produced by `compute_report`.
#[derive(Debug, Clone, Serialize)]
pub struct ShadowReport {
    /// Server game time at which the snapshot was taken (Unix seconds).
    pub server_gametime: i64,
    /// Reference time used by the server's `SetHolidayEventTime` (Unix seconds).
    pub resolve_reference_unixtime: i64,
    /// CHECK 1 — date math results.
    pub date_math: CheckResult,
    /// CHECK 2 — resolution results.
    pub resolution: CheckResult,
    /// CHECK 3 — active-set results (primary Inc-1 exit gate).
    pub active_set: CheckResult,
    /// All individual mismatches across all three checks.
    pub mismatches: Vec<Mismatch>,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Look up a `HolidaysEntry` from `gt.holidays` by `holiday_id`.
/// Returns `None` if the holiday is not present in the dump (no rule without data).
fn holidays_entry_for(
    holiday_id: u32,
    holidays: &[HolidaysEntry],
) -> Option<&HolidaysEntry> {
    holidays.iter().find(|h| h.holiday_id == holiday_id)
}

/// Look up a `HolidayRule` by `holiday_id` from the static rules table.
fn rule_for(holiday_id: u32, rules: &[HolidayRule]) -> Option<&HolidayRule> {
    rules.iter().find(|r| r.holiday_id == holiday_id)
}

/// Derive the generation year from the server_gametime.
/// This mirrors C++ `LoadHolidayDates`: `gen_year = localtime(curTime).tm_year + 1900`.
fn gen_year_from_unix(unix: i64, tz_offset_secs: i32) -> i32 {
    use crate::packed::unix_to_civil;
    unix_to_civil(unix, tz_offset_secs).year
}

// ── CHECK 1: DateMath ─────────────────────────────────────────────────────────

/// Run CHECK 1 for one holiday.
///
/// Builds the expected `date[]` array independently (Rust computes it from
/// the `HolidayRule`) then compares each non-trivially-populated slot against
/// the corresponding slot in `cpp_holiday.date[]`.
///
/// **Anti-circularity:** Only `holiday_id` and the rule are compute inputs.
/// `cpp_holiday.date[]` is the comparison target, never a compute input.
fn check_date_math_for_holiday(
    cpp_holiday: &HolidaysEntry,
    rule: &HolidayRule,
    gen_year: i32,
    result: &mut CheckResult,
    mismatches: &mut Vec<Mismatch>,
) {
    let key = format!("holiday:{}", cpp_holiday.holiday_id);

    if rule.calc_type == HolidayCalculationType::DarkmoonFaire {
        // Darkmoon path: generate_dynamic_dates fills dates from gen_year-1 for 4 years.
        // Build a synthetic entry with empty dates and the same duration as the C++ dump.
        let mut synthetic = HolidaysEntry {
            holiday_id: cpp_holiday.holiday_id,
            date: vec![],
            duration: cpp_holiday.duration.clone(),
            calendar_filter_type: cpp_holiday.calendar_filter_type,
            looping: cpp_holiday.looping,
            region: cpp_holiday.region,
        };
        generate_dynamic_dates(cpp_holiday.holiday_id, gen_year, &mut synthetic);

        // Compare every non-zero slot in the Rust-computed date array against the C++ dump.
        // Darkmoon: up to 4 dates per year × 4 years = 16 packed dates in synthetic.date.
        // The cpp_holiday.date[] has the same layout (filled from gen_year-1 forward).
        let compare_count = synthetic.date.len().max(cpp_holiday.date.len());
        // Zip to min length; extra slots beyond what Rust filled are checked as 0 vs cpp.
        let rust_dates = &synthetic.date;
        let cpp_dates = &cpp_holiday.date;

        for i in 0..compare_count {
            let rust_val = rust_dates.get(i).copied().unwrap_or(0);
            let cpp_val = cpp_dates.get(i).copied().unwrap_or(0);
            // Only compare slots where either side has a non-zero value — zero slots
            // are "not populated" and are both expected to be 0.
            if rust_val == 0 && cpp_val == 0 {
                continue;
            }
            let matches = rust_val == cpp_val;
            result.record(matches);
            if !matches {
                mismatches.push(Mismatch {
                    kind: CheckKind::DateMath,
                    key: key.clone(),
                    field: format!("date[{i}]"),
                    rust: format!("{rust_val:#010x}"),
                    cpp: format!("{cpp_val:#010x}"),
                });
            }
        }
    } else {
        // Per-year generation path: yearOffset ∈ [−1, 2], dateId = yearOffset + 1 (indices 0..3).
        for year_offset in -1i32..=2 {
            let year = gen_year + year_offset;
            if year > 2030 {
                break;
            }
            let date_id = (year_offset + 1) as usize; // 0, 1, 2, 3
            if date_id >= MAX_HOLIDAY_DATES {
                break;
            }

            // RUST computes independently from the rule — this is the non-circular part.
            let rust_val = get_packed_holiday_date(rule.holiday_id, year);
            let cpp_val = cpp_holiday.date.get(date_id).copied().unwrap_or(0);

            // If rust returns 0 and cpp also has 0, the slot is unpopulated — skip it.
            // (e.g., year > 2030 causes C++ to also skip.)
            if rust_val == 0 && cpp_val == 0 {
                continue;
            }

            let matches = rust_val == cpp_val;
            result.record(matches);
            if !matches {
                mismatches.push(Mismatch {
                    kind: CheckKind::DateMath,
                    key: key.clone(),
                    field: format!("date[{date_id}] (year {year})"),
                    rust: format!("{rust_val:#010x}"),
                    cpp: format!("{cpp_val:#010x}"),
                });
            }
        }
    }
}

// ── Main public entry point ────────────────────────────────────────────────────

/// Compute the three-check shadow diff report.
///
/// # Arguments
/// - `gt`: the C++ ground truth from `obs.game_events` (resolved events + active set +
///   sHolidaysStore dump + timing metadata).
/// - `raw_events`: the raw SQL `game_event_all` rows (the compute inputs for CHECK 2).
/// - `rules`: the static holiday rules from `holiday_rules()`.
///
/// # Anti-circularity guarantee
/// - CHECK 1 uses only `holiday_id` + the rule to compute dates; `gt.holidays[].date[]`
///   is the comparison target only.
/// - CHECK 2 uses `raw_events` rows + `HolidaysEntry` DBC data (not resolved start/end)
///   as inputs; `gt.events[].{start, end, occurence, length}` is the comparison target only.
/// - CHECK 3 uses only the Rust-resolved events + `gt.server_gametime`; never reads
///   `gt.events[].is_active` as a compute input.
pub fn compute_report(
    gt: &GroundTruth,
    raw_events: &[GameEventInput],
    rules: &[HolidayRule],
) -> ShadowReport {
    let mut date_math = CheckResult::new();
    let mut resolution = CheckResult::new();
    let mut active_set = CheckResult::new();
    let mut mismatches: Vec<Mismatch> = Vec::new();

    let gen_year = gen_year_from_unix(gt.server_gametime, gt.server_tz_offset_secs);

    // ── CHECK 1: DateMath ──────────────────────────────────────────────────────
    // For each holiday in the C++ dump, find its rule and compare Rust-computed dates.
    for cpp_holiday in &gt.holidays {
        if let Some(rule) = rule_for(cpp_holiday.holiday_id, rules) {
            check_date_math_for_holiday(
                cpp_holiday,
                rule,
                gen_year,
                &mut date_math,
                &mut mismatches,
            );
        }
        // If no rule is found (holiday not in our rules table), skip silently —
        // the rules table only covers calculable holidays; DBC-static ones have
        // no rule and their dates are constant, not computed.
    }

    // Build a lookup map: entry → &GroundTruthEvent (for CHECK 2 + 3 comparisons).
    // Vec is small (≤181) so linear scan is fine, but a HashMap avoids O(n²).
    let gt_event_by_entry: std::collections::HashMap<u16, &GroundTruthEvent> = gt
        .events
        .iter()
        .map(|e| (e.entry, e))
        .collect();

    // ── CHECK 2 + CHECK 3: Resolution + ActiveSet ──────────────────────────────
    for row in raw_events {
        // --- Rust resolution (CHECK 2 input) ---
        // For holiday events: use HolidaysEntry from gt.holidays as an INPUT.
        // The HolidaysEntry carries DBC data (date[], duration[], filter type, looping, region)
        // which are the inputs to SetHolidayEventTime.
        // CRITICALLY: we do NOT read gt.events[entry].start/end here — those are targets.
        let rust_resolved = if row.holiday != 0 {
            if let Some(hentry) = holidays_entry_for(row.holiday, &gt.holidays) {
                resolve_holiday_event(
                    row,
                    hentry,
                    gt.resolve_reference_unixtime,
                    gt.server_tz_offset_secs,
                )
            } else {
                // Holiday referenced by event but not in the dump — fallback to periodic.
                resolve_periodic_event(row)
            }
        } else {
            resolve_periodic_event(row)
        };

        let key = format!("event:{}", row.entry);

        // --- CHECK 2: Resolution ---
        // Compare Rust-resolved {start, end, occurence, length} to gt.events[entry].
        if let Some(&cpp_ev) = gt_event_by_entry.get(&row.entry) {
            let is_holiday = row.holiday != 0;

            // occurence and length: exact for both periodic and holiday.
            let occ_match = rust_resolved.occurence == cpp_ev.occurence;
            resolution.record(occ_match);
            if !occ_match {
                mismatches.push(Mismatch {
                    kind: CheckKind::Resolution,
                    key: key.clone(),
                    field: "occurence".into(),
                    rust: rust_resolved.occurence.to_string(),
                    cpp: cpp_ev.occurence.to_string(),
                });
            }

            let len_match = rust_resolved.length == cpp_ev.length;
            resolution.record(len_match);
            if !len_match {
                mismatches.push(Mismatch {
                    kind: CheckKind::Resolution,
                    key: key.clone(),
                    field: "length".into(),
                    rust: rust_resolved.length.to_string(),
                    cpp: cpp_ev.length.to_string(),
                });
            }

            // start and end: diagnostic for holiday events (reference-time-dependent),
            // exact for periodic events (pure copy of DB columns).
            let start_match = rust_resolved.start == cpp_ev.start;
            resolution.record(start_match);
            if !start_match {
                let field_name = if is_holiday {
                    "start[holiday-diagnostic]"
                } else {
                    "start"
                };
                mismatches.push(Mismatch {
                    kind: CheckKind::Resolution,
                    key: key.clone(),
                    field: field_name.into(),
                    rust: rust_resolved.start.to_string(),
                    cpp: cpp_ev.start.to_string(),
                });
            }

            let end_match = rust_resolved.end == cpp_ev.end;
            resolution.record(end_match);
            if !end_match {
                let field_name = if is_holiday {
                    "end[holiday-diagnostic]"
                } else {
                    "end"
                };
                mismatches.push(Mismatch {
                    kind: CheckKind::Resolution,
                    key: key.clone(),
                    field: field_name.into(),
                    rust: rust_resolved.end.to_string(),
                    cpp: cpp_ev.end.to_string(),
                });
            }

            // --- CHECK 3: ActiveSet ---
            // RUST computes is_active from the Rust-resolved event and server_gametime.
            // We do NOT read cpp_ev.is_active as a compute input — only as a target.
            let rust_active = is_active(&rust_resolved, gt.server_gametime, |_| None);
            let cpp_active_from_field = cpp_ev.is_active;
            let cpp_active_from_list = gt.active_event_list.contains(&row.entry);

            // Primary comparison: Rust vs the C++ is_active field.
            let active_match_field = rust_active == cpp_active_from_field;
            active_set.record(active_match_field);
            if !active_match_field {
                mismatches.push(Mismatch {
                    kind: CheckKind::ActiveSet,
                    key: key.clone(),
                    field: "is_active(vs_field)".into(),
                    rust: rust_active.to_string(),
                    cpp: cpp_active_from_field.to_string(),
                });
            }

            // Secondary comparison: Rust vs membership in the active_event_list.
            // This is an independent cross-check (same event, different C++ source).
            let active_match_list = rust_active == cpp_active_from_list;
            active_set.record(active_match_list);
            if !active_match_list {
                mismatches.push(Mismatch {
                    kind: CheckKind::ActiveSet,
                    key: key.clone(),
                    field: "is_active(vs_list)".into(),
                    rust: rust_active.to_string(),
                    cpp: cpp_active_from_list.to_string(),
                });
            }
        }
        // If no matching gt event for this raw row, skip CHECK 2+3 for it
        // (shouldn't happen in practice — gt contains all 181 events).
    }

    ShadowReport {
        server_gametime: gt.server_gametime,
        resolve_reference_unixtime: gt.resolve_reference_unixtime,
        date_math,
        resolution,
        active_set,
        mismatches,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::GroundTruth;
    use crate::holiday::holiday_rules;
    use crate::resolve::{HolidaysEntry, MAX_HOLIDAY_DATES, MAX_HOLIDAY_DURATIONS};
    use crate::schedule::GameEventState;

    // ── Fixture helpers ───────────────────────────────────────────────────────

    /// Build a `GroundTruthEvent` for testing.
    fn make_gt_event(
        entry: u16,
        start: i64,
        end: i64,
        occ: i64,
        len: i64,
        is_active: bool,
    ) -> crate::events::GroundTruthEvent {
        use crate::events::GroundTruthEvent;
        GroundTruthEvent {
            entry,
            start,
            end,
            occurence: occ,
            length: len,
            holiday: 0,
            holiday_stage: 0,
            is_active,
            next_start: 0,
            state: GameEventState::Normal,
        }
    }

    /// Build a `GroundTruthEvent` for a holiday event.
    #[allow(clippy::too_many_arguments)]
    fn make_gt_holiday_event(
        entry: u16,
        start: i64,
        end: i64,
        occ: i64,
        len: i64,
        holiday: u32,
        holiday_stage: u8,
        is_active: bool,
    ) -> crate::events::GroundTruthEvent {
        use crate::events::GroundTruthEvent;
        GroundTruthEvent {
            entry,
            start,
            end,
            occurence: occ,
            length: len,
            holiday,
            holiday_stage,
            is_active,
            next_start: 0,
            state: GameEventState::Normal,
        }
    }

    /// Build a periodic `GameEventInput`.
    fn make_raw_periodic(entry: u16, start: i64, end: i64, occ: i64, len: i64) -> GameEventInput {
        GameEventInput {
            entry,
            start_time: start,
            end_time: end,
            occurence: occ,
            length: len,
            holiday: 0,
            holiday_stage: 0,
            state: GameEventState::Normal,
            next_start: 0,
        }
    }

    /// Build a holiday `GameEventInput`.
    fn make_raw_holiday(
        entry: u16,
        start: i64,
        end: i64,
        occ: i64,
        len: i64,
        holiday: u32,
        holiday_stage: u8,
    ) -> GameEventInput {
        GameEventInput {
            entry,
            start_time: start,
            end_time: end,
            occurence: occ,
            length: len,
            holiday,
            holiday_stage,
            state: GameEventState::Normal,
            next_start: 0,
        }
    }

    /// Build a `HolidaysEntry` with the specified holiday_id and dates/durations.
    fn make_holidays_entry(
        holiday_id: u32,
        dates: Vec<u32>,
        durations: Vec<u32>,
        calendar_filter_type: i32,
        looping: bool,
    ) -> HolidaysEntry {
        // Pad dates to MAX_HOLIDAY_DATES and durations to MAX_HOLIDAY_DURATIONS with zeros.
        let mut date = dates;
        date.resize(MAX_HOLIDAY_DATES, 0);
        let mut duration = durations;
        duration.resize(MAX_HOLIDAY_DURATIONS, 0);
        HolidaysEntry { holiday_id, date, duration, calendar_filter_type, looping, region: 0 }
    }

    // ── The "all consistent" fixture: 3 events + 1 holiday + date_math case ────
    //
    // server_gametime = 2026-06-02 12:00:00 UTC = ?
    // Use a known UTC timestamp: 2026-06-02 12:00:00 UTC.
    // Periodic event 1: start=2026-01-01, end=2027-01-01, occ=WEEK(10080 min), len=72*60 min.
    //   At server_gametime (June 2), it is in a 3-day window starting each Monday.
    //   2026-01-01 is a Thursday. elapsed = (server_gametime - start) % (occ*60)
    //   We design this so it IS active (easier to reason about).
    //
    // We'll use a simpler approach: make occ=525600 (YEAR), len=20160 (14 days).
    // Event 1: start = 2026-01-01 (in the past), end = 2027-01-01, len=14 days.
    //   is_active = start < now < end AND elapsed_in_period < length
    //   start = Jan 1 2026. server_gametime = Jun 2 2026.
    //   elapsed = (Jun2 - Jan1) % (525600*60) = (Jun2-Jan1) seconds (less than 1 year)
    //   Jan1=1767225600, Jun2≈1780394400. elapsed = 13168800 s.
    //   period = 525600 * 60 = 31536000 s. elapsed < period since 13168800 < 31536000.
    //   length*60 = 20160 * 60 = 1209600 s.
    //   13168800 < 1209600? NO → event 1 is INACTIVE at Jun 2.
    //
    // Let's use occ=10080 (weekly), len=4320 (72h=3 days).
    //   Jan 1 2026 = Thursday. server_gametime = Jun 2 2026 = Tuesday.
    //   elapsed = (Jun2 - Jan1) % (10080*60) = 13168800 % 604800.
    //   13168800 / 604800 = 21.78, mod = 0.78 * 604800 ≈ 472608 s.
    //   length*60 = 4320*60 = 259200 s.
    //   472608 < 259200? NO → inactive. Let's just hand-design the exact timestamps.
    //
    // Simplest approach: anchor start exactly to server_gametime - 1h (active case).
    // server_gametime = 1780394400 (2026-06-02 12:00:00 UTC approx).
    // Event 10 (active): start = server_gametime - 3600, end = server_gametime + 86400,
    //   occ = 525600, len = 20160. elapsed = 3600 < 20160*60=1209600 → ACTIVE.
    // Event 11 (inactive): start = server_gametime - 20*86400, end = server_gametime + 86400,
    //   occ = 525600, len = 20160. elapsed = 20*86400 = 1728000.
    //   1728000 < 1209600? NO → INACTIVE.
    //
    // Holiday event (Fireworks Spectacular, holiday_id=62, weekly, filter=0):
    //   Fireworks Spectacular: fixed Jul 4. holiday_id=62.
    //   active_event_list contains entry 16 in our fixture when it's active.
    //   We'll make entry 16 a holiday event. We use the Fireworks Spectacular rule:
    //   FixedDate, month=7 (July), day=4. Duration = [18h] = [18].
    //   For 2026: Jul 4.
    //   At server_gametime (Jun 2), it is BEFORE Jul 4 → INACTIVE.
    //   We'll include it inactive.
    //
    // Date-math case: Hallow's End (holiday_id=324, FixedDate Oct 18).
    //   gen_year = 2026. dateId 0=2025, 1=2026, 2=2027, 3=2028.
    //   get_packed_holiday_date(324, 2025) should match cpp_holiday.date[0].
    //   We set cpp_holiday.date[0] = get_packed_holiday_date(324, 2025) → they agree.
    //
    // Constants:
    const SERVER_GAMETIME: i64 = 1780394400; // 2026-06-02 12:00:00 UTC (approx)
    const RESOLVE_REF: i64 = 1780394400;     // same for simplicity

    fn build_consistent_gt_and_raw(active_event_list_override: Option<Vec<u16>>) -> (GroundTruth, Vec<GameEventInput>) {
        // Event 10: periodic, ACTIVE (start < server_gametime, elapsed < length window)
        let start_10 = SERVER_GAMETIME - 3600; // 1 hour ago
        let end_10 = SERVER_GAMETIME + 86400;  // tomorrow
        let occ_10 = 525_600i64;               // 1 year in minutes
        let len_10 = 20_160i64;                // 14 days in minutes
        // elapsed_secs = 3600; length_secs = 20160*60 = 1209600. 3600 < 1209600 → ACTIVE.
        let is_active_10 = true;

        // Event 11: periodic, INACTIVE (elapsed > length window)
        let start_11 = SERVER_GAMETIME - 20 * 86400; // 20 days ago
        let end_11 = SERVER_GAMETIME + 86400;
        let occ_11 = 525_600i64;
        let len_11 = 20_160i64;
        // elapsed_secs = 20*86400 = 1728000; length_secs = 1209600. 1728000 > 1209600 → INACTIVE.
        let is_active_11 = false;

        // Event 16: holiday (Fireworks Spectacular, holiday_id=62, FixedDate Jul 4).
        // At Jun 2, the festival hasn't started yet → INACTIVE.
        // For resolve_holiday_event: we need a HolidaysEntry for holiday 62.
        // C++ will have computed start = Jul 4, 2026 = ? Let's compute it.
        // The rule for 62 is FixedDate, month=7, day=4. gen_year=2026.
        // Date[1] = get_packed_holiday_date(62, 2026) = Jul 4, 2026.
        // We build a HolidaysEntry that matches what the live dump would show.
        // Calendar_filter_type = -1 (yearly). Duration[0] = 18h (from the live fixture).
        // For find_start_time_for_stage at ref=SERVER_GAMETIME (Jun 2 2026):
        //   Date[0] = packed for 2025 Jul 4; Date[1] = packed for 2026 Jul 4.
        //   cur_time(Jun 2) < start_time(2025 Jul 4) + 0 + 18*60*60? No — 2025 is past.
        //   cur_time(Jun 2) < start_time(2026 Jul 4) + 0 + 18*60*60? YES → start = 2026 Jul 4.
        let end_16 = 1843316906i64; // from live fixture (far future end_time)
        let occ_16 = 525_600i64;    // YEAR/MINUTE (yearly, filter=-1)
        // length for fireworks: 18h * 60 = 1080 minutes
        let len_16 = 18i64 * 60;

        // Build HolidaysEntry for holiday_id=62 (Fireworks Spectacular).
        // Dates: Date[0]=2025 Jul 4, Date[1]=2026 Jul 4, Date[2]=2027 Jul 4, Date[3]=2028 Jul 4.
        let packed_62_2025 = get_packed_holiday_date(62, 2025);
        let packed_62_2026 = get_packed_holiday_date(62, 2026);
        let packed_62_2027 = get_packed_holiday_date(62, 2027);
        let packed_62_2028 = get_packed_holiday_date(62, 2028);
        let holidays_entry_62 = make_holidays_entry(
            62,
            vec![packed_62_2025, packed_62_2026, packed_62_2027, packed_62_2028],
            vec![18, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            -1,   // yearly
            false,
        );

        // Compute what Rust would resolve for event 16 (so we can set gt.events[16] consistently).
        // resolve_holiday_event uses the HolidaysEntry + resolve_ref.
        // filter=-1 → occurence = YEAR/MINUTE = 525600.
        // length = 18 * HOUR / MINUTE = 18 * 3600 / 60 = 1080 min.
        // start = find_start_time_for_stage(dates, stage_offset=0, length=1080*60=64800 s, ref):
        //   At ref=Jun 2 2026: 2025 Jul 4 + 18h window is past (2026-06-02 > 2025-07-04+18h).
        //   2026 Jul 4: cur_time(Jun2) < Jul4 + 18h → YES. Start = 2026 Jul 4.
        let raw_input_16 = make_raw_holiday(16, 0, end_16, occ_16, len_16, 62, 1);
        let rust_resolved_16 = resolve_holiday_event(
            &raw_input_16,
            &holidays_entry_62,
            RESOLVE_REF,
            0,
        );
        let start_16 = rust_resolved_16.start;
        let is_active_16 = false; // Jun 2 < Jul 4 2026 → not yet active (before start)

        // Hallow's End holiday entry for date_math CHECK 1.
        // holiday_id=324, FixedDate, gen_year=2026 → dates at indices 0..3.
        // We set cpp dates = Rust-computed dates → date_math check PASSES.
        let packed_324_2025 = get_packed_holiday_date(324, 2025);
        let packed_324_2026 = get_packed_holiday_date(324, 2026);
        let packed_324_2027 = get_packed_holiday_date(324, 2027);
        let packed_324_2028 = get_packed_holiday_date(324, 2028);
        let holidays_entry_324 = make_holidays_entry(
            324,
            vec![packed_324_2025, packed_324_2026, packed_324_2027, packed_324_2028],
            vec![336, 0, 0, 0, 0, 0, 0, 0, 0, 0], // 14 days
            -1,
            false,
        );

        let active_list = active_event_list_override.unwrap_or_else(|| {
            let mut v = Vec::new();
            if is_active_10 { v.push(10u16); }
            if is_active_11 { v.push(11u16); }
            if is_active_16 { v.push(16u16); }
            v
        });

        let gt = GroundTruth {
            events: vec![
                make_gt_event(10, start_10, end_10, occ_10, len_10, is_active_10),
                make_gt_event(11, start_11, end_11, occ_11, len_11, is_active_11),
                make_gt_holiday_event(16, start_16, end_16, occ_16, len_16, 62, 1, is_active_16),
            ],
            active_event_list: active_list,
            holidays: vec![holidays_entry_62, holidays_entry_324],
            server_gametime: SERVER_GAMETIME,
            resolve_reference_unixtime: RESOLVE_REF,
            server_tz_offset_secs: 0,
        };

        let raw_events = vec![
            make_raw_periodic(10, start_10, end_10, occ_10, len_10),
            make_raw_periodic(11, start_11, end_11, occ_11, len_11),
            raw_input_16,
        ];

        (gt, raw_events)
    }

    // ── Test: all-consistent fixture → zero active_set mismatches ────────────

    #[test]
    fn all_consistent_zero_active_set_mismatches() {
        let (gt, raw_events) = build_consistent_gt_and_raw(None);
        let rules = holiday_rules();
        let report = compute_report(&gt, &raw_events, rules);
        assert_eq!(
            report.active_set.mismatches(),
            0,
            "all-consistent fixture must have zero active_set mismatches"
        );
    }

    // ── Test: periodic event active at server_gametime ─────────────────────────

    #[test]
    fn periodic_active_event_passes_resolution_and_active_set() {
        let (gt, raw_events) = build_consistent_gt_and_raw(None);
        let rules = holiday_rules();
        let report = compute_report(&gt, &raw_events, rules);

        // Event 10 is active — should contribute matched entries to active_set.
        let active_mismatches_for_10: Vec<_> = report
            .mismatches
            .iter()
            .filter(|m| m.key == "event:10" && matches!(m.kind, CheckKind::ActiveSet))
            .collect();
        assert!(active_mismatches_for_10.is_empty(), "event 10 should have no active_set mismatches");
    }

    // ── Test: periodic event outside its window → is_active false, matches ─────

    #[test]
    fn periodic_inactive_event_matches_gt() {
        let (gt, raw_events) = build_consistent_gt_and_raw(None);
        let rules = holiday_rules();
        let report = compute_report(&gt, &raw_events, rules);

        // Event 11 is inactive — should also have no mismatches.
        let active_mismatches_for_11: Vec<_> = report
            .mismatches
            .iter()
            .filter(|m| m.key == "event:11" && matches!(m.kind, CheckKind::ActiveSet))
            .collect();
        assert!(active_mismatches_for_11.is_empty(), "event 11 should have no active_set mismatches");
    }

    // ── Test: holiday event mid-window → active_set matches ───────────────────

    #[test]
    fn holiday_event_active_set_matches() {
        let (gt, raw_events) = build_consistent_gt_and_raw(None);
        let rules = holiday_rules();
        let report = compute_report(&gt, &raw_events, rules);

        // Event 16 (holiday) is inactive at Jun 2 (before Jul 4 2026).
        // active_set check should agree.
        let active_mismatches_for_16: Vec<_> = report
            .mismatches
            .iter()
            .filter(|m| m.key == "event:16" && matches!(m.kind, CheckKind::ActiveSet))
            .collect();
        assert!(active_mismatches_for_16.is_empty(), "event 16 should have no active_set mismatches");
    }

    // ── Test: date_math case → Hallow's End dates agree ──────────────────────

    #[test]
    fn date_math_hallowsend_agrees() {
        let (gt, raw_events) = build_consistent_gt_and_raw(None);
        let rules = holiday_rules();
        let report = compute_report(&gt, &raw_events, rules);

        let date_math_mismatches: Vec<_> = report
            .mismatches
            .iter()
            .filter(|m| m.key == "holiday:324" && matches!(m.kind, CheckKind::DateMath))
            .collect();
        assert!(
            date_math_mismatches.is_empty(),
            "date_math for Hallow's End should have no mismatches: {:?}",
            date_math_mismatches
        );
    }

    // ── Test: INJECTED mismatch in active_set ──────────────────────────────────
    // Flip the C++ is_active for event 10 to false (but it's really active).
    // The active_set check should detect this.

    #[test]
    fn injected_active_set_mismatch_is_recorded() {
        let (mut gt, raw_events) = build_consistent_gt_and_raw(None);

        // Event 10 is active (Rust computes is_active=true).
        // Flip the C++ ground-truth is_active to false → mismatch.
        let ev10 = gt.events.iter_mut().find(|e| e.entry == 10).unwrap();
        ev10.is_active = false; // injected lie

        let rules = holiday_rules();
        let report = compute_report(&gt, &raw_events, rules);

        let active_mismatches_for_10: Vec<_> = report
            .mismatches
            .iter()
            .filter(|m| m.key == "event:10" && matches!(m.kind, CheckKind::ActiveSet))
            .collect();

        assert!(
            !active_mismatches_for_10.is_empty(),
            "injected active_set mismatch must be recorded for event 10"
        );
        assert!(
            active_mismatches_for_10.iter().any(|m| m.field.contains("is_active")),
            "mismatch field must reference is_active: {:?}",
            active_mismatches_for_10
        );
        assert_eq!(
            active_mismatches_for_10[0].rust, "true",
            "Rust computed is_active=true"
        );
        assert_eq!(
            active_mismatches_for_10[0].cpp, "false",
            "C++ (injected) is_active=false"
        );

        // active_set.mismatches() must be > 0
        assert!(report.active_set.mismatches() > 0, "active_set.mismatches() must be non-zero");
    }

    // ── Test: INJECTED mismatch in date_math ───────────────────────────────────
    // Corrupt one date[] entry in the Hallow's End holidays dump.
    // The date_math check should detect this.

    #[test]
    fn injected_date_math_mismatch_is_recorded() {
        let (mut gt, raw_events) = build_consistent_gt_and_raw(None);

        // Corrupt date[1] (gen_year=2026, Hallow's End 2026) for holiday 324.
        let h324 = gt.holidays.iter_mut().find(|h| h.holiday_id == 324).unwrap();
        h324.date[1] = 0xDEADBEEF; // injected wrong value

        let rules = holiday_rules();
        let report = compute_report(&gt, &raw_events, rules);

        let date_mismatches_324: Vec<_> = report
            .mismatches
            .iter()
            .filter(|m| m.key == "holiday:324" && matches!(m.kind, CheckKind::DateMath))
            .collect();

        assert!(
            !date_mismatches_324.is_empty(),
            "injected date_math mismatch must be recorded for holiday 324"
        );
        assert!(
            date_mismatches_324.iter().any(|m| m.field.contains("date[1]")),
            "mismatch must be on date[1]: {:?}",
            date_mismatches_324
        );
        assert!(
            date_mismatches_324.iter().any(|m| m.cpp.contains("deadbeef") || m.cpp.contains("DEADBEEF") || m.cpp.contains("0xdeadbeef") || m.cpp.contains("0xDEADBEEF")),
            "cpp value must contain 0xDEADBEEF: {:?}",
            date_mismatches_324
        );
    }

    // ── Test: report metadata fields ──────────────────────────────────────────

    #[test]
    fn report_metadata_fields() {
        let (gt, raw_events) = build_consistent_gt_and_raw(None);
        let rules = holiday_rules();
        let report = compute_report(&gt, &raw_events, rules);
        assert_eq!(report.server_gametime, SERVER_GAMETIME);
        assert_eq!(report.resolve_reference_unixtime, RESOLVE_REF);
    }

    // ── Test: ShadowReport is serde::Serialize (compile-time) ────────────────

    #[test]
    fn shadow_report_is_serializable() {
        let (gt, raw_events) = build_consistent_gt_and_raw(None);
        let rules = holiday_rules();
        let report = compute_report(&gt, &raw_events, rules);
        let json = serde_json::to_string(&report).expect("ShadowReport must serialize to JSON");
        assert!(json.contains("server_gametime"), "serialized JSON must contain server_gametime");
        assert!(json.contains("active_set"), "serialized JSON must contain active_set");
    }

    // ── Test: INJECTED mismatch via active_event_list (list vs field agree) ────
    // Remove event 10 from active_event_list while is_active=true on the gt event.
    // The vs_list check should fire.

    #[test]
    fn injected_active_list_mismatch_is_recorded() {
        // Build gt where active_event_list does NOT contain event 10, but is_active=true.
        let (gt, raw_events) = build_consistent_gt_and_raw(Some(vec![])); // empty active list
        let rules = holiday_rules();
        let report = compute_report(&gt, &raw_events, rules);

        // Rust computes event 10 as active (is_active=true).
        // vs_list: true vs false (not in list) → mismatch.
        // vs_field: true vs true (gt.events[10].is_active=true) → match.
        let vs_list_mismatch: Vec<_> = report
            .mismatches
            .iter()
            .filter(|m| m.key == "event:10" && m.field.contains("vs_list"))
            .collect();

        assert!(
            !vs_list_mismatch.is_empty(),
            "active_list mismatch must be recorded when list doesn't contain active event 10"
        );
    }

    // ── Test: CheckResult.mismatches() arithmetic ──────────────────────────────

    #[test]
    fn check_result_mismatches_arithmetic() {
        let mut cr = CheckResult::new();
        cr.record(true);
        cr.record(true);
        cr.record(false);
        assert_eq!(cr.checked, 3);
        assert_eq!(cr.matched, 2);
        assert_eq!(cr.mismatches(), 1);
    }
}
