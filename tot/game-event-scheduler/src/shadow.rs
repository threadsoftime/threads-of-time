//! Shadow diff engine — three-check comparison of Rust-computed schedule against
//! the C++ `obs.game_events` ground truth.
//!
//! `compute_report(gt, raw_events, rules, live_shadow_resolution)` is the single public entry point.
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
    effective_end, effective_start, generate_dynamic_dates, resolve_holiday_event,
    resolve_periodic_event, GameEventInput, HolidaysEntry, MAX_HOLIDAY_DATES,
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

/// Why a particular event was excluded from the active-set check or the resolution check.
///
/// Excluded events are counted separately; they do NOT contribute to
/// `active_set.checked` or `active_set.matched`.  Their C++ `is_active`
/// value is accepted as authoritative (not predicted via date math).
#[derive(Debug, Clone, Serialize)]
pub enum ExclusionReason {
    /// `state != Normal` (states 1-5: WorldInactive/Conditions/Nextphase/Finished/Internal).
    /// These are condition-driven, timer-driven, or sticky via GM command — NOT calendar
    /// date math.  C++ `is_active` is sticky; Rust date math cannot predict it.
    NonNormalState,
    /// NORMAL state + no holiday + raw `start_time` and `end_time` are BOTH `Some`,
    /// equal to each other, and less than `resolve_reference`.
    ///
    /// These are GM-started events where `StartEvent(overwrite=true)` rebases
    /// `Start → boot`.  The raw SQL start/end are non-null, equal, and far in the
    /// past — not date-predictable.
    ///
    /// **Null/null events are NOT ManualStart** — a NULL start or NULL end means
    /// C++ uses a date-math default (start=0, end=resolve_ref+2yr) and the event
    /// participates in date-scheduled active-set checking normally.
    ///
    /// Live example: event 60 (raw start_time=Some(946735200), end_time=Some(946735200),
    /// both far past 946735200 << resolve_ref ~1780244906).
    ManualStart,
}

