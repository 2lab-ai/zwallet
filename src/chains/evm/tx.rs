//! EVM transaction building, signing, and decoding. Pure functions; no network.

use crate::core::error::{Result, WalletError};
use alloy::consensus::transaction::SignerRecoverable;
use alloy::consensus::{SignableTransaction, Transaction, TxEip1559, TxEnvelope, TxLegacy};
use alloy::eips::eip2718::{Decodable2718, Encodable2718};
use alloy::network::TxSignerSync;
use alloy::primitives::{Address, B256, Bytes, TxKind, U128, U256};
use alloy::signers::local::PrivateKeySigner;
use alloy::sol;
use alloy::sol_types::SolCall;
use serde::{Deserialize, Serialize};

sol! {
    interface IGasPriceOracle {
        function getL1Fee(bytes memory data) external view returns (uint256);
    }

    interface IERC20 {
        function transfer(address to, uint256 amount) external returns (bool);
        function balanceOf(address owner) external view returns (uint256);
        function decimals() external view returns (uint8);
        function symbol() external view returns (string);
    }
}

/// Fee fields of an unsigned transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
pub enum EvmFees {
    Eip1559 {
        max_fee_per_gas: U128,
        max_priority_fee_per_gas: U128,
    },
    Legacy {
        gas_price: U128,
    },
}

impl EvmFees {
    /// Worst-case price per gas unit.
    pub fn max_price(&self) -> u128 {
        match self {
            Self::Eip1559 {
                max_fee_per_gas, ..
            } => max_fee_per_gas.to(),
            Self::Legacy { gas_price } => gas_price.to(),
        }
    }
}

/// The unsigned transaction the CLI hands the daemon (`sign_tx.tx`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnsignedEvmTx {
    pub chain_id: u64,
    pub nonce: u64,
    pub gas_limit: u64,
    pub fees: EvmFees,
    pub to: Address,
    pub value: U256,
    pub input: Bytes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedEvmTx {
    pub raw: Bytes,
    pub hash: B256,
}

/// Signs an unsigned transaction (EIP-1559 type 2, or EIP-155 legacy).
enum ConsensusTx {
    Eip1559(TxEip1559),
    Legacy(TxLegacy),
}

fn to_consensus(tx: &UnsignedEvmTx) -> Result<ConsensusTx> {
    Ok(match tx.fees {
        EvmFees::Eip1559 {
            max_fee_per_gas,
            max_priority_fee_per_gas,
        } => {
            if max_priority_fee_per_gas > max_fee_per_gas {
                return Err(WalletError::validation(
                    "maxPriorityFeePerGas exceeds maxFeePerGas",
                ));
            }
            ConsensusTx::Eip1559(TxEip1559 {
                chain_id: tx.chain_id,
                nonce: tx.nonce,
                gas_limit: tx.gas_limit,
                max_fee_per_gas: max_fee_per_gas.to(),
                max_priority_fee_per_gas: max_priority_fee_per_gas.to(),
                to: TxKind::Call(tx.to),
                value: tx.value,
                access_list: Default::default(),
                input: tx.input.clone(),
            })
        }
        EvmFees::Legacy { gas_price } => ConsensusTx::Legacy(TxLegacy {
            chain_id: Some(tx.chain_id),
            nonce: tx.nonce,
            gas_price: gas_price.to(),
            gas_limit: tx.gas_limit,
            to: TxKind::Call(tx.to),
            value: tx.value,
            input: tx.input.clone(),
        }),
    })
}

/// The unsigned RLP payload (what OP-stack `GasPriceOracle.getL1Fee` prices).
pub fn encoded_for_signing(tx: &UnsignedEvmTx) -> Result<Vec<u8>> {
    Ok(match to_consensus(tx)? {
        ConsensusTx::Eip1559(t) => t.encoded_for_signing(),
        ConsensusTx::Legacy(t) => t.encoded_for_signing(),
    })
}

/// Signs an unsigned transaction (EIP-1559 type 2, or EIP-155 legacy).
pub fn sign(signer: &PrivateKeySigner, tx: &UnsignedEvmTx) -> Result<SignedEvmTx> {
    let fail = |e: alloy::signers::Error| WalletError::validation(format!("signing failed: {e}"));
    let envelope: TxEnvelope = match to_consensus(tx)? {
        ConsensusTx::Eip1559(mut t) => {
            let sig = signer.sign_transaction_sync(&mut t).map_err(fail)?;
            t.into_signed(sig).into()
        }
        ConsensusTx::Legacy(mut t) => {
            let sig = signer.sign_transaction_sync(&mut t).map_err(fail)?;
            t.into_signed(sig).into()
        }
    };
    Ok(SignedEvmTx {
        raw: envelope.encoded_2718().into(),
        hash: *envelope.tx_hash(),
    })
}

/// What the raw bytes actually say, independent of whoever produced them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedEvmTx {
    pub chain_id: Option<u64>,
    pub hash: B256,
    pub signer: Address,
    pub to: Option<Address>,
    pub value: U256,
}

pub fn decode_signed(raw: &[u8]) -> Result<DecodedEvmTx> {
    let envelope = TxEnvelope::decode_2718(&mut &raw[..])
        .map_err(|e| WalletError::validation(format!("signed tx does not decode: {e}")))?;
    let signer = envelope
        .recover_signer()
        .map_err(|e| WalletError::validation(format!("cannot recover tx signer: {e}")))?;
    Ok(DecodedEvmTx {
        chain_id: envelope.chain_id(),
        hash: *envelope.tx_hash(),
        signer,
        to: envelope.to(),
        value: envelope.value(),
    })
}

