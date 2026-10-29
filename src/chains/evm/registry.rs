//! The built-in token registry, and the whitelist's per-chain defaults.
//!
//! Registry tokens are the only tokens the signing policy lets leave the wallet
//! (`.prd/04-whitelist.md` §Registry tokens). Only issuer-endorsed deployments are listed: the
//! token's issuer (Circle, Tether/USDT0, Maker/Sky) or, for WETH, the chain's own docs or
//! predeploy. Every address is copied verbatim (EIP-55) from the source named on its row and is
//! keyed by (chainId, address), never by symbol. Trust in a token's behaviour is trust in its
//! issuer; nothing here establishes it.
//!
//! Deliberately absent (not issuer-endorsed, or the sources disagree): bridged USDC (USDC.e,
//! USDbC); USDT and DAI on Base; the chain-list-only USDT on OP Mainnet; every BSC token; DAI and
//! WETH on Polygon (PoS bridge, not Maker); DAI.e, WETH.e and USDT.e on Avalanche; the wrapped
//! natives WPOL, WBNB and WAVAX; USDT, DAI and WETH on Sepolia.

use crate::core::whitelist::ChainRules;
use alloy::hex;
use alloy::primitives::Address;

/// One issuer-endorsed token deployment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub chain_id: u64,
    /// The registry's name for the asset: USDC, USDT, DAI or WETH.
    pub symbol: &'static str,
    /// What the contract's `symbol()` returns. It differs from `symbol` for the USDT0
    /// deployments (`USD₮0`, `USDT0`) and Avalanche's Tether (`USDt`).
    pub onchain_symbol: &'static str,
    /// EIP-55 checksummed, verbatim from the source.
    pub address: &'static str,
    /// The contract's `decimals()`.
    pub decimals: u8,
}

const fn token(
    chain_id: u64,
    symbol: &'static str,
    onchain_symbol: &'static str,
    address: &'static str,
    decimals: u8,
) -> Token {
    Token {
        chain_id,
        symbol,
        onchain_symbol,
        address,
        decimals,
    }
}

/// Every registry token (19 rows, fixed 2026-10-20). Adding one is a reviewed code change.
#[rustfmt::skip]
pub const TOKENS: &[Token] = &[
    // ethereum (1)
    // https://developers.circle.com/stablecoins/usdc-contract-addresses
    token(1, "USDC", "USDC", "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48", 6),
    // https://tether.to/en/supported-protocols (its `transfer()` returns no bool)
    token(1, "USDT", "USDT", "0xdAC17F958D2ee523a2206206994597C13D831ec7", 6),
    // https://chainlog.sky.money/api/mainnet/active.json
    token(1, "DAI", "DAI", "0x6B175474E89094C44Da98b954EedeAC495271d0F", 18),
    // https://chainlog.sky.money/api/mainnet/active.json (WETH9)
    token(1, "WETH", "WETH", "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2", 18),
    // base (8453)
    // https://developers.circle.com/stablecoins/usdc-contract-addresses
    token(8453, "USDC", "USDC", "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913", 6),
    // https://docs.base.org/base-chain/network-information/base-contracts (predeploy)
    token(8453, "WETH", "WETH", "0x4200000000000000000000000000000000000006", 18),
    // arbitrum (42161)
    // https://developers.circle.com/stablecoins/usdc-contract-addresses
    token(42161, "USDC", "USDC", "0xaf88d065e77c8cC2239327C5EDb3A432268e5831", 6),
    // https://docs.usdt0.to/technical-documentation/deployments (USDT0)
    token(42161, "USDT", "USD₮0", "0xFd086bC7CD5C481DCC9C85ebE478A1C0b69FCbb9", 6),
    // https://github.com/makerdao/arbitrum-dai-bridge#deployments
    token(42161, "DAI", "DAI", "0xDA10009cBd5D07dd0CeCc66161FC93D7c9000da1", 18),
    // https://docs.arbitrum.io/for-devs/useful-addresses
    token(42161, "WETH", "WETH", "0x82aF49447D8a07e3bd95BD0d56f35241523fBab1", 18),
    // optimism (10)
    // https://developers.circle.com/stablecoins/usdc-contract-addresses
    token(10, "USDC", "USDC", "0x0b2C639c533813f4Aa9D7837CAf62653d097Ff85", 6),
    // https://docs.usdt0.to/technical-documentation/deployments (USDT0)
    token(10, "USDT", "USD₮0", "0x01bFF41798a0BcF287b996046Ca68b395DbC1071", 6),
    // https://github.com/makerdao/optimism-dai-bridge#deployments (same address as Arbitrum's)
    token(10, "DAI", "DAI", "0xDA10009cBd5D07dd0CeCc66161FC93D7c9000da1", 18),
    // https://github.com/ethereum-optimism/specs/blob/main/specs/protocol/predeploys.md
    token(10, "WETH", "WETH", "0x4200000000000000000000000000000000000006", 18),
    // polygon (137)
    // https://developers.circle.com/stablecoins/usdc-contract-addresses
    token(137, "USDC", "USDC", "0x3c499c542cEF5E3811e1192ce70d8cC03d5c3359", 6),
    // https://docs.usdt0.to/technical-documentation/deployments (USDT0)
    token(137, "USDT", "USDT0", "0xc2132D05D31c914a87C6611C10748AEb04B58e8F", 6),
    // avalanche (43114)
    // https://developers.circle.com/stablecoins/usdc-contract-addresses
    token(43114, "USDC", "USDC", "0xB97EF9Ef8734C71904D8002F8b6Bc66Dd9c48a6E", 6),
    // https://tether.to/en/supported-protocols
    token(43114, "USDT", "USDt", "0x9702230A8Ea53601f5cD2dc00fDBc13d4dF4A8c7", 6),
    // sepolia (11155111): testnet, the live token-transfer target
    // https://developers.circle.com/stablecoins/usdc-contract-addresses
    token(11155111, "USDC", "USDC", "0x1c7D4B196Cb0C7B01d743Fbc6116a902379C7238", 6),
];

