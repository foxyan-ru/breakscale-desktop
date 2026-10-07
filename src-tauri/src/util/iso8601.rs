//! A minimal epoch -> ISO 8601 UTC formatter, so the crate does not need
//! `chrono`/`time` just to stamp `savedAt: "2024-01-15T10:30:00.000Z"`
//! fields the same way `new Date().toISOString()` does on the TS side (see
//! `designFile.ts`, `backup.ts`). See MIGRATION_PLAN.md #10.
//!
//! The calendar math is Howard Hinnant's `civil_from_days`
//! (<https://howardhinnant.github.io/date_algorithms.html>), public domain,
//! valid across the full range `chrono` would cover for any date this app
//! will ever produce (it only ever stamps "now").

use std::time::{SystemTime, UNIX_EPOCH};

/// Days since the Unix epoch -> (year, month 1-12, day 1-31), proleptic
/// Gregorian, UTC.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Format a Unix timestamp (seconds + sub-second millis) as
/// `YYYY-MM-DDTHH:MM:SS.mmmZ`.
pub fn format_unix_ms(total_ms: i64) -> String {
    let secs = total_ms.div_euclid(1000);
    let millis = total_ms.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let sec_of_day = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let hh = sec_of_day / 3600;
    let mm = (sec_of_day % 3600) / 60;
    let ss = sec_of_day % 60;
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}.{millis:03}Z")
}

/// `new Date().toISOString()`, now.
pub fn now() -> String {
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format_unix_ms(dur.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_formats_as_unix_epoch() {
        assert_eq!(format_unix_ms(0), "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn known_date_matches_js_toisostring() {
        // 2024-01-15T10:30:00.000Z, cross-checked against
        // `new Date('2024-01-15T10:30:00.000Z').getTime()` = 1705314600000.
        assert_eq!(
            format_unix_ms(1_705_314_600_000),
            "2024-01-15T10:30:00.000Z"
        );
    }
}
