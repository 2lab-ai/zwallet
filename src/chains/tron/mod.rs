//! The Tron family: secp256k1 keys at `m/44'/195'/0'/0/{index}`, base58check `T…` addresses,
//! TRX and TRC20 balances over Tron's HTTP API (`/wallet/*`). This version reads; it signs
//! nothing (`.prd/05-tron.md`).

pub mod address;

use crate::chains::evm::policy::ListView;
use crate::chains::{Chain, FamilyKeys};
use crate::core::error::{ErrorCode, Result, WalletError};
use crate::core::seed::Seed;
use alloy::signers::local::PrivateKeySigner;
use serde_json::Value;

/// Tron-specific registry configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TronChain {
    pub default_rpc: &'static str,
    pub explorer: &'static str,
    /// The network's block 0 id (`/wallet/getblockbynum {"num":0}` → `blockID`), 64 lower-case
    /// hex digits: what the chain identity check compares.
    pub genesis: &'static str,
}

impl TronChain {
    /// The chain id Tron's own JSON-RPC answers to `eth_chainId`: the last 4 bytes of the
    /// genesis block id (`TronJsonRpcImpl.java:426-427`). zwallet uses it for `chains` and file
    /// names only; a Tron transaction carries none.
    pub fn chain_id(&self) -> u64 {
        0
    }
}

/// Tron mainnet's genesis block id (05-tron.md §Network access, M6).
pub const MAINNET_GENESIS: &str =
    "00000000000000001ebf88508a03865c71d452e25f4d51194196a1d22b6653dc";

/// The Tron family implementation.
pub struct Tron;

/// `m/44'/195'/0'/0/{index}`: SLIP-44 coin type 195, TronWeb's default for index 0.
pub fn derivation_path(index: u32) -> String {
    let _ = index;
    String::new()
}

/// The secp256k1 key of wallet key `index` on Tron.
pub fn signer_at(seed: &Seed, index: u32) -> Result<PrivateKeySigner> {
    let _ = (seed, index);
    Err(WalletError::validation("not yet"))
}

impl FamilyKeys for Tron {
    fn derivation_path(&self, index: u32) -> String {
        derivation_path(index)
    }

    fn derive_address(&self, seed: &Seed, index: u32) -> Result<String> {
        let signer = signer_at(seed, index)?;
        Ok(address::from_body(&signer.address().0.0))
    }

    fn sign_message(
        &self,
        _seed: &Seed,
        _index: u32,
        _chain: &Chain,
        _message: &[u8],
        _list: &ListView<'_>,
    ) -> Result<Value> {
        Err(WalletError::validation("not yet"))
    }

    fn sign_tx(
        &self,
        _seed: &Seed,
        _index: u32,
        _chain: &Chain,
        _unsigned: &Value,
        _list: &ListView<'_>,
    ) -> Result<Value> {
        Err(WalletError::new(ErrorCode::ValidationError, "not yet"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chains::evm::{self, registry::WHITELIST_RULES};
    use crate::core::seed::TEST_MNEMONIC;
    use crate::core::whitelist::Whitelist;
    use zeroize::Zeroizing;

    fn seed() -> Seed {
        Seed::from_phrase(Zeroizing::new(TEST_MNEMONIC.into())).unwrap()
    }

    #[test]
    fn tr1_the_a1_mnemonic_gives_tronwebs_key_0_and_the_next_index() {
        // 05-tron.md §Derivation, computed by TronWeb 6.5.0 and by @scure apart (E4).
        assert_eq!(derivation_path(0), "m/44'/195'/0'/0/0");
        assert_eq!(derivation_path(1), "m/44'/195'/0'/0/1");
        assert_eq!(Tron.derivation_path(1), "m/44'/195'/0'/0/1");
        assert_eq!(
            Tron.derive_address(&seed(), 0).unwrap(),
            "TWer2Ygk5TEheHp3TPuYeqxmB6SsGZmaL6"
        );
        assert_eq!(
            Tron.derive_address(&seed(), 1).unwrap(),
            "TPjjvMwjPoDC32V2dGDYTkLH4E5LAtBZ6C"
        );
        // Not the EVM key: another path, another body.
        let evm0 = evm::Evm.derive_address(&seed(), 0).unwrap();
        assert_eq!(evm0, "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
        let tron0 = address::parse(&Tron.derive_address(&seed(), 0).unwrap()).unwrap();
        assert_ne!(
            alloy::hex::encode(&tron0[1..]),
            evm0[2..].to_ascii_lowercase()
        );
    }

    #[test]
    fn tr1_index_2_pow_31_is_refused() {
        assert!(Tron.derive_address(&seed(), (1 << 31) - 1).is_ok());
        for index in [1 << 31, u32::MAX] {
            let e = Tron.derive_address(&seed(), index).unwrap_err();
            assert_eq!(e.code, ErrorCode::ValidationError);
            assert!(e.message.contains("2^31"), "{}", e.message);
        }
    }

    #[test]
    fn the_chain_id_is_the_last_4_bytes_of_the_genesis_block_id() {
        let chain = TronChain {
            default_rpc: "",
            explorer: "",
            genesis: MAINNET_GENESIS,
        };
        assert_eq!(chain.chain_id(), 728126428);
        assert_eq!(chain.chain_id(), 0x2b6653dc);
    }

    #[test]
    fn tr24_the_daemon_never_signs_a_message_or_a_transaction_for_tron() {
        let mut list = Whitelist::empty(&WHITELIST_RULES);
        list.generation = 1;
        list.policy.allow_message_signing = true;
        let view = ListView::Usable(&list);
        let chain = crate::chains::find("ethereum").unwrap();
        let e = Tron
            .sign_message(&seed(), 0, chain, b"hello", &view)
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::CapabilityUnsupported, "{}", e.message);
        let e = Tron
            .sign_tx(&seed(), 0, chain, &serde_json::json!("0x0a02"), &view)
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::CapabilityUnsupported, "{}", e.message);
    }
}
