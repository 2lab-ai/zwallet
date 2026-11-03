//! The deliberately small signing language: one legacy SystemProgram SOL transfer.
//! No versioned messages, address tables, auxiliary instructions, or durable nonces.
use crate::chains::SignedTx;
use crate::core::error::{Result, WalletError};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde_json::{Value, json};

pub const MESSAGE_BYTES: usize = 150;
pub const WIRE_BYTES: usize = 215;
pub const BASE_FEE: u64 = 5_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transfer {
    pub from: [u8; 32],
    pub to: [u8; 32],
    pub blockhash: [u8; 32],
    pub lamports: u64,
}
impl Transfer {
    pub fn view(&self) -> Value {
        json!({"from":bs58::encode(self.from).into_string(),"to":bs58::encode(self.to).into_string(),
            "amountRaw":self.lamports.to_string(),"feeRaw":BASE_FEE.to_string(),
            "recentBlockhash":bs58::encode(self.blockhash).into_string(),"kind":"transfer"})
    }
}
fn invalid() -> WalletError {
    WalletError::validation(
        "Solana signing supports only a canonical legacy, single-signature SystemProgram native transfer",
    )
}

/// All vector lengths here fit one shortvec byte. Exact bytes reject overlong lengths too.
pub fn encode(t: &Transfer) -> Result<Vec<u8>> {
    if t.from == t.to
        || t.from == [0; 32]
        || t.to == [0; 32]
        || t.blockhash == [0; 32]
        || t.lamports == 0
    {
        return Err(invalid());
    }
    let mut b = Vec::with_capacity(MESSAGE_BYTES);
    b.extend_from_slice(&[1, 0, 1, 3]);
    b.extend_from_slice(&t.from);
    b.extend_from_slice(&t.to);
    b.extend_from_slice(&[0; 32]); // SystemProgram; the only readonly unsigned account.
    b.extend_from_slice(&t.blockhash);
    b.extend_from_slice(&[1, 2, 2, 0, 1, 12]); // one instruction, program #2, accounts #0/#1, 12 bytes.
    b.extend_from_slice(&2u32.to_le_bytes()); // SystemInstruction::Transfer.
    b.extend_from_slice(&t.lamports.to_le_bytes());
    Ok(b)
}

pub fn decode(bytes: &[u8]) -> Result<Transfer> {
    if bytes.len() != MESSAGE_BYTES
        || bytes[..4] != [1, 0, 1, 3]
        || bytes[68..100] != [0; 32]
        || bytes[132..138] != [1, 2, 2, 0, 1, 12]
        || bytes[138..142] != 2u32.to_le_bytes()
    {
        return Err(invalid());
    }
    let t = Transfer {
        from: bytes[4..36].try_into().map_err(|_| invalid())?,
        to: bytes[36..68].try_into().map_err(|_| invalid())?,
        blockhash: bytes[100..132].try_into().map_err(|_| invalid())?,
        lamports: u64::from_le_bytes(bytes[142..150].try_into().map_err(|_| invalid())?),
    };
    if encode(&t)? != bytes {
        return Err(invalid());
    }
    Ok(t)
}

