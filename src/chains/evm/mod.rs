//! The EVM family: secp256k1 keys at `m/44'/60'/0'/0/{index}`, EIP-191 messages, EIP-1559 /
//! EIP-155 transactions, ERC-20, and arbitrary contract calls.

pub mod abi;
pub mod rpc;
pub mod tx;

use crate::chains::{
    Balance, Chain, ChainFamily, Family, FamilyKeys, Net, PreparedTx, SignedTx, TransferRequest,
};
use crate::core::error::{Result, WalletError};
use crate::core::seed::Seed;
use alloy::hex;
use alloy::primitives::{Address, B256, Signature};
use alloy::signers::SignerSync;
use alloy::signers::local::coins_bip39::English;
use alloy::signers::local::{MnemonicBuilder, PrivateKeySigner};
use serde_json::{Value, json};
use std::time::Duration;

/// EVM-specific registry configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvmChain {
    pub chain_id: u64,
    pub default_rpc: &'static str,
    pub explorer: &'static str,
    /// OP-stack rollups (Base, OP Mainnet) charge an L1 data fee on top of gas.
    pub op_stack: bool,
}

/// The EVM family implementation.
pub struct Evm;

pub fn config(chain: &Chain) -> &EvmChain {
    match &chain.family {
        Family::Evm(c) => c,
    }
}

/// `m/44'/60'/0'/0/{index}`: the MetaMask / Trust account order, so the mnemonic imports into
/// standard wallets with the same accounts in the same order.
pub fn derivation_path(index: u32) -> String {
    format!("m/44'/60'/0'/0/{index}")
}

pub fn signer_at(seed: &Seed, index: u32) -> Result<PrivateKeySigner> {
    MnemonicBuilder::<English>::default()
        .phrase(seed.phrase())
        .derivation_path(derivation_path(index))
        .and_then(|b| b.build())
        .map_err(|_| WalletError::validation(format!("cannot derive EVM key at index {index}")))
}

/// Parses a `0x` address. Mixed case must carry a valid EIP-55 checksum. ENS names are refused.
pub fn parse_address(input: &str) -> Result<Address> {
    let s = input.trim();
    if s.contains('.') {
        return Err(WalletError::invalid_address(format!(
            "'{s}' looks like an ENS name; ENS is not supported, pass a 0x address"
        )));
    }
    let body = s.strip_prefix("0x").ok_or_else(|| {
        WalletError::invalid_address(format!("'{s}' is not a 0x-prefixed EVM address"))
    })?;
    if body.len() != 40 || !body.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(WalletError::invalid_address(format!(
            "'{s}' is not a 20-byte hex address"
        )));
    }
    let mixed = body.bytes().any(|b| b.is_ascii_lowercase())
        && body.bytes().any(|b| b.is_ascii_uppercase());
    if mixed {
        Address::parse_checksummed(s, None).map_err(|_| {
            WalletError::invalid_address(format!("'{s}' has an invalid EIP-55 checksum"))
        })
    } else {
        s.parse()
            .map_err(|_| WalletError::invalid_address(format!("'{s}' is not an EVM address")))
    }
}

fn hex0x(bytes: impl AsRef<[u8]>) -> String {
    format!("0x{}", hex::encode(bytes))
}

fn field<'a>(v: &'a Value, name: &str) -> Result<&'a str> {
    v.get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| WalletError::validation(format!("signer response is missing '{name}'")))
}

impl FamilyKeys for Evm {
    fn derivation_path(&self, index: u32) -> String {
        derivation_path(index)
    }

    fn derive_address(&self, seed: &Seed, index: u32) -> Result<String> {
        Ok(signer_at(seed, index)?.address().to_checksum(None))
    }

    fn sign_message(
        &self,
        seed: &Seed,
        index: u32,
        _chain: &Chain,
        message: &[u8],
    ) -> Result<Value> {
        let signer = signer_at(seed, index)?;
        let digest = alloy::primitives::eip191_hash_message(message);
        let signature = signer
            .sign_hash_sync(&digest)
            .map_err(|e| WalletError::validation(format!("signing failed: {e}")))?;
        Ok(json!({
            "address": signer.address().to_checksum(None),
            "signature": hex0x(signature.as_bytes()),
            "digest": hex0x(digest),
        }))
    }

