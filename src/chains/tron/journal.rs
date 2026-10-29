//! Shape local observations without granting signing authority or trusting stored symbols.
use super::{address, lookup, registry, send};
use crate::chains::{Chain, Family, JournalRow};
use crate::core::amount::{format_units, parse_units};
use alloy::primitives::U256;
use serde_json::Value;

pub(crate) fn row(chain: &'static Chain, owner: &str, entry: &Value) -> Option<JournalRow> {
    if !matches!(chain.family, Family::Tron(_)) || entry["chain"] != chain.key {
        return None;
    }
    address::parse(owner).ok()?;
    let from = entry["from"].as_str()?;
    let to = entry["to"].as_str()?;
    if from != owner || entry.get("nonce").is_some_and(|v| !v.is_null()) {
        return None;
    }
    address::parse(from).ok()?;
    address::parse(to).ok()?;
    let raw_hash = entry["txHash"].as_str()?;
    if raw_hash.len() != 64 {
        return None;
    }
    let tx_hash = lookup::parse_hash(raw_hash).ok()?;
    let (kind, token) = match entry["kind"].as_str()? {
        "transfer" if entry.get("token").is_none_or(Value::is_null) => ("transfer", None),
        "token-transfer" => (
            "token-transfer",
            Some(registry::lookup(
                &address::parse(entry["token"].as_str()?).ok()?,
            )?),
        ),
        _ => return None,
    };
    let (symbol, decimals) =
        token.map_or((chain.symbol, chain.decimals), |t| (t.symbol, t.decimals));
    let amount = match entry.get("amountRaw") {
        Some(Value::String(raw))
            if !raw.is_empty() && raw.len() <= 78 && raw.bytes().all(|b| b.is_ascii_digit()) =>
        {
            U256::from_str_radix(raw, 10).ok()?
        }
        None | Some(Value::Null) => {
            let (number, old_symbol) = entry["value"].as_str()?.split_once(' ')?;
            if old_symbol != symbol {
                return None;
            }
            parse_units(number, decimals).ok()?
        }
        _ => return None,
    };
    if token.is_none() && amount > U256::from(i64::MAX) {
        return None;
    }
    let expiration_ms = match entry.get("expiresAt") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => Some(send::parse_expiration(value)?),
        _ => return None,
    };
    Some(JournalRow {
        chain: chain.key,
        tx_hash,
        from: from.into(),
        to: to.into(),
        nonce: None,
        expiration_ms,
        timestamp: crate::core::time::parse_rfc3339_utc(entry["ts"].as_str()?)?,
        kind,
        amount_raw: Some(amount),
        value: format!("{} {symbol}", format_units(amount, decimals)),
        token: token.map(|t| t.address.into()),
    })
}
