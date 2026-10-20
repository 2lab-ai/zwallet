//! EVM transaction building, signing, and decoding. Pure functions; no network.
//!
//! Trust boundary: the CLI (agent side) builds an [`UnsignedEvmTx`] and sends the daemon only its
//! signing payload bytes. The daemon runs [`decode_unsigned`] on those bytes to get its own view
//! of what the transaction does, including the ABI decoding of known calldata selectors, and then
//! signs exactly that with [`sign_decoded`]. No intent field from the CLI is ever trusted. A
//! future signing policy checks the [`DecodedUnsigned`] between those two calls.

use crate::core::error::{Result, WalletError};
use alloy::consensus::transaction::{RlpEcdsaDecodableTx, SignerRecoverable};
use alloy::consensus::{SignableTransaction, Transaction, TxEip1559, TxEnvelope, TxLegacy};
use alloy::eips::eip2718::{Decodable2718, Encodable2718};
use alloy::hex;
use alloy::network::TxSignerSync;
use alloy::primitives::{Address, B256, Bytes, TxKind, U128, U256};
use alloy::rlp::Decodable;
use alloy::signers::local::PrivateKeySigner;
use alloy::sol;
use alloy::sol_types::SolCall;
use serde_json::{Value, json};

sol! {
    interface IGasPriceOracle {
        function getL1Fee(bytes memory data) external view returns (uint256);
    }

    interface IERC20 {
        function transfer(address to, uint256 amount) external returns (bool);
        function approve(address spender, uint256 amount) external returns (bool);
        function transferFrom(address from, address to, uint256 amount) external returns (bool);
        function balanceOf(address owner) external view returns (uint256);
        function decimals() external view returns (uint8);
        function symbol() external view returns (string);
    }
}

/// Fee fields of an unsigned transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// A transaction as the CLI builds it from chain state.
#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum ConsensusTx {
    Eip1559(TxEip1559),
    Legacy(TxLegacy),
}

impl ConsensusTx {
    fn encoded_for_signing(&self) -> Vec<u8> {
        match self {
            Self::Eip1559(t) => t.encoded_for_signing(),
            Self::Legacy(t) => t.encoded_for_signing(),
        }
    }

    fn chain_id(&self) -> u64 {
        match self {
            Self::Eip1559(t) => t.chain_id,
            // `decode_unsigned` refuses legacy payloads without an EIP-155 chain id.
            Self::Legacy(t) => t.chain_id.unwrap_or_default(),
        }
    }

    fn fees(&self) -> EvmFees {
        match self {
            Self::Eip1559(t) => EvmFees::Eip1559 {
                max_fee_per_gas: U128::from(t.max_fee_per_gas),
                max_priority_fee_per_gas: U128::from(t.max_priority_fee_per_gas),
            },
            Self::Legacy(t) => EvmFees::Legacy {
                gas_price: U128::from(t.gas_price),
            },
        }
    }

    fn fields(&self) -> (u64, u64, TxKind, U256, &Bytes) {
        match self {
            Self::Eip1559(t) => (t.nonce, t.gas_limit, t.to, t.value, &t.input),
            Self::Legacy(t) => (t.nonce, t.gas_limit, t.to, t.value, &t.input),
        }
    }
}

fn to_consensus(tx: &UnsignedEvmTx) -> ConsensusTx {
    match tx.fees {
        EvmFees::Eip1559 {
            max_fee_per_gas,
            max_priority_fee_per_gas,
        } => ConsensusTx::Eip1559(TxEip1559 {
            chain_id: tx.chain_id,
            nonce: tx.nonce,
            gas_limit: tx.gas_limit,
            max_fee_per_gas: max_fee_per_gas.to(),
            max_priority_fee_per_gas: max_priority_fee_per_gas.to(),
            to: TxKind::Call(tx.to),
            value: tx.value,
            access_list: Default::default(),
            input: tx.input.clone(),
        }),
        EvmFees::Legacy { gas_price } => ConsensusTx::Legacy(TxLegacy {
            chain_id: Some(tx.chain_id),
            nonce: tx.nonce,
            gas_price: gas_price.to(),
            gas_limit: tx.gas_limit,
            to: TxKind::Call(tx.to),
            value: tx.value,
            input: tx.input.clone(),
        }),
    }
}