    /// `unsigned` is the hex signing payload and nothing else. The daemon decodes it itself
    /// (`tx::decode_unsigned`), never trusting intent fields from the CLI; a future signing
    /// policy checks that decoded view right before `tx::sign_decoded`.
    fn sign_tx(&self, seed: &Seed, index: u32, chain: &Chain, unsigned: &Value) -> Result<Value> {
        let payload = unsigned
            .as_str()
            .and_then(|s| s.strip_prefix("0x"))
            .and_then(|h| hex::decode(h).ok())
            .ok_or_else(|| {
                WalletError::validation("sign_tx expects the 0x-hex unsigned signing payload")
            })?;
        let decoded = tx::decode_unsigned(&payload)?;
        let want = config(chain).chain_id;
        if decoded.chain_id != want {
            return Err(WalletError::validation(format!(
                "refusing to sign: tx chainId {} is not {} ({want})",
                decoded.chain_id, chain.key
            )));
        }
        let signed = tx::sign_decoded(&signer_at(seed, index)?, &decoded)?;
        Ok(json!({
            "rawTx": hex0x(&signed.raw),
            "txHash": hex0x(signed.hash),
            "decoded": decoded.to_json(),
        }))
    }
}

impl ChainFamily for Evm {
    fn normalize_address(&self, input: &str) -> Result<String> {
        Ok(parse_address(input)?.to_checksum(None))
    }

    fn verify_message(&self, message: &[u8], signed: &Value, expected: &str) -> Result<()> {
        let digest: B256 = field(signed, "digest")?
            .parse()
            .map_err(|_| WalletError::validation("signer returned a malformed digest"))?;
        if digest != alloy::primitives::eip191_hash_message(message) {
            return Err(WalletError::validation("signer signed a different message"));
        }
        let signature: Signature = field(signed, "signature")?
            .parse()
            .map_err(|_| WalletError::validation("signer returned a malformed signature"))?;
        let recovered = signature
            .recover_address_from_prehash(&digest)
            .map_err(|e| WalletError::validation(format!("signature recovery failed: {e}")))?;
        if recovered != parse_address(expected)? {
            return Err(WalletError::validation(format!(
                "signature recovers to {recovered}, expected {expected}"
            )));
        }
        Ok(())
    }

    fn inspect_signed(&self, chain: &Chain, signed: &Value) -> Result<SignedTx> {
        let raw_hex = field(signed, "rawTx")?;
        let raw = hex::decode(raw_hex.trim_start_matches("0x"))
            .map_err(|_| WalletError::validation("signer returned malformed rawTx"))?;
        let decoded = tx::decode_signed(&raw)?;
        if decoded.chain_id != Some(config(chain).chain_id) {
            return Err(WalletError::validation(format!(
                "signed tx is for chainId {:?}, not {}",
                decoded.chain_id, chain.key
            )));
        }
        Ok(SignedTx {
            raw: hex0x(&raw),
            tx_hash: hex0x(decoded.hash),
            signer: decoded.signer.to_checksum(None),
        })
    }

    fn supports_contract_calls(&self) -> bool {
        true
    }

    async fn native_balance(&self, net: &Net, owner: &str) -> Result<Balance> {
        let owner = parse_address(owner)?;
        let p = rpc::connect(net).await?;
        Ok(Balance {
            symbol: net.chain.symbol.to_string(),
            decimals: net.chain.decimals,
            raw: rpc::native_balance(&p, owner).await?,
        })
    }

    async fn token_balance(&self, net: &Net, owner: &str, token: &str) -> Result<Balance> {
        let owner = parse_address(owner)?;
        let token = parse_address(token)?;
        let p = rpc::connect(net).await?;
        let meta = rpc::erc20_meta(&p, net, token).await?;
        Ok(Balance {
            raw: rpc::erc20_balance(&p, token, owner).await?,
            symbol: meta.symbol,
            decimals: meta.decimals,
        })
    }

    async fn prepare_transfer(&self, net: &Net, req: &TransferRequest) -> Result<PreparedTx> {
        rpc::prepare_transfer(net, req).await
    }

    async fn broadcast(&self, net: &Net, signed: &SignedTx) -> Result<String> {
        let raw = hex::decode(signed.raw.trim_start_matches("0x"))
            .map_err(|_| WalletError::validation("malformed raw transaction"))?;
        let p = rpc::connect(net).await?;
        Ok(hex0x(rpc::broadcast(&p, &raw).await?))
    }

