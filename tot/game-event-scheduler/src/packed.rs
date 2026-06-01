//! Packed-date math ported 1:1 from C++ HolidayDateCalculator.cpp lines 30-94, 575-606.
//!
//! Uses a local `CivilDate` struct (no libc `tm`/`mktime`) so the math is
//! platform-independent and fully unit-testable.
//!
//! Field semantics mirror `std::tm`:
//!   mon0  = 0-indexed month (0 = January … 11 = December)
//!   mday  = 1-indexed day-of-month (1..=31)
//!   wday  = 0=Sunday … 6=Saturday  (POSIX convention, same as C++ tm_wday)
//!   hour  = 0-23
//!   min   = 0-59

/// Calendar date with time fields, equivalent to the subset of `std::tm` used by
/// `PackDate`/`UnpackDate`/`NormalizeDate`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CivilDate {
    pub year: i32,
    /// 0-indexed month (0 = January, 11 = December). Mirrors `tm_mon`.
    pub mon0: i32,
    /// 1-indexed day of month. Mirrors `tm_mday`.
    pub mday: i32,
    /// Day of week, 0 = Sunday … 6 = Saturday. Mirrors `tm_wday`.
    pub wday: i32,
    pub hour: i32,
    pub min: i32,
}

impl Default for CivilDate {
    fn default() -> Self {
        CivilDate {
            year: 2000,
            mon0: 0,
            mday: 1,
            wday: 0,
            hour: 0,
            min: 0,
        }
    }
}

// ── helpers (C++ lines 30-39) ────────────────────────────────────────────────

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