/// Whether `token.address` is `address` (compared as bytes, whatever the case).
fn same_address(token: &Token, address: &Address) -> bool {
    token
        .address
        .strip_prefix("0x")
        .is_some_and(|h| h.eq_ignore_ascii_case(&hex::encode(address)))
}

/// The registry token at `address` on chain `chain_id`, if there is one.
pub fn lookup(chain_id: u64, address: &Address) -> Option<&'static Token> {
    TOKENS
        .iter()
        .find(|t| t.chain_id == chain_id && same_address(t, address))
}

/// Whether `address` is a registry token on any chain. A whitelist contract entry may not name
/// one (its selectors would bypass the revoke-only approval rule), whatever chains it lists.
pub fn is_token_anywhere(address: &Address) -> bool {
    TOKENS.iter().any(|t| same_address(t, address))
}

/// The whitelist's default fee cap per chain key, in wei (`.prd/04-whitelist.md` §Policy): the
/// most `gasLimit × max price` may cost. Set at the first `whitelist add`.
pub const DEFAULT_FEE_CAPS_WEI: &[(&str, &str)] = &[
    // 0.003 ETH
    ("ethereum", "3000000000000000"),
    // 0.0005 ETH
    ("base", "500000000000000"),
    ("arbitrum", "500000000000000"),
    ("optimism", "500000000000000"),
    // 2 POL
    ("polygon", "2000000000000000000"),
    // 0.003 BNB
    ("bsc", "3000000000000000"),
    // 0.05 AVAX
    ("avalanche", "50000000000000000"),
    // 0.01 ETH
    ("sepolia", "10000000000000000"),
];

/// The chain facts the whitelist's validation needs: the supported chain keys, the registry
/// tokens, and the default fee caps.
pub const WHITELIST_RULES: ChainRules = ChainRules {
    chain_known,
    is_registry_token: is_token_anywhere_str,
    registry_token_on_chain: is_token_on_chain,
    default_fee_caps_wei: DEFAULT_FEE_CAPS_WEI,
};

fn chain_known(key: &str) -> bool {
    crate::chains::CHAINS.iter().any(|c| c.key == key)
}

fn is_token_on_chain(key: &str, address: &str) -> bool {
    let Some(chain) = crate::chains::CHAINS.iter().find(|c| c.key == key) else {
        return false;
    };
    let Ok(config) = super::config(chain) else {
        return false;
    };
    address
        .parse::<Address>()
        .is_ok_and(|address| lookup(config.chain_id, &address).is_some())
}