    async fn wait_for_receipt(
        &self,
        net: &Net,
        tx_hash: &str,
        timeout: Duration,
    ) -> Result<Option<Value>> {
        let hash: B256 = tx_hash
            .parse()
            .map_err(|_| WalletError::validation("malformed tx hash"))?;
        let p = rpc::connect(net).await?;
        rpc::wait_receipt(&p, hash, timeout).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chains::find;
    use crate::core::error::ErrorCode;
    use crate::core::seed::TEST_MNEMONIC;
    use zeroize::Zeroizing;

    fn seed() -> Seed {
        Seed::from_phrase(Zeroizing::new(TEST_MNEMONIC.into())).unwrap()
    }

    #[test]
    fn known_vectors_index_0_and_1() {
        assert_eq!(
            Evm.derive_address(&seed(), 0).unwrap(),
            "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
        );
        assert_eq!(
            Evm.derive_address(&seed(), 1).unwrap(),
            "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
        );
        assert_eq!(Evm.derivation_path(1), "m/44'/60'/0'/0/1");
    }

    #[test]
    fn sign_message_recovers_to_key_address() {
        let chain = find("ethereum").unwrap();
        for index in [0, 1] {
            let signed = Evm
                .sign_message(&seed(), index, chain, b"hello zwallet")
                .unwrap();
            let address = Evm.derive_address(&seed(), index).unwrap();
            assert_eq!(signed["address"], address.as_str());
            Evm.verify_message(b"hello zwallet", &signed, &address)
                .unwrap();
            assert!(Evm.verify_message(b"other", &signed, &address).is_err());
            let sig: Signature = signed["signature"].as_str().unwrap().parse().unwrap();
            assert!(matches!(sig.as_bytes()[64], 27 | 28));
            assert_eq!(
                sig.recover_address_from_msg(b"hello zwallet").unwrap(),
                parse_address(&address).unwrap()
            );
        }
    }

    #[test]
    fn invoke_data_tx_signs_and_inspects_offline() {
        // The `invoke --data` path: arbitrary calldata through the daemon-side signer and the
        // CLI-side inspector.
        let chain = find("base").unwrap();
        let unsigned = tx::UnsignedEvmTx {
            chain_id: 8453,
            nonce: 4,
            gas_limit: 90_000,
            fees: tx::EvmFees::Eip1559 {
                max_fee_per_gas: alloy::primitives::U128::from(2_000_000_000u64),
                max_priority_fee_per_gas: alloy::primitives::U128::from(1_000_000u64),
            },
            to: parse_address("0x4200000000000000000000000000000000000006").unwrap(),
            value: alloy::primitives::U256::from(1234u64),
            input: abi::parse_calldata("0xd0e30db0").unwrap(),
        };
        let payload = serde_json::json!(hex0x(tx::encoded_for_signing(&unsigned)));
        let signed = Evm.sign_tx(&seed(), 1, chain, &payload).unwrap();
        let inspected = Evm.inspect_signed(chain, &signed).unwrap();
        assert_eq!(
            inspected.signer,
            "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
        );
        assert_eq!(inspected.tx_hash, signed["txHash"].as_str().unwrap());
        assert_eq!(signed["decoded"]["action"]["kind"], "contract-call");
        assert_eq!(signed["decoded"]["value"], "1234");
        // Wrong chain: refused at signing and at inspection.
        let eth = find("ethereum").unwrap();
        assert!(Evm.sign_tx(&seed(), 1, eth, &payload).is_err());
        assert!(Evm.inspect_signed(eth, &signed).is_err());
        // Structured "intent" JSON is not accepted; only the bytes are.
        let intent = serde_json::json!({ "to": "0x4200000000000000000000000000000000000006" });
        assert!(Evm.sign_tx(&seed(), 1, chain, &intent).is_err());
    }

    #[test]
    fn address_parsing_rules() {
        let ok = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
        assert_eq!(Evm.normalize_address(ok).unwrap(), ok);
        assert_eq!(Evm.normalize_address(&ok.to_lowercase()).unwrap(), ok);
        let bad_checksum = "0x70997970c51812dc3A010C7d01b50e0d17dc79C8";
        for bad in [
            "vitalik.eth",
            bad_checksum,
            "0x1234",
            "70997970C51812dc3A010C7d01b50e0d17dc79C8",
        ] {
            assert_eq!(
                Evm.normalize_address(bad).unwrap_err().code,
                ErrorCode::InvalidAddress,
                "{bad}"
            );
        }
        assert!(
            Evm.normalize_address("vitalik.eth")
                .unwrap_err()
                .message
                .contains("ENS")
        );
    }
}
