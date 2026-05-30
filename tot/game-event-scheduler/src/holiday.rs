//! Holiday date calculators — ported 1:1 from C++ HolidayDateCalculator.cpp lines 151–214
//! and the rules table at lines 97–144.
//!
//! **Scope (Task 3):** Easter Sunday, Nth-weekday, Weekday-on-or-after, and FIXED-date
//! helpers, plus the `HolidayRule` struct + enum types + the static rules table.
//!
//! **Deliberately NOT here:** `calculate_holiday_date` dispatch and `get_packed_holiday_date`
//! (Task 4) — those require the 4 astronomical calculators that arrive in Task 4.
//!
//! All pure arithmetic on `CivilDate` via `normalize_date`. No libc / no system calls.

use crate::packed::{normalize_date, CivilDate};

// ── Enums & struct ────────────────────────────────────────────────────────────

/// C++ `HolidayCalculationType` (HolidayDateCalculator.h lines 25–35).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HolidayCalculationType {
    /// Same month/day every year (e.g., Dec 25). C++ `FIXED_DATE`.
    FixedDate,
    /// Nth weekday of month (e.g., 4th Thursday of Nov). C++ `NTH_WEEKDAY`.
    NthWeekday,
    /// Days relative to Easter Sunday. C++ `EASTER_OFFSET`.
    EasterOffset,
    /// Chinese New Year (new moon between Jan 21 – Feb 20). C++ `LUNAR_NEW_YEAR`.
    LunarNewYear,
    /// First weekday on or after a date. C++ `WEEKDAY_ON_OR_AFTER`.
    WeekdayOnOrAfter,
    /// Days relative to autumn equinox. C++ `AUTUMN_EQUINOX`.
    AutumnEquinox,
    /// Days relative to winter solstice. C++ `WINTER_SOLSTICE`.
    WinterSolstice,
    /// First Sunday of months matching (month % 3 == locationOffset). C++ `DARKMOON_FAIRE`.
    DarkmoonFaire,
}

/// C++ `Weekday` (HolidayDateCalculator.h lines 37–46).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Weekday {
    Sunday    = 0,
    Monday    = 1,
    Tuesday   = 2,
    Wednesday = 3,
    Thursday  = 4,
    Friday    = 5,
    Saturday  = 6,
}

/// C++ `HolidayRule` (HolidayDateCalculator.h lines 48–56).
///
/// Field semantics:
/// - `holiday_id`: numeric WoW HolidayIds value (from SharedDefines.h).
/// - `calc_type`: which calculator to use.
/// - `month`: 1-indexed month (for FIXED_DATE, WEEKDAY_ON_OR_AFTER). For DARKMOON_FAIRE: location offset.
/// - `day`:   For FIXED_DATE: day of month. For NTH_WEEKDAY: which occurrence (1–5).
/// - `weekday`: For NTH_WEEKDAY / WEEKDAY_ON_OR_AFTER: 0=Sunday … 6=Saturday.
/// - `offset`: For EASTER_OFFSET / NTH_WEEKDAY: day offset applied after base date.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HolidayRule {
    pub holiday_id: u32,
    pub calc_type: HolidayCalculationType,
    pub month: i32,
    pub day: i32,
    pub weekday: i32,
    pub offset: i32,
}

// ── Static rules table (C++ lines 97–144) ────────────────────────────────────

