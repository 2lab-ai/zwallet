//! Chain registry and the chain-family boundary.
//!
//! A family (EVM today; Solana and TON later) owns key derivation from the shared seed, signing,
//! and its chain I/O. The split is deliberate: [`FamilyKeys`] is everything the unlock daemon may
//! do (no network), [`ChainFamily`] adds what the one-shot CLI does over RPC.

pub mod evm;

use crate::core::error::{ErrorCode, Result, WalletError};
use crate::core::seed::Seed;
use alloy::primitives::U256;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::time::Duration;

/// Family plus its family-specific configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Evm(evm::EvmChain),
}

impl Family {
    pub fn name(&self) -> &'static str {
        match self {
            Family::Evm(_) => "evm",
        }
    }

    /// Key-side operations for this family (the only part the daemon can reach).
    pub fn keys(&self) -> &'static dyn FamilyKeys {
        match self {
            Family::Evm(_) => &evm::Evm,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chain {
    pub key: &'static str,
    pub name: &'static str,
    pub symbol: &'static str,
    pub decimals: u8,
    pub family: Family,
}

const fn evm_chain(
    key: &'static str,
    name: &'static str,
    symbol: &'static str,
    chain_id: u64,
    default_rpc: &'static str,
    explorer: &'static str,
) -> Chain {
    Chain {
        key,
        name,
        symbol,
        decimals: 18,
        family: Family::Evm(evm::EvmChain {
            chain_id,
            default_rpc,
            explorer,
            op_stack: matches!(chain_id, 8453 | 10),
        }),
    }
}

pub const CHAINS: &[Chain] = &[
    evm_chain(
        "ethereum",
        "Ethereum",
        "ETH",
        1,
        "https://ethereum-rpc.publicnode.com",
        "https://etherscan.io",
    ),
    evm_chain(
        "base",
        "Base",
        "ETH",
        8453,
        "https://base-rpc.publicnode.com",
        "https://basescan.org",
    ),
    evm_chain(
        "arbitrum",
        "Arbitrum One",
        "ETH",
        42161,
        "https://arbitrum-one-rpc.publicnode.com",
        "https://arbiscan.io",
    ),
    evm_chain(
        "optimism",
        "OP Mainnet",
        "ETH",
        10,
        "https://optimism-rpc.publicnode.com",
        "https://optimistic.etherscan.io",
    ),
    evm_chain(
        "polygon",
        "Polygon PoS",
        "POL",
        137,
        "https://polygon-bor-rpc.publicnode.com",
        "https://polygonscan.com",
    ),
    evm_chain(
        "bsc",
        "BNB Smart Chain",
        "BNB",
        56,
        "https://bsc-rpc.publicnode.com",
        "https://bscscan.com",
    ),
    evm_chain(
        "avalanche",
        "Avalanche C-Chain",
        "AVAX",
        43114,
        "https://avalanche-c-chain-rpc.publicnode.com",
        "https://snowtrace.io",
    ),
    evm_chain(
        "sepolia",
        "Sepolia",
        "ETH",
        11155111,
        "https://ethereum-sepolia-rpc.publicnode.com",
        "https://sepolia.etherscan.io",
    ),
];

/// The chain whose cached address is reported as the wallet's primary `address`.
pub const PRIMARY_CHAIN: &str = "ethereum";

pub fn find(key: &str) -> Result<&'static Chain> {
    let wanted = key.trim().to_ascii_lowercase();
    CHAINS.iter().find(|c| c.key == wanted).ok_or_else(|| {
        let keys: Vec<&str> = CHAINS.iter().map(|c| c.key).collect();
        WalletError::new(
            ErrorCode::ChainUnsupported,
            format!("unsupported chain '{key}'; supported: {}", keys.join(", ")),
        )
    })
}

impl Chain {
    /// Environment variable that overrides this chain's RPC URL.
    pub fn rpc_env_var(&self) -> String {
        format!("ZWALLET_RPC_{}", self.key.to_ascii_uppercase())
    }

    pub fn default_rpc(&self) -> &'static str {
        match &self.family {
            Family::Evm(c) => c.default_rpc,
        }
    }

    /// RPC precedence: `--rpc` flag > `ZWALLET_RPC_<KEY>` env > built-in default.
    pub fn resolve_rpc(&self, flag: Option<&str>, env_value: Option<&str>) -> String {
        fn pick(v: Option<&str>) -> Option<&str> {
            v.map(str::trim).filter(|s| !s.is_empty())
        }
        pick(flag)
            .or_else(|| pick(env_value))
            .unwrap_or(self.default_rpc())
            .to_string()
    }

    /// Family-specific chain id for display (`1` for Ethereum).
    pub fn chain_id_json(&self) -> Value {
        match &self.family {
            Family::Evm(c) => Value::from(c.chain_id),
        }
    }

    pub fn explorer_tx_url(&self, tx_hash: &str) -> String {
        match &self.family {
            Family::Evm(c) => format!("{}/tx/{tx_hash}", c.explorer),
        }
    }
}

/// Address cache for the key at `index`: chain key → address. Derives once per family.
pub fn derive_addresses(seed: &Seed, index: u32) -> Result<BTreeMap<String, String>> {
    let mut per_family: BTreeMap<&'static str, String> = BTreeMap::new();
    let mut out = BTreeMap::new();
    for chain in CHAINS {
        let family = chain.family.name();
        let address = match per_family.get(family) {
            Some(a) => a.clone(),
            None => {
                let a = chain.family.keys().derive_address(seed, index)?;
                per_family.insert(family, a.clone());
                a
            }
        };
        out.insert(chain.key.to_string(), address);
    }
    Ok(out)
}

