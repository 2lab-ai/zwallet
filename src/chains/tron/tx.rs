//! Strict, bounded Tron transaction codec. No network or policy decisions.
//!
//! Every nesting level rejects unknown/duplicate/out-of-order fields and non-minimal varints.
//! This is deliberately narrower than general protobuf. See `.prd/05-tron.md` (TR12–TR13).

use super::address;
use crate::core::error::{Result, WalletError};
use alloy::primitives::{B256, Signature, U256};
use alloy::signers::{SignerSync, local::PrivateKeySigner};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const MAX_RAW_BYTES: usize = 1024;
const TRANSFER_URL: &[u8] = b"type.googleapis.com/protocol.TransferContract";
const TRIGGER_URL: &[u8] = b"type.googleapis.com/protocol.TriggerSmartContract";
const TRANSFER_SELECTOR: [u8; 4] = [0xa9, 0x05, 0x9c, 0xbb];

/// The understood contract wire shapes. A TriggerSmart is not necessarily allowed: policy
/// must require [`DecodedTron::transfer_call`] and check the registry and whitelist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Contract {
    Transfer {
        to: [u8; 21],
        amount_sun: u64,
    },
    TriggerSmart {
        contract_address: [u8; 21],
        data: Vec<u8>,
    },
}

/// Whole decoded Transaction.raw. Clock bounds, owner identity and whitelist are policy checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedTron {
    pub ref_block_bytes: [u8; 2],
    pub ref_block_hash: [u8; 8],
    pub expiration: u64,
    pub timestamp: u64,
    pub owner: [u8; 21],
    pub contract: Contract,
    /// Absent is protobuf's zero default. A present field must be positive and is allowed only
    /// for TriggerSmartContract. The CLI supplies a positive bound for token sends.
    pub fee_limit: Option<u64>,
}

impl DecodedTron {
    /// Only exact transfer(address,uint256) calldata is a transfer. Dirty address padding,
    /// another selector, short data or a trailing ABI word must never acquire this meaning.
    pub fn transfer_call(&self) -> Option<([u8; 21], U256)> {
        let Contract::TriggerSmart { data, .. } = &self.contract else {
            return None;
        };
        if data.len() != 68 || data[..4] != TRANSFER_SELECTOR || data[4..16] != [0; 12] {
            return None;
        }
        let mut to = [0u8; 21];
        to[0] = address::PREFIX;
        to[1..].copy_from_slice(&data[16..36]);
        Some((to, U256::from_be_slice(&data[36..])))
    }

    /// Independent review summary. A selector alone never identifies an asset standard.
    pub fn to_json(&self) -> Result<Value> {
        let raw = encode_raw(self)?;
        let owner = address::encode(&self.owner);
        let mut result = match &self.contract {
            Contract::Transfer { to, amount_sun } => json!({
                "kind": "native-transfer", "owner": owner,
                "to": address::encode(to), "amount": amount_sun.to_string(),
            }),
            Contract::TriggerSmart {
                contract_address,
                data,
            } => {
                let token = address::encode(contract_address);
                match self.transfer_call() {
                    Some((to, amount)) => json!({
                        "kind": "transfer-call", "owner": owner, "token": token,
                        "to": address::encode(&to), "amount": amount.to_string(),
                        "feeLimit": self.fee_limit.unwrap_or(0).to_string(),
                        "assetType": "unverified",
                    }),
                    None => json!({
                        "kind": "contract-call", "owner": owner, "contract": token,
                        "selector": format!("0x{}", alloy::hex::encode(&data[..data.len().min(4)])),
                        "feeLimit": self.fee_limit.unwrap_or(0).to_string(),
                    }),
                }
            }
        };
        result["expiration"] = json!(self.expiration);
        result["txHash"] = json!(alloy::hex::encode(txid(&raw)));
        Ok(result)
    }
}

fn refused(why: impl std::fmt::Display) -> WalletError {
    WalletError::validation(format!("refusing to sign: {why}"))
}