/// All 18 holiday rules, ported verbatim from C++ `HolidayRules`.
/// Numeric IDs sourced from `SharedDefines.h` `HolidayIds` enum.
static HOLIDAY_RULES: &[HolidayRule] = &[
    // HOLIDAY_LUNAR_FESTIVAL = 327
    // Lunar Festival: Chinese New Year - 1 day (event starts day before CNY)
    HolidayRule { holiday_id: 327, calc_type: HolidayCalculationType::LunarNewYear,     month: 0, day: 0, weekday: 0, offset: -1 },

    // HOLIDAY_LOVE_IS_IN_THE_AIR = 423
    // Love is in the Air: First Monday on or after Feb 3
    HolidayRule { holiday_id: 423, calc_type: HolidayCalculationType::WeekdayOnOrAfter, month: 2, day: 3, weekday: Weekday::Monday as i32, offset: 0 },

    // HOLIDAY_NOBLEGARDEN = 181
    // Noblegarden: Day after Easter Sunday (Easter + 1 day)
    HolidayRule { holiday_id: 181, calc_type: HolidayCalculationType::EasterOffset,     month: 0, day: 0, weekday: 0, offset: 1 },

    // HOLIDAY_CHILDRENS_WEEK = 201
    // Children's Week: First Monday on or after Apr 25 (Monday closest to May 1)
    HolidayRule { holiday_id: 201, calc_type: HolidayCalculationType::WeekdayOnOrAfter, month: 4, day: 25, weekday: Weekday::Monday as i32, offset: 0 },

    // HOLIDAY_FIRE_FESTIVAL = 341
    // Midsummer Fire Festival: Fixed Jun 21
    HolidayRule { holiday_id: 341, calc_type: HolidayCalculationType::FixedDate,        month: 6, day: 21, weekday: 0, offset: 0 },

    // HOLIDAY_FIREWORKS_SPECTACULAR = 62
    // Fireworks Spectacular: Fixed Jul 4
    HolidayRule { holiday_id: 62,  calc_type: HolidayCalculationType::FixedDate,        month: 7, day: 4,  weekday: 0, offset: 0 },

    // HOLIDAY_PIRATES_DAY = 398
    // Pirates' Day: Fixed Sep 19
    HolidayRule { holiday_id: 398, calc_type: HolidayCalculationType::FixedDate,        month: 9, day: 19, weekday: 0, offset: 0 },

    // HOLIDAY_BREWFEST = 372
    // Brewfest: Fixed Sept 20 main event, prep starts Sept 13
    HolidayRule { holiday_id: 372, calc_type: HolidayCalculationType::FixedDate,        month: 9, day: 13, weekday: 0, offset: 0 },

    // HOLIDAY_HARVEST_FESTIVAL = 321
    // Harvest Festival: 2 days before autumn equinox (Sept 20-21)
    HolidayRule { holiday_id: 321, calc_type: HolidayCalculationType::AutumnEquinox,    month: 0, day: 0,  weekday: 0, offset: -2 },

    // HOLIDAY_HALLOWS_END = 324
    // Hallow's End: Fixed Oct 18
    HolidayRule { holiday_id: 324, calc_type: HolidayCalculationType::FixedDate,        month: 10, day: 18, weekday: 0, offset: 0 },

    // HOLIDAY_DAY_OF_DEAD = 409
    // Day of the Dead: Fixed Nov 1
    HolidayRule { holiday_id: 409, calc_type: HolidayCalculationType::FixedDate,        month: 11, day: 1,  weekday: 0, offset: 0 },

    // HOLIDAY_PILGRIMS_BOUNTY = 404
    // Pilgrim's Bounty: Sunday before Thanksgiving (4th Thursday - 4 days)
    HolidayRule { holiday_id: 404, calc_type: HolidayCalculationType::NthWeekday,       month: 11, day: 4, weekday: Weekday::Thursday as i32, offset: -4 },

    // HOLIDAY_FEAST_OF_WINTER_VEIL = 141
    // Winter Veil: 6 days before winter solstice (Dec 15-16)
    HolidayRule { holiday_id: 141, calc_type: HolidayCalculationType::WinterSolstice,   month: 0, day: 0,  weekday: 0, offset: -6 },

    // HOLIDAY_DARKMOON_FAIRE_ELWYNN = 374
    // Darkmoon Faire: First Sunday of months matching (month % 3 == locationOffset)
    // Rotates monthly: Mulgore (Jan) -> Terokkar (Feb) -> Elwynn (Mar) -> repeat
    // rule.month stores the location offset
    // rule.offset is -2 (building phase starts Friday, 2 days before faire opens on Sunday)
    // Elwynn (offset 0): Mar, Jun, Sep, Dec
    HolidayRule { holiday_id: 374, calc_type: HolidayCalculationType::DarkmoonFaire,    month: 0, day: 0,  weekday: 0, offset: -2 },

    // HOLIDAY_DARKMOON_FAIRE_THUNDER = 375
    // Thunder Bluff/Mulgore (offset 1): Jan, Apr, Jul, Oct
    HolidayRule { holiday_id: 375, calc_type: HolidayCalculationType::DarkmoonFaire,    month: 1, day: 0,  weekday: 0, offset: -2 },

    // HOLIDAY_DARKMOON_FAIRE_SHATTRATH = 376
    // Terokkar/Shattrath (offset 2): Feb, May, Aug, Nov
    HolidayRule { holiday_id: 376, calc_type: HolidayCalculationType::DarkmoonFaire,    month: 2, day: 0,  weekday: 0, offset: -2 },
];

