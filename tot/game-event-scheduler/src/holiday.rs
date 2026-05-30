//! Holiday date calculators — ported 1:1 from C++ HolidayDateCalculator.cpp lines 151–480,
//! the rules table at lines 97–144, and the full dispatch + helpers at lines 482–673.
//!
//! **Scope (Tasks 3+4):** Easter Sunday, Nth-weekday, Weekday-on-or-after; astronomical
//! calculators (Lunar New Year, Autumn Equinox, Winter Solstice); Darkmoon Faire helpers;
//! unified `calculate_holiday_date` dispatch over all 8 `HolidayCalculationType` variants;
//! `get_packed_holiday_date`, `get_darkmoon_faire_dates`, `civil_to_unix`,
//! `find_start_time_for_stage`.
//!
//! All pure arithmetic on `CivilDate` via `normalize_date`. No libc / no system calls.
//!
//! **DST caveat:** `civil_to_unix` assumes a fixed UTC offset (no DST).
//! If Task 7 discovers that the server TZ observes DST, Task 6 must handle it by
//! querying the actual UTC offset for the given instant rather than using a fixed offset.

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

/// All 16 holiday rules, ported verbatim from C++ `HolidayRules`.
/// Numeric IDs sourced from `SharedDefines.h` `HolidayIds` enum.
/// Note: the C++ comment says 18 but the actual table at lines 97–144 has 16 entries.
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

// ── Astronomical helpers ───────────────────────────────────────────────────────

use std::f64::consts::PI;

/// Sine of degrees — used throughout the Meeus algorithms.
/// C++ equivalent: `sind(x)` = `sin(x * DEG_TO_RAD)`.
#[inline]
fn sind(deg: f64) -> f64 {
    (deg * PI / 180.0).sin()
}

/// Convert a Julian Day Number to a calendar date.
///
/// Port of `HolidayDateCalculator::DateToJulianDay` (C++ lines 223–233).
fn date_to_julian_day(year: i32, month: i32, day: f64) -> f64 {
    let (year, month) = if month <= 2 {
        (year - 1, month + 12)
    } else {
        (year, month)
    };
    let a = year / 100;
    let b = 2 - a + (a / 4);
    f64::floor(365.25 * (year as f64 + 4716.0))
        + f64::floor(30.6001 * (month as f64 + 1.0))
        + day
        + b as f64
        - 1524.5
}

/// Convert a Julian Day Number back to a calendar (year, month, day).
///
/// Port of `HolidayDateCalculator::JulianDayToDate` (C++ lines 235–253).
/// All `static_cast<int>` → `as i32` (truncation toward zero; all intermediate
/// values here are positive so this is correct, matching C++ behaviour).
fn julian_day_to_date(jd: f64) -> (i32, i32, i32) {
    let jd = jd + 0.5;
    let z = jd as i32; // C++: static_cast<int>(jd)
    let a = if z >= 2299161 {
        let alpha = ((z as f64 - 1_867_216.25) / 36524.25) as i32;
        z + 1 + alpha - (alpha / 4)
    } else {
        z
    };
    let b = a + 1524;
    let c = ((b as f64 - 122.1) / 365.25) as i32;
    let d = (365.25 * c as f64) as i32;
    let e = ((b - d) as f64 / 30.6001) as i32;

    let day = b - d - (30.6001 * e as f64) as i32;
    let month = if e < 14 { e - 1 } else { e - 13 };
    let year = if month > 2 { c - 4716 } else { c - 4715 };
    (year, month, day)
}

/// Calculate the Julian Day of the new moon closest to lunation k.
///
/// Port of `HolidayDateCalculator::CalculateNewMoon` (C++ lines 255–326).
/// All Table 49.A and Table 49.B coefficients are copied verbatim.
fn calculate_new_moon(k: f64) -> f64 {
    let t = k / 1236.85;
    let t2 = t * t;
    let t3 = t2 * t;
    let t4 = t3 * t;

    // Mean phase (Eq 49.1)
    let jde = 2_451_550.097_66
        + 29.530_588_861 * k
        + 0.000_154_37 * t2
        - 0.000_000_150 * t3
        + 0.000_000_000_73 * t4;

    // Eccentricity correction
    let e = 1.0 - 0.002516 * t - 0.0000074 * t2;
    let e2 = e * e;

    // Sun's mean anomaly (Eq 49.4)
    let m = 2.5534 + 29.105_356_70 * k - 0.000_001_4 * t2 - 0.000_000_11 * t3;

    // Moon's mean anomaly (Eq 49.5)
    let mprime = 201.5643
        + 385.816_935_28 * k
        + 0.010_758_2 * t2
        + 0.000_012_38 * t3
        - 0.000_000_058 * t4;

    // Moon's argument of latitude (Eq 49.6)
    let f = 160.7108 + 390.670_502_84 * k
        - 0.001_611_8 * t2
        - 0.000_002_27 * t3
        + 0.000_000_011 * t4;

    // Longitude of ascending node (Eq 49.7)
    let omega = 124.7746 - 1.563_755_88 * k + 0.002_067_2 * t2 + 0.000_002_15 * t3;

    // New Moon corrections (Table 49.A) — copied verbatim, same term order
    let correction = -0.40720 * sind(mprime)
        + 0.17241 * e * sind(m)
        + 0.01608 * sind(2.0 * mprime)
        + 0.01039 * sind(2.0 * f)
        + 0.00739 * e * sind(mprime - m)
        - 0.00514 * e * sind(mprime + m)
        + 0.00208 * e2 * sind(2.0 * m)
        - 0.00111 * sind(mprime - 2.0 * f)
        - 0.00057 * sind(mprime + 2.0 * f)
        + 0.00056 * e * sind(2.0 * mprime + m)
        - 0.00042 * sind(3.0 * mprime)
        + 0.00042 * e * sind(m + 2.0 * f)
        + 0.00038 * e * sind(m - 2.0 * f)
        - 0.00024 * e * sind(2.0 * mprime - m)
        - 0.00017 * sind(omega);

    // Additional planetary corrections (Table 49.B) — all 14, verbatim
    let a1 = 299.77 + 0.107408 * k - 0.009173 * t2;
    let a2 = 251.88 + 0.016321 * k;
    let a3 = 251.83 + 26.651886 * k;
    let a4 = 349.42 + 36.412478 * k;
    let a5 = 84.66 + 18.206239 * k;
    let a6 = 141.74 + 53.303771 * k;
    let a7 = 207.14 + 2.453732 * k;
    let a8 = 154.84 + 7.306860 * k;
    let a9 = 34.52 + 27.261239 * k;
    let a10 = 207.19 + 0.121824 * k;
    let a11 = 291.34 + 1.844379 * k;
    let a12 = 161.72 + 24.198154 * k;
    let a13 = 239.56 + 25.513099 * k;
    let a14 = 331.55 + 3.592518 * k;

    let correction = correction
        + 0.000325 * sind(a1)
        + 0.000165 * sind(a2)
        + 0.000164 * sind(a3)
        + 0.000126 * sind(a4)
        + 0.000110 * sind(a5)
        + 0.000062 * sind(a6)
        + 0.000060 * sind(a7)
        + 0.000056 * sind(a8)
        + 0.000047 * sind(a9)
        + 0.000042 * sind(a10)
        + 0.000040 * sind(a11)
        + 0.000037 * sind(a12)
        + 0.000035 * sind(a13)
        + 0.000023 * sind(a14);

    jde + correction
}