pub fn unsigned_bytes(value: &Value) -> Result<Vec<u8>> {
    let s = value
        .as_str()
        .and_then(|s| s.strip_prefix("0x"))
        .filter(|s| s.len() == MESSAGE_BYTES * 2)
        .ok_or_else(invalid)?;
    let b = alloy::hex::decode(s).map_err(|_| invalid())?;
    decode(&b)?;
    Ok(b)
}
pub fn sign(bytes: &[u8], key: &SigningKey) -> Result<Value> {
    let t = decode(bytes)?;
    if t.from != key.verifying_key().to_bytes() {
        return Err(WalletError::validation(
            "Solana transfer sender is not this key",
        ));
    }
    let sig = key.sign(bytes);
    let mut wire = Vec::with_capacity(WIRE_BYTES);
    wire.push(1);
    wire.extend_from_slice(&sig.to_bytes());
    wire.extend_from_slice(bytes);
    Ok(
        json!({"rawTx":STANDARD.encode(wire),"txHash":bs58::encode(sig.to_bytes()).into_string(),"decoded":t.view()}),
    )
}
pub fn inspect(value: &Value) -> Result<SignedTx> {
    let raw = value
        .get("rawTx")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    let (signed, _) = inspect_raw(raw)?;
    // Cross-check optional advertised identity instead of trusting it.
    if value
        .get("txHash")
        .is_some_and(|h| h.as_str() != Some(signed.tx_hash.as_str()))
    {
        return Err(WalletError::validation(
            "Solana signer returned a mismatched transaction id",
        ));
    }
    Ok(signed)
}
pub fn inspect_raw(raw: &str) -> Result<(SignedTx, Transfer)> {
    if raw.len() != WIRE_BYTES.div_ceil(3) * 4 {
        return Err(invalid());
    }
    let wire = STANDARD.decode(raw).map_err(|_| invalid())?;
    if wire.len() != WIRE_BYTES || wire[0] != 1 || STANDARD.encode(&wire) != raw {
        return Err(invalid());
    }
    let t = decode(&wire[65..])?;
    let signature = Signature::from_slice(&wire[1..65]).map_err(|_| invalid())?;
    let key = VerifyingKey::from_bytes(&t.from).map_err(|_| invalid())?;
    key.verify_strict(&wire[65..], &signature)
        .map_err(|_| WalletError::validation("Solana transaction signature does not verify"))?;
    Ok((
        SignedTx {
            raw: raw.into(),
            tx_hash: bs58::encode(signature.to_bytes()).into_string(),
            signer: bs58::encode(t.from).into_string(),
            nonce: None,
        },
        t,
    ))
}
pub fn verify_signed_payload(signed: &SignedTx, unsigned: &Value) -> Result<()> {
    let (verified, _) = inspect_raw(&signed.raw)?;
    let wire = STANDARD.decode(&signed.raw).map_err(|_| invalid())?;
    if verified != *signed || wire[65..] != unsigned_bytes(unsigned)? {
        return Err(WalletError::validation(
            "Solana signature does not match the reviewed message",
        ));
    }
    Ok(())
}
pub fn simulation_wire(unsigned: &[u8]) -> Result<String> {
    decode(unsigned)?;
    let mut wire = vec![0; 65];
    wire[0] = 1;
    wire.extend_from_slice(unsigned);
    Ok(STANDARD.encode(wire))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> Transfer {
        Transfer {
            from: SigningKey::from_bytes(&[7; 32]).verifying_key().to_bytes(),
            to: [8; 32],
            blockhash: [9; 32],
            lamports: 1_000_000,
        }
    }
    #[test]
    fn strict_codec_rejects_every_non_transfer_layout() {
        let b = encode(&sample()).unwrap();
        assert_eq!(b.len(), 150);
        assert_eq!(decode(&b).unwrap(), sample());
        for (i, n) in [
            (0, 0x80),
            (1, 1),
            (2, 0),
            (3, 4),
            (68, 1),
            (132, 2),
            (133, 1),
            (134, 3),
            (135, 1),
            (136, 0),
            (137, 13),
            (138, 3),
        ] {
            let mut changed = b.clone();
            changed[i] = n;
            assert!(decode(&changed).is_err(), "offset {i}");
        }
        for size in 0..150 {
            assert!(decode(&b[..size]).is_err());
        }
        let mut extra = b.clone();
        extra.push(0);
        assert!(decode(&extra).is_err());
        let mut overlong = b.clone();
        overlong.splice(3..4, [0x83, 0]);
        assert!(decode(&overlong).is_err());
        for t in [
            Transfer {
                lamports: 0,
                ..sample()
            },
            Transfer {
                to: sample().from,
                ..sample()
            },
            Transfer {
                to: [0; 32],
                ..sample()
            },
            Transfer {
                blockhash: [0; 32],
                ..sample()
            },
        ] {
            assert!(encode(&t).is_err());
        }
    }
    #[test]
    fn signed_bytes_prove_identity_and_reviewed_payload() {
        let b = encode(&sample()).unwrap();
        let unsigned = json!(format!("0x{}", alloy::hex::encode(&b)));
        let value = sign(&b, &SigningKey::from_bytes(&[7; 32])).unwrap();
        let s = inspect(&value).unwrap();
        verify_signed_payload(&s, &unsigned).unwrap();
        let mut wire = STANDARD.decode(&s.raw).unwrap();
        wire[70] ^= 1;
        assert!(inspect_raw(&STANDARD.encode(wire)).is_err());
        let mut other = sample();
        other.lamports += 1;
        assert!(
            verify_signed_payload(
                &s,
                &json!(format!("0x{}", alloy::hex::encode(encode(&other).unwrap())))
            )
            .is_err()
        );
        let mut lie = value;
        lie["txHash"] = json!("wrong");
        assert!(inspect(&lie).is_err());
        assert!(sign(&b, &SigningKey::from_bytes(&[8; 32])).is_err());
    }
}
