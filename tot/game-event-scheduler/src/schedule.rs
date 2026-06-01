//! Per-tick schedule math ported 1:1 from C++ GameEventMgr.cpp lines 46-121.
//!
//! `is_active`  ← `CheckOneGameEvent` (lines 46-82)
//! `next_check` ← `NextCheck`         (lines 84-121)
//!
//! Both are pure functions of a `ResolvedEvent` + `now` (server unixtime,
//! seconds).  No mutable global state; trivially unit-testable.
//!
//! # Units
//! `ResolvedEvent::occurence` and `::length` are stored in **minutes**, matching
//! the `game_event` DB column semantics.  The C++ code multiplies by `MINUTE`
//! (= 60) inside `CheckOneGameEvent` / `NextCheck`; we do the same here.
//!
//! # Confirmed C++ constants (verified 2026-05-30 against AC source)
//! - `MINUTE` = 60  (Common.h line 47: `constexpr auto MINUTE = SECOND * 60`)
//! - `DAY`    = 86400 (`HOUR * 24`, Common.h lines 47-49)
//! - `max_ge_check_delay` = `DAY` = **86400 seconds**  (GameEventMgr.h line 27)
//!
//! # GAMEEVENT_* enum → GameEventState mapping (GameEventMgr.h lines 31-36)
//! | C++ name                    | value | Rust variant        |
//! |-----------------------------|-------|---------------------|
//! | GAMEEVENT_NORMAL            |   0   | Normal              |
//! | GAMEEVENT_WORLD_INACTIVE    |   1   | WorldInactive       |
//! | GAMEEVENT_WORLD_CONDITIONS  |   2   | WorldConditions     |
//! | GAMEEVENT_WORLD_NEXTPHASE   |   3   | WorldNextphase      |
//! | GAMEEVENT_WORLD_FINISHED    |   4   | WorldFinished       |
//! | GAMEEVENT_INTERNAL          |   5   | Internal            |
//!
//! # WORLD_INACTIVE prerequisite branch
//! `CheckOneGameEvent` lines 68-80 iterate `_gameEvent[entry].PrerequisiteEvents`
//! to check whether each prerequisite is in NEXTPHASE or FINISHED state and its
//! `NextStart <= now`.  This requires reading OTHER events by entry number.
//!
//! Since this server has **zero** prerequisite links in its live data, we keep
//! the branch faithful but require the caller to supply a lookup:
//!
//! ```text
//! is_active(event, now, prereq_lookup)
//! ```
//!
//! where `prereq_lookup: impl Fn(u16) -> Option<(GameEventState, i64)>` returns
//! `(state, next_start)` for the given entry ID.  The Normal path ignores this
//! closure entirely; callers that have no prerequisite data can pass
//! `|_| None` safely (the empty-prerequisites check returns `false` per C++
//! line 79: "if there are no prerequisites, this can be only activated through
//! gm command").

/// Mirrors the C++ `GameEventState` enum (GameEventMgr.h lines 31-36).
///
/// Deserialization: the harness payload sends state as an integer 0..5.
/// `TryFrom<u8>` maps the integer to the enum; `events.rs` uses it when
/// deserializing `GroundTruthEvent`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameEventState {
    /// GAMEEVENT_NORMAL (0) — standard recurring game events.
    Normal,
    /// GAMEEVENT_WORLD_NEXTPHASE (3) — conditions met; length timer to next event.
    WorldNextphase,
    /// GAMEEVENT_WORLD_CONDITIONS (2) — condition matching phase.
    WorldConditions,
    /// GAMEEVENT_WORLD_FINISHED (4) — next events started; unapply this one.
    WorldFinished,
    /// GAMEEVENT_INTERNAL (5) — never handled in update.
    Internal,
    /// GAMEEVENT_WORLD_INACTIVE (1) — not yet started; needs prereq check.
    WorldInactive,
}

