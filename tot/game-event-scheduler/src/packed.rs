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
}
