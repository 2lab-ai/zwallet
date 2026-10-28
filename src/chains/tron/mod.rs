//! The Tron family: secp256k1 keys at `m/44'/195'/0'/0/{index}`, base58check `T…` addresses,
//! TRX and TRC20 balances over Tron's HTTP API (`/wallet/*`). This version reads; it signs
//! nothing (`.prd/05-tron.md`).

pub mod address;
pub mod registry;
pub mod rpc;

use crate::chains::evm::policy::ListView;
use crate::chains::{Balance, Chain, Family, FamilyKeys, Net, TokenBalance};
use crate::core::error::{ErrorCode, Result, WalletError};
use crate::core::seed::Seed;
use crate::core::store::KEY_INDEX_LIMIT;
use alloy::primitives::U256;
use alloy::signers::local::coins_bip39::English;
use alloy::signers::local::{MnemonicBuilder, PrivateKeySigner};
use serde_json::Value;
use std::time::Duration;

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
        let tail = &self.genesis[self.genesis.len().saturating_sub(8)..];
        u64::from_str_radix(tail, 16).unwrap_or(0)
    }
}

/// Tron mainnet's genesis block id (05-tron.md §Network access, M6).
pub const MAINNET_GENESIS: &str =
    "00000000000000001ebf88508a03865c71d452e25f4d51194196a1d22b6653dc";

/// The Tron family implementation.
pub struct Tron;

/// `m/44'/195'/0'/0/{index}`: SLIP-44 coin type 195, TronWeb's default for index 0.
pub fn derivation_path(index: u32) -> String {
    format!("m/44'/195'/0'/0/{index}")
}

/// The secp256k1 key of wallet key `index` on Tron.
pub fn signer_at(seed: &Seed, index: u32) -> Result<PrivateKeySigner> {
    // The last step of the path is non-hardened; from 2^31 up it would be a hardened child.
    if index >= KEY_INDEX_LIMIT {
        return Err(WalletError::validation(format!(
            "cannot derive Tron key at index {index}: key indices stay below 2^31, where BIP32's \
             hardened range starts"
        )));
    }
    MnemonicBuilder::<English>::default()
        .phrase(seed.phrase())
        .derivation_path(derivation_path(index))
        .and_then(|b| b.build())
        .map_err(|_| WalletError::validation(format!("cannot derive Tron key at index {index}")))
}

/// `CAPABILITY_UNSUPPORTED` for what no Tron key does in this version.
fn unsupported(what: &str) -> WalletError {
    WalletError::new(
        ErrorCode::CapabilityUnsupported,
        format!("tron (tron family) does not support {what}"),
    )
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
        Err(unsupported("message signing"))
    }

    fn sign_tx(
        &self,
        _seed: &Seed,
        _index: u32,
        _chain: &Chain,
        _unsigned: &Value,
        _list: &ListView<'_>,
    ) -> Result<Value> {
        // The Tron signer, with its decoder and policy, comes with a later version
        // (`.prd/v0-3-tron/units.md` TRON-4, TRON-5); until then the daemon signs nothing here.
        Err(unsupported("signing transactions in this version"))
    }
}

/// `chain`'s Tron configuration; `VALIDATION_ERROR` for a chain of another family.
pub fn config(chain: &Chain) -> Result<&TronChain> {
    match &chain.family {
        Family::Tron(c) => Ok(c),
        other => Err(WalletError::validation(format!(
            "{} is not a Tron chain ({} family)",
            chain.key,
            other.name()
        ))),
    }
}

/// A token row of `balance --chain tron --token`: its contract, and whether its symbol is the
/// registry's or the contract's own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenShown {
    pub address: String,
    pub from_registry: bool,
}

/// What `balance --chain tron [--token …]` shows besides the address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OneBalance {
    pub balance: Balance,
    pub activated: bool,
    pub warning: Option<String>,
    pub token: Option<TokenShown>,
}

/// TRX in sun as a [`Balance`] of `chain`'s native coin.
fn trx(chain: &Chain, sun: u64) -> Balance {
    Balance {
        symbol: chain.symbol.to_string(),
        decimals: chain.decimals,
        raw: U256::from(sun),
    }
}

