//! Reconstruct a node's pending transaction before treating it as replacement authority.
use super::*;

/// The node's `from` is only a consistency check. The encoded hash and recovered signature
/// establish the original, and every required field must be present.
pub fn decode_pending_original(
    value: &Value,
    chain_id: u64,
    hash: B256,
    from: Address,
    nonce: u64,
) -> Result<DecodedUnsigned> {
    let bad = || {
        WalletError::validation(
            "the node's original transaction is missing, inconsistent or unsupported; nothing was signed",
        )
    };
    let number = |key: &str| -> Result<U256> {
        let text = value.get(key).and_then(Value::as_str).ok_or_else(&bad)?;
        if !(3..=66).contains(&text.len())
            || !text.starts_with("0x")
            || !text[2..].bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(bad());
        }
        U256::from_str_radix(&text[2..], 16).map_err(|_| bad())
    };
    let small = |key: &str| u64::try_from(number(key)?).map_err(|_| bad());
    let price = |key: &str| u128::try_from(number(key)?).map_err(|_| bad());
    let address = |key: &str| -> Result<Address> {
        let text = value.get(key).and_then(Value::as_str).ok_or_else(&bad)?;
        if text.len() != 42 || !text.starts_with("0x") {
            return Err(bad());
        }
        text.parse().map_err(|_| bad())
    };
    if value.get("blockNumber") != Some(&Value::Null)
        || value.get("blockHash") != Some(&Value::Null)
        || value
            .get("hash")
            .and_then(Value::as_str)
            .and_then(|h| h.parse::<B256>().ok())
            != Some(hash)
        || small("chainId")? != chain_id
        || small("nonce")? != nonce
        || address("from")? != from
    {
        return Err(bad());
    }
    let input = value
        .get("input")
        .and_then(Value::as_str)
        .ok_or_else(&bad)?;
    // No unbounded calldata allocation or reflection of node text in an error.
    if input.len() > 262_146 || !input.starts_with("0x") || input.len() % 2 != 0 {
        return Err(bad());
    }
    let input = Bytes::from(hex::decode(&input[2..]).map_err(|_| bad())?);
    let tx_type = small("type")?;
    let parity = if tx_type == 0 {
        let base = chain_id
            .checked_mul(2)
            .and_then(|n| n.checked_add(35))
            .ok_or_else(&bad)?;
        small("v")?
            .checked_sub(base)
            .filter(|p| *p <= 1)
            .ok_or_else(&bad)?
    } else if tx_type == 2 {
        if value
            .get("accessList")
            .and_then(Value::as_array)
            .is_none_or(|a| !a.is_empty())
        {
            return Err(bad());
        }
        let parity = if value.get("yParity").is_some() {
            small("yParity")?
        } else {
            small("v")?
        };
        if parity > 1 || (value.get("v").is_some() && small("v")? != parity) {
            return Err(bad());
        }
        parity
    } else {
        return Err(bad());
    };
    let signature = alloy::primitives::Signature::new(number("r")?, number("s")?, parity == 1);
    if signature.normalize_s().is_some() {
        return Err(bad());
    }
    let unsigned = UnsignedEvmTx {
        chain_id,
        nonce,
        gas_limit: small("gas")?,
        to: address("to")?,
        value: number("value")?,
        input,
        fees: if tx_type == 0 {
            EvmFees::Legacy {
                gas_price: U128::from(price("gasPrice")?),
            }
        } else {
            EvmFees::Eip1559 {
                max_fee_per_gas: U128::from(price("maxFeePerGas")?),
                max_priority_fee_per_gas: U128::from(price("maxPriorityFeePerGas")?),
            }
        },
    };
    let consensus = to_consensus(&unsigned);
    let decoded = decode_unsigned(&consensus.encoded_for_signing()).map_err(|_| bad())?;
    let envelope: TxEnvelope = match consensus {
        ConsensusTx::Legacy(tx) => tx.into_signed(signature).into(),
        ConsensusTx::Eip1559(tx) => tx.into_signed(signature).into(),
    };
    let encoded = envelope.encoded_2718();
    if alloy::primitives::keccak256(&encoded) != hash
        || envelope.recover_signer().map_err(|_| bad())? != from
    {
        return Err(WalletError::validation(
            "the node's original transaction hash or recovered signer disagrees; nothing was signed",
        ));
    }
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::consensus::SignableTransaction;
    use alloy::primitives::U128;

    fn fixture(legacy: bool, key: u8) -> (Value, B256, Address) {
        let signer: PrivateKeySigner = format!("{key:064x}").parse().unwrap();
        let unsigned = UnsignedEvmTx {
            chain_id: 1,
            nonce: 7,
            gas_limit: 25_200,
            fees: if legacy {
                EvmFees::Legacy {
                    gas_price: U128::from(2_000_000_000u128),
                }
            } else {
                EvmFees::Eip1559 {
                    max_fee_per_gas: U128::from(2_000_000_000u128),
                    max_priority_fee_per_gas: U128::from(100_000_000u128),
                }
            },
            to: Address::repeat_byte(3),
            value: U256::from(123),
            input: Bytes::new(),
        };
        let decoded = decode_unsigned(&encoded_for_signing(&unsigned)).unwrap();
        let signed = sign_decoded(&signer, &decoded).unwrap();
        let envelope = TxEnvelope::decode_2718(&mut signed.raw.as_ref()).unwrap();
        let mut value = serde_json::to_value(&envelope).unwrap();
        value["hash"] = json!(format!("{:#x}", signed.hash));
        value["from"] = json!(signer.address().to_checksum(None));
        value["blockNumber"] = Value::Null;
        value["blockHash"] = Value::Null;
        // Consensus serialization supplies legacy v and typed yParity, but not node metadata.
        match &envelope {
            TxEnvelope::Legacy(s) => {
                value["chainId"] = json!(format!("{:#x}", s.tx().chain_id.unwrap()))
            }
            TxEnvelope::Eip1559(s) => {
                let _ = s.tx().signature_hash();
            }
            _ => unreachable!(),
        }
        (value, signed.hash, signer.address())
    }

    #[test]
    fn t27_reconstructs_only_supported_signed_pending_originals() {
        for legacy in [false, true] {
            let (value, hash, from) = fixture(legacy, 1);
            let original = decode_pending_original(&value, 1, hash, from, 7).unwrap();
            assert_eq!(original.nonce(), 7);
            assert_eq!(original.to(), Address::repeat_byte(3));
            assert_eq!(original.value(), U256::from(123));
        }
    }

    #[test]
    fn t27_forged_sender_changed_bytes_wrong_scope_or_missing_fields_are_refused() {
        let (valid, hash, from) = fixture(false, 1);
        let (mut other, other_hash, _) = fixture(false, 2);
        other["from"] = json!(from.to_checksum(None));
        assert!(decode_pending_original(&other, 1, other_hash, from, 7).is_err());
        for (field, replacement) in [
            ("value", json!("0x7c")),
            ("nonce", json!("0x8")),
            ("chainId", json!("0x2")),
            ("blockNumber", json!("0x1")),
            ("from", json!(Address::repeat_byte(4).to_checksum(None))),
            ("type", json!("0x1")),
            ("accessList", json!([{"address":from,"storageKeys":[]}])),
        ] {
            let mut changed = valid.clone();
            changed[field] = replacement;
            assert!(
                decode_pending_original(&changed, 1, hash, from, 7).is_err(),
                "{field}"
            );
        }
        for field in [
            "type",
            "chainId",
            "nonce",
            "gas",
            "to",
            "value",
            "input",
            "accessList",
            "r",
            "s",
            "blockNumber",
        ] {
            let mut missing = valid.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(
                decode_pending_original(&missing, 1, hash, from, 7).is_err(),
                "{field}"
            );
        }
    }
}