/// The signing payload: `0x02 || rlp(fields)` for EIP-1559, the EIP-155 RLP for legacy. This is
/// what the CLI sends the daemon, and what OP-stack `GasPriceOracle.getL1Fee` prices.
pub fn encoded_for_signing(tx: &UnsignedEvmTx) -> Vec<u8> {
    to_consensus(tx).encoded_for_signing()
}

/// What a transaction does, from the daemon's own ABI decoding of the calldata. Amounts are raw
/// base units (the daemon makes no RPC, so it does not know token decimals).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Empty calldata: plain value transfer to `to`.
    NativeTransfer { to: Address, value: U256 },
    Erc20Transfer {
        token: Address,
        recipient: Address,
        amount: U256,
    },
    Erc20Approve {
        token: Address,
        spender: Address,
        amount: U256,
    },
    Erc20TransferFrom {
        token: Address,
        from: Address,
        recipient: Address,
        amount: U256,
    },
    /// Any other selector.
    ContractCall {
        contract: Address,
        selector: [u8; 4],
        value: U256,
    },
}

impl Action {
    pub fn to_json(&self) -> Value {
        let a = |x: &Address| x.to_checksum(None);
        match self {
            Self::NativeTransfer { to, value } => {
                json!({ "kind": "native-transfer", "to": a(to), "value": value.to_string() })
            }
            Self::Erc20Transfer {
                token,
                recipient,
                amount,
            } => json!({
                "kind": "erc20-transfer", "token": a(token), "recipient": a(recipient),
                "amount": amount.to_string(),
            }),
            Self::Erc20Approve {
                token,
                spender,
                amount,
            } => json!({
                "kind": "erc20-approve", "token": a(token), "spender": a(spender),
                "amount": amount.to_string(),
            }),
            Self::Erc20TransferFrom {
                token,
                from,
                recipient,
                amount,
            } => json!({
                "kind": "erc20-transfer-from", "token": a(token), "from": a(from),
                "recipient": a(recipient), "amount": amount.to_string(),
            }),
            Self::ContractCall {
                contract,
                selector,
                value,
            } => json!({
                "kind": "contract-call", "contract": a(contract),
                "selector": format!("0x{}", hex::encode(selector)), "value": value.to_string(),
            }),
        }
    }
}

/// Decodes known-selector calldata and requires it to be that call's canonical ABI encoding.
/// alloy's default decoder ignores trailing bytes and the 12 high bytes of address words, so
/// different byte strings would decode to the same action; re-encoding must reproduce `input`.
fn decode_canonical<C: SolCall>(input: &[u8], name: &str) -> Result<C> {
    let malformed = || {
        WalletError::validation(format!(
            "refusing to sign: calldata has the ERC-20 {name} selector but is not its canonical \
             ABI encoding"
        ))
    };
    let call = C::abi_decode(input).map_err(|_| malformed())?;
    if call.abi_encode() != input {
        return Err(malformed());
    }
    Ok(call)
}