impl TryFrom<u8> for GameEventState {
    type Error = u8;
    fn try_from(v: u8) -> Result<Self, Self::Error> {
        match v {
            0 => Ok(GameEventState::Normal),
            1 => Ok(GameEventState::WorldInactive),
            2 => Ok(GameEventState::WorldConditions),
            3 => Ok(GameEventState::WorldNextphase),
            4 => Ok(GameEventState::WorldFinished),
            5 => Ok(GameEventState::Internal),
            other => Err(other),
        }
    }
}

/// A fully-resolved game event record, combining `game_event` DB columns with
/// any dynamically-computed holiday dates.
///
/// Field semantics match the C++ `GameEventData` struct used inside
/// `CheckOneGameEvent` / `NextCheck`.
#[derive(Debug, Clone)]
pub struct ResolvedEvent {
    /// DB `entry` column.
    pub entry: u16,
    /// Event start time (server unixtime, seconds).
    pub start: i64,
    /// Event end time (server unixtime, seconds).
    pub end: i64,
    /// Recurrence period in **minutes** (DB `occurence` column).
    pub occurence: i64,
    /// Active window duration in **minutes** (DB `length` column).
    pub length: i64,
    /// State machine state.
    pub state: GameEventState,
    /// Prerequisite event entry IDs (for `WorldInactive` branch only).
    pub prerequisites: Vec<u16>,
    /// Next scheduled start for world-phase events (server unixtime, seconds).
    pub next_start: i64,
}

// ── constants (mirroring Common.h + GameEventMgr.h) ─────────────────────────

/// C++ `MINUTE` — 60 seconds.
const MINUTE: i64 = 60;

/// C++ `max_ge_check_delay` = `DAY` = 86 400 seconds (GameEventMgr.h line 27).
pub const MAX_GE_CHECK_DELAY: u32 = 86_400;

// ── public API ───────────────────────────────────────────────────────────────

/// Port of `GameEventMgr::CheckOneGameEvent` (lines 46-82).
///
/// Returns `true` when the event should be considered active right now.
///
/// `prereq_lookup` is used only for the `WorldInactive` branch; pass `|_| None`
/// when prerequisite data is unavailable (the branch returns `false` for an
/// empty prerequisite set, matching C++ line 79).
pub fn is_active(
    event: &ResolvedEvent,
    now: i64,
    prereq_lookup: impl Fn(u16) -> Option<(GameEventState, i64)>,
) -> bool {
    match event.state {
        // default / GAMEEVENT_NORMAL (lines 51-58)
        GameEventState::Normal => {
            event.start < now
                && now < event.end
                && (now - event.start) % (event.occurence * MINUTE)
                    < event.length * MINUTE
        }
        // if the state is conditions or nextphase, then the event should be active (lines 59-62)
        GameEventState::WorldConditions | GameEventState::WorldNextphase => true,
        // finished world events are inactive (lines 63-66)
        GameEventState::WorldFinished | GameEventState::Internal => false,
        // if inactive world event, check the prerequisite events (lines 67-80)
        GameEventState::WorldInactive => {
            for &prereq_entry in &event.prerequisites {
                match prereq_lookup(prereq_entry) {
                    Some((state, next_start)) => {
                        // prereq must be in NEXTPHASE or FINISHED, and NextStart <= now
                        if (state != GameEventState::WorldNextphase
                            && state != GameEventState::WorldFinished)
                            || next_start > now
                        {
                            return false;
                        }
                    }
                    // prereq not found — treat as not satisfied
                    None => return false,
                }
            }
            // all prereqs met — but empty prereq list means GM-only activation
            !event.prerequisites.is_empty()
        }
    }
}