fn is_token_anywhere_str(address: &str) -> bool {
    address
        .parse::<Address>()
        .is_ok_and(|a| is_token_anywhere(&a))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chains::{CHAINS, evm};
    use crate::core::amount::parse_units;
    use alloy::primitives::U256;
    use std::collections::BTreeSet;

    fn addr(s: &str) -> Address {
        s.parse().unwrap()
    }

    #[test]
    fn rows_are_the_nineteen_fixed_ones_checksummed_and_unique() {
        assert_eq!(TOKENS.len(), 19);
        let mut keys = BTreeSet::new();
        for t in TOKENS {
            let a = Address::parse_checksummed(t.address, None)
                .unwrap_or_else(|_| panic!("{} on {} is not EIP-55", t.address, t.chain_id));
            assert!(keys.insert((t.chain_id, a)), "duplicate {t:?}");
            assert!(["USDC", "USDT", "DAI", "WETH"].contains(&t.symbol), "{t:?}");
            let expected = match t.symbol {
                "USDC" | "USDT" => 6,
                _ => 18,
            };
            assert_eq!(t.decimals, expected, "{t:?}");
            // Every row's chain is one zwallet supports.
            assert!(
                CHAINS
                    .iter()
                    .any(|c| evm::config(c).is_ok_and(|e| e.chain_id == t.chain_id)),
                "{t:?}"
            );
        }
        // Per chain, as fixed in registry-final.md.
        let count = |id| TOKENS.iter().filter(|t| t.chain_id == id).count();
        let per_chain = [
            (1, 4),
            (8453, 2),
            (42161, 4),
            (10, 4),
            (137, 2),
            (56, 0),
            (43114, 2),
            (11155111, 1),
        ];
        for (id, n) in per_chain {
            assert_eq!(count(id), n, "chain {id}");
        }
    }

    #[test]
    fn rows_match_the_fixed_registry_row_for_row() {
        // (chainId, symbol, symbol() on chain, address, decimals), copied from the registry
        // decision (registry-final.md, 2026-10-20) and the on-chain read of 2026-10-20, kept
        // apart from TOKENS so a drifted row fails here.
        let expected: [(u64, &str, &str, &str, u8); 19] = [
            (
                1,
                "USDC",
                "USDC",
                "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
                6,
            ),
            (
                1,
                "USDT",
                "USDT",
                "0xdAC17F958D2ee523a2206206994597C13D831ec7",
                6,
            ),
            (
                1,
                "DAI",
                "DAI",
                "0x6B175474E89094C44Da98b954EedeAC495271d0F",
                18,
            ),
            (
                1,
                "WETH",
                "WETH",
                "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2",
                18,
            ),
            (
                8453,
                "USDC",
                "USDC",
                "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
                6,
            ),
            (
                8453,
                "WETH",
                "WETH",
                "0x4200000000000000000000000000000000000006",
                18,
            ),
            (
                42161,
                "USDC",
                "USDC",
                "0xaf88d065e77c8cC2239327C5EDb3A432268e5831",
                6,
            ),
            (
                42161,
                "USDT",
                "USD₮0",
                "0xFd086bC7CD5C481DCC9C85ebE478A1C0b69FCbb9",
                6,
            ),
            (
                42161,
                "DAI",
                "DAI",
                "0xDA10009cBd5D07dd0CeCc66161FC93D7c9000da1",
                18,
            ),
            (
                42161,
                "WETH",
                "WETH",
                "0x82aF49447D8a07e3bd95BD0d56f35241523fBab1",
                18,
            ),
            (
                10,
                "USDC",
                "USDC",
                "0x0b2C639c533813f4Aa9D7837CAf62653d097Ff85",
                6,
            ),
            (
                10,
                "USDT",
                "USD₮0",
                "0x01bFF41798a0BcF287b996046Ca68b395DbC1071",
                6,
            ),
            (
                10,
                "DAI",
                "DAI",
                "0xDA10009cBd5D07dd0CeCc66161FC93D7c9000da1",
                18,
            ),
            (
                10,
                "WETH",
                "WETH",
                "0x4200000000000000000000000000000000000006",
                18,
            ),
            (
                137,
                "USDC",
                "USDC",
                "0x3c499c542cEF5E3811e1192ce70d8cC03d5c3359",
                6,
            ),
            (
                137,
                "USDT",
                "USDT0",
                "0xc2132D05D31c914a87C6611C10748AEb04B58e8F",
                6,
            ),
            (
                43114,
                "USDC",
                "USDC",
                "0xB97EF9Ef8734C71904D8002F8b6Bc66Dd9c48a6E",
                6,
            ),
            (
                43114,
                "USDT",
                "USDt",
                "0x9702230A8Ea53601f5cD2dc00fDBc13d4dF4A8c7",
                6,
            ),
            (
                11155111,
                "USDC",
                "USDC",
                "0x1c7D4B196Cb0C7B01d743Fbc6116a902379C7238",
                6,
            ),
        ];
        let actual: Vec<_> = TOKENS
            .iter()
            .map(|t| {
                (
                    t.chain_id,
                    t.symbol,
                    t.onchain_symbol,
                    t.address,
                    t.decimals,
                )
            })
            .collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn lookup_is_by_chain_and_address_in_any_case() {
        let usdc = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48";
        let t = lookup(1, &addr(&usdc.to_ascii_lowercase())).unwrap();
        assert_eq!((t.symbol, t.decimals), ("USDC", 6));
        // The same address on another chain is not that token.
        assert!(lookup(8453, &addr(usdc)).is_none());
        // One address on two chains (Arbitrum and OP DAI; Base and OP WETH).
        let dai = addr("0xDA10009cBd5D07dd0CeCc66161FC93D7c9000da1");
        assert_eq!(lookup(42161, &dai).unwrap().symbol, "DAI");
        assert_eq!(lookup(10, &dai).unwrap().symbol, "DAI");
        assert!(lookup(1, &dai).is_none());
        let weth = addr("0x4200000000000000000000000000000000000006");
        assert_eq!(lookup(8453, &weth).unwrap().symbol, "WETH");
        assert_eq!(lookup(10, &weth).unwrap().symbol, "WETH");
        assert!(lookup(42161, &weth).is_none());
        // Deliberately absent: bridged USDC.e on Arbitrum, Binance-Peg USDT on BSC.
        assert!(lookup(42161, &addr("0xFF970A61A04b1cA14834A43f5dE4533eBDDB5CC8")).is_none());
        assert!(lookup(56, &addr("0x55d398326f99059fF775485246999027B3197955")).is_none());
        // The USDT0 rows keep the registry symbol and record the on-chain one.
        let usdt0 = lookup(42161, &addr("0xFd086bC7CD5C481DCC9C85ebE478A1C0b69FCbb9")).unwrap();
        assert_eq!((usdt0.symbol, usdt0.onchain_symbol), ("USDT", "USD₮0"));
    }

    #[test]
    fn any_chain_check_sees_every_row_and_nothing_else() {
        for t in TOKENS {
            assert!(is_token_anywhere(&addr(t.address)), "{t:?}");
        }
        assert!(!is_token_anywhere(&Address::ZERO));
        assert!(!is_token_anywhere(&addr(
            "0x55d398326f99059fF775485246999027B3197955"
        )));
    }

    #[test]
    fn whitelist_rules_are_the_real_chains_and_registry() {
        use crate::core::whitelist::{Entry, Whitelist};
        let rules = WHITELIST_RULES;
        for c in CHAINS {
            assert!((rules.chain_known)(c.key), "{}", c.key);
        }
        for bad in ["Base", " base", "*", "solana", ""] {
            assert!(!(rules.chain_known)(bad), "{bad:?}");
        }
        assert!((rules.is_registry_token)(
            "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"
        ));
        assert!(!(rules.is_registry_token)(
            "0x1111111111111111111111111111111111111111"
        ));
        assert!(!(rules.is_registry_token)("not an address"));
        assert_eq!(rules.default_fee_caps_wei, DEFAULT_FEE_CAPS_WEI);

        let mut list = Whitelist::empty(&rules);
        list.generation = 1;
        list.entries.push(Entry::new_address(
            "alice",
            "0x1111111111111111111111111111111111111111",
            &["base", "sepolia"],
        ));
        list.validate(&rules).unwrap();
        // Arbitrum's DAI as a contract entry, even scoped to a chain where it is no token.
        list.entries.push(Entry::new_contract(
            "dai",
            "0xDA10009cBd5D07dd0CeCc66161FC93D7c9000da1",
            &["ethereum"],
            &["0x12345678"],
        ));
        let err = list.validate(&rules).unwrap_err();
        assert!(err.message.contains("registry token"), "{}", err.message);
    }

    /// One built-in cap per EVM chain. Tron has none until its signer comes
    /// (`.prd/v0-3-tron/units.md` TRON-1; B2 gives it one with the signer).
    #[test]
    fn default_fee_caps_match_the_spec_for_every_evm_chain() {
        let spec = [
            ("ethereum", "0.003"),
            ("base", "0.0005"),
            ("arbitrum", "0.0005"),
            ("optimism", "0.0005"),
            ("polygon", "2"),
            ("bsc", "0.003"),
            ("avalanche", "0.05"),
            ("sepolia", "0.01"),
        ];
        let evm: Vec<_> = CHAINS.iter().filter(|c| evm::config(c).is_ok()).collect();
        assert_eq!(DEFAULT_FEE_CAPS_WEI.len(), evm.len());
        assert!(!DEFAULT_FEE_CAPS_WEI.iter().any(|(k, _)| *k == "tron"));
        for c in evm {
            let (_, human) = spec.iter().find(|(k, _)| *k == c.key).unwrap();
            let (_, wei) = DEFAULT_FEE_CAPS_WEI
                .iter()
                .find(|(k, _)| *k == c.key)
                .unwrap_or_else(|| panic!("no default fee cap for {}", c.key));
            assert_eq!(
                U256::from_str_radix(wei, 10).unwrap(),
                parse_units(human, c.decimals).unwrap(),
                "{}",
                c.key
            );
        }
    }
}