pub fn erc20_transfer_calldata(to: Address, amount: U256) -> Bytes {
    IERC20::transferCall { to, amount }.abi_encode().into()
}

/// Gas limit with headroom over `eth_estimateGas`. A plain 21000 transfer is exact.
pub fn gas_with_margin(estimate: u64) -> u64 {
    if estimate == 21_000 {
        estimate
    } else {
        estimate.saturating_add(estimate / 5)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chains::evm::signer_at;
    use crate::core::seed::{Seed, TEST_MNEMONIC};
    use alloy::primitives::address;
    use zeroize::Zeroizing;

    const HARDHAT0: Address = address!("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
    const USDC: Address = address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
    const RECIPIENT: Address = address!("0x70997970C51812dc3A010C7d01b50e0d17dc79C8");

    fn signer() -> PrivateKeySigner {
        let seed = Seed::from_phrase(Zeroizing::new(TEST_MNEMONIC.into())).unwrap();
        signer_at(&seed, 0).unwrap()
    }

    fn eip1559() -> EvmFees {
        EvmFees::Eip1559 {
            max_fee_per_gas: U128::from(30_000_000_000u64),
            max_priority_fee_per_gas: U128::from(1_000_000_000u64),
        }
    }

    #[test]
    fn native_transfer_signs_and_recovers_offline() {
        let tx = UnsignedEvmTx {
            chain_id: 1,
            nonce: 7,
            gas_limit: 21_000,
            fees: eip1559(),
            to: RECIPIENT,
            value: U256::from(10u64).pow(U256::from(15u64)),
            input: Bytes::new(),
        };
        let signed = sign(&signer(), &tx).unwrap();
        assert_eq!(signed.raw[0], 0x02, "EIP-1559 typed envelope");
        let decoded = decode_signed(&signed.raw).unwrap();
        assert_eq!(decoded.signer, HARDHAT0);
        assert_eq!(decoded.chain_id, Some(1));
        assert_eq!(decoded.hash, signed.hash);
        assert_eq!(decoded.to, Some(RECIPIENT));
        assert_eq!(decoded.value, tx.value);
    }

    #[test]
    fn erc20_transfer_signs_and_recovers_offline() {
        let amount = U256::from(1_500_000u64); // 1.5 USDC
        let data = erc20_transfer_calldata(RECIPIENT, amount);
        assert_eq!(
            &data[..4],
            &[0xa9, 0x05, 0x9c, 0xbb],
            "transfer(address,uint256)"
        );
        assert_eq!(data.len(), 4 + 32 + 32);
        assert_eq!(&data[4 + 12..4 + 32], RECIPIENT.as_slice());
        assert_eq!(U256::from_be_slice(&data[36..68]), amount);
        let tx = UnsignedEvmTx {
            chain_id: 8453,
            nonce: 0,
            gas_limit: 65_000,
            fees: eip1559(),
            to: USDC,
            value: U256::ZERO,
            input: data,
        };
        let decoded = decode_signed(&sign(&signer(), &tx).unwrap().raw).unwrap();
        assert_eq!(decoded.signer, HARDHAT0);
        assert_eq!(decoded.chain_id, Some(8453));
        assert_eq!(decoded.to, Some(USDC));
    }

    #[test]
    fn legacy_transfer_is_eip155() {
        let tx = UnsignedEvmTx {
            chain_id: 56,
            nonce: 3,
            gas_limit: 21_000,
            fees: EvmFees::Legacy {
                gas_price: U128::from(100_000_000u64),
            },
            to: RECIPIENT,
            value: U256::from(1u64),
            input: Bytes::new(),
        };
        let decoded = decode_signed(&sign(&signer(), &tx).unwrap().raw).unwrap();
        assert_eq!(decoded.signer, HARDHAT0);
        assert_eq!(decoded.chain_id, Some(56));
    }

    #[test]
    fn unsigned_tx_json_round_trip() {
        let tx = UnsignedEvmTx {
            chain_id: 1,
            nonce: 1,
            gas_limit: 21_000,
            fees: eip1559(),
            to: RECIPIENT,
            value: U256::from(5u64),
            input: Bytes::from_static(&[1, 2, 3]),
        };
        let v = serde_json::to_value(&tx).unwrap();
        assert_eq!(v["fees"]["type"], "eip1559");
        assert!(v["fees"]["maxFeePerGas"].is_string());
        assert_eq!(serde_json::from_value::<UnsignedEvmTx>(v).unwrap(), tx);
    }

    #[test]
    fn priority_above_max_fee_is_refused() {
        let tx = UnsignedEvmTx {
            chain_id: 1,
            nonce: 0,
            gas_limit: 21_000,
            fees: EvmFees::Eip1559 {
                max_fee_per_gas: U128::from(1u64),
                max_priority_fee_per_gas: U128::from(2u64),
            },
            to: RECIPIENT,
            value: U256::ZERO,
            input: Bytes::new(),
        };
        assert!(sign(&signer(), &tx).is_err());
    }

    #[test]
    fn gas_margin() {
        assert_eq!(gas_with_margin(21_000), 21_000);
        assert_eq!(gas_with_margin(50_000), 60_000);
    }
}