/// Port of `GameEventMgr::NextCheck` (lines 84-121).
///
/// Returns the number of seconds until this event should next be re-evaluated.
/// Capped at [`MAX_GE_CHECK_DELAY`] (1 day = 86 400 s).
pub fn next_check(event: &ResolvedEvent, now: i64) -> u32 {
    // NEXTPHASE / FINISHED with a future NextStart: return delay to NextStart (lines 88-90)
    if (event.state == GameEventState::WorldNextphase
        || event.state == GameEventState::WorldFinished)
        && event.next_start >= now
    {
        return (event.next_start - now) as u32;
    }

    // CONDITIONS: return length * 60, or max if length == 0 (lines 92-99)
    if event.state == GameEventState::WorldConditions {
        return if event.length != 0 {
            (event.length * 60) as u32
        } else {
            MAX_GE_CHECK_DELAY
        };
    }

    // outdated event: return max (lines 101-103)
    if now > event.end {
        return MAX_GE_CHECK_DELAY;
    }

    // never started event: return delay before start (lines 105-107)
    if event.start > now {
        return (event.start - now) as u32;
    }

    // in-window or out-of-window within [start, end] (lines 109-115)
    let delay: i64 = {
        let elapsed = (now - event.start) % (event.occurence * MINUTE);
        if elapsed < event.length * MINUTE {
            // in window: return time remaining in this occurrence
            (event.length * MINUTE) - elapsed
        } else {
            // out of window: return time until next occurrence
            (event.occurence * MINUTE) - elapsed
        }
    };

    // cap: if end is before next check, return End - now (lines 116-120)
    if event.end < now + delay {
        (event.end - now) as u32
    } else {
        delay as u32
    }
}

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a `ResolvedEvent` with Normal state and no prerequisites.
    fn ev(start: i64, end: i64, occ_min: i64, len_min: i64, state: GameEventState) -> ResolvedEvent {
        ResolvedEvent {
            entry: 1,
            start,
            end,
            occurence: occ_min,
            length: len_min,
            state,
            prerequisites: vec![],
            next_start: 0,
        }
    }

    /// Convenience: `is_active` with a no-op prereq lookup (fine for all
    /// branches except WorldInactive-with-prerequisites).
    fn active(event: &ResolvedEvent, now: i64) -> bool {
        is_active(event, now, |_| None)
    }

    // ── is_active: Normal ────────────────────────────────────────────────────

    #[test]
    fn normal_active_inside_window() {
        // start=1000, end=1_000_000, occ=60min, len=30min
        // Window: [1000 .. 1000+30*60) = [1000..2800), then gap until 1000+60*60=4600
        let e = ev(1000, 1_000_000, 60, 30, GameEventState::Normal);
        assert!(active(&e, 1000 + 10 * 60));  // 600s into a 1800s window → active
        assert!(!active(&e, 1000 + 45 * 60)); // 2700s in: past the 1800s window → inactive
        assert!(!active(&e, 999));             // before start (strict <) → inactive
        assert!(!active(&e, 1_000_001));       // after end → inactive
    }

    #[test]
    fn normal_boundary_exactly_at_start() {
        // C++ uses Start < currenttime (strict), so exactly at start is NOT active
        let e = ev(1000, 1_000_000, 60, 30, GameEventState::Normal);
        assert!(!active(&e, 1000)); // start itself: not active (strict <)
        assert!(active(&e, 1001));  // one second after start: active
    }

    #[test]
    fn normal_boundary_exactly_at_end() {
        // C++ uses currenttime < End (strict), so exactly at End is NOT active
        let e = ev(1000, 2000, 60, 30, GameEventState::Normal);
        assert!(!active(&e, 2000)); // at End: not active (strict <)
        assert!(!active(&e, 2001)); // past End: not active
    }

    #[test]
    fn normal_second_occurrence_active() {
        // occ=60min, len=30min: second window starts at start + 60*60 = 1000 + 3600 = 4600
        let e = ev(1000, 1_000_000, 60, 30, GameEventState::Normal);
        // At 4600 + 1: strictly inside second window
        assert!(active(&e, 4601));
        // At 4600 + 30*60 - 1 = 6399: still inside second window
        assert!(active(&e, 6399));
        // At 4600 + 30*60 = 6400: out of second window
        assert!(!active(&e, 6400));
    }

    #[test]
    fn normal_between_occurrences_inactive() {
        // gap between end-of-window-1 (2800) and start-of-window-2 (4600)
        let e = ev(1000, 1_000_000, 60, 30, GameEventState::Normal);
        assert!(!active(&e, 3000)); // in the gap between occurrences
    }

    // ── is_active: world-phase states ────────────────────────────────────────

    #[test]
    fn conditions_and_nextphase_always_active() {
        let cond = ev(1000, 2000, 60, 30, GameEventState::WorldConditions);
        let next = ev(1000, 2000, 60, 30, GameEventState::WorldNextphase);
        assert!(active(&cond, 500));  // even before start
        assert!(active(&cond, 9999)); // even after end
        assert!(active(&next, 500));
        assert!(active(&next, 9999));
    }

    #[test]
    fn finished_and_internal_always_inactive() {
        let fin = ev(1000, 2000, 60, 30, GameEventState::WorldFinished);
        let int = ev(1000, 2000, 60, 30, GameEventState::Internal);
        assert!(!active(&fin, 1500)); // mid-window, but FINISHED → false
        assert!(!active(&int, 1500)); // mid-window, but INTERNAL → false
    }

    // ── is_active: WorldInactive (prerequisite branch) ────────────────────────

    #[test]
    fn world_inactive_no_prerequisites_returns_false() {
        // C++ line 79: empty prereq set = GM-only activation → false
        let e = ev(1000, 2000, 60, 30, GameEventState::WorldInactive);
        // no prerequisites in the vec → false regardless of lookup result
        assert!(!active(&e, 1500));
    }

    #[test]
    fn world_inactive_prereq_in_nextphase_and_past_next_start() {
        // prereq entry 42 is WorldNextphase, NextStart = 900 < now=1500 → satisfied
        let mut e = ev(1000, 2000, 60, 30, GameEventState::WorldInactive);
        e.prerequisites = vec![42];
        let result = is_active(&e, 1500, |entry| {
            if entry == 42 {
                Some((GameEventState::WorldNextphase, 900))
            } else {
                None
            }
        });
        assert!(result);
    }

    #[test]
    fn world_inactive_prereq_in_nextphase_but_next_start_in_future() {
        // prereq NextStart > now → not yet satisfied
        let mut e = ev(1000, 2000, 60, 30, GameEventState::WorldInactive);
        e.prerequisites = vec![42];
        let result = is_active(&e, 1500, |entry| {
            if entry == 42 {
                Some((GameEventState::WorldNextphase, 2000)) // next_start > now
            } else {
                None
            }
        });
        assert!(!result);
    }

    #[test]
    fn world_inactive_prereq_in_finished_satisfies() {
        // WorldFinished + next_start <= now → satisfied
        let mut e = ev(1000, 2000, 60, 30, GameEventState::WorldInactive);
        e.prerequisites = vec![7];
        let result = is_active(&e, 1500, |entry| {
            if entry == 7 {
                Some((GameEventState::WorldFinished, 1000))
            } else {
                None
            }
        });
        assert!(result);
    }

    #[test]
    fn world_inactive_prereq_wrong_state_returns_false() {
        // prereq is WorldConditions (not NEXTPHASE or FINISHED) → false
        let mut e = ev(1000, 2000, 60, 30, GameEventState::WorldInactive);
        e.prerequisites = vec![5];
        let result = is_active(&e, 1500, |entry| {
            if entry == 5 {
                Some((GameEventState::WorldConditions, 900))
            } else {
                None
            }
        });
        assert!(!result);
    }

    // ── next_check: before start ──────────────────────────────────────────────

    #[test]
    fn nextcheck_before_start_returns_delay() {
        let e = ev(1000, 1_000_000, 60, 30, GameEventState::Normal);
        assert_eq!(next_check(&e, 900), 100); // start(1000) - now(900) = 100
    }

    // ── next_check: in window ─────────────────────────────────────────────────

    #[test]
    fn nextcheck_in_window_returns_time_to_window_end() {
        // start=1000, occ=60min=3600s, len=30min=1800s
        // At now=1600: elapsed = (1600-1000) % 3600 = 600; 600 < 1800 → in window
        // delay = 1800 - 600 = 1200
        let e = ev(1000, 1_000_000, 60, 30, GameEventState::Normal);
        assert_eq!(next_check(&e, 1600), 1200);
    }

    // ── next_check: out of window (between occurrences) ───────────────────────

    #[test]
    fn nextcheck_out_of_window_returns_time_to_next_occurrence() {
        // start=1000, occ=60min=3600s, len=30min=1800s
        // At now=3500: elapsed = (3500-1000) % 3600 = 2500; 2500 >= 1800 → out of window
        // delay = 3600 - 2500 = 1100
        let e = ev(1000, 1_000_000, 60, 30, GameEventState::Normal);
        assert_eq!(next_check(&e, 3500), 1100);
    }

    // ── next_check: end caps the delay ────────────────────────────────────────

    #[test]
    fn nextcheck_end_before_next_check_returns_end_minus_now() {
        // start=1000, end=2000, occ=60min, len=30min
        // At now=1800: elapsed = (1800-1000) % 3600 = 800; 800 < 1800 → in window
        // delay = 1800 - 800 = 1000; end(2000) < now(1800) + delay(1000)=2800 → cap to end-now=200
        let e = ev(1000, 2000, 60, 30, GameEventState::Normal);
        assert_eq!(next_check(&e, 1800), 200);
    }

    // ── next_check: outdated event ────────────────────────────────────────────

    #[test]
    fn nextcheck_outdated_returns_max_ge_check_delay() {
        // now > end → max
        let e = ev(1000, 2000, 60, 30, GameEventState::Normal);
        assert_eq!(next_check(&e, 3000), MAX_GE_CHECK_DELAY);
    }

    // ── next_check: NEXTPHASE / FINISHED state ────────────────────────────────

    #[test]
    fn nextcheck_nextphase_with_future_next_start() {
        let mut e = ev(1000, 1_000_000, 60, 30, GameEventState::WorldNextphase);
        e.next_start = 5000;
        assert_eq!(next_check(&e, 3000), 2000); // next_start - now
    }

    #[test]
    fn nextcheck_finished_with_future_next_start() {
        let mut e = ev(1000, 1_000_000, 60, 30, GameEventState::WorldFinished);
        e.next_start = 5000;
        assert_eq!(next_check(&e, 4000), 1000); // next_start - now
    }

    #[test]
    fn nextcheck_nextphase_next_start_in_past_falls_through_to_normal_logic() {
        // next_start < now: the NEXTPHASE/FINISHED branch is NOT taken (C++ line 89 uses >=).
        // Falls through to the body:  now=3000 > end=2000 → max
        let mut e = ev(1000, 2000, 60, 30, GameEventState::WorldNextphase);
        e.next_start = 500; // past
        // now > end → max_ge_check_delay
        assert_eq!(next_check(&e, 3000), MAX_GE_CHECK_DELAY);
    }

    // ── next_check: CONDITIONS state ──────────────────────────────────────────

    #[test]
    fn nextcheck_conditions_with_length_returns_length_times_60() {
        let e = ev(1000, 1_000_000, 60, 30, GameEventState::WorldConditions);
        // length=30min → 30*60=1800
        assert_eq!(next_check(&e, 1500), 1800);
    }

    #[test]
    fn nextcheck_conditions_zero_length_returns_max() {
        let e = ev(1000, 1_000_000, 60, 0, GameEventState::WorldConditions);
        assert_eq!(next_check(&e, 1500), MAX_GE_CHECK_DELAY);
    }
}
