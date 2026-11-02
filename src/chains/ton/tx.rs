//! The exact supported V4R2 simple-send grammar. The signature covers the signing-cell hash;
//! the outer message envelope and StateInit are independently reconstructed, never trusted.
use super::{
    FEE_RESERVE, MAX_LIFETIME, TON_WALLET_ID, address,
    cell::{self, Builder, Cell},
};
use crate::core::error::{Result, WalletError};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde_json::{Value, json};
use std::sync::{Arc, OnceLock};

pub(super) const CODE_HASH: &str =
    "feb5ff6820e2ff0d9483e7e0d62c817d846789fb4ae580c878866d959dabd5c0";
fn bad() -> WalletError {
    WalletError::validation("refusing unsupported TON V4R2 transaction shape")
}
pub(super) fn code() -> Arc<Cell> {
    static CODE: OnceLock<Arc<Cell>> = OnceLock::new();
    CODE.get_or_init(|| {
        cell::decode(include_bytes!("wallet-v4r2.boc")).expect("pinned V4R2 bytecode")
    })
    .clone()
}
pub(super) fn data(public: &[u8; 32], seqno: u32) -> Arc<Cell> {
    let mut b = Builder::default();
    b.uint(seqno as u64, 32)
        .uint(TON_WALLET_ID as u64, 32)
        .bytes(public)
        .uint(0, 1);
    b.end()
}
pub(super) fn state_init(public: &[u8; 32]) -> Arc<Cell> {
    let mut b = Builder::default();
    b.uint(0, 2)
        .uint(1, 1)
        .reference(code())
        .uint(1, 1)
        .reference(data(public, 0))
        .uint(0, 1);
    b.end()
}
pub(super) fn wallet_address(public: &[u8; 32]) -> String {
    address::raw(&state_init(public).hash())
}
pub(super) fn public_data(cell: &Cell) -> Result<([u8; 32], u32)> {
    let mut r = cell.reader();
    let seqno = r.uint(32)? as u32;
    r.expect(32, TON_WALLET_ID as u64)?;
    let key = r.bytes()?;
    r.expect(1, 0)?;
    r.finish()?;
    Ok((key, seqno))
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Intent {
    pub to: [u8; 32],
    pub amount: u64,
    pub seqno: u32,
    pub valid_until: u32,
}
impl Intent {
    pub fn fields(&self) -> Value {
        json!({"to":address::raw(&self.to),"amountRaw":self.amount.to_string(),"seqno":self.seqno,"validUntil":self.valid_until,"walletVersion":"v4r2","walletId":TON_WALLET_ID,"bounce":false,"sendMode":3,"feeGuarantee":"rpc-estimate-and-reserve-only","feeReserveRaw":FEE_RESERVE.to_string()})
    }
    pub fn fresh(&self, now: u64) -> Result<()> {
        if self.valid_until as u64 <= now
            || self.valid_until as u64 > now.saturating_add(MAX_LIFETIME)
        {
            return Err(WalletError::validation(
                "TON validUntil must be in the future and at most 300 seconds from the local clock",
            ));
        }
        Ok(())
    }
}
fn store_addr(b: &mut Builder, addr: &[u8; 32]) {
    b.uint(2, 2).uint(0, 1).uint(0, 8).bytes(addr);
}
fn read_addr(r: &mut super::cell::Reader<'_>) -> Result<[u8; 32]> {
    r.expect(2, 2)?;
    r.expect(1, 0)?;
    r.expect(8, 0)?;
    r.bytes()
}
pub(super) fn unsigned(intent: &Intent) -> Arc<Cell> {
    let mut inner = Builder::default();
    inner.uint(0x10, 6);
    store_addr(&mut inner, &intent.to);
    inner
        .coins(intent.amount)
        .uint(0, 1)
        .coins(0)
        .coins(0)
        .uint(0, 64)
        .uint(0, 32)
        .uint(0, 1)
        .uint(0, 1);
    let mut b = Builder::default();
    b.uint(TON_WALLET_ID as u64, 32)
        .uint(intent.valid_until as u64, 32)
        .uint(intent.seqno as u64, 32)
        .uint(0, 8)
        .uint(3, 8)
        .reference(inner.end());
    b.end()
}
pub(super) fn decode_unsigned(cell: &Cell) -> Result<Intent> {
    let mut r = cell.reader();
    r.expect(32, TON_WALLET_ID as u64)?;
    let valid_until = r.uint(32)? as u32;
    let seqno = r.uint(32)? as u32;
    r.expect(8, 0)?;
    r.expect(8, 3)?;
    let message = r.reference()?;
    r.finish()?;
    let mut m = message.reader();
    m.expect(6, 0x10)?;
    let to = read_addr(&mut m)?;
    let amount = m.coins()?;
    m.expect(1, 0)?;
    if m.coins()? != 0 || m.coins()? != 0 {
        return Err(bad());
    }
    m.expect(64, 0)?;
    m.expect(32, 0)?;
    m.expect(1, 0)?;
    m.expect(1, 0)?;
    m.finish()?;
    if amount == 0 || to == [0; 32] || seqno == u32::MAX {
        return Err(bad());
    }
    Ok(Intent {
        to,
        amount,
        seqno,
        valid_until,
    })
}
pub(super) fn from_value(v: &Value) -> Result<(Arc<Cell>, Intent)> {
    let s = v
        .as_str()
        .and_then(|s| s.strip_prefix("0x"))
        .filter(|s| s.len() <= 1024)
        .ok_or_else(bad)?;
    let bytes = alloy::hex::decode(s).map_err(|_| bad())?;
    let c = cell::decode(&bytes)?;
    let intent = decode_unsigned(&c)?;
    if cell::encode(&c) != bytes {
        return Err(WalletError::validation(
            "TON unsigned BOC must use zwallet's canonical encoding",
        ));
    }
    Ok((c, intent))
}
pub(super) fn envelope(public: &[u8; 32], body: Arc<Cell>, init: bool) -> Arc<Cell> {
    let from = state_init(public).hash();
    let mut b = Builder::default();
    b.uint(2, 2).uint(0, 2);
    store_addr(&mut b, &from);
    b.coins(0);
    if init {
        b.uint(1, 1).uint(1, 1).reference(state_init(public));
    } else {
        b.uint(0, 1);
    }
    b.uint(1, 1).reference(body);
    b.end()
}
pub(super) fn signed_body(unsigned: &Arc<Cell>, signature: &[u8; 64]) -> Arc<Cell> {
    let mut b = Builder::default();
    b.bytes(signature).append(unsigned);
    b.end()
}
pub(super) fn sign(key: &SigningKey, unsigned: &Arc<Cell>) -> Arc<Cell> {
    let signature = key.sign(&unsigned.hash()).to_bytes();
    envelope(
        &key.verifying_key().to_bytes(),
        signed_body(unsigned, &signature),
        true,
    )
}
pub(super) struct Inspected {
    pub public: [u8; 32],
    pub unsigned: Arc<Cell>,
    pub body: Arc<Cell>,
    pub intent: Intent,
}
pub(super) fn inspect_body(body: &Cell) -> Result<(Arc<Cell>, Intent, [u8; 64])> {
    let mut r = body.reader();
    let signature = r.bytes::<64>()?;
    let mut b = Builder::default();
    b.uint(r.uint(32)?, 32)
        .uint(r.uint(32)?, 32)
        .uint(r.uint(32)?, 32)
        .uint(r.uint(8)?, 8)
        .uint(r.uint(8)?, 8)
        .reference(r.reference()?);
    r.finish()?;
    let unsigned = b.end();
    let intent = decode_unsigned(&unsigned)?;
    Ok((unsigned, intent, signature))
}
pub(super) fn inspect(external: &Arc<Cell>) -> Result<Inspected> {
    let mut e = external.reader();
    e.expect(2, 2)?;
    e.expect(2, 0)?;
    let sender = read_addr(&mut e)?;
    if e.coins()? != 0 {
        return Err(bad());
    }
    e.expect(1, 1)?;
    e.expect(1, 1)?;
    let init = e.reference()?;
    e.expect(1, 1)?;
    let body = e.reference()?;
    e.finish()?;
    let mut s = init.reader();
    s.expect(2, 0)?;
    s.expect(1, 1)?;
    let c = s.reference()?;
    s.expect(1, 1)?;
    let d = s.reference()?;
    s.expect(1, 0)?;
    s.finish()?;
    if alloy::hex::encode(c.hash()) != CODE_HASH {
        return Err(bad());
    }
    let (public, seqno) = public_data(&d)?;
    if seqno != 0 || *init != *state_init(&public) || sender != init.hash() {
        return Err(bad());
    }
    let (unsigned, intent, signature) = inspect_body(&body)?;
    VerifyingKey::from_bytes(&public)
        .map_err(|_| bad())?
        .verify_strict(&unsigned.hash(), &Signature::from_bytes(&signature))
        .map_err(|_| WalletError::validation("TON signature does not verify"))?;
    if **external != *envelope(&public, body.clone(), true) {
        return Err(bad());
    }
    Ok(Inspected {
        public,
        unsigned,
        body,
        intent,
    })
}
