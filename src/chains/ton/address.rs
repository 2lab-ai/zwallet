use crate::core::error::{Result, WalletError};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
pub(super) fn parse(input: &str) -> Result<[u8; 32]> {
    let input = input.trim();
    let bad = || {
        WalletError::invalid_address(
            "TON address must be a mainnet workchain-0 raw or CRC-checked friendly address",
        )
    };
    if let Some(h) = input.strip_prefix("0:") {
        return alloy::hex::decode(h)
            .ok()
            .and_then(|b| b.try_into().ok())
            .filter(|_| h.len() == 64)
            .ok_or_else(bad);
    }
    if input.len() != 48 {
        return Err(bad());
    }
    let b = URL_SAFE_NO_PAD
        .decode(input)
        .or_else(|_| STANDARD.decode(input))
        .map_err(|_| bad())?;
    if b.len() != 36 || !matches!(b[0], 0x11 | 0x51) || b[1] != 0 || b[34..] != crc16(&b[..34]) {
        return Err(bad());
    }
    b[2..34].try_into().map_err(|_| bad())
}
pub(super) fn raw(hash: &[u8; 32]) -> String {
    format!("0:{}", alloy::hex::encode(hash))
}
fn crc16(bytes: &[u8]) -> [u8; 2] {
    let mut c = 0u16;
    for b in bytes {
        c ^= (*b as u16) << 8;
        for _ in 0..8 {
            c = (c << 1) ^ if c & 0x8000 != 0 { 0x1021 } else { 0 };
        }
    }
    c.to_be_bytes()
}
#[cfg(test)]
pub(super) fn friendly(hash: &[u8; 32], tag: u8) -> String {
    let mut b = vec![tag, 0];
    b.extend(hash);
    b.extend(crc16(&b));
    URL_SAFE_NO_PAD.encode(b)
}