fn classify(to: Address, value: U256, input: &[u8]) -> Result<Action> {
    let Some(selector) = input.get(..4) else {
        return if input.is_empty() {
            Ok(Action::NativeTransfer { to, value })
        } else {
            Err(WalletError::validation(
                "refusing to sign: calldata shorter than a selector",
            ))
        };
    };
    // zwallet never attaches native value to an ERC-20 call; a policy keyed on the ERC-20 action
    // would not see that native spend, so such a transaction is refused outright.
    let erc20 = |name: &str| {
        if value.is_zero() {
            Ok(())
        } else {
            Err(WalletError::validation(format!(
                "refusing to sign: ERC-20 {name} call that also sends {value} wei of native value"
            )))
        }
    };
    let selector: [u8; 4] = selector.try_into().unwrap_or_default();
    Ok(match selector {
        IERC20::transferCall::SELECTOR => {
            erc20("transfer")?;
            let c: IERC20::transferCall = decode_canonical(input, "transfer")?;
            Action::Erc20Transfer {
                token: to,
                recipient: c.to,
                amount: c.amount,
            }
        }
        IERC20::approveCall::SELECTOR => {
            erc20("approve")?;
            let c: IERC20::approveCall = decode_canonical(input, "approve")?;
            Action::Erc20Approve {
                token: to,
                spender: c.spender,
                amount: c.amount,
            }
        }
        IERC20::transferFromCall::SELECTOR => {
            erc20("transferFrom")?;
            let c: IERC20::transferFromCall = decode_canonical(input, "transferFrom")?;
            Action::Erc20TransferFrom {
                token: to,
                from: c.from,
                recipient: c.to,
                amount: c.amount,
            }
        }
        _ => Action::ContractCall {
            contract: to,
            selector,
            value,
        },
    })
}

/// The daemon's view of an unsigned transaction, decoded from the exact bytes it will sign.
///
/// Read-only by construction: the only state is the consensus transaction that
/// [`sign_decoded`] signs and the [`Action`] classified from it at decode time, both private.
/// Every accessor reads that same object, so a policy cannot be shown one transaction while
/// another is signed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedUnsigned {
    consensus: ConsensusTx,
    action: Action,
}

impl DecodedUnsigned {
    pub fn chain_id(&self) -> u64 {
        self.consensus.chain_id()
    }

    pub fn nonce(&self) -> u64 {
        self.consensus.fields().0
    }

    pub fn gas_limit(&self) -> u64 {
        self.consensus.fields().1
    }

    pub fn fees(&self) -> EvmFees {
        self.consensus.fees()
    }

    /// The called address (`decode_unsigned` refuses contract creation).
    pub fn to(&self) -> Address {
        self.consensus.fields().2.to().copied().unwrap_or_default()
    }

    pub fn value(&self) -> U256 {
        self.consensus.fields().3
    }

    pub fn input(&self) -> &Bytes {
        self.consensus.fields().4
    }

    pub fn action(&self) -> &Action {
        &self.action
    }

    pub fn to_json(&self) -> Value {
        json!({
            "chainId": self.chain_id(),
            "nonce": self.nonce(),
            "gasLimit": self.gas_limit(),
            "maxGasPrice": self.fees().max_price().to_string(),
            "to": self.to().to_checksum(None),
            "value": self.value().to_string(),
            "action": self.action.to_json(),
        })
    }
}