/// A bounded slice cursor. No recursion or allocation from untrusted length fields.
struct Reader<'a> {
    rest: &'a [u8],
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { rest: bytes }
    }

    fn varint(&mut self) -> Result<u64> {
        let mut result = 0u64;
        for i in 0..10 {
            let (&byte, rest) = self
                .rest
                .split_first()
                .ok_or_else(|| refused("truncated varint"))?;
            self.rest = rest;
            if i == 9 && byte > 1 {
                return Err(refused("varint overflow"));
            }
            result |= u64::from(byte & 0x7f) << (i * 7);
            if byte & 0x80 == 0 {
                if i > 0 && byte == 0 {
                    return Err(refused("non-minimal varint"));
                }
                return Ok(result);
            }
        }
        Err(refused("varint overflow"))
    }

    fn key(&mut self, field: u64, wire: u64) -> Result<()> {
        if self.varint()? != field * 8 + wire {
            return Err(refused(format!(
                "expected field {field} with wire type {wire}; unknown, duplicate, out-of-order or wrong-wire field"
            )));
        }
        Ok(())
    }

    fn uint(&mut self, field: u64) -> Result<u64> {
        self.key(field, 0)?;
        self.varint()
    }

    fn bytes(&mut self, field: u64) -> Result<&'a [u8]> {
        self.key(field, 2)?;
        let len = usize::try_from(self.varint()?).map_err(|_| refused("length overflow"))?;
        let value = self
            .rest
            .get(..len)
            .ok_or_else(|| refused("length past the end"))?;
        self.rest = &self.rest[len..];
        Ok(value)
    }

    fn fixed<const N: usize>(&mut self, field: u64) -> Result<[u8; N]> {
        self.bytes(field)?
            .try_into()
            .map_err(|_| refused(format!("field {field} must be {N} bytes")))
    }

    fn finish(self) -> Result<()> {
        if self.rest.is_empty() {
            Ok(())
        } else {
            Err(refused(
                "unknown, duplicate, out-of-order field or trailing bytes",
            ))
        }
    }
}

fn varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn uint(out: &mut Vec<u8>, field: u64, value: u64) {
    varint(out, field * 8);
    varint(out, value);
}

fn bytes(out: &mut Vec<u8>, field: u64, value: &[u8]) {
    varint(out, field * 8 + 2);
    varint(out, value.len() as u64);
    out.extend_from_slice(value);
}

fn positive_int64(value: u64, field: &str) -> Result<()> {
    if value == 0 || value > i64::MAX as u64 {
        Err(refused(format!("{field} must be positive and below 2^63")))
    } else {
        Ok(())
    }
}

fn check_address(value: &[u8; 21]) -> Result<()> {
    if value[0] != address::PREFIX {
        Err(refused("address prefix must be 0x41"))
    } else {
        Ok(())
    }
}

/// Encode exactly the understood fields in ascending order. Locally built objects have the
/// same validation as decoded input. No ABI/policy check is smuggled into the encoder.
pub fn encode_raw(tx: &DecodedTron) -> Result<Vec<u8>> {
    positive_int64(tx.timestamp, "timestamp")?;
    positive_int64(tx.expiration, "expiration")?;
    check_address(&tx.owner)?;
    if let Some(fee) = tx.fee_limit {
        positive_int64(fee, "fee_limit")?;
    }
    let mut value = Vec::new();
    bytes(&mut value, 1, &tx.owner);
    let (kind, url) = match &tx.contract {
        Contract::Transfer { to, amount_sun } => {
            check_address(to)?;
            positive_int64(*amount_sun, "amount")?;
            if tx.fee_limit.is_some() {
                return Err(refused("TransferContract cannot carry fee_limit"));
            }
            bytes(&mut value, 2, to);
            uint(&mut value, 3, *amount_sun);
            (1, TRANSFER_URL)
        }
        Contract::TriggerSmart {
            contract_address,
            data,
        } => {
            check_address(contract_address)?;
            if data.is_empty() || data.len() > MAX_RAW_BYTES {
                return Err(refused(
                    "TriggerSmartContract data must be present and bounded",
                ));
            }
            bytes(&mut value, 2, contract_address);
            bytes(&mut value, 4, data);
            (31, TRIGGER_URL)
        }
    };
    let mut any = Vec::new();
    bytes(&mut any, 1, url);
    bytes(&mut any, 2, &value);
    let mut contract = Vec::new();
    uint(&mut contract, 1, kind);
    bytes(&mut contract, 2, &any);
    let mut raw = Vec::new();
    bytes(&mut raw, 1, &tx.ref_block_bytes);
    bytes(&mut raw, 4, &tx.ref_block_hash);
    uint(&mut raw, 8, tx.expiration);
    bytes(&mut raw, 11, &contract);
    uint(&mut raw, 14, tx.timestamp);
    if let Some(fee) = tx.fee_limit {
        uint(&mut raw, 18, fee);
    }
    if raw.len() > MAX_RAW_BYTES {
        return Err(refused("raw_data exceeds 1 KiB"));
    }
    Ok(raw)
}

