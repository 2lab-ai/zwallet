//! Tron addresses: `0x41 ‖ 20-byte body` in base58check (Bitcoin's alphabet, a 4-byte checksum =
//! the first 4 bytes of `sha256(sha256(payload))`), 34 characters starting with `T`. The body is
//! what an EVM address of the same key is: `keccak256(uncompressed public key)[12..32]`.
//!
//! Unlike an EVM address, case is part of the value: two strings that differ only in case are
//! two different strings, and at most one of them passes the checksum. So Tron addresses are
//! compared byte for byte ([`same`]), never with `eq_ignore_ascii_case`.

use crate::core::error::{Result, WalletError};

/// The first byte of every mainnet Tron address payload.
pub const PREFIX: u8 = 0x41;

/// Characters in a Tron address (`T…`).
pub const LEN: usize = 34;

/// The most characters of a refused input an error shows, escaped.
const SHOWN: usize = 64;

/// The 21-byte payload (`0x41 ‖ body`) of the Tron address `input`: 34 characters of the base58
/// alphabet whose checksum holds and whose first byte is `0x41`; else `INVALID_ADDRESS`, the
/// input shown escaped.
pub fn parse(input: &str) -> Result<[u8; 21]> {
    let refused = |why: &str| {
        WalletError::invalid_address(format!(
            "{} is not a Tron address: {why}",
            crate::core::text::shown(input, SHOWN)
        ))
    };
    if input.chars().count() != LEN {
        return Err(refused("a Tron address has 34 characters"));
    }
    let payload = bs58::decode(input).with_check(None).into_vec().map_err(|e| {
        refused(match e {
            bs58::decode::Error::InvalidChecksum { .. } => {
                "its base58check checksum does not match (a letter in the wrong case, or a typo)"
            }
            bs58::decode::Error::InvalidCharacter { .. }
            | bs58::decode::Error::NonAsciiCharacter { .. } => {
                "it has a character outside the base58 alphabet"
            }
            _ => "it does not decode as base58check",
        })
    })?;
    let payload: [u8; 21] = payload
        .try_into()
        .map_err(|_| refused("its payload is not 21 bytes"))?;
    if payload[0] != PREFIX {
        return Err(refused("its first byte is not 0x41 (Tron mainnet)"));
    }
    Ok(payload)
}

/// The `T…` form of a 21-byte payload.
pub fn encode(payload: &[u8; 21]) -> String {
    bs58::encode(payload).with_check().into_string()
}

/// The `T…` address whose payload is `0x41 ‖ body`.
pub fn from_body(body: &[u8; 20]) -> String {
    let mut payload = [0u8; 21];
    payload[0] = PREFIX;
    payload[1..].copy_from_slice(body);
    encode(&payload)
}

/// The payload as Tron's own hex (`41…`, lower case), as file names and node replies write it.
pub fn hex41(payload: &[u8; 21]) -> String {
    alloy::hex::encode(payload)
}

/// Whether `value` is a Tron address ([`parse`] accepts it).
pub fn is_address(value: &str) -> bool {
    parse(value).is_ok()
}