/// Calculate Chinese New Year (the new moon between Jan 21 and Feb 20).
///
/// Port of `HolidayDateCalculator::CalculateLunarNewYear` (C++ lines 328–370).
/// Includes the DeltaT −70/86400 TT→UT correction and +8/24 CST shift.
/// Fallback (should never trigger for 2000–2031): Jan 25 of the given year.
pub fn calculate_lunar_new_year(year: i32) -> CivilDate {
    let jan21_jd = date_to_julian_day(year, 1, 21.0);
    let feb21_jd = date_to_julian_day(year, 2, 21.0);

    let approx_k = (year as f64 - 2000.0) * 12.3685;
    let k = f64::floor(approx_k);

    for i in -2i32..=2 {
        let nm_jde = calculate_new_moon(k + i as f64);

        // TT → UT (DeltaT ≈ 70s for 2020s)
        let mut nm_jd = nm_jde - 70.0 / 86400.0;

        // Add 8 hours for China Standard Time (UTC+8)
        nm_jd += 8.0 / 24.0;

        if nm_jd >= jan21_jd && nm_jd < feb21_jd {
            let (cny_year, cny_month, cny_day) = julian_day_to_date(nm_jd);
            let mut result = CivilDate {
                year: cny_year,
                mon0: cny_month - 1,
                mday: cny_day,
                ..Default::default()
            };
            normalize_date(&mut result);
            return result;
        }
    }

    // Fallback — should never happen for 2000–2031
    let mut fallback = CivilDate {
        year,
        mon0: 0, // January
        mday: 25,
        ..Default::default()
    };
    normalize_date(&mut fallback);
    fallback
}

/// Calculate the date of the Autumn Equinox (September equinox).
///
/// Port of `HolidayDateCalculator::CalculateAutumnEquinox` (C++ lines 378–426).
/// Uses Meeus Chapter 27 algorithm (valid 2000–3000). All 12 cosine terms verbatim.
pub fn calculate_autumn_equinox(year: i32) -> CivilDate {
    let y = (year as f64 - 2000.0) / 1000.0;
    let y2 = y * y;
    let y3 = y2 * y;
    let y4 = y3 * y;

    // Mean September equinox JDE0 (Table 27.C)
    let jde0 = 2_451_810.217_15
        + 365_242.017_67 * y
        - 0.11575 * y2
        + 0.00337 * y3
        + 0.00078 * y4;

    // Periodic terms correction (Table 27.B)
    let t = (jde0 - 2_451_545.0) / 36525.0;
    let w = 35999.373 * t - 2.47;
    let delta_lambda = 1.0
        + 0.0334 * (w * PI / 180.0).cos()
        + 0.0007 * (2.0 * w * PI / 180.0).cos();

    // Simplified correction — 12 cosine terms (Table 27.C), verbatim
    let s = 485.0 * ((324.96 + 1934.136 * t) * PI / 180.0).cos()
        + 203.0 * ((337.23 + 32964.467 * t) * PI / 180.0).cos()
        + 199.0 * ((342.08 + 20.186 * t) * PI / 180.0).cos()
        + 182.0 * ((27.85 + 445267.112 * t) * PI / 180.0).cos()
        + 156.0 * ((73.14 + 45036.886 * t) * PI / 180.0).cos()
        + 136.0 * ((171.52 + 22518.443 * t) * PI / 180.0).cos()
        + 77.0 * ((222.54 + 65928.934 * t) * PI / 180.0).cos()
        + 74.0 * ((296.72 + 3034.906 * t) * PI / 180.0).cos()
        + 70.0 * ((243.58 + 9037.513 * t) * PI / 180.0).cos()
        + 58.0 * ((119.81 + 33718.147 * t) * PI / 180.0).cos()
        + 52.0 * ((297.17 + 150.678 * t) * PI / 180.0).cos()
        + 50.0 * ((21.02 + 2281.226 * t) * PI / 180.0).cos();

    let jde = jde0 + (0.00001 * s) / delta_lambda;

    let (eq_year, eq_month, eq_day) = julian_day_to_date(jde);
    let mut result = CivilDate {
        year: eq_year,
        mon0: eq_month - 1,
        mday: eq_day,
        ..Default::default()
    };
    normalize_date(&mut result);
    result
}