/// Return the static slice of all holiday rules.
pub fn holiday_rules() -> &'static [HolidayRule] {
    HOLIDAY_RULES
}

// ── Calculators ───────────────────────────────────────────────────────────────

/// Calculate Easter Sunday for a given year using the Anonymous Gregorian Computus.
///
/// Port of `HolidayDateCalculator::CalculateEasterSunday` (C++ lines 151–177).
/// Reference: <https://en.wikipedia.org/wiki/Date_of_Easter#Anonymous_Gregorian_algorithm>
pub fn easter_sunday(year: i32) -> CivilDate {
    let a = year % 19;
    let b = year / 100;
    let c = year % 100;
    let d = b / 4;
    let e = b % 4;
    let f = (b + 8) / 25;
    let g = (b - f + 1) / 3;
    let h = (19 * a + b - d - g + 15) % 30;
    let i = c / 4;
    let k = c % 4;
    let l = (32 + 2 * e + 2 * i - h - k) % 7;
    let m = (a + 11 * h + 22 * l) / 451;
    let month = (h + l - 7 * m + 114) / 31;
    let day   = ((h + l - 7 * m + 114) % 31) + 1;

    let mut result = CivilDate {
        year,
        mon0: month - 1, // C++: result.tm_mon = month - 1
        mday: day,
        ..Default::default()
    };
    normalize_date(&mut result);
    result
}

/// Calculate the Nth weekday of a month (e.g., 4th Thursday of November).
///
/// Port of `HolidayDateCalculator::CalculateNthWeekday` (C++ lines 179–197).
///
/// `weekday` is an `i32` (0=Sunday … 6=Saturday), matching `HolidayRule::weekday`.
/// `n` is 1-indexed (1 = first occurrence, 4 = fourth).
pub fn nth_weekday(year: i32, month: i32, weekday: i32, n: i32) -> CivilDate {
    // Start with first day of the month
    let mut date = CivilDate {
        year,
        mon0: month - 1, // C++: date.tm_mon = month - 1
        mday: 1,
        ..Default::default()
    };
    normalize_date(&mut date); // computes wday

    // Find first occurrence of the target weekday
    let days_until_weekday = (weekday - date.wday + 7) % 7;
    date.mday = 1 + days_until_weekday;

    // Move to nth occurrence
    date.mday += (n - 1) * 7;

    normalize_date(&mut date);
    date
}