/// Decode the whole protobuf: nested Contract, Any and concrete contract included. A returned
/// value re-encodes byte for byte. Unknown fields are never skipped.
pub fn decode_raw(raw: &[u8]) -> Result<DecodedTron> {
    if raw.len() > MAX_RAW_BYTES {
        return Err(refused("raw_data exceeds 1 KiB"));
    }
    let mut r = Reader::new(raw);
    let ref_block_bytes = r.fixed(1)?;
    let ref_block_hash = r.fixed(4)?;
    let expiration = r.uint(8)?;
    let mut c = Reader::new(r.bytes(11)?);
    let kind = c.uint(1)?;
    let expected_url = match kind {
        1 => TRANSFER_URL,
        31 => TRIGGER_URL,
        46 => {
            return Err(refused(
                "AccountPermissionUpdateContract (type 46) hands this account to other keys",
            ));
        }
        other => return Err(refused(format!("unsupported contract type {other}"))),
    };
    let mut any = Reader::new(c.bytes(2)?);
    c.finish()?;
    if any.bytes(1)? != expected_url {
        return Err(refused("type_url does not match contract type"));
    }
    let mut value = Reader::new(any.bytes(2)?);
    any.finish()?;
    let owner = value.fixed(1)?;
    let target = value.fixed(2)?;
    let contract = if kind == 1 {
        Contract::Transfer {
            to: target,
            amount_sun: value.uint(3)?,
        }
    } else {
        Contract::TriggerSmart {
            contract_address: target,
            data: value.bytes(4)?.to_vec(),
        }
    };
    value.finish()?;
    let timestamp = r.uint(14)?;
    let fee_limit = if r.rest.is_empty() {
        None
    } else {
        Some(r.uint(18)?)
    };
    r.finish()?;
    let tx = DecodedTron {
        ref_block_bytes,
        ref_block_hash,
        expiration,
        timestamp,
        owner,
        contract,
        fee_limit,
    };
    if encode_raw(&tx)? != raw {
        return Err(refused("non-canonical raw_data"));
    }
    Ok(tx)
}

/// SHA-256 of exact unsigned bytes; signatures and envelope bytes do not enter the txID.
pub fn txid(raw: &[u8]) -> B256 {
    B256::from_slice(&Sha256::digest(raw))
}

/// Sign validated transaction bytes with no message prefix. The caller must check policy
/// first. This primitive additionally checks owner and emits low-s, v = 0/1.
pub fn sign(signer: &PrivateKeySigner, tx: &DecodedTron) -> Result<[u8; 65]> {
    let raw = encode_raw(tx)?;
    if tx.owner[1..] != signer.address().as_slice()[..] {
        return Err(refused("owner_address is not this signing key"));
    }
    signer
        .sign_hash_sync(&txid(&raw))
        .map(|s| s.normalized_s().as_rsy())
        .map_err(|_| refused("Tron transaction signing failed"))
}

/// Recover public transaction bytes, including historical high-s and 27/28-v signatures.
/// EIP-155 v values are not Tron signatures. Does not apply signing policy: a historical
/// transaction can contain fields this version refuses to sign.
pub fn recover(raw: &[u8], signature: &[u8]) -> Result<[u8; 21]> {
    if signature.len() != 65 || !matches!(signature[64], 0 | 1 | 27 | 28) {
        return Err(WalletError::validation(
            "Tron signature must be 65 bytes with v 0, 1, 27 or 28",
        ));
    }
    let sig = Signature::from_raw(signature)
        .map_err(|_| WalletError::validation("invalid Tron signature"))?;
    let body = sig
        .recover_address_from_prehash(&txid(raw))
        .map_err(|_| WalletError::validation("cannot recover Tron transaction signer"))?;
    let mut owner = [0u8; 21];
    owner[0] = address::PREFIX;
    owner[1..].copy_from_slice(body.as_slice());
    Ok(owner)
}

/// Transaction {raw_data: 1, signature: 2} for broadcast. Preserve the exact signed raw bytes,
/// and verify their signature and owner before wrapping. Network replies never supply this.
pub fn encode_signed(raw: &[u8], signature: &[u8]) -> Result<Vec<u8>> {
    let tx = decode_raw(raw)?;
    if recover(raw, signature)? != tx.owner {
        return Err(WalletError::validation(
            "Tron signature does not match owner_address",
        ));
    }
    let mut envelope = Vec::new();
    bytes(&mut envelope, 1, raw);
    bytes(&mut envelope, 2, signature);
    Ok(envelope)
}

#[cfg(test)]
#[path = "tx_tests.rs"]
mod tests;