/// What `--token` names on Tron: a registry row by its symbol, or a contract by its `T…` address.
enum TokenArg {
    Registry(&'static registry::Token),
    Contract(String),
}

/// `--token` on Tron, checked before any request: `usdt` (in any case) or a `T…` address.
fn token_arg(input: &str) -> Result<TokenArg> {
    if let Some(t) = registry::by_symbol(input) {
        return Ok(TokenArg::Registry(t));
    }
    let payload = address::parse(input.trim()).map_err(|e| {
        WalletError::invalid_address(format!(
            "--token on tron is `usdt` or a TRC20 contract's T... address: {}",
            e.message
        ))
    })?;
    Ok(TokenArg::Contract(address::encode(&payload)))
}

/// `balance --chain tron`: the account of `owner` (a `T…` address) at `net`, after the chain
/// identity check: TRX, or with `token` that token. `usdt` is the registry row (its symbol and
/// decimals, one `balanceOf`); a `T…` contract answers its own `decimals()` and `symbol()`, as
/// EVM `--token` does. The account is read either way, for `activated` and the owner-permission
/// warning.
pub async fn balance(net: &Net, owner: &str, token: Option<&str>) -> Result<OneBalance> {
    address::parse(owner)?;
    let token = token.map(token_arg).transpose()?;
    let c = rpc::connect(net).await?;
    let account = rpc::account(&c, owner).await?;
    let (balance, token) = match token {
        None => (trx(net.chain, account.balance), None),
        Some(TokenArg::Registry(t)) => (
            Balance {
                symbol: t.symbol.to_string(),
                decimals: t.decimals,
                raw: rpc::trc20_balance(&c, t.address, owner).await?,
            },
            Some(TokenShown {
                address: t.address.to_string(),
                from_registry: true,
            }),
        ),
        Some(TokenArg::Contract(contract)) => {
            let meta = rpc::trc20_meta(&c, &contract, owner).await?;
            let raw = rpc::trc20_balance(&c, &contract, owner).await?;
            (
                Balance {
                    symbol: meta.symbol,
                    decimals: meta.decimals,
                    raw,
                },
                Some(TokenShown {
                    address: contract,
                    from_registry: meta.from_registry,
                }),
            )
        }
    };
    Ok(OneBalance {
        balance,
        activated: account.activated,
        warning: account.warning,
        token,
    })
}

/// `balance` without `--chain` on Tron: TRX and every Tron registry token (symbol and decimals
/// the registry's), all within `limit`. A token read that fails or is late is that token's
/// error; the call fails only when the endpoint, its genesis check or the account does.
pub async fn wallet_balances(net: &Net, owner: &str, limit: Duration) -> Result<rpc::Read> {
    let deadline = tokio::time::Instant::now() + limit;
    let late =
        move |what: &str| WalletError::rpc(format!("{what} timed out after {}s", limit.as_secs()));
    address::parse(owner)?;
    let c = tokio::time::timeout_at(deadline, rpc::connect(net))
        .await
        .map_err(|_| late("the genesis block check"))??;
    let c = std::sync::Arc::new(c);
    // Every token at once, each until the deadline, so one that hangs delays no other.
    let reads: Vec<_> = registry::TOKENS
        .iter()
        .map(|t| {
            let (c, owner) = (c.clone(), owner.to_string());
            let read = tokio::spawn(async move {
                tokio::time::timeout_at(deadline, rpc::trc20_balance(&c, t.address, &owner))
                    .await
                    .map_err(|_| late("balanceOf"))?
            });
            (t, read)
        })
        .collect();
    let account = tokio::time::timeout_at(deadline, rpc::account(&c, owner))
        .await
        .map_err(|_| late("/wallet/getaccount"))??;
    let mut tokens = Vec::with_capacity(reads.len());
    for (t, read) in reads {
        tokens.push(TokenBalance {
            address: t.address.to_string(),
            symbol: t.symbol.to_string(),
            decimals: t.decimals,
            raw: read
                .await
                .unwrap_or_else(|_| Err(WalletError::rpc("balanceOf stopped unexpectedly"))),
        });
    }
    Ok(rpc::Read {
        native: trx(net.chain, account.balance),
        tokens,
        activated: account.activated,
        warning: account.warning,
    })
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
