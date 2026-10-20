//! Unlock durations (`30m`, `24h`, `365d`, `1y`) and RFC 3339 timestamps.

use crate::core::error::{Result, WalletError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Parses a positional unlock duration: a positive integer followed by one unit of
/// `s`, `m`, `h`, `d`, `w`, or `y` (`1y` = 365 days).
pub fn parse_duration(input: &str) -> Result<Duration> {
    let s = input.trim();
    let bad = |why: &str| {
        WalletError::validation(format!(
            "invalid duration '{input}': {why} (examples: 30m, 24h, 365d, 1y; units s m h d w y)"
        ))
    };
    let unit_at = s
        .find(|c: char| !c.is_ascii_digit())
        .ok_or_else(|| bad("missing unit"))?;
    let (digits, unit) = s.split_at(unit_at);
    if digits.is_empty() {
        return Err(bad("expected a positive whole number before the unit"));
    }
    let n: u64 = digits.parse().map_err(|_| bad("number too large"))?;
    if n == 0 {
        return Err(bad("must be greater than zero"));
    }
    let unit_secs: u64 = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3_600,
        "d" => 86_400,
        "w" => 7 * 86_400,
        "y" => 365 * 86_400,
        _ => return Err(bad("unknown unit")),
    };
    let secs = n.checked_mul(unit_secs).ok_or_else(|| bad("too long"))?;
    // Must stay representable as a wall-clock deadline.
    SystemTime::now()
        .checked_add(Duration::from_secs(secs))
        .ok_or_else(|| bad("too long"))?;
    Ok(Duration::from_secs(secs))
}

pub fn unix_secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn now_rfc3339() -> String {
    rfc3339_utc(unix_secs(SystemTime::now()))
}

/// Formats UNIX seconds as RFC 3339 UTC (`2026-10-20T04:05:06Z`).
pub fn rfc3339_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // Civil-from-days (H. Hinnant), proleptic Gregorian.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::error::ErrorCode;

    #[test]
    fn parses_all_units() {
        let secs = |s: &str| parse_duration(s).unwrap().as_secs();
        assert_eq!(secs("1s"), 1);
        assert_eq!(secs("30m"), 1_800);
        assert_eq!(secs("24h"), 86_400);
        assert_eq!(secs("365d"), 365 * 86_400);
        assert_eq!(secs("2w"), 14 * 86_400);
        assert_eq!(secs("1y"), 365 * 86_400);
        assert_eq!(secs(" 1y "), secs("365d"));
    }

    #[test]
    fn rejects_zero_negative_fraction_unknown() {
        for bad in [
            "",
            "0s",
            "0y",
            "-1h",
            "1.5h",
            "10",
            "h",
            "1x",
            "1 h",
            "1hh",
            "1H",
            "+1h",
            "99999999999999999999y",
            "1000000000000y",
        ] {
            let err = parse_duration(bad).unwrap_err();
            assert_eq!(err.code, ErrorCode::ValidationError, "{bad:?}");
        }
    }

    #[test]
    fn rfc3339_known_dates() {
        assert_eq!(rfc3339_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339_utc(1_700_000_000), "2023-11-14T22:13:20Z");
    }
}
