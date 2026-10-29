//! Read-only Tron observations. The C lookup verifies raw-data hashes and network genesis;
//! this adapter keeps its uncertainty and registry semantics intact for the common detail UI.
use super::State;
use crate::chains::{Family, JournalRow, Net, tron};
use crate::core::{
    amount::format_units,
    error::{Result, WalletError},
    text::ascii_inert,
};
use alloy::primitives::U256;
use serde_json::{Map, Value};

#[derive(Clone, Debug)]
pub struct AsOf {
    pub block: u64,
    pub block_time_ms: u64,
    pub host: String,
}
#[derive(Clone, Debug)]
pub struct Detail {
    pub chain: &'static str,
    pub tx_hash: String,
    pub state: State,
    pub success: Option<bool>,
    pub from: Option<String>,
    pub to: Option<String>,
    /// A decoded transfer intent, not proof a token moved. Units come from the registry.
    pub value: Option<String>,
    pub token: Option<&'static str>,
    pub fee_sun: Option<u64>,
    pub block: Option<u64>,
    pub expiration_ms: Option<u64>,
    pub energy_used: Option<u64>,
    pub net_usage: Option<u64>,
    pub as_of: Option<AsOf>,
    pub node_behind_secs: Option<u64>,
    pub hint: String,
}

pub async fn lookup(
    net: &Net,
    hash: &str,
    owner: &str,
    journal: Option<&JournalRow>,
) -> Result<Detail> {
    if !matches!(net.chain.family, Family::Tron(_)) {
        return Err(WalletError::validation(
            "Tron history requires a Tron network",
        ));
    }
    tron::address::parse(owner)?;
    let hash = tron::lookup::parse_hash(hash)?;
    let expiration = journal
        .filter(|row| {
            row.chain == net.chain.key
                && row.from == owner
                && row.tx_hash == hash
                && row.nonce.is_none()
        })
        .and_then(|row| row.expiration_ms);
    let observed = tron::lookup::lookup_tx(net, &hash, expiration).await?;
    shape(net, &hash, &observed)
}
fn malformed() -> WalletError {
    WalletError::rpc("Tron lookup returned inconsistent detail fields")
}
fn number(value: Option<&Value>) -> Result<Option<u64>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .filter(|n| *n <= i64::MAX as u64)
            .map(Some)
            .ok_or_else(malformed),
    }
}
fn timestamp(value: Option<&Value>) -> Result<Option<u64>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => tron::send::parse_expiration(s)
            .map(Some)
            .ok_or_else(malformed),
        _ => Err(malformed()),
    }
}
fn address(value: Option<&Value>) -> Result<Option<String>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => {
            tron::address::parse(s).map_err(|_| malformed())?;
            Ok(Some(s.clone()))
        }
        _ => Err(malformed()),
    }
}
fn shape(net: &Net, hash: &str, value: &Map<String, Value>) -> Result<Detail> {
    if value.get("chain").and_then(Value::as_str) != Some(net.chain.key)
        || value.get("txHash").and_then(Value::as_str) != Some(hash)
    {
        return Err(malformed());
    }
    let state = match (
        value.get("status").and_then(Value::as_str),
        value.get("found").and_then(Value::as_bool),
    ) {
        (Some("mined"), Some(true)) => State::Mined,
        (Some("pending"), Some(true)) => State::Pending,
        (Some("not found"), Some(false)) => State::NotFound,
        (Some("expired-unseen"), Some(false)) => State::ExpiredUnseen,
        (Some("unknown"), None) => State::Unknown,
        _ => return Err(malformed()),
    };
    let block = number(value.get("blockNumber"))?;
    if (state == State::Mined) != block.is_some() {
        return Err(malformed());
    }
    let success = match value.get("success") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(b)) if state == State::Mined => Some(*b),
        _ => return Err(malformed()),
    };
    let from = address(value.get("from"))?;
    let to = address(value.get("to"))?;
    let token = match value.get("token") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(
            tron::registry::lookup(&tron::address::parse(s).map_err(|_| malformed())?)
                .ok_or_else(malformed)?,
        ),
        _ => return Err(malformed()),
    };
    let amount = match value.get("valueRaw") {
        None | Some(Value::Null) if token.is_none() => None,
        Some(Value::String(s))
            if !s.is_empty()
                && s.len() <= 78
                && s.bytes().all(|b| b.is_ascii_digit())
                && from.is_some()
                && to.is_some() =>
        {
            let amount = U256::from_str_radix(s, 10).map_err(|_| malformed())?;
            if token.is_none() && amount > U256::from(i64::MAX) {
                return Err(malformed());
            }
            let (symbol, decimals) = token.map_or((net.chain.symbol, net.chain.decimals), |t| {
                (t.symbol, t.decimals)
            });
            Some(format!("{} {symbol}", format_units(amount, decimals)))
        }
        _ => return Err(malformed()),
    };
    let fee_sun = match value.get("feeSun") {
        None | Some(Value::Null) => None,
        Some(Value::String(s))
            if !s.is_empty() && s.len() <= 19 && s.bytes().all(|b| b.is_ascii_digit()) =>
        {
            Some(
                s.parse::<u64>()
                    .ok()
                    .filter(|n| *n <= i64::MAX as u64)
                    .ok_or_else(malformed)?,
            )
        }
        _ => return Err(malformed()),
    };
    let expiration_ms = timestamp(value.get("expiresAt"))?;
    let as_of = if state == State::ExpiredUnseen {
        let at = value
            .get("asOf")
            .and_then(Value::as_object)
            .ok_or_else(malformed)?;
        let block_time_ms = timestamp(at.get("blockTime"))?.ok_or_else(malformed)?;
        let expiration = expiration_ms.ok_or_else(malformed)?;
        let host = crate::chains::url_origin(&net.rpc_url);
        if block_time_ms <= expiration.saturating_add(60_000)
            || at.get("host").and_then(Value::as_str) != Some(&host)
        {
            return Err(malformed());
        }
        Some(AsOf {
            block: number(at.get("block"))?.ok_or_else(malformed)?,
            block_time_ms,
            host,
        })
    } else {
        None
    };
    let hint = value
        .get("hint")
        .and_then(Value::as_str)
        .map(|s| ascii_inert(s, 512))
        .unwrap_or_default();
    Ok(Detail {
        chain: net.chain.key,
        tx_hash: hash.into(),
        state,
        success,
        from,
        to,
        value: amount,
        token: token.map(|t| t.address),
        fee_sun,
        block,
        expiration_ms,
        energy_used: number(value.get("energyUsed"))?,
        net_usage: number(value.get("netUsage"))?,
        as_of,
        node_behind_secs: number(value.get("nodeBehindSecs"))?,
        hint,
    })
}