/// A single event excluded from the active-set date-math check.
#[derive(Debug, Clone, Serialize)]
pub struct ExcludedEvent {
    pub entry: u16,
    pub reason: ExclusionReason,
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
    ///
    /// **Scope:** ManualStart events are excluded from resolution checking too.
    /// Event 60's start/end are GM-rebased (`StartEvent overwrite=true`) and
    /// not date-predictable, so start/end mismatches from ManualStart events
    /// are expected and out-of-scope.  See `resolution_exclusions` for the list.
    /// NonNormalState events' resolution is kept IN scope — for Internal events
    /// C++ retains the raw loaded start/end which the resolver reproduces; only
    /// ManualStart needs resolution exclusion.
    pub resolution: CheckResult,
    /// CHECK 3 — active-set results (primary Inc-1 exit gate).
    ///
    /// **Scope:** Only events where `state == Normal` AND the event is
    /// date-scheduled (not a GM-started / manual-start event) contribute to
    /// `checked` and `matched`.  See `active_set_exclusions` for the rest.
    pub active_set: CheckResult,
    /// Events excluded from the resolution check (CHECK 2), with reasons.
    ///
    /// Currently only `ManualStart` events are excluded from resolution.
    /// Their raw start/end are GM-rebased and not date-predictable; any
    /// resolution mismatch is expected rather than indicative of a Rust bug.
    pub resolution_exclusions: Vec<ExcludedEvent>,
    /// Events excluded from the active-set check (CHECK 3), with reasons.
    ///
    /// These events are NOT counted in `active_set.checked`.  C++ `is_active`
    /// is accepted as authoritative for them; no date-math prediction is made.
    /// Reporting them prevents silently hiding events that could reveal bugs in
    /// the date-math path if they ever transition to Normal/date-scheduled state.
    pub active_set_exclusions: Vec<ExcludedEvent>,
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

/// Determine whether an event is a ManualStart.
///
/// **ManualStart** ⇔ `state == Normal` AND `holiday == 0` AND `raw_start_time` is
/// `Some(t)` AND `raw_end_time` is `Some(t)` (both non-null, equal) AND `t < resolve_ref`.
///
/// These are GM-started events where `StartEvent(overwrite=true)` rebases
/// `Start → boot`.  The raw SQL timestamps are non-null, equal, and far in the
/// past — not date-predictable.
///
/// **Critical: null/null events are NOT ManualStart.**  If either `raw_start_time` or
/// `raw_end_time` is `None`, the event uses C++ null-default math
/// (start=0, end=resolve_ref+2yr) and participates in date-scheduled checking.
/// The ManualStart check MUST key off the raw `Option<i64>` values, NOT the
/// post-`effective_*` materialized integers (which would map None→0 and could
/// create false equality).
///
/// Live example of a real ManualStart: event 60 has
/// `raw_start_time=Some(946735200)`, `raw_end_time=Some(946735200)` — both
/// non-null, equal, and far before `resolve_reference_unixtime ≈ 1780244906`.
fn is_manual_start(row: &GameEventInput, resolve_ref: i64) -> bool {
    use crate::schedule::GameEventState;
    if row.state != GameEventState::Normal || row.holiday != 0 {
        return false;
    }
    // Both timestamps must be Some, equal, and before the resolve reference.
    match (row.start_time, row.end_time) {
        (Some(s), Some(e)) => s == e && e < resolve_ref,
        _ => false, // None on either side → date-scheduled, NOT manual-start
    }
}

/// Determine whether an event should be excluded from the active-set date-math check.
///
/// Returns `Some(reason)` if excluded, `None` if in-scope.
///
/// This is also used by the drive step in `tick.rs` to ensure that the drive
/// logic applies to EXACTLY the same set of events as the active-set shadow check.
///
/// ## Exclusion rules (checked in this precedence order):
///
/// **(a) Non-Normal state (`gt_event.state != Normal`):**
/// States 1-5 (WorldInactive, WorldConditions, WorldNextphase, WorldFinished, Internal)
/// are NOT calendar-driven.  Internal is sticky via `StartInternalEvent`; world-phase
/// states are condition/timer-driven.  C++ `is_active` is authoritative for these.
///
/// **Critical:** The runtime state is read from `gt_event.state` (the C++ ground-truth
/// `obs.game_events` response), NOT from `row.state` (the SQL-derived `GameEventInput`).
/// The AC fork has no `state` column in `game_event`, so `From<SqlGameEventRow>` always
/// sets `GameEventInput.state = Normal`.  Reading `row.state` for this check would cause
/// NonNormalState to NEVER fire (live-caught bug, 2026-05-31).
///
/// **(b) Manual-start events:**
/// `state == Normal` + `holiday == 0` + `raw_start_time == raw_end_time` (both
/// `Some`, equal) AND `raw_end_time.unwrap() < resolve_ref`.
/// These are events started by GM command (`StartEvent overwrite=true` rebases
/// `Start → boot`).  The raw SQL start/end are non-null, equal, and far in the
/// past — the date math cannot predict whether they are currently running.
/// Live example: event 60 (raw start_time=Some(946735200), end_time=Some(946735200)).
///
/// Null/null events (raw_start_time=None, raw_end_time=None) are **in scope** —
/// they use C++ null-default math (start=0, end=resolve_ref+2yr) and ARE
/// date-schedulable.
///
/// ## Why reading gt_event.state is NOT a circularity violation
///
/// `state` is a structural classification of the event's activation mechanism
/// (calendar-driven vs. condition/GM-driven) — it is NOT a date-math answer
/// and NOT a prediction of `is_active`.  Reading it to decide WHETHER to run
/// our date-math check is scoping, not prediction.  The anti-circularity rule
/// applies to using `gt.events[].start/end/is_active` as compute inputs; `state`
/// is a different field with a different role.
///
/// ## Why scoping does NOT hide date-math bugs
///
/// Non-Normal events are not calendar-driven — their `is_active` is set by C++
/// world-phase logic or GM commands, not by `CheckOneGameEvent` date math.
/// If our date math had a bug affecting Normal events, it would still be caught
/// because Normal in-scope events ARE checked.  Excluding state!=0 events only
/// removes events whose `is_active` state is NOT produced by the date math we
/// are testing.
pub(crate) fn active_set_exclusion_reason(
    row: &GameEventInput,
    gt_event: &GroundTruthEvent,
    resolve_ref: i64,
) -> Option<ExclusionReason> {
    use crate::schedule::GameEventState;
    // (a) Non-Normal state: key off the C++ runtime state from the ground truth,
    //     NOT row.state (which is always Normal for SQL-derived rows in this fork).
    //     Check this FIRST so Internal events with start==end are labeled NonNormalState,
    //     not ManualStart.
    if gt_event.state != GameEventState::Normal {
        return Some(ExclusionReason::NonNormalState);
    }
    // (b) Manual-start: keys off RAW Option<i64> values — see is_manual_start.
    //     Only reached when gt_event.state == Normal.
    if is_manual_start(row, resolve_ref) {
        return Some(ExclusionReason::ManualStart);
    }
    None
}

/// Determine whether an event should be excluded from the resolution check (CHECK 2).
///
/// Returns `Some(reason)` if excluded, `None` if in-scope.
///
/// Only `ManualStart` events are excluded from resolution — their start/end are
/// GM-rebased and not date-predictable, so any resolution mismatch is expected.
/// NonNormalState events' resolution is kept IN scope: for Internal events C++
/// retains the raw loaded start/end, which the Rust resolver reproduces, so they
/// match; if an Internal event's resolution unexpectedly mismatches, that is a real
/// bug that should surface.
fn resolution_exclusion_reason(
    row: &GameEventInput,
    resolve_ref: i64,
) -> Option<ExclusionReason> {
    if is_manual_start(row, resolve_ref) {
        return Some(ExclusionReason::ManualStart);
    }
    None
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
/// - `live_shadow_resolution`: when `true`, CHECK 1 (DateMath) and CHECK 2 (Resolution)
///   are executed and their mismatches appear in `report.mismatches`.  When `false`
///   (the post-Inc-3 default), those checks are skipped — their `CheckResult` fields
///   remain at `{checked:0, matched:0}` and no `DateMath`/`Resolution` mismatches are
///   produced.
///
///   **Why this defaults to `false`:** the native C++ date resolver
///   (`SetHolidayEventTime`, `LoadHolidayDates`, `HolidayDateCalculator`) was deleted
///   in GES Inc-3.  The C++ `obs.game_events` adapter now reports raw/unresolved holiday
///   dates and event Start/End.  Diffing Rust's correct resolved values against C++ raw
///   values produces ~126 spurious mismatches (100 DateMath + 26 Resolution) on every
///   tick, making the report misleading.  The active_set check retains valid live ground
///   truth (the C++ active event list) and is the sole live invariant.  DateMath and
///   Resolution correctness are validated by deterministic unit tests (the ported
///   `HolidayDateCalculatorTest` vectors + forward-date probes) that do not depend on
///   live C++ state.
///
/// # Anti-circularity guarantee (unchanged)
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
    live_shadow_resolution: bool,
) -> ShadowReport {
    let mut date_math = CheckResult::new();
    let mut resolution = CheckResult::new();
    let mut active_set = CheckResult::new();
    let mut resolution_exclusions: Vec<ExcludedEvent> = Vec::new();
    let mut active_set_exclusions: Vec<ExcludedEvent> = Vec::new();
    let mut mismatches: Vec<Mismatch> = Vec::new();

    let gen_year = gen_year_from_unix(gt.server_gametime, gt.server_tz_offset_secs);

    // ── CHECK 1: DateMath ──────────────────────────────────────────────────────
    // Gated by `live_shadow_resolution`.  When false (post-Inc-3 default), the C++
    // date resolver has been deleted so the C++ holiday date dump is raw/unresolved —
    // there is no valid ground truth to diff against.  The correctness of this math
    // is covered by deterministic unit tests; skip the live diff to avoid spurious
    // mismatches in the report.
    if live_shadow_resolution {
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
        // --- Apply C++ null-timestamp semantics (FIX 1) ---
        // C++ LoadGameEvents lines 359-360: if end_time IS NULL, endtime = curTime + 63_072_000
        //   (63_072_000 = 730 * 86_400 = exactly 2 years).
        // C++ line 357: Get<uint64>() on null start → 0 (no null-guard).
        let row_start = effective_start(row.start_time);
        let row_end = effective_end(row.end_time, gt.resolve_reference_unixtime);

        // --- Rust resolution (CHECK 2 input) ---
        // For holiday events: use HolidaysEntry from gt.holidays as an INPUT.
        // The HolidaysEntry carries DBC data (date[], duration[], filter type, looping, region)
        // which are the inputs to SetHolidayEventTime.
        // CRITICALLY: we do NOT read gt.events[entry].start/end here — those are targets.
        let rust_resolved = if row.holiday != 0 {
            if let Some(hentry) = holidays_entry_for(row.holiday, &gt.holidays) {
                resolve_holiday_event(
                    row,
                    row_start,
                    row_end,
                    hentry,
                    gt.resolve_reference_unixtime,
                    gt.server_tz_offset_secs,
                )
            } else {
                // Holiday referenced by event but not in the dump — fallback to periodic.
                resolve_periodic_event(row, row_start, row_end)
            }
        } else {
            resolve_periodic_event(row, row_start, row_end)
        };

        let key = format!("event:{}", row.entry);

        // --- CHECK 2: Resolution ---
        // Gated by `live_shadow_resolution`.  When false (post-Inc-3 default), the C++
        // date resolver has been deleted so gt.events[].{start,end,occurence,length} are
        // raw/unresolved for holiday events — there is no valid ground truth to diff against.
        // Resolution correctness is covered by deterministic unit tests.
        if let Some(&cpp_ev) = gt_event_by_entry.get(&row.entry) {
            let is_holiday = row.holiday != 0;

            if live_shadow_resolution {
                match resolution_exclusion_reason(row, gt.resolve_reference_unixtime) {
                    Some(reason) => {
                        resolution_exclusions.push(ExcludedEvent { entry: row.entry, reason });
                    }
                    None => {
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
                    }
                }
            }
            // When live_shadow_resolution=false: skip CHECK 2 entirely — no entries in
            // resolution.checked / resolution_exclusions / mismatches[Resolution].

            // --- CHECK 3: ActiveSet ---
            // Scope: only check events where state==Normal (per C++ runtime state) AND
            // not a GM-started event.  Excluded events are logged to active_set_exclusions
            // but NOT counted in active_set.checked — their C++ is_active is authoritative.
            // NOTE: cpp_ev (the matched GroundTruthEvent) provides the runtime state;
            // row.state is always Normal for SQL-derived rows and MUST NOT be used here.
            match active_set_exclusion_reason(row, cpp_ev, gt.resolve_reference_unixtime) {
                Some(reason) => {
                    active_set_exclusions.push(ExcludedEvent { entry: row.entry, reason });
                }
                None => {
                    // In-scope: RUST computes is_active from the Rust-resolved event and server_gametime.
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
        resolution_exclusions,
        active_set_exclusions,
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

    /// Build a periodic `GameEventInput` with concrete (non-null) timestamps.
    fn make_raw_periodic(entry: u16, start: i64, end: i64, occ: i64, len: i64) -> GameEventInput {
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

    /// Build a holiday `GameEventInput` with concrete (non-null) timestamps.
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
            start_time: Some(start),
            end_time: Some(end),
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
        let s16 = effective_start(raw_input_16.start_time);
        let e16 = effective_end(raw_input_16.end_time, RESOLVE_REF);
        let rust_resolved_16 = resolve_holiday_event(
            &raw_input_16,
            s16,
            e16,
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
        let report = compute_report(&gt, &raw_events, rules, true);
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
        let report = compute_report(&gt, &raw_events, rules, true);

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
        let report = compute_report(&gt, &raw_events, rules, true);

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
        let report = compute_report(&gt, &raw_events, rules, true);

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
        let report = compute_report(&gt, &raw_events, rules, true);

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
        let report = compute_report(&gt, &raw_events, rules, true);

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
        let report = compute_report(&gt, &raw_events, rules, true);

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
        let report = compute_report(&gt, &raw_events, rules, true);
        assert_eq!(report.server_gametime, SERVER_GAMETIME);
        assert_eq!(report.resolve_reference_unixtime, RESOLVE_REF);
    }

    // ── Test: ShadowReport is serde::Serialize (compile-time) ────────────────

    #[test]
    fn shadow_report_is_serializable() {
        let (gt, raw_events) = build_consistent_gt_and_raw(None);
        let rules = holiday_rules();
        let report = compute_report(&gt, &raw_events, rules, true);
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
        let report = compute_report(&gt, &raw_events, rules, true);

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

    // ── FIX 2 tests: active-set scoping ──────────────────────────────────────
    //
    // These tests verify that:
    //  (a) Internal (state=5) events are EXCLUDED from the active-set check,
    //      regardless of C++ is_active.
    //  (b) Manual-start events (state=Normal, holiday=0, raw_start==raw_end<resolve_ref)
    //      are EXCLUDED from the active-set check.
    //  (c) The excluded events do NOT inflate active_set.checked.
    //  (d) All-in-scope-consistent fixtures still yield active_set.matched==active_set.checked.
    //  (e) active_set_exclusions list is populated with the right entries and reasons.

    /// Build a `GroundTruthEvent` with a non-Normal state.
    fn make_gt_non_normal_event(
        entry: u16,
        start: i64,
        end: i64,
        state: GameEventState,
        is_active: bool,
    ) -> crate::events::GroundTruthEvent {
        use crate::events::GroundTruthEvent;
        GroundTruthEvent {
            entry,
            start,
            end,
            occurence: 525_600,
            length: 20_160,
            holiday: 0,
            holiday_stage: 0,
            is_active,
            next_start: 0,
            state,
        }
    }

    /// Build a `GameEventInput` with a given state (for non-Normal states in raw rows).
    fn make_raw_with_state(
        entry: u16,
        start: Option<i64>,
        end: Option<i64>,
        state: GameEventState,
    ) -> GameEventInput {
        GameEventInput {
            entry,
            start_time: start,
            end_time: end,
            occurence: 525_600,
            length: 20_160,
            holiday: 0,
            holiday_stage: 0,
            state,
            next_start: 0,
        }
    }

    // ── (a) Internal event → EXCLUDED, not a mismatch ─────────────────────────

    #[test]
    fn internal_event_is_excluded_not_mismatch() {
        // Event 50: state=Internal (5), C++ reports is_active=true.
        // Rust date math would compute is_active=false (start=0, end=resolve_ref-1 → outdated).
        // Without scoping → this would be a mismatch.
        // With scoping → it should be EXCLUDED; active_set.checked does NOT include it.
        let start_50: i64 = 946_735_200; // far past (2000-01-01 approx)
        let end_50: i64 = 946_735_200 + 86_400; // one day later, still in the past vs RESOLVE_REF

        let gt = GroundTruth {
            events: vec![
                make_gt_non_normal_event(50, start_50, end_50, GameEventState::Internal, true),
            ],
            active_event_list: vec![50],
            holidays: vec![],
            server_gametime: SERVER_GAMETIME,
            resolve_reference_unixtime: RESOLVE_REF,
            server_tz_offset_secs: 0,
        };

        let raw_events = vec![
            make_raw_with_state(50, Some(start_50), Some(end_50), GameEventState::Internal),
        ];

        let rules = holiday_rules();
        let report = compute_report(&gt, &raw_events, rules, true);

        // active_set.checked must be 0 (excluded, not checked)
        assert_eq!(
            report.active_set.checked, 0,
            "Internal event must NOT contribute to active_set.checked"
        );

        // active_set.mismatches() must be 0
        assert_eq!(
            report.active_set.mismatches(), 0,
            "Internal event (excluded) must not produce any mismatch"
        );

        // The exclusion list must contain event 50 with NonNormalState reason
        assert_eq!(report.active_set_exclusions.len(), 1, "one exclusion expected");
        assert_eq!(report.active_set_exclusions[0].entry, 50);
        assert!(
            matches!(report.active_set_exclusions[0].reason, ExclusionReason::NonNormalState),
            "reason must be NonNormalState for Internal state"
        );
    }

    // ── (b) Manual-start event → EXCLUDED, not a mismatch ─────────────────────

    #[test]
    fn manual_start_event_is_excluded_not_mismatch() {
        // Event 60: state=Normal, holiday=0, raw_start == raw_end == 946735200 (far past < resolve_ref).
        // C++ reports is_active=true (GM started it). Rust date math: start < server_gametime? No —
        // start < end? end == start → no time range, would be inactive. But the key point: excluded.
        let gm_ts: i64 = 946_735_200; // Jan 1 2000 — far before RESOLVE_REF (~2026)

        let gt = GroundTruth {
            events: vec![
                make_gt_event(60, gm_ts, gm_ts, 525_600, 20_160, true), // C++ says active
            ],
            active_event_list: vec![60],
            holidays: vec![],
            server_gametime: SERVER_GAMETIME,
            resolve_reference_unixtime: RESOLVE_REF,
            server_tz_offset_secs: 0,
        };

        // raw row: start_time == end_time == gm_ts (far past, both equal)
        let raw_events = vec![
            make_raw_periodic(60, gm_ts, gm_ts, 525_600, 20_160),
        ];

        let rules = holiday_rules();
        let report = compute_report(&gt, &raw_events, rules, true);

        // active_set.checked must be 0 (excluded)
        assert_eq!(
            report.active_set.checked, 0,
            "manual-start event must NOT contribute to active_set.checked"
        );

        assert_eq!(
            report.active_set.mismatches(), 0,
            "manual-start event (excluded) must not produce any mismatch"
        );

        // The exclusion list must contain event 60 with ManualStart reason
        assert_eq!(report.active_set_exclusions.len(), 1, "one exclusion expected");
        assert_eq!(report.active_set_exclusions[0].entry, 60);
        assert!(
            matches!(report.active_set_exclusions[0].reason, ExclusionReason::ManualStart),
            "reason must be ManualStart for event with raw_start == raw_end < resolve_ref"
        );
    }

    // ── (c) Mixed fixture: excluded + in-scope events; excluded not counted ─────

    #[test]
    fn excluded_events_not_counted_in_active_set_checked() {
        // Fixture: event 10 (in-scope, active), event 50 (Internal, excluded).
        // active_set.checked should count only event 10 (×2 for vs_field + vs_list = 2).
        let (mut gt, mut raw_events) = build_consistent_gt_and_raw(None);

        // Add event 50 (Internal, C++-active) to gt.events and raw_events
        let start_50: i64 = SERVER_GAMETIME - 3600;
        let end_50: i64 = SERVER_GAMETIME + 86400;
        gt.events.push(make_gt_non_normal_event(50, start_50, end_50, GameEventState::Internal, true));
        gt.active_event_list.push(50);
        raw_events.push(make_raw_with_state(50, Some(start_50), Some(end_50), GameEventState::Internal));

        let rules = holiday_rules();
        let report = compute_report(&gt, &raw_events, rules, true);

        // active_set_exclusions must contain event 50
        let excluded_50 = report.active_set_exclusions.iter().find(|e| e.entry == 50);
        assert!(excluded_50.is_some(), "event 50 must be in active_set_exclusions");

        // active_set.mismatches() must be 0 (all in-scope events are consistent)
        assert_eq!(
            report.active_set.mismatches(), 0,
            "no in-scope mismatches when only excluded events would have caused mismatch"
        );

        // active_set.checked must NOT count event 50
        // The in-scope events are 10, 11, 16 → 3 events × 2 checks (vs_field + vs_list) = 6
        assert_eq!(
            report.active_set.checked, 6,
            "active_set.checked must be 6 (3 in-scope events × 2 comparisons each)"
        );
    }

    // ── (d) All-in-scope-consistent: matched == checked; exclusions separate ────

    #[test]
    fn all_in_scope_consistent_matched_equals_checked() {
        // The standard consistent fixture has no excluded events — all are Normal
        // with non-equal start/end relative to RESOLVE_REF.
        let (gt, raw_events) = build_consistent_gt_and_raw(None);
        let rules = holiday_rules();
        let report = compute_report(&gt, &raw_events, rules, true);

        assert_eq!(
            report.active_set.matched, report.active_set.checked,
            "all-consistent: matched must equal checked when no mismatches"
        );
        assert_eq!(
            report.active_set_exclusions.len(), 0,
            "all-consistent standard fixture has no excluded events"
        );
    }

    // ── (e) WorldInactive (state=1) → excluded as NonNormalState ─────────────

    #[test]
    fn world_inactive_state_excluded_as_non_normal() {
        let start_70: i64 = SERVER_GAMETIME - 3600;
        let end_70: i64 = SERVER_GAMETIME + 86400;
        let gt = GroundTruth {
            events: vec![
                make_gt_non_normal_event(70, start_70, end_70, GameEventState::WorldInactive, false),
            ],
            active_event_list: vec![],
            holidays: vec![],
            server_gametime: SERVER_GAMETIME,
            resolve_reference_unixtime: RESOLVE_REF,
            server_tz_offset_secs: 0,
        };
        let raw_events = vec![
            make_raw_with_state(70, Some(start_70), Some(end_70), GameEventState::WorldInactive),
        ];
        let rules = holiday_rules();
        let report = compute_report(&gt, &raw_events, rules, true);

        assert_eq!(report.active_set.checked, 0, "WorldInactive event must not contribute to checked");
        assert_eq!(report.active_set_exclusions.len(), 1);
        assert!(
            matches!(report.active_set_exclusions[0].reason, ExclusionReason::NonNormalState),
            "WorldInactive must be excluded as NonNormalState"
        );
    }

    // ── NEW (e2) Internal event with DISTINCT start/end → NonNormalState (not mismatch) ──
    //
    // This is the regression test for the live-caught bug (2026-05-31):
    // Before the fix, `active_set_exclusion_reason` checked `row.state` (always Normal
    // from SQL), so this Internal event with distinct start/end would NOT be excluded
    // and would produce a mismatch (Rust computes inactive from date math; C++ says active).
    // After the fix, it checks `gt_event.state` (Internal=5) and correctly excludes it
    // as NonNormalState — even though its start != end (no ManualStart coincidence to rely on).

    #[test]
    fn internal_event_distinct_start_end_excluded_as_non_normal_state() {
        // Event 55: state=Internal (5), C++ reports is_active=true.
        // start != end (distinct timestamps) — so the ManualStart coincidence is absent.
        // Rust date math would compute is_active=false (end is past relative to RESOLVE_REF).
        // Before the fix: not excluded → mismatch. After the fix: excluded as NonNormalState.
        let start_55: i64 = SERVER_GAMETIME - 3600;   // 1 hour ago
        let end_55: i64 = SERVER_GAMETIME - 1800;     // 30 minutes ago (past → Rust inactive)

        let gt = GroundTruth {
            events: vec![
                make_gt_non_normal_event(55, start_55, end_55, GameEventState::Internal, true),
            ],
            active_event_list: vec![55],  // C++ says active (Internal sticky)
            holidays: vec![],
            server_gametime: SERVER_GAMETIME,
            resolve_reference_unixtime: RESOLVE_REF,
            server_tz_offset_secs: 0,
        };

        // Raw row has start != end and state=Internal (but SQL-derived rows always have
        // state=Normal — that was the bug: the old code checked row.state here).
        let raw_events = vec![
            make_raw_with_state(55, Some(start_55), Some(end_55), GameEventState::Internal),
        ];

        let rules = holiday_rules();
        let report = compute_report(&gt, &raw_events, rules, true);

        // Must be excluded — NOT checked in active_set
        assert_eq!(
            report.active_set.checked, 0,
            "Internal event with distinct start/end must NOT contribute to active_set.checked (regression: previously mismatched)"
        );

        // Must be zero mismatches
        assert_eq!(
            report.active_set.mismatches(), 0,
            "Internal event with distinct start/end must not produce any mismatch"
        );

        // Exclusion list must contain entry 55 as NonNormalState
        assert_eq!(report.active_set_exclusions.len(), 1, "one exclusion expected");
        assert_eq!(report.active_set_exclusions[0].entry, 55);
        assert!(
            matches!(report.active_set_exclusions[0].reason, ExclusionReason::NonNormalState),
            "reason must be NonNormalState (not ManualStart): {:?}",
            report.active_set_exclusions[0].reason
        );
    }

    // ── NEW (e3) Internal event WITH start==end → NonNormalState (not ManualStart) ──
    //
    // Verifies the precedence rule: NonNormalState is checked BEFORE ManualStart.
    // An Internal event that also satisfies the ManualStart criteria (start==end, past)
    // must be labeled NonNormalState, not ManualStart, because state!=Normal fires first.

    #[test]
    fn internal_event_equal_start_end_labeled_non_normal_not_manual_start() {
        // Event 56: state=Internal (5), raw_start==raw_end (past) — satisfies both
        // NonNormalState AND ManualStart criteria on paper.
        // Precedence: NonNormalState fires first → reason must be NonNormalState.
        let gm_ts: i64 = 946_735_200; // far past (2000-01-01 approx)

        let gt = GroundTruth {
            events: vec![
                make_gt_non_normal_event(56, gm_ts, gm_ts, GameEventState::Internal, true),
            ],
            active_event_list: vec![56],
            holidays: vec![],
            server_gametime: SERVER_GAMETIME,
            resolve_reference_unixtime: RESOLVE_REF,
            server_tz_offset_secs: 0,
        };

        // Raw row: start==end (would satisfy ManualStart if state were Normal).
        let raw_events = vec![
            make_raw_with_state(56, Some(gm_ts), Some(gm_ts), GameEventState::Internal),
        ];

        let rules = holiday_rules();
        let report = compute_report(&gt, &raw_events, rules, true);

        // Must be excluded
        assert_eq!(
            report.active_set.checked, 0,
            "Internal event (start==end) must not contribute to active_set.checked"
        );
        assert_eq!(report.active_set.mismatches(), 0, "no mismatches expected");
        assert_eq!(report.active_set_exclusions.len(), 1, "one exclusion expected");
        assert_eq!(report.active_set_exclusions[0].entry, 56);

        // Must be NonNormalState, NOT ManualStart — precedence check
        assert!(
            matches!(report.active_set_exclusions[0].reason, ExclusionReason::NonNormalState),
            "Internal state must produce NonNormalState exclusion, not ManualStart: {:?}",
            report.active_set_exclusions[0].reason
        );
        assert!(
            !matches!(report.active_set_exclusions[0].reason, ExclusionReason::ManualStart),
            "reason must NOT be ManualStart for Internal state even when start==end"
        );
    }

    // ── (f) normal event with start != end → not excluded (in-scope) ─────────

    #[test]
    fn normal_event_with_distinct_start_end_is_in_scope() {
        // A Normal event where start != end is NOT excluded (even if start < resolve_ref).
        // This verifies the manual-start exclusion does not false-positive.
        let (gt, raw_events) = build_consistent_gt_and_raw(None);
        let rules = holiday_rules();
        let report = compute_report(&gt, &raw_events, rules, true);

        // No exclusions expected for the standard fixture
        let excluded_10 = report.active_set_exclusions.iter().find(|e| e.entry == 10);
        assert!(excluded_10.is_none(), "event 10 (Normal, start != end) must NOT be excluded");
        let excluded_11 = report.active_set_exclusions.iter().find(|e| e.entry == 11);
        assert!(excluded_11.is_none(), "event 11 (Normal, start != end) must NOT be excluded");
    }

    // ── (g) ShadowReport serialization includes active_set_exclusions ────────

    #[test]
    fn shadow_report_serialization_includes_exclusions_field() {
        let (gt, raw_events) = build_consistent_gt_and_raw(None);
        let rules = holiday_rules();
        let report = compute_report(&gt, &raw_events, rules, true);
        let json = serde_json::to_string(&report).expect("must serialize");
        assert!(
            json.contains("active_set_exclusions"),
            "serialized JSON must contain active_set_exclusions field"
        );
        assert!(
            json.contains("resolution_exclusions"),
            "serialized JSON must contain resolution_exclusions field"
        );
    }

    // ── ManualStart predicate: null/null is NOT ManualStart ───────────────────
    //
    // A null/null event (raw start_time=None, end_time=None) must NOT be excluded.
    // With effective_* applied: start=0, end=resolve_ref+63072000 — the event is
    // date-scheduled and must participate in active-set and resolution checking.

    /// Build a `GameEventInput` with null start_time AND null end_time.
    fn make_raw_null_null(entry: u16) -> GameEventInput {
        GameEventInput {
            entry,
            start_time: None,
            end_time: None,
            occurence: 525_600,
            length: 20_160,
            holiday: 0,
            holiday_stage: 0,
            state: GameEventState::Normal,
            next_start: 0,
        }
    }

    // ── (h) null/null event is in-scope (NOT ManualStart) ─────────────────────

    #[test]
    fn null_null_event_is_not_manual_start_stays_in_scope() {
        // Event 80: Normal, holiday=0, raw start_time=None, end_time=None.
        // effective_start(None)=0, effective_end(None, RESOLVE_REF)=RESOLVE_REF+63_072_000.
        // is_active: start=0 < SERVER_GAMETIME, so elapsed = SERVER_GAMETIME - 0.
        // elapsed % (525600*60) and length*60: with occ=525600min, len=20160min...
        // occ_secs=31536000, elapsed=SERVER_GAMETIME=1780394400 >> 31536000.
        // elapsed % 31536000 = 1780394400 % 31536000. Let's compute:
        // 1780394400 / 31536000 ≈ 56.45. remainder ≈ 0.45 * 31536000 ≈ 14191200.
        // len_secs = 20160 * 60 = 1209600. 14191200 > 1209600 → INACTIVE.
        // The point of the test is that it is IN SCOPE (checked), not its activity value.
        // We set the C++ ground truth to match Rust's computation.
        let raw = make_raw_null_null(80);
        let row_start = crate::resolve::effective_start(raw.start_time);
        let row_end = crate::resolve::effective_end(raw.end_time, RESOLVE_REF);
        let rust_resolved = crate::resolve::resolve_periodic_event(&raw, row_start, row_end);
        let rust_active = crate::schedule::is_active(&rust_resolved, SERVER_GAMETIME, |_| None);

        let gt = GroundTruth {
            events: vec![make_gt_event(80, rust_resolved.start, rust_resolved.end, rust_resolved.occurence, rust_resolved.length, rust_active)],
            active_event_list: if rust_active { vec![80] } else { vec![] },
            holidays: vec![],
            server_gametime: SERVER_GAMETIME,
            resolve_reference_unixtime: RESOLVE_REF,
            server_tz_offset_secs: 0,
        };
        let raw_events = vec![raw];

        let rules = holiday_rules();
        let report = compute_report(&gt, &raw_events, rules, true);

        // Must NOT be in active_set_exclusions
        let excluded_80 = report.active_set_exclusions.iter().find(|e| e.entry == 80);
        assert!(excluded_80.is_none(), "null/null event must NOT be excluded from active-set");

        // Must NOT be in resolution_exclusions
        let res_excluded_80 = report.resolution_exclusions.iter().find(|e| e.entry == 80);
        assert!(res_excluded_80.is_none(), "null/null event must NOT be excluded from resolution");

        // Must contribute to active_set.checked (×2 for vs_field + vs_list)
        assert!(report.active_set.checked >= 2, "null/null event must be counted in active_set.checked");

        // Must have zero active_set mismatches (we set gt to match Rust)
        assert_eq!(report.active_set.mismatches(), 0, "null/null event with matching gt must have zero active_set mismatches");
    }

    // ── (i) null/null event that is date-active is checked and matches ─────────

    #[test]
    fn null_null_event_date_active_is_checked_and_matches() {
        // Craft a null/null event that is ACTIVE at SERVER_GAMETIME.
        // effective_start = 0. We need elapsed % occ_secs < len_secs.
        // Use occ=YEAR (525600 min, 31536000 sec), len=len such that the
        // remainder from SERVER_GAMETIME is within it.
        // elapsed_in_period = SERVER_GAMETIME % 31536000 = 1780394400 % 31536000.
        // 1780394400 / 31536000 = 56 remainder = 1780394400 - 56*31536000 = 1780394400 - 1766016000 = 14378400.
        // So we need length * 60 > 14378400, i.e., length > 239640 minutes = ~166.4 days.
        // Use length = 400000 minutes (> 277 days) so elapsed_in_period < len_secs.
        let raw = GameEventInput {
            entry: 81,
            start_time: None,
            end_time: None,
            occurence: 525_600,
            length: 400_000,   // huge window → active
            holiday: 0,
            holiday_stage: 0,
            state: GameEventState::Normal,
            next_start: 0,
        };
        let row_start = crate::resolve::effective_start(raw.start_time);
        let row_end = crate::resolve::effective_end(raw.end_time, RESOLVE_REF);
        let rust_resolved = crate::resolve::resolve_periodic_event(&raw, row_start, row_end);
        let rust_active = crate::schedule::is_active(&rust_resolved, SERVER_GAMETIME, |_| None);

        // With length=400000min (> 239640 remainder), Rust should compute is_active=true.
        assert!(rust_active, "fixture design: null/null event with length=400000 should be active at SERVER_GAMETIME");

        let gt = GroundTruth {
            events: vec![make_gt_event(81, rust_resolved.start, rust_resolved.end, rust_resolved.occurence, rust_resolved.length, true)],
            active_event_list: vec![81],
            holidays: vec![],
            server_gametime: SERVER_GAMETIME,
            resolve_reference_unixtime: RESOLVE_REF,
            server_tz_offset_secs: 0,
        };
        let raw_events = vec![raw];

        let rules = holiday_rules();
        let report = compute_report(&gt, &raw_events, rules, true);

        // Active-set: event 81 must be checked (×2) and have zero mismatches
        assert!(report.active_set.checked >= 2, "null/null active event must contribute to active_set.checked");
        assert_eq!(report.active_set.mismatches(), 0, "null/null active event with matching gt must have zero mismatches");

        // Not in any exclusion list
        assert!(!report.active_set_exclusions.iter().any(|e| e.entry == 81), "event 81 must not be in active_set_exclusions");
        assert!(!report.resolution_exclusions.iter().any(|e| e.entry == 81), "event 81 must not be in resolution_exclusions");
    }

    // ── (j) ManualStart excluded from BOTH active-set AND resolution ──────────

    #[test]
    fn manual_start_excluded_from_both_active_set_and_resolution() {
        // Event 60: state=Normal, holiday=0, raw start_time=Some(gm_ts), end_time=Some(gm_ts),
        // both far past (946735200 << RESOLVE_REF ~1780394400). → ManualStart.
        // C++ says is_active=true AND reports mismatched start/end in resolution.
        // Both checks should exclude this event — not count it as a mismatch.
        let gm_ts: i64 = 946_735_200;

        // Give the gt event a different start/end from what Rust would compute,
        // to confirm that resolution does NOT fire a mismatch (it's excluded).
        let cpp_start = gm_ts + 1000; // deliberately different from Rust's effective_start
        let cpp_end   = gm_ts + 2000; // deliberately different from Rust's effective_end

        let gt = GroundTruth {
            events: vec![
                make_gt_event(60, cpp_start, cpp_end, 525_600, 20_160, true),
            ],
            active_event_list: vec![60],
            holidays: vec![],
            server_gametime: SERVER_GAMETIME,
            resolve_reference_unixtime: RESOLVE_REF,
            server_tz_offset_secs: 0,
        };

        // raw row: both Some, equal, < RESOLVE_REF → ManualStart
        let raw_events = vec![
            make_raw_periodic(60, gm_ts, gm_ts, 525_600, 20_160),
        ];

        let rules = holiday_rules();
        let report = compute_report(&gt, &raw_events, rules, true);

        // active_set: excluded, not checked
        assert_eq!(report.active_set.checked, 0, "ManualStart must not contribute to active_set.checked");
        assert_eq!(report.active_set.mismatches(), 0, "ManualStart must not produce active_set mismatch");
        assert_eq!(report.active_set_exclusions.len(), 1, "one active_set exclusion expected");
        assert_eq!(report.active_set_exclusions[0].entry, 60);
        assert!(
            matches!(report.active_set_exclusions[0].reason, ExclusionReason::ManualStart),
            "active_set exclusion reason must be ManualStart"
        );

        // resolution: excluded, not checked, NO mismatch despite deliberately different start/end in gt
        assert_eq!(report.resolution.checked, 0, "ManualStart must not contribute to resolution.checked");
        assert_eq!(report.resolution.mismatches(), 0, "ManualStart must not produce resolution mismatch (even though gt start/end differ)");
        assert_eq!(report.resolution_exclusions.len(), 1, "one resolution exclusion expected");
        assert_eq!(report.resolution_exclusions[0].entry, 60);
        assert!(
            matches!(report.resolution_exclusions[0].reason, ExclusionReason::ManualStart),
            "resolution exclusion reason must be ManualStart"
        );

        // No mismatches of any kind
        assert!(report.mismatches.is_empty(), "no mismatches for a ManualStart-only fixture");
    }

    // ── (k) Some(a) start with None end is NOT ManualStart ───────────────────

    #[test]
    fn some_start_none_end_is_not_manual_start() {
        // An event with a real start_time but null end_time is NOT ManualStart.
        // The null end gets materialized to resolve_ref+2yr and the event is
        // date-scheduled.
        let real_start: i64 = SERVER_GAMETIME - 3600; // 1 hour ago
        let raw = GameEventInput {
            entry: 82,
            start_time: Some(real_start),
            end_time: None,
            occurence: 525_600,
            length: 20_160,
            holiday: 0,
            holiday_stage: 0,
            state: GameEventState::Normal,
            next_start: 0,
        };

        let row_start = crate::resolve::effective_start(raw.start_time);
        let row_end = crate::resolve::effective_end(raw.end_time, RESOLVE_REF);
        let rust_resolved = crate::resolve::resolve_periodic_event(&raw, row_start, row_end);
        let rust_active = crate::schedule::is_active(&rust_resolved, SERVER_GAMETIME, |_| None);

        let gt = GroundTruth {
            events: vec![make_gt_event(82, rust_resolved.start, rust_resolved.end, rust_resolved.occurence, rust_resolved.length, rust_active)],
            active_event_list: if rust_active { vec![82] } else { vec![] },
            holidays: vec![],
            server_gametime: SERVER_GAMETIME,
            resolve_reference_unixtime: RESOLVE_REF,
            server_tz_offset_secs: 0,
        };
        let raw_events = vec![raw];

        let rules = holiday_rules();
        let report = compute_report(&gt, &raw_events, rules, true);

        // Must NOT be excluded
        assert!(!report.active_set_exclusions.iter().any(|e| e.entry == 82),
            "Some(start)+None(end) event must NOT be ManualStart-excluded from active_set");
        assert!(!report.resolution_exclusions.iter().any(|e| e.entry == 82),
            "Some(start)+None(end) event must NOT be ManualStart-excluded from resolution");

        // Must be checked in active_set
        assert!(report.active_set.checked >= 2, "Some(start)+None(end) event must be counted in active_set.checked");
        assert_eq!(report.active_set.mismatches(), 0, "matching gt must yield zero mismatches");
    }

    // ── (l) ManualStart: None+None → not excluded; Some(t)/Some(t) past → excluded ──

    #[test]
    fn manual_start_requires_both_some_and_equal_and_past() {
        // Three events tested in isolation via is_manual_start:
        //   event A: None/None → NOT ManualStart (date-scheduled)
        //   event B: Some(t)/Some(t), t < RESOLVE_REF → ManualStart
        //   event C: Some(t)/Some(t+1), t < RESOLVE_REF → NOT ManualStart (unequal)

        let gm_ts: i64 = 946_735_200;

        // Event A: null/null → in-scope
        let ev_a = make_raw_null_null(90);
        assert!(!is_manual_start(&ev_a, RESOLVE_REF), "null/null must NOT be ManualStart");

        // Event B: Some(gm_ts)/Some(gm_ts) → ManualStart
        let ev_b = make_raw_periodic(91, gm_ts, gm_ts, 525_600, 20_160);
        assert!(is_manual_start(&ev_b, RESOLVE_REF), "Some(t)==Some(t), t<ref must be ManualStart");

        // Event C: Some(gm_ts)/Some(gm_ts+1) → NOT ManualStart (unequal)
        let ev_c = make_raw_periodic(92, gm_ts, gm_ts + 1, 525_600, 20_160);
        assert!(!is_manual_start(&ev_c, RESOLVE_REF), "Some(t)!=Some(t+1) must NOT be ManualStart");

        // Event D: Some(gm_ts)/None → NOT ManualStart
        let ev_d = GameEventInput {
            entry: 93,
            start_time: Some(gm_ts),
            end_time: None,
            occurence: 525_600,
            length: 20_160,
            holiday: 0,
            holiday_stage: 0,
            state: GameEventState::Normal,
            next_start: 0,
        };
        assert!(!is_manual_start(&ev_d, RESOLVE_REF), "Some(start)+None(end) must NOT be ManualStart");

        // Event E: None/Some(gm_ts) → NOT ManualStart
        let ev_e = GameEventInput {
            entry: 94,
            start_time: None,
            end_time: Some(gm_ts),
            occurence: 525_600,
            length: 20_160,
            holiday: 0,
            holiday_stage: 0,
            state: GameEventState::Normal,
            next_start: 0,
        };
        assert!(!is_manual_start(&ev_e, RESOLVE_REF), "None(start)+Some(end) must NOT be ManualStart");

        // Event F: Some(gm_ts)/Some(gm_ts), holiday != 0 → NOT ManualStart
        let ev_f = GameEventInput {
            entry: 95,
            start_time: Some(gm_ts),
            end_time: Some(gm_ts),
            occurence: 525_600,
            length: 20_160,
            holiday: 62,   // has a holiday → not manual-start
            holiday_stage: 1,
            state: GameEventState::Normal,
            next_start: 0,
        };
        assert!(!is_manual_start(&ev_f, RESOLVE_REF), "holiday event must NOT be ManualStart even if start==end");
    }

    // ── live_shadow_resolution=false gate tests ───────────────────────────────
    //
    // Post-GES-Inc-3: the native C++ date resolver (SetHolidayEventTime /
    // LoadHolidayDates / HolidayDateCalculator) was deleted.  The C++ ground
    // truth for holiday dates and resolved event Start/End is now raw/unresolved.
    // These tests verify:
    //
    //  (m) When live_shadow_resolution=false, a raw-vs-resolved DateMath difference
    //      does NOT produce a mismatch (the check is skipped entirely).
    //  (n) When live_shadow_resolution=false, a raw-vs-resolved Resolution difference
    //      does NOT produce a mismatch.
    //  (o) When live_shadow_resolution=false, an active_set divergence still fires.
    //  (p) When live_shadow_resolution=false, date_math and resolution CheckResults
    //      remain at {checked:0, matched:0} even with corrupted C++ ground truth.
    //  (q) When live_shadow_resolution=true, the same corrupted data produces mismatches
    //      (confirming the gate actually controls behaviour).

    // ── (m) date_math corruption suppressed when gate=false ──────────────────

    #[test]
    fn date_math_corruption_suppressed_when_gate_off() {
        // Use the standard consistent fixture but corrupt date[1] for holiday 324.
        let (mut gt, raw_events) = build_consistent_gt_and_raw(None);
        let h324 = gt.holidays.iter_mut().find(|h| h.holiday_id == 324).unwrap();
        h324.date[1] = 0xDEADBEEF; // injected wrong value — would fire when gate=true

        let rules = holiday_rules();
        // Gate OFF (post-Inc-3 default): spurious C++ raw data must not produce mismatches.
        let report = compute_report(&gt, &raw_events, rules, false);

        // No DateMath mismatches
        let date_mismatches: Vec<_> = report
            .mismatches
            .iter()
            .filter(|m| matches!(m.kind, CheckKind::DateMath))
            .collect();
        assert!(
            date_mismatches.is_empty(),
            "DateMath mismatches must be suppressed when live_shadow_resolution=false: {:?}",
            date_mismatches
        );

        // date_math CheckResult stays at {0, 0}
        assert_eq!(
            report.date_math.checked, 0,
            "date_math.checked must be 0 when live_shadow_resolution=false"
        );
        assert_eq!(
            report.date_math.matched, 0,
            "date_math.matched must be 0 when live_shadow_resolution=false"
        );
    }

    // ── (n) resolution corruption suppressed when gate=false ─────────────────

    #[test]
    fn resolution_corruption_suppressed_when_gate_off() {
        // Corrupt the C++ start value for event 10 (periodic, in-scope).
        let (mut gt, raw_events) = build_consistent_gt_and_raw(None);
        let ev10 = gt.events.iter_mut().find(|e| e.entry == 10).unwrap();
        ev10.start = ev10.start + 99_999; // deliberate mismatch vs Rust-computed start

        let rules = holiday_rules();
        // Gate OFF: resolution diff must be suppressed.
        let report = compute_report(&gt, &raw_events, rules, false);

        // No Resolution mismatches
        let res_mismatches: Vec<_> = report
            .mismatches
            .iter()
            .filter(|m| matches!(m.kind, CheckKind::Resolution))
            .collect();
        assert!(
            res_mismatches.is_empty(),
            "Resolution mismatches must be suppressed when live_shadow_resolution=false: {:?}",
            res_mismatches
        );

        // resolution CheckResult stays at {0, 0}
        assert_eq!(
            report.resolution.checked, 0,
            "resolution.checked must be 0 when live_shadow_resolution=false"
        );
        assert_eq!(
            report.resolution.matched, 0,
            "resolution.matched must be 0 when live_shadow_resolution=false"
        );
    }

    // ── (o) active_set divergence still fires when gate=false ─────────────────

    #[test]
    fn active_set_mismatch_still_fires_when_gate_off() {
        // Flip event 10's C++ is_active to produce a divergence.
        let (mut gt, raw_events) = build_consistent_gt_and_raw(None);
        let ev10 = gt.events.iter_mut().find(|e| e.entry == 10).unwrap();
        ev10.is_active = false; // injected lie — Rust computes true

        let rules = holiday_rules();
        // Gate OFF: active_set check must still run and detect the mismatch.
        let report = compute_report(&gt, &raw_events, rules, false);

        let active_mismatches: Vec<_> = report
            .mismatches
            .iter()
            .filter(|m| matches!(m.kind, CheckKind::ActiveSet))
            .collect();
        assert!(
            !active_mismatches.is_empty(),
            "active_set mismatch must still be recorded even when live_shadow_resolution=false"
        );
        assert!(
            report.active_set.mismatches() > 0,
            "active_set.mismatches() must be > 0"
        );
    }

    // ── (p) gate=false → date_math and resolution stay {0,0} even with corrupt data ──

    #[test]
    fn gate_off_leaves_date_math_and_resolution_zeroed() {
        // Corrupt BOTH date_math and resolution ground truth.
        let (mut gt, raw_events) = build_consistent_gt_and_raw(None);
        // Corrupt holiday 324 dates
        let h324 = gt.holidays.iter_mut().find(|h| h.holiday_id == 324).unwrap();
        h324.date[0] = 0x11111111;
        h324.date[1] = 0x22222222;
        h324.date[2] = 0x33333333;
        // Corrupt event 10 resolution
        let ev10 = gt.events.iter_mut().find(|e| e.entry == 10).unwrap();
        ev10.start = 0;
        ev10.end = 1;
        ev10.occurence = 1;
        ev10.length = 1;

        let rules = holiday_rules();
        let report = compute_report(&gt, &raw_events, rules, false);

        assert_eq!(report.date_math.checked, 0, "date_math.checked must remain 0");
        assert_eq!(report.date_math.matched, 0, "date_math.matched must remain 0");
        assert_eq!(report.resolution.checked, 0, "resolution.checked must remain 0");
        assert_eq!(report.resolution.matched, 0, "resolution.matched must remain 0");

        // Only active_set mismatches may appear (event 10's is_active is still correct here
        // since we only corrupted start/end/occ/len — is_active is still the right value
        // matching Rust's computation from the raw event… actually since we corrupted
        // ev10.start/end this means the gt event start/end are wrong but we're not using
        // those for is_active in CHECK 3 — Rust uses raw_events.  So active_set should be fine.)
        // Active_set check computes Rust is_active from raw_events (not from gt.events[].start/end),
        // so it is unaffected by the ev10 start/end corruption.
        // The is_active field on ev10 is still 'true' (from is_active_10=true above).
        // So active_set.mismatches() should still be 0.
        assert_eq!(
            report.active_set.mismatches(), 0,
            "active_set.mismatches() must be 0 when active_set ground truth is untouched"
        );

        // Total mismatches in the report: only from active_set (which is 0 here).
        assert_eq!(
            report.mismatches.len(), 0,
            "report.mismatches must be empty when only date_math/resolution ground truth is corrupted and gate=false"
        );
    }

    // ── (q) gate=true with same corrupted data → mismatches fire ─────────────

    #[test]
    fn gate_on_with_corrupt_data_fires_mismatches() {
        // Confirming the gate is actually what suppresses: same corruption as (m),
        // but gate=true → DateMath mismatches appear.
        let (mut gt, raw_events) = build_consistent_gt_and_raw(None);
        let h324 = gt.holidays.iter_mut().find(|h| h.holiday_id == 324).unwrap();
        h324.date[1] = 0xDEADBEEF;

        let rules = holiday_rules();
        let report = compute_report(&gt, &raw_events, rules, true); // gate ON

        let date_mismatches: Vec<_> = report
            .mismatches
            .iter()
            .filter(|m| matches!(m.kind, CheckKind::DateMath))
            .collect();
        assert!(
            !date_mismatches.is_empty(),
            "DateMath mismatches must fire when live_shadow_resolution=true and data is corrupt"
        );
        assert!(
            report.date_math.checked > 0,
            "date_math.checked must be > 0 when gate=true"
        );
    }
}