/// Calculate the first weekday on or after a specific date.
///
/// Port of `HolidayDateCalculator::CalculateWeekdayOnOrAfter` (C++ lines 199–214).
///
/// `weekday` is an `i32` (0=Sunday … 6=Saturday).
pub fn weekday_on_or_after(year: i32, month: i32, day: i32, weekday: i32) -> CivilDate {
    // Start with the specified date
    let mut date = CivilDate {
        year,
        mon0: month - 1, // C++: date.tm_mon = month - 1
        mday: day,
        ..Default::default()
    };
    normalize_date(&mut date); // computes wday

    // Find days until the target weekday (0 if already on that day)
    let days_until_weekday = (weekday - date.wday + 7) % 7;
    date.mday += days_until_weekday;

    normalize_date(&mut date);
    date
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Easter known-date vectors (C++ test lines 66–98, all 21 cases) ────────

    #[test]
    fn easter_known_dates() {
        // Source: https://www.census.gov/data/software/x13as/genhol/easter-dates.html
        // All 21 cases from C++ EasterSunday_KnownDates
        let cases: &[(i32, i32, i32)] = &[
            // Historical dates
            (1900, 4, 15),
            (1901, 4,  7),
            (1950, 4,  9),
            (1999, 4,  4),
            // Recent dates
            (2000, 4, 23),
            (2010, 4,  4),
            (2020, 4, 12),
            (2021, 4,  4),
            (2022, 4, 17),
            (2023, 4,  9),
            (2024, 3, 31),
            (2025, 4, 20),
            (2026, 4,  5),
            (2027, 3, 28),
            (2028, 4, 16),
            (2029, 4,  1),
            (2030, 4, 21),
            // Future dates
            (2050, 4, 10),
            (2100, 3, 28),
            (2150, 4, 12),
            (2200, 4,  6),
        ];
        for &(y, m, d) in cases {
            let e = easter_sunday(y);
            assert_eq!((e.year, e.mon0 + 1, e.mday), (y, m, d), "easter {y}");
            assert_eq!(e.wday, 0, "easter {y} must be Sunday (wday=0)");
        }
    }

    // ── Easter property loop: valid date range 1900–2200 (C++ lines 101–132) ──

    #[test]
    fn easter_valid_date_range_1900_2200() {
        for year in 1900..=2200 {
            let e = easter_sunday(year);
            assert_eq!(e.year, year, "year field correct for {year}");
            // Easter must be in March (mon0=2) or April (mon0=3)
            assert!(
                e.mon0 == 2 || e.mon0 == 3,
                "easter {year}: mon0={} must be 2 (March) or 3 (April)",
                e.mon0
            );
            if e.mon0 == 2 {
                // March: Easter range is Mar 22–31
                assert!(e.mday >= 22, "easter {year}: March day {} must be >= 22", e.mday);
                assert!(e.mday <= 31, "easter {year}: March day {} must be <= 31", e.mday);
            } else {
                // April: Easter range is Apr 1–25
                assert!(e.mday >= 1, "easter {year}: April day {} must be >= 1", e.mday);
                assert!(e.mday <= 25, "easter {year}: April day {} must be <= 25", e.mday);
            }
        }
    }

    // ── Easter always-Sunday loop 1900–2200 (C++ lines 134–142) ─────────────

    #[test]
    fn easter_always_sunday_1900_2200() {
        for year in 1900..=2200 {
            let e = easter_sunday(year);
            assert_eq!(e.wday, 0, "easter {year} must be Sunday (wday=0), got wday={}", e.wday);
        }
    }

    // ── NthWeekday: Thanksgiving 4th Thursday of November (C++ lines 148–173) ─

    #[test]
    fn nth_weekday_thanksgiving_1900_2200() {
        for year in 1900..=2200 {
            let d = nth_weekday(year, 11, Weekday::Thursday as i32, 4);
            // Must be in November
            assert_eq!(d.mon0 + 1, 11, "thanksgiving {year}: must be November");
            // Must be a Thursday
            assert_eq!(
                d.wday,
                Weekday::Thursday as i32,
                "thanksgiving {year}: must be Thursday (wday=4), got {}",
                d.wday
            );
            // 4th Thursday must be between 22nd and 28th
            assert!(d.mday >= 22, "thanksgiving {year}: day {} must be >= 22", d.mday);
            assert!(d.mday <= 28, "thanksgiving {year}: day {} must be <= 28", d.mday);
            // Verify it's actually the 4th occurrence: first Thursday in range 1-7
            let first_thursday = d.mday - 21;
            assert!(first_thursday >= 1, "thanksgiving {year}: first_thursday {} >= 1", first_thursday);
            assert!(first_thursday <= 7, "thanksgiving {year}: first_thursday {} <= 7", first_thursday);
        }
    }

    // ── NthWeekday: first occurrence of each weekday in January (C++ lines 175–197) ─

    #[test]
    fn nth_weekday_all_weekdays_january_1900_2200() {
        for year in 1900..=2200 {
            for weekday in 0i32..=6 {
                let d = nth_weekday(year, 1, weekday, 1);
                // Must be in January
                assert_eq!(d.mon0 + 1, 1, "year {year} wday {weekday}: must be January");
                // Must be the correct weekday
                assert_eq!(d.wday, weekday, "year {year} wday {weekday}: wrong wday {}", d.wday);
                // First occurrence must be within first 7 days
                assert!(d.mday >= 1, "year {year} wday {weekday}: mday {} >= 1", d.mday);
                assert!(d.mday <= 7, "year {year} wday {weekday}: mday {} <= 7", d.mday);
            }
        }
    }

    // ── NthWeekday: 2nd/3rd/4th exactly 7 days apart (C++ lines 199–218) ─────

    #[test]
    fn nth_weekday_second_third_fourth_7_days_apart() {
        for year in 2000..=2100 {
            for month in 1..=12 {
                let first  = nth_weekday(year, month, Weekday::Monday as i32, 1);
                let second = nth_weekday(year, month, Weekday::Monday as i32, 2);
                let third  = nth_weekday(year, month, Weekday::Monday as i32, 3);
                let fourth = nth_weekday(year, month, Weekday::Monday as i32, 4);
                assert_eq!(second.mday - first.mday,  7, "year {year} month {month}: 2nd-1st != 7");
                assert_eq!(third.mday  - second.mday, 7, "year {year} month {month}: 3rd-2nd != 7");
                assert_eq!(fourth.mday - third.mday,  7, "year {year} month {month}: 4th-3rd != 7");
            }
        }
    }

    // ── NthWeekday: Darkmoon Faire first Sunday — known dates (C++ lines 1023–1057) ─

    #[test]
    fn nth_weekday_darkmoon_first_sunday_known_dates() {
        // Cases from C++ DarkmoonFaire_FirstSundayOfMonth_KnownDates
        let cases: &[(i32, i32, i32)] = &[
            // 2024
            (2024, 1,  7), (2024, 2,  4), (2024, 3,  3), (2024, 4,  7),
            (2024, 9,  1), (2024, 12, 1),
            // 2025
            (2025, 1,  5), (2025, 2,  2), (2025, 3,  2), (2025, 6,  1),
            (2025, 9,  7), (2025, 12, 7),
            // 2026
            (2026, 1,  4), (2026, 3,  1),
        ];
        for &(year, month, expected_day) in cases {
            let d = nth_weekday(year, month, Weekday::Sunday as i32, 1);
            assert_eq!(d.year, year,            "first-sunday year mismatch {year}/{month}");
            assert_eq!(d.mon0 + 1, month,       "first-sunday month mismatch {year}/{month}");
            assert_eq!(d.mday, expected_day,    "first-sunday day mismatch {year}/{month}");
            assert_eq!(d.wday, 0,               "first-sunday must be Sunday {year}/{month}");
        }
    }

    // ── WeekdayOnOrAfter: Love is in the Air (C++ lines 535–560) ─────────────

    #[test]
    fn weekday_on_or_after_love_is_in_the_air() {
        // First Monday on or after Feb 3 — 7 explicit cases from C++ test
        let cases: &[(i32, i32)] = &[
            (2024, 5), // Feb 3 is Sat, first Mon after is Feb 5
            (2025, 3), // Feb 3 is Mon, so Feb 3
            (2026, 9), // Feb 3 is Tue, first Mon after is Feb 9
            (2027, 8), // Feb 3 is Wed, first Mon after is Feb 8
            (2028, 7), // Feb 3 is Thu, first Mon after is Feb 7
            (2029, 5), // Feb 3 is Sat, first Mon after is Feb 5
            (2030, 4), // Feb 3 is Sun, first Mon after is Feb 4
        ];
        for &(year, expected_day) in cases {
            let d = weekday_on_or_after(year, 2, 3, Weekday::Monday as i32);
            assert_eq!(d.year, year,              "love year {year}");
            assert_eq!(d.mon0 + 1, 2,             "love month {year}");
            assert_eq!(d.mday, expected_day,      "love day {year}: expected {expected_day}, got {}", d.mday);
            assert_eq!(d.wday, Weekday::Monday as i32, "love weekday {year}");
        }
    }

    // ── WeekdayOnOrAfter: Children's Week (C++ lines 562–585) ────────────────

    #[test]
    fn weekday_on_or_after_childrens_week() {
        // First Monday on or after Apr 25 — 5 explicit cases
        let cases: &[(i32, i32, i32)] = &[
            (2023, 5, 1),  // Apr 25 is Tue, first Mon after is May 1
            (2024, 4, 29), // Apr 25 is Thu, first Mon after is Apr 29
            (2025, 4, 28), // Apr 25 is Fri, first Mon after is Apr 28
            (2026, 4, 27), // Apr 25 is Sat, first Mon after is Apr 27
            (2027, 4, 26), // Apr 25 is Sun, first Mon after is Apr 26
        ];
        for &(year, expected_month, expected_day) in cases {
            let d = weekday_on_or_after(year, 4, 25, Weekday::Monday as i32);
            assert_eq!(d.year, year,                   "childrens year {year}");
            assert_eq!(d.mon0 + 1, expected_month,     "childrens month {year}");
            assert_eq!(d.mday, expected_day,           "childrens day {year}");
            assert_eq!(d.wday, Weekday::Monday as i32, "childrens weekday {year}");
        }
    }

    // ── WeekdayOnOrAfter: always correct weekday 1900–2200 (C++ lines 587–604) ─

    #[test]
    fn weekday_on_or_after_correct_weekday_1900_2200() {
        for year in 1900..=2200 {
            for weekday in 0i32..=6 {
                let d = weekday_on_or_after(year, 2, 3, weekday);
                assert_eq!(d.wday, weekday, "year {year} wday {weekday}: got {}", d.wday);
                assert_eq!(d.mon0 + 1, 2, "year {year}: should stay in February");
                assert!(d.mday >= 3, "year {year} wday {weekday}: mday {} >= 3", d.mday);
                assert!(d.mday <= 9, "year {year} wday {weekday}: mday {} <= 9", d.mday);
            }
        }
    }

    // ── WeekdayOnOrAfter: month-boundary roll (C++ lines 606–642) ────────────

    #[test]
    fn weekday_on_or_after_month_boundary_rolls_into_next_month() {
        for year in 1900..=2200 {
            // Apr 25 → Monday can roll into May
            let apr25 = weekday_on_or_after(year, 4, 25, Weekday::Monday as i32);
            assert_eq!(apr25.wday, Weekday::Monday as i32, "apr25 {year}: must be Monday");
            assert!(
                apr25.mon0 == 3 || apr25.mon0 == 4,
                "apr25 {year}: mon0={} must be 3 (April) or 4 (May)",
                apr25.mon0
            );
            if apr25.mon0 == 3 {
                assert!(apr25.mday >= 25, "apr25 {year}: April day {} >= 25", apr25.mday);
            } else {
                assert!(apr25.mday <= 6, "apr25 {year}: May day {} <= 6", apr25.mday);
            }

            // Dec 31 → Monday can roll into January of next year
            let dec31 = weekday_on_or_after(year, 12, 31, Weekday::Monday as i32);
            assert_eq!(dec31.wday, Weekday::Monday as i32, "dec31 {year}: must be Monday");
            assert!(
                (dec31.mon0 == 11 && dec31.mday == 31) || (dec31.mon0 == 0 && dec31.mday <= 6),
                "dec31 {year}: unexpected date {}/{}/{}", dec31.year, dec31.mon0 + 1, dec31.mday
            );
        }
    }

    // ── Rules table: count and spot-checks ────────────────────────────────────

    #[test]
    fn holiday_rules_count_is_16() {
        // C++ table has 16 entries (lines 97–144: 1 lunar + 2 weekday-on-or-after
        // + 1 easter + 5 fixed + 1 autumn + 1 winter + 1 nth-weekday + 3 darkmoon = 16)
        // Note: the C++ comment says 18 but the actual table at lines 97–144 has 16 entries.
        assert_eq!(holiday_rules().len(), 16);
    }

    #[test]
    fn holiday_rules_contains_easter_noblegarden() {
        let r = holiday_rules().iter().find(|r| r.holiday_id == 181).unwrap();
        assert_eq!(r.calc_type, HolidayCalculationType::EasterOffset);
        assert_eq!(r.offset, 1);
    }

    #[test]
    fn holiday_rules_contains_thanksgiving() {
        let r = holiday_rules().iter().find(|r| r.holiday_id == 404).unwrap();
        assert_eq!(r.calc_type, HolidayCalculationType::NthWeekday);
        assert_eq!(r.month, 11);
        assert_eq!(r.day, 4);
        assert_eq!(r.weekday, Weekday::Thursday as i32);
        assert_eq!(r.offset, -4);
    }

    #[test]
    fn holiday_rules_darkmoon_three_locations() {
        // Matches C++ DarkmoonFaire_InHolidayRules test (lines 1219–1238)
        let rules = holiday_rules();
        let elwynn   = rules.iter().find(|r| r.holiday_id == 374).unwrap();
        let thunder  = rules.iter().find(|r| r.holiday_id == 375).unwrap();
        let shattrath = rules.iter().find(|r| r.holiday_id == 376).unwrap();

        assert_eq!(elwynn.calc_type,    HolidayCalculationType::DarkmoonFaire);
        assert_eq!(thunder.calc_type,   HolidayCalculationType::DarkmoonFaire);
        assert_eq!(shattrath.calc_type, HolidayCalculationType::DarkmoonFaire);

        assert_eq!(elwynn.month,    0, "Elwynn offset 0");
        assert_eq!(thunder.month,   1, "Thunder offset 1");
        assert_eq!(shattrath.month, 2, "Shattrath offset 2");
    }
}