/// Decodes a signing payload. Accepts only what zwallet itself produces: EIP-1559 (type 2) with
/// an empty access list, or EIP-155 legacy; a call (never a contract creation); canonical bytes
/// (re-encoding must reproduce the input exactly, so the bytes decoded are the bytes signed).
pub fn decode_unsigned(payload: &[u8]) -> Result<DecodedUnsigned> {
    let refuse = |why: &str| WalletError::validation(format!("refusing to sign: {why}"));
    let consensus = match payload.first() {
        Some(0x02) => {
            let mut buf = &payload[1..];
            let tx = TxEip1559::rlp_decode(&mut buf)
                .map_err(|e| refuse(&format!("bad EIP-1559 payload: {e}")))?;
            if !buf.is_empty() {
                return Err(refuse("trailing bytes after the transaction"));
            }
            if !tx.access_list.is_empty() {
                return Err(refuse("access lists are not supported"));
            }
            if tx.max_priority_fee_per_gas > tx.max_fee_per_gas {
                return Err(refuse("maxPriorityFeePerGas exceeds maxFeePerGas"));
            }
            ConsensusTx::Eip1559(tx)
        }
        Some(b) if *b >= 0xc0 => {
            let mut buf = payload;
            let tx = <TxLegacy as Decodable>::decode(&mut buf)
                .map_err(|e| refuse(&format!("bad legacy payload: {e}")))?;
            if tx.chain_id.is_none() {
                return Err(refuse("legacy transaction without EIP-155 chain id"));
            }
            ConsensusTx::Legacy(tx)
        }
        Some(t) => return Err(refuse(&format!("unsupported transaction type 0x{t:02x}"))),
        None => return Err(refuse("empty payload")),
    };
    if consensus.encoded_for_signing() != payload {
        return Err(refuse("non-canonical transaction encoding"));
    }
    let (_, _, kind, value, input) = consensus.fields();
    let TxKind::Call(to) = kind else {
        return Err(refuse("contract creation is not supported"));
    };
    let action = classify(to, value, input)?;
    Ok(DecodedUnsigned { consensus, action })
}

