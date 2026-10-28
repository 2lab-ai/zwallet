//! The Tron token registry (`.prd/05-tron.md` §The registry): the only TRC20 token zwallet reads
//! by name, and later the only one its signer lets leave the wallet. Issuer-direct deployments
//! only, keyed by contract, never by symbol; adding one is a reviewed code change.
//!
//! Deliberately absent: USDC (Circle stopped minting it on TRON on 2024-02-21 and lists no TRON
//! contract), every other TRC20 token, and TRC10 assets.

use super::address;

/// One issuer-direct TRC20 deployment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    /// The registry's name for the asset, and what `--token` takes (in any case).
    pub symbol: &'static str,
    /// What the contract's `symbol()` returns.
    pub onchain_symbol: &'static str,
    /// The `T…` address, verbatim from the source.
    pub address: &'static str,
    /// The contract's `decimals()`.
    pub decimals: u8,
    /// Where the address was copied from.
    pub source: &'static str,
}

/// Every Tron registry token (1 row, fixed 2026-10-28).
pub const TOKENS: &[Token] = &[Token {
    symbol: "USDT",
    onchain_symbol: "USDT",
    // "The integration path is the same as any TRC20 compatible token"; `decimals()` = 6 and
    // `symbol()` = USDT on both measured endpoints (05 M2).
    address: "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t",
    decimals: 6,
    source: "https://tether.to/en/supported-protocols",
}];

/// The registry token whose contract payload is `payload` (`0x41 ‖ body`), if there is one.
pub fn lookup(payload: &[u8; 21]) -> Option<&'static Token> {
    TOKENS
        .iter()
        .find(|t| address::parse(t.address).is_ok_and(|p| p == *payload))
}

/// The registry token `--token` names by its symbol (`usdt`, in any case).
pub fn by_symbol(symbol: &str) -> Option<&'static Token> {
    TOKENS
        .iter()
        .find(|t| t.symbol.eq_ignore_ascii_case(symbol.trim()))
}

/// Whether `payload` is a Tron registry token.
pub fn is_token(payload: &[u8; 21]) -> bool {
    lookup(payload).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_registry_is_tethers_usdt_only() {
        assert_eq!(TOKENS.len(), 1);
        let t = TOKENS[0];
        assert_eq!(
            (t.symbol, t.onchain_symbol, t.address, t.decimals),
            ("USDT", "USDT", "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t", 6)
        );
        assert_eq!(t.source, "https://tether.to/en/supported-protocols");
        // Its payload, as both converters printed it (05 §Derivation).
        let payload = address::parse(t.address).unwrap();
        assert_eq!(
            address::hex41(&payload),
            "41a614f803b6fd780986a42c78ec9c7f77e6ded13c"
        );
        assert_eq!(lookup(&payload), Some(&TOKENS[0]));
        assert!(is_token(&payload));
        assert_eq!(by_symbol("usdt"), Some(&TOKENS[0]));
        assert_eq!(by_symbol("USDT"), Some(&TOKENS[0]));
        // Keyed by contract, never by symbol: no USDC, and another contract is no row.
        assert_eq!(by_symbol("usdc"), None);
        let other = address::parse("TWer2Ygk5TEheHp3TPuYeqxmB6SsGZmaL6").unwrap();
        assert!(!is_token(&other));
    }
}