/// Whether `a` and `b` are the same address text: byte equality, without parsing either.
pub fn same(a: &str, b: &str) -> bool {
    a.as_bytes() == b.as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::error::ErrorCode;
    use sha2::{Digest, Sha256};

    /// The A1 mnemonic's key 0 (`m/44'/195'/0'/0/0`), 05-tron.md §Derivation.
    const TWER: &str = "TWer2Ygk5TEheHp3TPuYeqxmB6SsGZmaL6";
    const TWER_HEX: &str = "41e2e1a54926527fbb4e4420de4c6bab82beaee24d";
    /// Tether's USDT on Tron and its payload (05 §Derivation, both converters agreed).
    const USDT: &str = "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t";
    const USDT_HEX: &str = "41a614f803b6fd780986a42c78ec9c7f77e6ded13c";

    /// Every single-letter case flip of [`TWER`] whose flipped letter is in the base58 alphabet,
    /// fixed here (05 M11, 2026-10-28: each decodes as base58 and fails the checksum).
    const TWER_CASE_FLIPS: [&str; 28] = [
        "tWer2Ygk5TEheHp3TPuYeqxmB6SsGZmaL6",
        "Twer2Ygk5TEheHp3TPuYeqxmB6SsGZmaL6",
        "TWEr2Ygk5TEheHp3TPuYeqxmB6SsGZmaL6",
        "TWeR2Ygk5TEheHp3TPuYeqxmB6SsGZmaL6",
        "TWer2ygk5TEheHp3TPuYeqxmB6SsGZmaL6",
        "TWer2YGk5TEheHp3TPuYeqxmB6SsGZmaL6",
        "TWer2YgK5TEheHp3TPuYeqxmB6SsGZmaL6",
        "TWer2Ygk5tEheHp3TPuYeqxmB6SsGZmaL6",
        "TWer2Ygk5TeheHp3TPuYeqxmB6SsGZmaL6",
        "TWer2Ygk5TEHeHp3TPuYeqxmB6SsGZmaL6",
        "TWer2Ygk5TEhEHp3TPuYeqxmB6SsGZmaL6",
        "TWer2Ygk5TEhehp3TPuYeqxmB6SsGZmaL6",
        "TWer2Ygk5TEheHP3TPuYeqxmB6SsGZmaL6",
        "TWer2Ygk5TEheHp3tPuYeqxmB6SsGZmaL6",
        "TWer2Ygk5TEheHp3TpuYeqxmB6SsGZmaL6",
        "TWer2Ygk5TEheHp3TPUYeqxmB6SsGZmaL6",
        "TWer2Ygk5TEheHp3TPuyeqxmB6SsGZmaL6",
        "TWer2Ygk5TEheHp3TPuYEqxmB6SsGZmaL6",
        "TWer2Ygk5TEheHp3TPuYeQxmB6SsGZmaL6",
        "TWer2Ygk5TEheHp3TPuYeqXmB6SsGZmaL6",
        "TWer2Ygk5TEheHp3TPuYeqxMB6SsGZmaL6",
        "TWer2Ygk5TEheHp3TPuYeqxmb6SsGZmaL6",
        "TWer2Ygk5TEheHp3TPuYeqxmB6ssGZmaL6",
        "TWer2Ygk5TEheHp3TPuYeqxmB6SSGZmaL6",
        "TWer2Ygk5TEheHp3TPuYeqxmB6SsgZmaL6",
        "TWer2Ygk5TEheHp3TPuYeqxmB6SsGzmaL6",
        "TWer2Ygk5TEheHp3TPuYeqxmB6SsGZMaL6",
        "TWer2Ygk5TEheHp3TPuYeqxmB6SsGZmAL6",
    ];

    fn payload(hex: &str) -> [u8; 21] {
        alloy::hex::decode(hex).unwrap().try_into().unwrap()
    }

    #[test]
    fn the_usdt_contract_and_key_0_convert_both_ways() {
        for (address, hex) in [(USDT, USDT_HEX), (TWER, TWER_HEX)] {
            assert_eq!(parse(address).unwrap(), payload(hex), "{address}");
            assert_eq!(encode(&payload(hex)), address, "{hex}");
            assert_eq!(hex41(&parse(address).unwrap()), hex, "{address}");
            let body: [u8; 20] = payload(hex)[1..].try_into().unwrap();
            assert_eq!(from_body(&body), address);
            assert!(is_address(address), "{address}");
            assert_eq!(address.len(), LEN);
        }
    }

    #[test]
    fn the_checksum_is_the_first_4_bytes_of_sha256_twice() {
        // Computed here with sha2, apart from the codec: base58 of payload ‖ checksum.
        let p = payload(TWER_HEX);
        let check = Sha256::digest(Sha256::digest(p));
        let mut full = p.to_vec();
        full.extend_from_slice(&check[..4]);
        assert_eq!(encode(&p), bs58::encode(full).into_string());
    }

    #[test]
    fn every_single_letter_case_flip_of_key_0_fails_the_checksum() {
        // The fixed list is every flip: each letter of TWER whose other case is in the alphabet.
        let alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
        let mut flips = Vec::new();
        for (i, c) in TWER.char_indices() {
            let other = if c.is_ascii_lowercase() {
                c.to_ascii_uppercase()
            } else {
                c.to_ascii_lowercase()
            };
            if other != c && alphabet.contains(other) {
                flips.push(format!("{}{other}{}", &TWER[..i], &TWER[i + 1..]));
            }
        }
        assert_eq!(flips, TWER_CASE_FLIPS);
        assert!(parse(TWER).is_ok());
        for flip in TWER_CASE_FLIPS {
            // Every character is in the alphabet, so it is the checksum that refuses it.
            assert!(bs58::decode(flip).into_vec().is_ok(), "{flip}");
            let e = parse(flip).unwrap_err();
            assert_eq!(e.code, ErrorCode::InvalidAddress, "{flip}");
            assert!(e.message.contains("checksum"), "{flip}: {}", e.message);
        }
    }

    #[test]
    fn a_bad_checksum_a_wrong_length_a_wrong_prefix_and_a_0x_string_are_refused() {
        // The last character changed: the checksum fails.
        let bad_checksum = format!("{}7", &TWER[..33]);
        // 0x42 ‖ body with a valid checksum: the prefix is wrong.
        let mut p = payload(TWER_HEX);
        p[0] = 0x42;
        let prefix_42 = bs58::encode(p).with_check().into_string();
        assert_eq!(prefix_42.len(), LEN, "{prefix_42}");
        let refused = [
            (bad_checksum.as_str(), "checksum"),
            (&TWER[..33], "34 characters"),
            (&format!("{TWER}1"), "34 characters"),
            (prefix_42.as_str(), "0x41"),
            (
                "0xe2e1a54926527fbb4e4420de4c6bab82beaee24d",
                "34 characters",
            ),
            ("0x41e2e1a54926527fbb4e4420de4c6bab82be", "34 characters"),
            // 34 characters, one outside the alphabet (`0`).
            ("TWer2Ygk5TEheHp3TPuYeqxmB6SsGZma06", "base58"),
            ("", "34 characters"),
        ];
        for (input, why) in refused {
            let e = parse(input).unwrap_err();
            assert_eq!(e.code, ErrorCode::InvalidAddress, "{input}");
            assert!(e.message.contains(why), "{input}: {}", e.message);
            assert!(!is_address(input), "{input}");
        }
        // Surrounding space is not trimmed into an address either.
        assert!(parse(&format!(" {TWER}")).is_err());
    }

    #[test]
    fn a_refused_input_is_shown_escaped_and_cut() {
        let evil = format!("\u{1b}[2J{}", "T".repeat(500));
        let e = parse(&evil).unwrap_err();
        assert!(!e.message.contains('\u{1b}'), "{}", e.message);
        assert!(e.message.len() < 200, "{}", e.message);
    }

    #[test]
    fn same_is_byte_equality_without_parsing() {
        assert!(same(TWER, TWER));
        // A case flip is another string, though only a case apart.
        assert!(!same(TWER, TWER_CASE_FLIPS[0]));
        assert!(!same(TWER, &TWER.to_ascii_lowercase()));
        // Nothing is parsed: two equal non-addresses are the same text.
        assert!(same("not an address", "not an address"));
        assert!(!same(TWER, &format!("{TWER} ")));
    }
}