/// Calculate the date of the Winter Solstice (December solstice).
///
/// Port of `HolidayDateCalculator::CalculateWinterSolstice` (C++ lines 433–480).
/// Uses Meeus Chapter 27 algorithm (valid 2000–3000). All 12 cosine terms verbatim.
pub fn calculate_winter_solstice(year: i32) -> CivilDate {
    let y = (year as f64 - 2000.0) / 1000.0;
    let y2 = y * y;
    let y3 = y2 * y;
    let y4 = y3 * y;

    // Mean December solstice JDE0 (Table 27.C)
    let jde0 = 2_451_900.059_52
        + 365_242.740_49 * y
        - 0.06223 * y2
        - 0.00823 * y3
        + 0.00032 * y4;

    // Periodic terms correction (Table 27.B)
    let t = (jde0 - 2_451_545.0) / 36525.0;
    let w = 35999.373 * t - 2.47;
    let delta_lambda = 1.0
        + 0.0334 * (w * PI / 180.0).cos()
        + 0.0007 * (2.0 * w * PI / 180.0).cos();

    // Simplified correction — 12 cosine terms (Table 27.C), verbatim
    let s = 485.0 * ((324.96 + 1934.136 * t) * PI / 180.0).cos()
        + 203.0 * ((337.23 + 32964.467 * t) * PI / 180.0).cos()
        + 199.0 * ((342.08 + 20.186 * t) * PI / 180.0).cos()
        + 182.0 * ((27.85 + 445267.112 * t) * PI / 180.0).cos()
        + 156.0 * ((73.14 + 45036.886 * t) * PI / 180.0).cos()
        + 136.0 * ((171.52 + 22518.443 * t) * PI / 180.0).cos()
        + 77.0 * ((222.54 + 65928.934 * t) * PI / 180.0).cos()
        + 74.0 * ((296.72 + 3034.906 * t) * PI / 180.0).cos()
        + 70.0 * ((243.58 + 9037.513 * t) * PI / 180.0).cos()
        + 58.0 * ((119.81 + 33718.147 * t) * PI / 180.0).cos()
        + 52.0 * ((297.17 + 150.678 * t) * PI / 180.0).cos()
        + 50.0 * ((21.02 + 2281.226 * t) * PI / 180.0).cos();

    let jde = jde0 + (0.00001 * s) / delta_lambda;

    let (sol_year, sol_month, sol_day) = julian_day_to_date(jde);
    let mut result = CivilDate {
        year: sol_year,
        mon0: sol_month - 1,
        mday: sol_day,
        ..Default::default()
    };
    normalize_date(&mut result);
    result
}

// ── Unified dispatch ───────────────────────────────────────────────────────────

/// Calculate the holiday start date for a given rule and year.
///
/// Port of `HolidayDateCalculator::CalculateHolidayDate` (C++ lines 482–573).
/// Covers all 8 `HolidayCalculationType` variants. The DARKMOON_FAIRE branch
/// returns the first qualifying occurrence for the year (C++ lines 553–569).
pub fn calculate_holiday_date(rule: &HolidayRule, year: i32) -> CivilDate {
    match rule.calc_type {
        HolidayCalculationType::FixedDate => {
            let mut result = CivilDate {
                year,
                mon0: rule.month - 1,
                mday: rule.day,
                ..Default::default()
            };
            normalize_date(&mut result);
            result
        }
        HolidayCalculationType::NthWeekday => {
            let mut result = nth_weekday(year, rule.month, rule.weekday, rule.day);
            if rule.offset != 0 {
                result.mday += rule.offset;
                normalize_date(&mut result);
            }
            result
        }
        HolidayCalculationType::EasterOffset => {
            let mut result = easter_sunday(year);
            result.mday += rule.offset;
            normalize_date(&mut result);
            result
        }
        HolidayCalculationType::LunarNewYear => {
            let mut result = calculate_lunar_new_year(year);
            if rule.offset != 0 {
                result.mday += rule.offset;
                normalize_date(&mut result);
            }
            result
        }
        HolidayCalculationType::WeekdayOnOrAfter => {
            let mut result = weekday_on_or_after(year, rule.month, rule.day, rule.weekday);
            if rule.offset != 0 {
                result.mday += rule.offset;
                normalize_date(&mut result);
            }
            result
        }
        HolidayCalculationType::AutumnEquinox => {
            let mut result = calculate_autumn_equinox(year);
            if rule.offset != 0 {
                result.mday += rule.offset;
                normalize_date(&mut result);
            }
            result
        }
        HolidayCalculationType::WinterSolstice => {
            let mut result = calculate_winter_solstice(year);
            if rule.offset != 0 {
                result.mday += rule.offset;
                normalize_date(&mut result);
            }
            result
        }
        HolidayCalculationType::DarkmoonFaire => {
            // Return the first occurrence for the year.
            // rule.month contains the location offset (0, 1, or 2).
            let location_offset = rule.month;
            for month in 1i32..=12 {
                if month % 3 == location_offset {
                    return nth_weekday(year, month, Weekday::Sunday as i32, 1);
                }
            }
            // Should never reach here (every locationOffset 0/1/2 has a match by month 3)
            CivilDate { year, ..Default::default() }
        }
    }
}

