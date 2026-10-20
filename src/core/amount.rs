//! Exact decimal <-> base-unit conversion. No floating point anywhere.

use crate::core::error::{Result, WalletError};
use alloy::primitives::U256;

/// Parses a non-negative decimal string (e.g. `"1.5"`) into base units with `decimals` places.
/// Rejects signs, exponents, separators, and more fractional digits than `decimals`.
pub fn parse_units(input: &str, decimals: u8) -> Result<U256> {
    let s = input.trim();
    let bad = || {
        WalletError::validation(format!(
            "invalid amount '{input}': expected a non-negative decimal like 1 or 0.25"
        ))
    };
    if s.is_empty() {
        return Err(bad());
    }
    let (int_part, frac_part) = match s.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (s, None),
    };
    let digits_only = |p: &str| p.bytes().all(|b| b.is_ascii_digit());
    if !digits_only(int_part) {
        return Err(bad());
    }
    let frac = match frac_part {
        Some(f) if f.is_empty() || !digits_only(f) => return Err(bad()),
        Some(f) => f,
        None => "",
    };
    if int_part.is_empty() && frac.is_empty() {
        return Err(bad());
    }
    if frac.len() > decimals as usize {
        return Err(WalletError::validation(format!(
            "amount '{input}' has {} fractional digits; this asset supports at most {decimals}",
            frac.len()
        )));
    }
    let overflow =
        || WalletError::validation(format!("amount '{input}' is too large to represent"));
    let mut combined = String::with_capacity(int_part.len() + decimals as usize);
    combined.push_str(if int_part.is_empty() { "0" } else { int_part });
    combined.push_str(frac);
    for _ in frac.len()..decimals as usize {
        combined.push('0');
    }
    U256::from_str_radix(&combined, 10).map_err(|_| overflow())
}

/// Formats base units as a plain decimal string without trailing zeros (`"1.5"`, `"0"`).
pub fn format_units(value: U256, decimals: u8) -> String {
    let digits = value.to_string();
    let d = decimals as usize;
    if d == 0 {
        return digits;
    }
    let padded = if digits.len() <= d {
        format!("{}{digits}", "0".repeat(d - digits.len() + 1))
    } else {
        digits
    };
    let (int_part, frac_part) = padded.split_at(padded.len() - d);
    let frac = frac_part.trim_end_matches('0');
    if frac.is_empty() {
        int_part.to_string()
    } else {
        format!("{int_part}.{frac}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::error::ErrorCode;

    fn u(s: &str) -> U256 {
        U256::from_str_radix(s, 10).unwrap()
    }

    #[test]
    fn parses_18_decimals_exactly() {
        assert_eq!(parse_units("1", 18).unwrap(), u("1000000000000000000"));
        assert_eq!(parse_units("1.5", 18).unwrap(), u("1500000000000000000"));
        assert_eq!(
            parse_units("0.000000000000000001", 18).unwrap(),
            U256::from(1)
        );
        assert_eq!(parse_units(".25", 18).unwrap(), u("250000000000000000"));
        assert_eq!(parse_units("0", 18).unwrap(), U256::ZERO);
        assert_eq!(
            parse_units("123456789.123456789123456789", 18).unwrap(),
            u("123456789123456789123456789")
        );
    }

    #[test]
    fn parses_6_decimals_exactly() {
        assert_eq!(parse_units("1", 6).unwrap(), U256::from(1_000_000u64));
        assert_eq!(
            parse_units("12.345678", 6).unwrap(),
            U256::from(12_345_678u64)
        );
        assert_eq!(parse_units("0.1", 6).unwrap(), U256::from(100_000u64));
    }

    #[test]
    fn rejects_too_many_fractional_digits() {
        let err = parse_units("1.0000001", 6).unwrap_err();
        assert_eq!(err.code, ErrorCode::ValidationError);
        assert!(err.message.contains("at most 6"), "{}", err.message);
        assert!(parse_units("0.0000000000000000001", 18).is_err());
    }

    #[test]
    fn rejects_malformed_input() {
        for bad in [
            "", " ", "-1", "+1", "1e18", "1,000", "1.", ".", "1.2.3", "abc", "0x10", "1_000",
        ] {
            assert!(parse_units(bad, 18).is_err(), "accepted {bad:?}");
        }
        let too_big = "1".repeat(80);
        assert!(parse_units(&too_big, 18).is_err());
    }

    #[test]
    fn formats_round_trip() {
        assert_eq!(format_units(u("1500000000000000000"), 18), "1.5");
        assert_eq!(format_units(U256::ZERO, 18), "0");
        assert_eq!(format_units(U256::from(1), 18), "0.000000000000000001");
        assert_eq!(format_units(U256::from(12_345_678u64), 6), "12.345678");
        assert_eq!(format_units(U256::from(1_000_000u64), 6), "1");
        assert_eq!(format_units(U256::from(42u64), 0), "42");
        for s in ["1", "0.5", "123.000001"] {
            assert_eq!(format_units(parse_units(s, 6).unwrap(), 6), s);
        }
    }
}