/// Signs exactly the decoded transaction.
pub fn sign_decoded(signer: &PrivateKeySigner, decoded: &DecodedUnsigned) -> Result<SignedEvmTx> {
    let fail = |e: alloy::signers::Error| WalletError::validation(format!("signing failed: {e}"));
    let envelope: TxEnvelope = match decoded.consensus.clone() {
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

/// What the raw signed bytes actually say, independent of whoever produced them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedEvmTx {
    pub chain_id: Option<u64>,
    pub hash: B256,
    pub signer: Address,
    pub nonce: u64,
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
        nonce: envelope.nonce(),
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

    fn tx(chain_id: u64, fees: EvmFees, to: Address, value: U256, input: Bytes) -> UnsignedEvmTx {
        UnsignedEvmTx {
            chain_id,
            nonce: 7,
            gas_limit: 65_000,
            fees,
            to,
            value,
            input,
        }
    }

    /// The full daemon path: payload bytes -> own decode -> sign -> recover from raw.
    fn sign_via_payload(t: &UnsignedEvmTx) -> (DecodedUnsigned, DecodedEvmTx) {
        let decoded = decode_unsigned(&encoded_for_signing(t)).unwrap();
        let signed = sign_decoded(&signer(), &decoded).unwrap();
        let back = decode_signed(&signed.raw).unwrap();
        assert_eq!(back.hash, signed.hash);
        // The policy view and the signed bytes describe one transaction.
        assert_eq!(Some(decoded.chain_id()), back.chain_id);
        assert_eq!(decoded.nonce(), back.nonce);
        assert_eq!(Some(decoded.to()), back.to);
        assert_eq!(decoded.value(), back.value);
        assert_eq!((decoded.gas_limit(), decoded.fees()), (t.gas_limit, t.fees));
        assert_eq!(decoded.input(), &t.input);
        (decoded, back)
    }

    #[test]
    fn native_transfer_signs_and_recovers_offline() {
        let value = U256::from(10u64).pow(U256::from(15u64));
        let t = tx(1, eip1559(), RECIPIENT, value, Bytes::new());
        let (decoded, back) = sign_via_payload(&t);
        assert_eq!(
            *decoded.action(),
            Action::NativeTransfer {
                to: RECIPIENT,
                value
            }
        );
        assert_eq!(decoded.chain_id(), 1);
        assert_eq!(back.signer, HARDHAT0);
        assert_eq!(back.chain_id, Some(1));
        assert_eq!(back.to, Some(RECIPIENT));
        assert_eq!(back.value, value);
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
        let t = tx(8453, eip1559(), USDC, U256::ZERO, data);
        let (decoded, back) = sign_via_payload(&t);
        // The daemon's own reading of the calldata, not anything the CLI claimed.
        assert_eq!(
            *decoded.action(),
            Action::Erc20Transfer {
                token: USDC,
                recipient: RECIPIENT,
                amount
            }
        );
        assert_eq!(decoded.to_json()["action"]["kind"], "erc20-transfer");
        assert_eq!(back.signer, HARDHAT0);
        assert_eq!(back.chain_id, Some(8453));
    }

    #[test]
    fn approve_transfer_from_and_unknown_selectors_are_classified() {
        let approve = IERC20::approveCall {
            spender: RECIPIENT,
            amount: U256::MAX,
        }
        .abi_encode();
        let d = decode_unsigned(&encoded_for_signing(&tx(
            1,
            eip1559(),
            USDC,
            U256::ZERO,
            approve.into(),
        )))
        .unwrap();
        assert!(
            matches!(*d.action(), Action::Erc20Approve { spender, amount, .. }
            if spender == RECIPIENT && amount == U256::MAX)
        );

        let from = IERC20::transferFromCall {
            from: HARDHAT0,
            to: RECIPIENT,
            amount: U256::from(5u64),
        }
        .abi_encode();
        let d = decode_unsigned(&encoded_for_signing(&tx(
            1,
            eip1559(),
            USDC,
            U256::ZERO,
            from.into(),
        )))
        .unwrap();
        assert!(matches!(d.action(), Action::Erc20TransferFrom { .. }));

        let weth_deposit = Bytes::from_static(&[0xd0, 0xe3, 0x0d, 0xb0]);
        let d = decode_unsigned(&encoded_for_signing(&tx(
            1,
            eip1559(),
            USDC,
            U256::from(9u64),
            weth_deposit,
        )))
        .unwrap();
        assert_eq!(
            d.action().to_json()["selector"],
            "0xd0e30db0",
            "unknown selectors are reported, not guessed"
        );
    }

    #[test]
    fn legacy_transfer_is_eip155() {
        let t = tx(
            56,
            EvmFees::Legacy {
                gas_price: U128::from(100_000_000u64),
            },
            RECIPIENT,
            U256::from(1u64),
            Bytes::new(),
        );
        let (decoded, back) = sign_via_payload(&t);
        assert_eq!(decoded.chain_id(), 56);
        assert_eq!(back.signer, HARDHAT0);
        assert_eq!(back.chain_id, Some(56));
    }

    #[test]
    fn decoder_refuses_anything_zwallet_does_not_produce() {
        let good = encoded_for_signing(&tx(1, eip1559(), RECIPIENT, U256::ZERO, Bytes::new()));
        let mut trailing = good.clone();
        trailing.push(0);
        let mut type1 = good.clone();
        type1[0] = 0x01;
        let mut type4 = good.clone();
        type4[0] = 0x04;
        let bad_transfer = {
            let mut d = erc20_transfer_calldata(RECIPIENT, U256::from(1u64)).to_vec();
            d.truncate(40);
            encoded_for_signing(&tx(1, eip1559(), USDC, U256::ZERO, d.into()))
        };
        let inverted_fees = encoded_for_signing(&tx(
            1,
            EvmFees::Eip1559 {
                max_fee_per_gas: U128::from(1u64),
                max_priority_fee_per_gas: U128::from(2u64),
            },
            RECIPIENT,
            U256::ZERO,
            Bytes::new(),
        ));
        // Legacy with non-zero EIP-155 sentinels: decodes leniently, refused as non-canonical.
        let legacy = encoded_for_signing(&tx(
            56,
            EvmFees::Legacy {
                gas_price: U128::from(1u64),
            },
            RECIPIENT,
            U256::ZERO,
            Bytes::new(),
        ));
        let mut sentinel = legacy.clone();
        let n = sentinel.len();
        sentinel[n - 1] = 0x01;
        for (name, payload) in [
            ("empty", vec![]),
            ("trailing", trailing),
            ("type1", type1),
            ("type4", type4),
            ("malformed transfer", bad_transfer),
            ("inverted fees", inverted_fees),
            ("short selector", {
                encoded_for_signing(&tx(1, eip1559(), USDC, U256::ZERO, vec![1, 2].into()))
            }),
            ("legacy sentinel", sentinel),
            ("json instead of bytes", b"{\"to\":\"0x\"}".to_vec()),
        ] {
            assert!(decode_unsigned(&payload).is_err(), "{name} was accepted");
        }
        assert!(decode_unsigned(&good).is_ok());
        assert!(decode_unsigned(&legacy).is_ok());
    }

    /// Calldata for the three known ERC-20 selectors, each with every address word listed by
    /// byte offset (selector included).
    fn known_selector_calldata() -> Vec<(&'static str, Vec<u8>, Vec<usize>)> {
        vec![
            (
                "transfer",
                IERC20::transferCall {
                    to: RECIPIENT,
                    amount: U256::from(5u64),
                }
                .abi_encode(),
                vec![4],
            ),
            (
                "approve",
                IERC20::approveCall {
                    spender: RECIPIENT,
                    amount: U256::MAX,
                }
                .abi_encode(),
                vec![4],
            ),
            (
                "transferFrom",
                IERC20::transferFromCall {
                    from: HARDHAT0,
                    to: RECIPIENT,
                    amount: U256::from(5u64),
                }
                .abi_encode(),
                vec![4, 36],
            ),
        ]
    }

    fn decode_calldata(value: U256, data: Vec<u8>) -> Result<DecodedUnsigned> {
        decode_unsigned(&encoded_for_signing(&tx(
            1,
            eip1559(),
            USDC,
            value,
            data.into(),
        )))
    }

    #[test]
    fn known_selectors_refuse_trailing_bytes() {
        let mut accepted = Vec::new();
        for (name, data, _) in known_selector_calldata() {
            assert!(decode_calldata(U256::ZERO, data.clone()).is_ok(), "{name}");
            for suffix in [vec![0u8], vec![0u8; 32], vec![0xff; 5]] {
                let mut d = data.clone();
                d.extend_from_slice(&suffix);
                if decode_calldata(U256::ZERO, d).is_ok() {
                    accepted.push(format!("{name}+{}", suffix.len()));
                }
            }
        }
        assert!(
            accepted.is_empty(),
            "accepted with trailing bytes: {accepted:?}"
        );
    }

    #[test]
    fn known_selectors_refuse_dirty_address_padding() {
        let mut accepted = Vec::new();
        for (name, data, address_words) in known_selector_calldata() {
            for word in address_words {
                // Any non-zero byte in the 12 high bytes of an address word.
                for at in [word, word + 11] {
                    let mut d = data.clone();
                    d[at] = 0x01;
                    if decode_calldata(U256::ZERO, d).is_ok() {
                        accepted.push(format!("{name}@{at}"));
                    }
                }
            }
        }
        assert!(
            accepted.is_empty(),
            "accepted with dirty padding: {accepted:?}"
        );
    }

    #[test]
    fn known_erc20_selectors_refuse_native_value() {
        let mut accepted = Vec::new();
        for (name, data, _) in known_selector_calldata() {
            match decode_calldata(U256::from(1u64), data) {
                Ok(_) => accepted.push(name),
                Err(e) => assert!(e.message.contains("refusing to sign"), "{name}: {e}"),
            }
        }
        assert!(
            accepted.is_empty(),
            "accepted with native value: {accepted:?}"
        );
        // An unknown selector may carry value (e.g. WETH deposit).
        let deposit = vec![0xd0, 0xe3, 0x0d, 0xb0];
        assert!(decode_calldata(U256::from(1u64), deposit).is_ok());
    }

    #[test]
    fn gas_margin() {
        assert_eq!(gas_with_margin(21_000), 21_000);
        assert_eq!(gas_with_margin(50_000), 60_000);
    }
}