/// Look up the rule for `holiday_id`, compute the date, and return a packed u32.
///
/// Port of `HolidayDateCalculator::GetPackedHolidayDate` (C++ lines 608–619).
/// Returns 0 if the holiday_id is not found.
pub fn get_packed_holiday_date(holiday_id: u32, year: i32) -> u32 {
    use crate::packed::pack_date;
    for rule in HOLIDAY_RULES {
        if rule.holiday_id == holiday_id {
            let date = calculate_holiday_date(rule, year);
            return pack_date(&date);
        }
    }
    0
}

/// Return packed dates for all Darkmoon Faire occurrences in the given year range.
///
/// Port of `HolidayDateCalculator::GetDarkmoonFaireDates` (C++ lines 621–649).
/// `location_offset`: 0 = Elwynn (Mar/Jun/Sep/Dec), 1 = Mulgore (Jan/Apr/Jul/Oct),
///                    2 = Terokkar (Feb/May/Aug/Nov).
/// `start_year`: first year to compute.
/// `num_years`: how many years to compute (capped at year ≤ 2030, matching C++).
/// `day_offset`: applied to each first-Sunday date before packing (e.g. −2).
pub fn get_darkmoon_faire_dates(
    location_offset: i32,
    start_year: i32,
    num_years: i32,
    day_offset: i32,
) -> Vec<u32> {
    use crate::packed::pack_date;
    let mut dates = Vec::new();
    let mut year = start_year;
    while year < start_year + num_years && year <= 2030 {
        for month in 1i32..=12 {
            if month % 3 == location_offset {
                let mut date = nth_weekday(year, month, Weekday::Sunday as i32, 1);
                if day_offset != 0 {
                    date.mday += day_offset;
                    normalize_date(&mut date);
                }
                dates.push(pack_date(&date));
            }
        }
        year += 1;
    }
    dates
}

// ── Civil-to-unix + FindStartTimeForStage ─────────────────────────────────────

/// Convert a `CivilDate` to a Unix timestamp (seconds since 1970-01-01T00:00:00Z),
/// using a fixed timezone offset (no DST adjustment).
///
/// This is the `mktime`-replacement used by `find_start_time_for_stage`.
///
/// The calculation:
///   1. Count days from 1970-01-01 to the date via the Julian Day method.
///   2. Multiply by 86400, add h*3600 + m*60 + s.
///   3. Subtract `tz_offset_secs` (positive = east of UTC, e.g. UTC+8 → 28800).
///
/// **DST caveat:** assumes a fixed offset. If the server TZ observes DST,
/// Task 6 must supply the correct offset for the given instant; leave a
/// `// TODO(Task 6): DST` comment at the call site if needed.
///
/// C++ note: `FindStartTimeForStage` uses `tm_year = ((date>>24)&0x1F) + 100`
/// which maps to year = 2000 + offset (since tm_year is years since 1900 and
/// the packed year-offset is from 2000, so offset+100 ≡ offset+2000−1900).
pub fn civil_to_unix(date: &CivilDate, tz_offset_secs: i32) -> i64 {
    // Julian Day for 1970-01-01 midnight is 2440587.5.
    // date_to_julian_day(y, m, d) with integer d returns the JD at midnight (JD + 0.5 fractional).
    let jd = date_to_julian_day(date.year, date.mon0 + 1, date.mday as f64);
    let days_since_epoch = (jd - 2_440_587.5).floor() as i64;
    let secs = days_since_epoch * 86400
        + date.hour as i64 * 3600
        + date.min as i64 * 60;
    secs - tz_offset_secs as i64
}