fn days_in_month(year: i32, month: i32) -> i32 {
    // month is 1-indexed here (matching C++ DaysInMonth signature)
    const TABLE: [i32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if month == 2 && is_leap_year(year) {
        29
    } else {
        TABLE[(month - 1) as usize]
    }
}

// ── NormalizeDate (C++ lines 41-94) ─────────────────────────────────────────

/// Normalise a `CivilDate` in place: resolve month/day over/underflows, then
/// compute `wday` via Zeller's congruence (C++ comment: "remap 0=Sat to POSIX
/// 0=Sun").
///
/// `d.mon0` is 0-indexed on entry/exit; the arithmetic temporarily uses 1-indexed
/// `month` exactly as the C++ does with `tm_mon+1`.
pub fn normalize_date(d: &mut CivilDate) {
    let mut year = d.year;
    let mut month = d.mon0 + 1; // C++: int month = date.tm_mon + 1
    let mut day = d.mday;       // C++: int day   = date.tm_mday

    // C++ lines 47-56: month overflow/underflow
    while month > 12 {
        month -= 12;
        year += 1;
    }
    while month < 1 {
        month += 12;
        year -= 1;
    }

    // C++ lines 57-66: day overflow (spill into next month)
    while day > days_in_month(year, month) {
        day -= days_in_month(year, month);
        month += 1;
        if month > 12 {
            month = 1;
            year += 1;
        }
    }

    // C++ lines 67-76: day underflow (borrow from previous month)
    while day < 1 {
        month -= 1;
        if month < 1 {
            month = 12;
            year -= 1;
        }
        day += days_in_month(year, month);
    }

    d.year = year;
    d.mon0 = month - 1; // C++: date.tm_mon = month - 1
    d.mday = day;

    // Zeller's congruence (C++ lines 82-93); remap 0=Sat → POSIX 0=Sun.
    let mut zy = year;
    let mut zm = month;
    if zm < 3 {
        zm += 12;
        zy -= 1;
    }
    let k = zy % 100;
    let j = zy / 100;
    let h = (day + (13 * (zm + 1)) / 5 + k + k / 4 + j / 4 + 5 * j) % 7;
    d.wday = (h + 6) % 7; // C++: date.tm_wday = (h + 6) % 7
}

// ── PackDate (C++ lines 575-593) ─────────────────────────────────────────────

/// Pack a `CivilDate` into the WoW packed-date `uint32` format.
///
/// Bit layout (identical to `ByteBuffer::AppendPackedTime`):
///   bits 24-28: year offset from 2000 (clamped to 0 for pre-2000)
///   bits 20-23: month (0-indexed)
///   bits 14-19: day (0-indexed, i.e. `mday - 1`)
///   bits 11-13: weekday (0=Sunday … 6=Saturday)
///   bits  6-10: hour
///   bits  0-5:  minute
///
/// Port divergence from C++ `HolidayDateCalculator::PackDate` (HolidayDateCalculator.cpp:592):
/// The C++ returns only `(yearOffset<<24)|(month<<20)|(day<<14)|(weekday<<11)` — it omits
/// hour/min.  This Rust port additionally packs `(hour<<6)|min` to match the canonical
/// `ByteBuffer::AppendPackedTime` layout that the C++ comment itself references.
/// This is benign for the slice: holiday dates are computed with hour=min=0, so this function
/// produces bit-identical output to C++ for all values this slice actually packs.
/// `unpack_date` must still read hour/min because client-DBC packed dates can carry them.
pub fn pack_date(d: &CivilDate) -> u32 {
    let year_offset: u32 = if d.year < 2000 {
        0
    } else {
        (d.year - 2000) as u32
    };
    let month: u32 = d.mon0 as u32;           // already 0-indexed
    let day: u32 = (d.mday - 1) as u32;       // convert to 0-indexed
    let weekday: u32 = d.wday as u32;         // 0=Sunday … 6=Saturday
    let hour: u32 = d.hour as u32;
    let minute: u32 = d.min as u32;

    (year_offset << 24) | (month << 20) | (day << 14) | (weekday << 11)
        | (hour << 6) | minute
}

// ── UnpackDate (C++ lines 595-606) ───────────────────────────────────────────

/// Unpack a WoW packed-date `uint32` into a `CivilDate`.
///
/// C++ calls `mktime(&result)` after extracting fields to normalise and fill
/// `tm_yday`/`tm_isdst`.  Here we call `normalize_date` instead so that `wday`
/// is computed deterministically without libc, matching the roundtrip contract.
pub fn unpack_date(packed: u32) -> CivilDate {
    let mut d = CivilDate {
        year: ((packed >> 24) & 0x1F) as i32 + 2000,
        mon0: ((packed >> 20) & 0xF) as i32,
        mday: (((packed >> 14) & 0x3F) + 1) as i32,
        wday: ((packed >> 11) & 0x7) as i32,
        hour: ((packed >> 6) & 0x1F) as i32,
        min: (packed & 0x3F) as i32,
    };
    // Re-derive wday via Zeller's (matches C++ mktime normalisation path).
    normalize_date(&mut d);
    d
}

// ── civil_to_unix / unix_to_civil (calendar primitives) ─────────────────────

/// Convert a `CivilDate` to a Unix timestamp (seconds), applying a fixed TZ offset.
///
/// `tz_offset_secs`: positive = east of UTC (e.g., UTC+8 → 28800).
/// Local time = UTC time + tz_offset_secs → UTC = local − tz_offset_secs.
///
/// Uses the Julian Day algorithm (Meeus, *Astronomical Algorithms*):
/// JD for 1970-01-01 midnight = 2440587.5.
///
/// **DST caveat:** uses a fixed offset. If the server TZ has DST, the caller
/// must supply the correct wall-clock offset for the given instant.
/// Moved here from `holiday.rs` (Tasks 3+4) — calendar primitive belongs next
/// to `CivilDate`.
pub fn civil_to_unix(date: &CivilDate, tz_offset_secs: i32) -> i64 {
    // Julian Day Number at midnight for (year, 1-indexed month, mday).
    let (y, m) = if date.mon0 < 2 {
        (date.year - 1, date.mon0 + 1 + 12)
    } else {
        (date.year, date.mon0 + 1)
    };
    let a = y / 100;
    let b = 2 - a + (a / 4);
    let jd = (365.25 * (y as f64 + 4716.0)).floor()
        + (30.6001 * (m as f64 + 1.0)).floor()
        + date.mday as f64
        + b as f64
        - 1524.5;
    let days_since_epoch = (jd - 2_440_587.5).floor() as i64;
    let secs = days_since_epoch * 86_400
        + date.hour as i64 * 3_600
        + date.min as i64 * 60;
    secs - tz_offset_secs as i64
}

/// Convert a Unix timestamp to a `CivilDate`, applying a fixed TZ offset.
///
/// This is the inverse of `civil_to_unix` — the Rust equivalent of
/// `Acore::Time::TimeBreakdown(t)` (which calls `localtime_r`).
///
/// Algorithm: Hinnant "chrono-Compatible Low-Level Date Algorithms", `civil_from_days`.
/// `tz_offset_secs`: positive = east of UTC. Local time = UTC + tz_offset_secs.
///
/// **DST caveat:** uses a fixed offset.  If the server TZ has DST, the caller
/// must supply the correct wall-clock offset for the given instant.
/// Moved here from `resolve.rs` (Task 6) — calendar primitive belongs next to
/// `CivilDate`.
pub fn unix_to_civil(unix: i64, tz_offset_secs: i32) -> CivilDate {
    // Shift to local time, then decompose.
    let local = unix + tz_offset_secs as i64;

    // Intraday seconds
    let intraday = local.rem_euclid(86_400);
    let hour = (intraday / 3_600) as i32;
    let min = ((intraday % 3_600) / 60) as i32;

    // Days since 1970-01-01
    let days = (local - intraday) / 86_400;

    // Convert days-since-epoch to (year, month, day).
    // Shift epoch to 1 Mar 0000 (makes leap-year handling regular),
    // then apply the 400/100/4-year cycle.
    let z = days + 719_468; // shift to 1 Mar 0000
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // day of era [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // year of era [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day of year [0, 365]
    let mp = (5 * doy + 2) / 153; // month of year [0, 11] within [Mar,Feb]
    let d = doy - (153 * mp + 2) / 5 + 1; // day [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let yr = if m <= 2 { y + 1 } else { y };

    let mut date = CivilDate {
        year: yr as i32,
        mon0: (m - 1) as i32,
        mday: d as i32,
        hour,
        min,
        wday: 0,
    };
    normalize_date(&mut date);
    date
}

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a `CivilDate` from (year, 1-indexed month, day) and call
    /// `normalize_date`, then return it.  Test helper only.
    fn normalized(y: i32, m: i32, d: i32) -> CivilDate {
        let mut date = CivilDate {
            year: y,
            mon0: m - 1,
            mday: d,
            ..Default::default()
        };
        normalize_date(&mut date);
        date
    }

    #[test]
    fn weekday_known_dates() {
        assert_eq!(normalized(2000, 1, 1).wday, 6);  // 2000-01-01 = Saturday
        assert_eq!(normalized(2026, 5, 30).wday, 6); // 2026-05-30 = Saturday
        assert_eq!(normalized(2024, 2, 29).wday, 4); // leap day = Thursday
    }

    #[test]
    fn normalize_month_day_overflow() {
        let d = normalized(2025, 1, 32); // Jan 32 -> Feb 1
        assert_eq!((d.year, d.mon0 + 1, d.mday), (2025, 2, 1));
    }

    #[test]
    fn pack_unpack_roundtrip() {
        let d = normalized(2026, 12, 25);
        let p = pack_date(&d);
        let u = unpack_date(p);
        assert_eq!(
            (u.year, u.mon0, u.mday, u.wday),
            (d.year, d.mon0, d.mday, d.wday)
        );
    }

    #[test]
    fn pack_exact_bits() {
        // 2026-12-25 (Friday=5): yearOffset=26, month=11, day=24, wday=5
        let d = normalized(2026, 12, 25);
        assert_eq!(pack_date(&d), (26u32 << 24) | (11 << 20) | (24 << 14) | (5 << 11));
    }

    #[test]
    fn pack_includes_hour_and_minute() {
        // Verify that hour and minute are packed in the low bits (diverges from C++ PackDate).
        let mut d = CivilDate {
            year: 2026,
            mon0: 11,
            mday: 25,
            hour: 13,
            min: 45,
            ..Default::default()
        };
        normalize_date(&mut d);
        let p = pack_date(&d);
        assert_eq!((p >> 6) & 0x1F, 13, "hour bits mismatch");
        assert_eq!(p & 0x3F, 45, "minute bits mismatch");
        let u = unpack_date(p);
        assert_eq!((u.hour, u.min), (13, 45));
    }

    /// Helper: extract (year, 1-indexed month, day) from a `CivilDate`.
    fn ymd(d: CivilDate) -> (i32, i32, i32) {
        (d.year, d.mon0 + 1, d.mday)
    }

    #[test]
    fn normalize_underflow_and_year_boundary() {
        assert_eq!(ymd(normalized(2025, 1, 0)), (2024, 12, 31)); // day underflow borrows into prev year
        assert_eq!(ymd(normalized(2025, 13, 1)), (2026, 1, 1));  // month overflow rolls into next year
        assert_eq!(ymd(normalized(2024, 3, 0)), (2024, 2, 29));  // leap-year Feb borrow
    }
}