/// Key-side operations. Runs inside the unlock daemon; must never touch the network.
///
/// `index` is the wallet's key index (creation order); each family maps it to its own path, e.g.
/// EVM `m/44'/60'/0'/0/{index}` (later: Solana `m/44'/501'/{index}'/0'`, TON `m/44'/607'/{index}'`).
pub trait FamilyKeys: Sync {
    fn derivation_path(&self, index: u32) -> String;
    fn derive_address(&self, seed: &Seed, index: u32) -> Result<String>;
    /// Returns `{ address, signature, digest }` (family-defined encodings).
    fn sign_message(&self, seed: &Seed, index: u32, chain: &Chain, message: &[u8])
    -> Result<Value>;
    /// Signs a family-specific unsigned transaction; returns `{ rawTx, txHash }`.
    fn sign_tx(&self, seed: &Seed, index: u32, chain: &Chain, unsigned: &Value) -> Result<Value>;
}

/// An RPC endpoint for one chain.
#[derive(Debug, Clone)]
pub struct Net {
    pub chain: &'static Chain,
    pub rpc_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Balance {
    pub symbol: String,
    pub decimals: u8,
    pub raw: U256,
}

#[derive(Debug, Clone)]
pub struct TransferRequest {
    pub from: String,
    pub to: String,
    pub amount: String,
    pub token: Option<String>,
    pub dry_run: bool,
}

/// An unsigned transaction plus the output fields describing it (nonce, fees, shortfall, ...).
#[derive(Debug, Clone)]
pub struct PreparedTx {
    pub unsigned: Value,
    pub fields: Map<String, Value>,
}

/// A signed transaction as decoded from its raw bytes (not as claimed by the signer).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedTx {
    pub raw: String,
    pub tx_hash: String,
    pub signer: String,
}

/// Network-side operations, run by the one-shot CLI process (no key access).
#[allow(async_fn_in_trait)]
pub trait ChainFamily: FamilyKeys {
    /// Validates and normalizes a user-supplied address (`INVALID_ADDRESS` on failure).
    fn normalize_address(&self, input: &str) -> Result<String>;
    /// Checks a `sign_message` result really is `expected`'s signature over `message`.
    fn verify_message(&self, message: &[u8], signed: &Value, expected: &str) -> Result<()>;
    /// Decodes a `sign_tx` result from its raw bytes and recovers the signer.
    fn inspect_signed(&self, chain: &Chain, signed: &Value) -> Result<SignedTx>;
    /// Arbitrary contract reads and writes (`call` / `invoke`).
    fn supports_contract_calls(&self) -> bool {
        false
    }
    async fn native_balance(&self, net: &Net, owner: &str) -> Result<Balance>;
    async fn token_balance(&self, net: &Net, owner: &str, token: &str) -> Result<Balance>;
    /// Builds an unsigned transfer from live chain state. Outside dry-run this fails with
    /// `INSUFFICIENT_FUNDS` instead of returning a transaction that cannot pay for itself.
    async fn prepare_transfer(&self, net: &Net, req: &TransferRequest) -> Result<PreparedTx>;
    async fn broadcast(&self, net: &Net, signed: &SignedTx) -> Result<String>;
    /// `Ok(None)` when no receipt appeared within `timeout`.
    async fn wait_for_receipt(
        &self,
        net: &Net,
        tx_hash: &str,
        timeout: Duration,
    ) -> Result<Option<Value>>;
}

/// `CAPABILITY_UNSUPPORTED` unless the chain's family supports contract calls.
pub fn require_contract_calls(chain: &Chain) -> Result<()> {
    let supported = match &chain.family {
        Family::Evm(_) => evm::Evm.supports_contract_calls(),
    };
    if supported {
        Ok(())
    } else {
        Err(WalletError::new(
            ErrorCode::CapabilityUnsupported,
            format!(
                "{} ({} family) does not support contract calls",
                chain.key,
                chain.family.name()
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::seed::{Seed, TEST_MNEMONIC};
    use zeroize::Zeroizing;

    #[test]
    fn find_is_case_insensitive_and_rejects_unknown() {
        assert_eq!(find("Base").unwrap().chain_id_json(), Value::from(8453));
        assert_eq!(
            find("solana").unwrap_err().code,
            ErrorCode::ChainUnsupported
        );
    }

    #[test]
    fn rpc_precedence_flag_env_default() {
        let c = find("ethereum").unwrap();
        assert_eq!(c.rpc_env_var(), "ZWALLET_RPC_ETHEREUM");
        assert_eq!(
            c.resolve_rpc(Some("http://flag"), Some("http://env")),
            "http://flag"
        );
        assert_eq!(c.resolve_rpc(None, Some("http://env")), "http://env");
        assert_eq!(c.resolve_rpc(Some(" "), None), c.default_rpc());
    }

    #[test]
    fn keys_and_chain_ids_are_unique() {
        let mut keys: Vec<&str> = CHAINS.iter().map(|c| c.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), CHAINS.len());
        let mut ids: Vec<String> = CHAINS
            .iter()
            .map(|c| c.chain_id_json().to_string())
            .collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), CHAINS.len());
    }

    #[test]
    fn every_evm_chain_shares_one_address() {
        let seed = Seed::from_phrase(Zeroizing::new(TEST_MNEMONIC.into())).unwrap();
        let addrs = derive_addresses(&seed, 0).unwrap();
        assert_eq!(addrs.len(), CHAINS.len());
        for a in addrs.values() {
            assert_eq!(a, "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
        }
        let second = derive_addresses(&seed, 1).unwrap();
        assert!(
            second
                .values()
                .all(|a| a == "0x70997970C51812dc3A010C7d01b50e0d17dc79C8")
        );
        assert!(require_contract_calls(find("ethereum").unwrap()).is_ok());
    }
}