/// Find the start time of a stage within a sequence of packed holiday dates.
///
/// Port of `HolidayDateCalculator::FindStartTimeForStage` (C++ lines 651–673).
///
/// Iterates `packed_dates` (stopping at the first 0 entry), unpacks each date to a
/// Unix timestamp using `civil_to_unix(date, tz_offset_secs)`, and returns
/// `start_time + stage_offset_secs` for the first date where:
///   `cur_time < start_time + stage_offset_secs + stage_length_minutes * 60`
///
/// Returns 0 if no qualifying date is found (all dates exhausted or past).
///
/// **Unit tests use `tz_offset_secs = 0` (UTC)** so the expected values can be
/// verified against known Unix timestamps without local-TZ ambiguity.
/// The real server TZ is wired in Task 6.
///
/// **DST caveat:** `civil_to_unix` uses a fixed offset; see its doc comment.
pub fn find_start_time_for_stage(
    packed_dates: &[u32],
    stage_offset_secs: i64,
    stage_length_minutes: u32,
    cur_time: i64,
    tz_offset_secs: i32,
) -> i64 {
    for &date_packed in packed_dates {
        if date_packed == 0 {
            break;
        }
        // Unpack fields exactly as C++ lines 659–665:
        //   tm_year = ((date >> 24) & 0x1F) + 100  →  year = 2000 + offset
        let year_offset = ((date_packed >> 24) & 0x1F) as i32;
        let year = 2000 + year_offset;
        let mon0 = ((date_packed >> 20) & 0xF) as i32;
        let mday = (((date_packed >> 14) & 0x3F) + 1) as i32;
        let hour = ((date_packed >> 6) & 0x1F) as i32;
        let min = (date_packed & 0x3F) as i32;

        let date = CivilDate { year, mon0, mday, hour, min, wday: 0 };
        let start_time = civil_to_unix(&date, tz_offset_secs);

        if cur_time < start_time + stage_offset_secs + (stage_length_minutes as i64) * 60 {
            return start_time + stage_offset_secs;
        }
    }
    0
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

    // ── Lunar New Year known-date vectors (C++ test lines 811–860) ────────────

    #[test]
    fn lunar_new_year_known_dates() {
        // Source: Official astronomical calculations and historical records.
        // All 31 cases from C++ LunarNewYear_KnownDates (lines 816–852).
        let cases: &[(i32, i32, i32)] = &[
            (2000, 2,  5),
            (2001, 1, 24),
            (2002, 2, 12),
            (2003, 2,  1),
            (2004, 1, 22),
            (2005, 2,  9),
            (2006, 1, 29),
            (2007, 2, 18),
            (2008, 2,  7),
            (2009, 1, 26),
            (2010, 2, 14),
            (2011, 2,  3),
            (2012, 1, 23),
            (2013, 2, 10),
            (2014, 1, 31),
            (2015, 2, 19),
            (2016, 2,  8),
            (2017, 1, 28),
            (2018, 2, 16),
            (2019, 2,  5),
            (2020, 1, 25),
            (2021, 2, 12),
            (2022, 2,  1),
            (2023, 1, 22),
            (2024, 2, 10),
            (2025, 1, 29),
            (2026, 2, 17),
            (2027, 2,  6),
            (2028, 1, 26),
            (2029, 2, 13),
            (2030, 2,  3),
            (2031, 1, 23),
        ];
        for &(y, m, d) in cases {
            let r = calculate_lunar_new_year(y);
            assert_eq!((r.year, r.mon0 + 1, r.mday), (y, m, d), "lunar new year {y}");
        }
    }

    // ── Lunar New Year range property (C++ test lines 862–895) ───────────────

    #[test]
    fn lunar_new_year_valid_range_2000_2031() {
        // Chinese New Year must fall between Jan 21 and Feb 20 (inclusive).
        for year in 2000..=2031 {
            let r = calculate_lunar_new_year(year);
            assert_eq!(r.year, year, "year field correct for {year}");
            assert!(
                r.mon0 == 0 || r.mon0 == 1,
                "lunar new year {year}: mon0={} must be 0 (January) or 1 (February)",
                r.mon0
            );
            if r.mon0 == 0 {
                assert!(r.mday >= 21, "lunar new year {year}: January day {} >= 21", r.mday);
                assert!(r.mday <= 31, "lunar new year {year}: January day {} <= 31", r.mday);
            } else {
                assert!(r.mday >= 1,  "lunar new year {year}: February day {} >= 1", r.mday);
                assert!(r.mday <= 20, "lunar new year {year}: February day {} <= 20", r.mday);
            }
        }
    }

    // ── Lunar Festival (CNY − 1) known dates (C++ test lines 922–942) ─────────

    #[test]
    fn lunar_festival_known_dates() {
        // Lunar Festival = CNY − 1 day.  Rule: holiday_id=327, offset=-1.
        let rule = HolidayRule {
            holiday_id: 327,
            calc_type: HolidayCalculationType::LunarNewYear,
            month: 0,
            day: 0,
            weekday: 0,
            offset: -1,
        };
        let cases: &[(i32, i32, i32)] = &[
            (2024, 2,  9),  // CNY Feb 10 − 1
            (2025, 1, 28),  // CNY Jan 29 − 1
            (2026, 2, 16),  // CNY Feb 17 − 1
            (2027, 2,  5),  // CNY Feb 6  − 1
        ];
        for &(y, m, d) in cases {
            let r = calculate_holiday_date(&rule, y);
            assert_eq!((r.year, r.mon0 + 1, r.mday), (y, m, d), "lunar festival {y}");
        }
    }

    // ── Autumn Equinox range property (C++ test lines 447–485) ───────────────

    #[test]
    fn autumn_equinox_always_september_2000_2030() {
        // Autumn equinox is always in September; Harvest Festival (equinox − 2)
        // is always between Sept 18 and Sept 22.
        let rule = HolidayRule {
            holiday_id: 321,
            calc_type: HolidayCalculationType::AutumnEquinox,
            month: 0, day: 0, weekday: 0,
            offset: -2,
        };
        for year in 2000..=2030 {
            let equinox = calculate_autumn_equinox(year);
            assert_eq!(equinox.mon0 + 1, 9,
                "autumn equinox {year}: must be September, got mon0={}", equinox.mon0);

            let harvest = calculate_holiday_date(&rule, year);
            assert_eq!(harvest.mon0 + 1, 9,
                "harvest festival {year}: must be September, got mon0={}", harvest.mon0);
            assert!(harvest.mday >= 18,
                "harvest festival {year}: day {} must be >= 18", harvest.mday);
            assert!(harvest.mday <= 22,
                "harvest festival {year}: day {} must be <= 22", harvest.mday);
        }
    }

    #[test]
    fn autumn_equinox_always_september_1900_2200() {
        // Broader range: Harvest Festival (equinox − 2) always in September, >= 18, <= 22.
        let rule = HolidayRule {
            holiday_id: 321,
            calc_type: HolidayCalculationType::AutumnEquinox,
            month: 0, day: 0, weekday: 0,
            offset: -2,
        };
        for year in 1900..=2200 {
            let d = calculate_holiday_date(&rule, year);
            assert_eq!(d.mon0 + 1, 9, "harvest festival {year}: must be September");
            assert!(d.mday >= 18, "harvest festival {year}: day {} >= 18", d.mday);
            assert!(d.mday <= 22, "harvest festival {year}: day {} <= 22", d.mday);
        }
    }

    // ── Winter Solstice range property (C++ test lines 492–528) ──────────────

    #[test]
    fn winter_veil_always_december_2000_2030() {
        // Winter Veil = solstice − 6 days. Solstice is Dec 21-22, so Winter Veil is Dec 15-16.
        let rule = HolidayRule {
            holiday_id: 141,
            calc_type: HolidayCalculationType::WinterSolstice,
            month: 0, day: 0, weekday: 0,
            offset: -6,
        };
        for year in 2000..=2030 {
            let solstice = calculate_winter_solstice(year);
            assert_eq!(solstice.mon0 + 1, 12,
                "winter solstice {year}: must be December, got mon0={}", solstice.mon0);

            let wv = calculate_holiday_date(&rule, year);
            assert_eq!(wv.mon0 + 1, 12, "winter veil {year}: must be December");
            assert!(wv.mday >= 14, "winter veil {year}: day {} >= 14", wv.mday);
            assert!(wv.mday <= 17, "winter veil {year}: day {} <= 17", wv.mday);
        }
    }

    #[test]
    fn winter_veil_always_december_1900_2200() {
        let rule = HolidayRule {
            holiday_id: 141,
            calc_type: HolidayCalculationType::WinterSolstice,
            month: 0, day: 0, weekday: 0,
            offset: -6,
        };
        for year in 1900..=2200 {
            let d = calculate_holiday_date(&rule, year);
            assert_eq!(d.mon0 + 1, 12, "winter veil {year}: must be December");
            assert!(d.mday >= 14, "winter veil {year}: day {} >= 14", d.mday);
            assert!(d.mday <= 17, "winter veil {year}: day {} <= 17", d.mday);
        }
    }

    // ── Darkmoon Faire GetDarkmoonFaireDates (C++ test lines 1060–1165) ───────

    #[test]
    fn darkmoon_faire_dates_elwynn_2025_one_year() {
        // Elwynn (offset 0): Mar, Jun, Sep, Dec — 4 dates, no day_offset
        let dates = get_darkmoon_faire_dates(0, 2025, 1, 0);
        assert_eq!(dates.len(), 4, "Elwynn should have 4 dates for 2025");
        let expected_months = [3, 6, 9, 12];
        for (i, &packed) in dates.iter().enumerate() {
            // Unpack manually to verify
            let year_offset = ((packed >> 24) & 0x1F) as i32;
            let mon0 = ((packed >> 20) & 0xF) as i32;
            let mday = (((packed >> 14) & 0x3F) + 1) as i32;
            let wday = ((packed >> 11) & 0x7) as i32;
            let year = 2000 + year_offset;
            assert_eq!(year, 2025, "elwynn date {i}: year");
            assert_eq!(mon0 + 1, expected_months[i], "elwynn date {i}: month");
            assert_eq!(wday, 0, "elwynn date {i}: must be Sunday (wday=0)");
            assert!((1..=7).contains(&mday), "elwynn date {i}: first-sunday day {mday}");
        }
    }

    #[test]
    fn darkmoon_faire_dates_mulgore_2025_one_year() {
        // Mulgore (offset 1): Jan, Apr, Jul, Oct — 4 dates
        let dates = get_darkmoon_faire_dates(1, 2025, 1, 0);
        assert_eq!(dates.len(), 4, "Mulgore should have 4 dates for 2025");
        let expected_months = [1, 4, 7, 10];
        for (i, &packed) in dates.iter().enumerate() {
            let year_offset = ((packed >> 24) & 0x1F) as i32;
            let mon0 = ((packed >> 20) & 0xF) as i32;
            let wday = ((packed >> 11) & 0x7) as i32;
            let year = 2000 + year_offset;
            assert_eq!(year, 2025, "mulgore date {i}: year");
            assert_eq!(mon0 + 1, expected_months[i], "mulgore date {i}: month");
            assert_eq!(wday, 0, "mulgore date {i}: must be Sunday");
        }
    }

    #[test]
    fn darkmoon_faire_dates_terokkar_2025_one_year() {
        // Terokkar (offset 2): Feb, May, Aug, Nov — 4 dates
        let dates = get_darkmoon_faire_dates(2, 2025, 1, 0);
        assert_eq!(dates.len(), 4, "Terokkar should have 4 dates for 2025");
        let expected_months = [2, 5, 8, 11];
        for (i, &packed) in dates.iter().enumerate() {
            let year_offset = ((packed >> 24) & 0x1F) as i32;
            let mon0 = ((packed >> 20) & 0xF) as i32;
            let wday = ((packed >> 11) & 0x7) as i32;
            let year = 2000 + year_offset;
            assert_eq!(year, 2025, "terokkar date {i}: year");
            assert_eq!(mon0 + 1, expected_months[i], "terokkar date {i}: month");
            assert_eq!(wday, 0, "terokkar date {i}: must be Sunday");
        }
    }

    #[test]
    fn darkmoon_faire_dates_multi_year_elwynn() {
        // 4 years * 4 dates/year = 16 dates (C++ test lines 1126–1145)
        let dates = get_darkmoon_faire_dates(0, 2025, 4, 0);
        assert_eq!(dates.len(), 16, "Elwynn 4 years: expected 16 dates");
    }

    #[test]
    fn darkmoon_faire_all_sundays_all_locations_2000_2030() {
        // All Darkmoon Faire dates should be Sundays (C++ test lines 1147–1165)
        for offset in 0i32..=2 {
            let dates = get_darkmoon_faire_dates(offset, 2000, 31, 0);
            for (i, &packed) in dates.iter().enumerate() {
                let wday = ((packed >> 11) & 0x7) as i32;
                assert_eq!(wday, 0,
                    "offset={offset} date[{i}]: expected Sunday (wday=0), got wday={wday}");
            }
        }
    }

    #[test]
    fn darkmoon_faire_no_overlap_all_locations() {
        // All three locations are in different months, so no date can overlap
        // (C++ test lines 1192–1217).
        for year in 2000..=2030 {
            let elwynn   = get_darkmoon_faire_dates(0, year, 1, 0);
            let mulgore  = get_darkmoon_faire_dates(1, year, 1, 0);
            let terokkar = get_darkmoon_faire_dates(2, year, 1, 0);
            for &e in &elwynn {
                for &m in &mulgore { assert_ne!(e, m, "elwynn/mulgore overlap year {year}"); }
                for &t in &terokkar { assert_ne!(e, t, "elwynn/terokkar overlap year {year}"); }
            }
            for &m in &mulgore {
                for &t in &terokkar { assert_ne!(m, t, "mulgore/terokkar overlap year {year}"); }
            }
        }
    }

    #[test]
    fn darkmoon_calculate_holiday_date_returns_first_occurrence() {
        // C++ test lines 1167–1190: CalculateHolidayDate with DARKMOON_FAIRE
        // returns the first qualifying month for the year.
        let elwynn_rule = HolidayRule {
            holiday_id: 374, calc_type: HolidayCalculationType::DarkmoonFaire,
            month: 0, day: 0, weekday: 0, offset: -2,
        };
        let mulgore_rule = HolidayRule {
            holiday_id: 375, calc_type: HolidayCalculationType::DarkmoonFaire,
            month: 1, day: 0, weekday: 0, offset: -2,
        };
        let terokkar_rule = HolidayRule {
            holiday_id: 376, calc_type: HolidayCalculationType::DarkmoonFaire,
            month: 2, day: 0, weekday: 0, offset: -2,
        };

        let elwynn = calculate_holiday_date(&elwynn_rule, 2025);
        // Elwynn offset=0: first month where month%3==0 is 3 (March)
        assert_eq!(elwynn.mon0 + 1, 3, "elwynn 2025: first occurrence should be March");
        assert_eq!(elwynn.wday, 0, "elwynn 2025: should be Sunday");

        let mulgore = calculate_holiday_date(&mulgore_rule, 2025);
        // Mulgore offset=1: first month where month%3==1 is 1 (January)
        assert_eq!(mulgore.mon0 + 1, 1, "mulgore 2025: first occurrence should be January");
        assert_eq!(mulgore.wday, 0, "mulgore 2025: should be Sunday");

        let terokkar = calculate_holiday_date(&terokkar_rule, 2025);
        // Terokkar offset=2: first month where month%3==2 is 2 (February)
        assert_eq!(terokkar.mon0 + 1, 2, "terokkar 2025: first occurrence should be February");
        assert_eq!(terokkar.wday, 0, "terokkar 2025: should be Sunday");
    }

    // ── get_packed_holiday_date (C++ test line 718–723) ───────────────────────

    #[test]
    fn get_packed_holiday_date_unknown_returns_zero() {
        assert_eq!(get_packed_holiday_date(99999, 2025), 0);
    }

    #[test]
    fn get_packed_holiday_date_easter_2026() {
        // Easter 2026 is Apr 5 (Sunday). Noblegarden (id=181, Easter+1) is Apr 6 (Monday).
        // Packed: yearOffset=26, month=3 (0-indexed Apr), day=5 (0-indexed for Apr 6), wday=1
        let packed = get_packed_holiday_date(181, 2026);
        let year_offset = ((packed >> 24) & 0x1F) as i32;
        let mon0 = ((packed >> 20) & 0xF) as i32;
        let mday = (((packed >> 14) & 0x3F) + 1) as i32;
        let wday = ((packed >> 11) & 0x7) as i32;
        assert_eq!(2000 + year_offset, 2026, "packed year");
        assert_eq!(mon0 + 1, 4, "packed month (April)");
        assert_eq!(mday, 6, "packed day");
        assert_eq!(wday, 1, "packed wday (Monday)");
    }

    // ── civil_to_unix UTC sanity check ────────────────────────────────────────

    #[test]
    fn civil_to_unix_epoch() {
        // Unix epoch: 1970-01-01T00:00:00 UTC → 0
        use crate::holiday::civil_to_unix;
        let d = crate::packed::CivilDate {
            year: 1970, mon0: 0, mday: 1, hour: 0, min: 0, wday: 4,
        };
        assert_eq!(civil_to_unix(&d, 0), 0, "Unix epoch should be 0");
    }

    #[test]
    fn civil_to_unix_known_date() {
        // 2026-03-06 00:00:00 UTC = Unix timestamp 1772755200
        // (verified: python3 -c "import datetime; print(int(datetime.datetime(2026,3,6,tzinfo=datetime.timezone.utc).timestamp()))")
        use crate::holiday::civil_to_unix;
        let d = crate::packed::CivilDate {
            year: 2026, mon0: 2, mday: 6, hour: 0, min: 0, wday: 5,
        };
        assert_eq!(civil_to_unix(&d, 0), 1_772_755_200, "2026-03-06 UTC");
    }

    // ── find_start_time_for_stage with tz_offset=0 (UTC) ─────────────────────

    // Helper: pack a date (year, 1-indexed month, day) at midnight UTC.
    fn pack_date_utc(year: i32, month: i32, day: i32) -> u32 {
        use crate::packed::{normalize_date, pack_date, CivilDate};
        let mut d = CivilDate { year, mon0: month - 1, mday: day, ..Default::default() };
        normalize_date(&mut d);
        pack_date(&d)
    }

    #[test]
    fn find_start_time_stage1_before_start_selects_first_date() {
        // C++ test Stage1_BeforeStart_SelectsFirstDate (lines 1284–1295).
        // Two dates: 2026-03-06 and 2026-04-03, stageOffset=0, length=72h=4320min.
        // curTime = 2026-03-01 (before first date). Should select 2026-03-06.
        // Timestamps (UTC, verified via Python):
        //   2026-03-01 = 1_772_323_200, 2026-03-06 = 1_772_755_200, 2026-04-03 = 1_775_174_400
        let mut dates = vec![
            pack_date_utc(2026, 3, 6),
            pack_date_utc(2026, 4, 3),
        ];
        dates.resize(26, 0);

        let cur_time: i64 = 1_772_323_200; // 2026-03-01 00:00:00 UTC
        let result = find_start_time_for_stage(&dates, 0, 72 * 60, cur_time, 0);
        assert_eq!(result, 1_772_755_200, // 2026-03-06 00:00:00 UTC
            "stage1 before start: should select 2026-03-06");
    }

    #[test]
    fn find_start_time_stage1_during_event_selects_current_date() {
        // C++ test Stage1_DuringEvent_SelectsCurrentDate (lines 1298–1309).
        // curTime = 2026-03-07 12:00:00 (mid-event), same expected result 2026-03-06.
        // 2026-03-07 12:00:00 UTC = 1_772_884_800
        let mut dates = vec![
            pack_date_utc(2026, 3, 6),
            pack_date_utc(2026, 4, 3),
        ];
        dates.resize(26, 0);

        let cur_time: i64 = 1_772_884_800; // 2026-03-07 12:00:00 UTC
        let result = find_start_time_for_stage(&dates, 0, 72 * 60, cur_time, 0);
        assert_eq!(result, 1_772_755_200, "stage1 during: should select 2026-03-06");
    }

    #[test]
    fn find_start_time_stage1_after_end_selects_next_date() {
        // C++ test Stage1_AfterEnd_SelectsNextDate (lines 1312–1323).
        // curTime = 2026-03-20, should select 2026-04-03.
        // 2026-03-20 = 1_773_964_800, 2026-04-03 = 1_775_174_400
        let mut dates = vec![
            pack_date_utc(2026, 3, 6),
            pack_date_utc(2026, 4, 3),
        ];
        dates.resize(26, 0);

        let cur_time: i64 = 1_773_964_800; // 2026-03-20 00:00:00 UTC
        let result = find_start_time_for_stage(&dates, 0, 72 * 60, cur_time, 0);
        let apr3: i64 = 1_775_174_400; // 2026-04-03 00:00:00 UTC
        assert_eq!(result, apr3, "stage1 after end: should select 2026-04-03");
    }

    #[test]
    fn find_start_time_stage2_during_late_window() {
        // C++ test Stage2_DuringLateWindow_SelectsCurrentDate (lines 1329–1351).
        // stageOffset = 72h = 259200s. stage2 length = 168h = 10080min.
        // curTime = 2026-03-14 12:00:00 (day 8 of holiday, day 5 of stage 2).
        // Expected result: 2026-03-06 + stageOffset = 2026-03-09 00:00:00 UTC.
        // 2026-03-14 12:00 = 1_773_489_600, 2026-03-06 = 1_772_755_200
        let mut dates = vec![
            pack_date_utc(2026, 3, 6),
            pack_date_utc(2026, 4, 3),
        ];
        dates.resize(26, 0);

        let stage_offset: i64 = 72 * 3600; // 259200
        let cur_time: i64 = 1_773_489_600; // 2026-03-14 12:00:00 UTC
        let result = find_start_time_for_stage(&dates, stage_offset, 168 * 60, cur_time, 0);
        let mar6: i64 = 1_772_755_200;
        assert_eq!(result, mar6 + stage_offset, // 2026-03-09 00:00:00 UTC
            "stage2 late window: should select 2026-03-06 + stageOffset");
    }

    #[test]
    fn find_start_time_stage2_after_holiday_ends_selects_next() {
        // C++ test Stage2_AfterHolidayEnds_SelectsNextDate (lines 1354–1367).
        // curTime = 2026-03-20. Expected: 2026-04-03 + stageOffset.
        // 2026-03-20 = 1_773_964_800, 2026-04-03 = 1_775_174_400
        let mut dates = vec![
            pack_date_utc(2026, 3, 6),
            pack_date_utc(2026, 4, 3),
        ];
        dates.resize(26, 0);

        let stage_offset: i64 = 72 * 3600;
        let cur_time: i64 = 1_773_964_800; // 2026-03-20 00:00:00 UTC
        let result = find_start_time_for_stage(&dates, stage_offset, 168 * 60, cur_time, 0);
        let apr3: i64 = 1_775_174_400; // 2026-04-03 00:00:00 UTC
        assert_eq!(result, apr3 + stage_offset, "stage2 after end: should select apr3 + offset");
    }

    #[test]
    fn find_start_time_no_dates_returns_zero() {
        // C++ test NoDates_ReturnsZero (lines 1370–1377).
        let dates = vec![0u32; 26];
        // cur_time = some future date in 2026 — irrelevant since all entries are 0
        let result = find_start_time_for_stage(&dates, 0, 168 * 60, 1_780_272_000, 0);
        assert_eq!(result, 0, "no dates: should return 0");
    }

    #[test]
    fn find_start_time_all_dates_past_returns_zero() {
        // C++ test AllDatesPast_ReturnsZero (lines 1380–1391).
        // Two past dates (2026-01-05, 2026-02-06), curTime = 2026-06-01.
        // 2026-01-05 = 1_767_571_200, 2026-02-06 = 1_770_336_000, 2026-06-01 = 1_780_272_000
        let mut dates = vec![
            pack_date_utc(2026, 1, 5),
            pack_date_utc(2026, 2, 6),
        ];
        dates.resize(26, 0);

        let cur_time: i64 = 1_780_272_000; // 2026-06-01 00:00:00 UTC
        let result = find_start_time_for_stage(&dates, 0, 72 * 60, cur_time, 0);
        assert_eq!(result, 0, "all dates past: should return 0");
    }
}
