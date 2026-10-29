//! Broadcast classification and transaction observations. Absence at one node is never evidence
//! of non-payment. Only a fresh node head plus a journal expiration permits expired-unseen.
use super::{
    address, registry, rpc, send,
    tx::{self, Contract, DecodedTron},
};
use crate::chains::{Broadcast, Net, SendRecord, SignedTx, before_send_error, url_origin};
use crate::core::{
    amount::format_units,
    error::{ErrorCode, Result, WalletError},
    text::{PROSE_MAX, ascii_inert},
};
use alloy::{hex, primitives::U256};
use serde_json::{Map, Value, json};
use std::time::Duration;

pub const SECOND_PAYMENT: &str =
    "Sending this payment again is a second payment if this one was included";
pub fn parse_hash(input: &str) -> Result<String> {
    let s = input.trim().strip_prefix("0x").unwrap_or(input.trim());
    if s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(WalletError::validation(
            "a Tron txID is exactly 64 hexadecimal digits, optionally prefixed with 0x",
        ));
    }
    Ok(s.to_ascii_lowercase())
}

fn checked_transaction(v: &Map<String, Value>, hash: &str) -> Result<Option<DecodedTron>> {
    if v.is_empty() {
        return Ok(None);
    }
    let raw = v
        .get("raw_data_hex")
        .and_then(Value::as_str)
        .filter(|s| s.len() <= 2 * (1 << 20))
        .and_then(|s| hex::decode(s).ok())
        .ok_or_else(|| WalletError::rpc("lookup returned no raw_data_hex to verify"))?;
    if hex::encode(tx::txid(&raw)) != hash || v.get("txID").is_some_and(|id| id != hash) {
        return Err(WalletError::rpc(
            "lookup returned raw_data or txID for another transaction",
        ));
    }
    // Historical transactions can have memo or permission fields this signer deliberately
    // refuses. Their hash is still checked; do not give unsupported bytes transfer semantics.
    Ok(tx::decode_raw(&raw).ok())
}
fn details(out: &mut Map<String, Value>, decoded: &DecodedTron) {
    out.insert("from".into(), json!(address::encode(&decoded.owner)));
    out.insert(
        "expiresAt".into(),
        json!(send::expires_at(decoded.expiration)),
    );
    let (to, amount, symbol, token) = match &decoded.contract {
        Contract::Transfer { to, amount_sun } => (*to, U256::from(*amount_sun), "TRX", None),
        Contract::TriggerSmart {
            contract_address, ..
        } => {
            let Some((to, amount)) = decoded.transfer_call() else {
                return;
            };
            let Some(token) = registry::lookup(contract_address) else {
                return;
            };
            (to, amount, token.symbol, Some(token.address))
        }
    };
    out.insert("to".into(), json!(address::encode(&to)));
    out.insert("value".into(), json!(format_units(amount, 6)));
    out.insert("valueRaw".into(), json!(amount.to_string()));
    out.insert("symbol".into(), json!(symbol));
    if let Some(token) = token {
        out.insert("token".into(), json!(token));
    }
}

pub(super) async fn observe(
    c: &rpc::TronClient,
    net: &Net,
    hash: &str,
    expiration: Option<u64>,
) -> Result<Map<String, Value>> {
    let body = json!({"value": hash});
    let info = c.post("gettransactioninfobyid", &body).await?;
    let transaction = c.post("gettransactionbyid", &body).await?;
    let mut decoded = checked_transaction(&transaction, hash)?;
    let mut found = !transaction.is_empty();
    let mined = info.get("blockNumber").is_some();
    if !info.is_empty() && info.get("id").and_then(Value::as_str) != Some(hash) {
        return Err(WalletError::rpc(
            "receipt lookup answered about another transaction or omitted its id",
        ));
    }
    if mined && !found {
        return Err(WalletError::rpc(
            "the node returned a receipt without the transaction's raw_data to verify",
        ));
    }
    if !found {
        let pending = c.post("gettransactionfrompending", &body).await?;
        decoded = checked_transaction(&pending, hash)?;
        found = !pending.is_empty();
    }
    let mut out = json!({"chain": net.chain.key, "txHash": hash, "found": found,
        "status": if mined {"mined"} else if found {"pending"} else {"not found"},
        "explorerUrl": net.chain.explorer_tx_url(hash)})
    .as_object()
    .unwrap()
    .clone();
    if let Some(expiration) = expiration {
        out.insert("expiresAt".into(), json!(send::expires_at(expiration)));
    }
    if let Some(decoded) = &decoded {
        details(&mut out, decoded);
    }
    if mined {
        let block = send::number(info.get("blockNumber"), "receipt blockNumber", None)?;
        let receipt = info.get("receipt");
        let result = receipt.and_then(|r| r.get("result"));
        let native = decoded
            .as_ref()
            .is_some_and(|d| matches!(d.contract, Contract::Transfer { .. }));
        let status = if native || (decoded.is_none() && result.is_none()) {
            transaction
                .get("ret")
                .and_then(|r| r.get(0))
                .and_then(|r| r.get("contractRet"))
        } else {
            result
        };
        // Pinned protocol/Tron.proto Result.contractResult. Missing/default/future values
        // carry no evidence of success or failure; an included transaction may have paid.
        let success = match status.and_then(Value::as_str) {
            Some("SUCCESS") => Some(true),
            Some(
                "REVERT"
                | "BAD_JUMP_DESTINATION"
                | "OUT_OF_MEMORY"
                | "PRECOMPILED_CONTRACT"
                | "STACK_TOO_SMALL"
                | "STACK_TOO_LARGE"
                | "ILLEGAL_OPERATION"
                | "STACK_OVERFLOW"
                | "OUT_OF_ENERGY"
                | "OUT_OF_TIME"
                | "JVM_STACK_OVER_FLOW"
                | "TRANSFER_FAILED"
                | "INVALID_CODE",
            ) => Some(false),
            _ => None,
        };
        let fee = send::number(info.get("fee"), "receipt fee", Some(0))?;
        let energy = send::number(
            receipt.and_then(|r| r.get("energy_usage_total")),
            "energy_usage_total",
            Some(0),
        )?;
        let bandwidth = send::number(
            receipt.and_then(|r| r.get("net_usage")),
            "net_usage",
            Some(0),
        )?;
        out.insert("blockNumber".into(), json!(block));
        out.insert("success".into(), json!(success));
        out.insert("energyUsed".into(), json!(energy));
        out.insert("netUsage".into(), json!(bandwidth));
        out.insert("feeSun".into(), json!(fee.to_string()));
        if success != Some(true) {
            let why = status
                .map(|s| c.shown(s))
                .unwrap_or_else(|| "execution result unavailable".into());
            out.insert(
                "hint".into(),
                json!(if success == Some(false) {
                    format!("{why}; the transaction failed and {fee} sun of fees were burned")
                } else {
                    format!("execution result is unknown ({why}); the node reports {fee} sun of fees. {SECOND_PAYMENT}")
                }),
            );
        }
        return Ok(out);
    }
    if found {
        return Ok(out);
    }
    let head = send::head(c).await?;
    let behind = send::now_ms().saturating_sub(head.timestamp);
    if behind > 60_000 {
        out.insert("nodeBehindSecs".into(), json!(behind / 1000));
        out.insert(
            "hint".into(),
            json!(format!(
                "this node is {} s behind; ask another node with --rpc",
                behind / 1000
            )),
        );
    } else if head.timestamp <= send::now_ms().saturating_add(30_000)
        && expiration.is_some_and(|expires| head.timestamp > expires.saturating_add(60_000))
    {
        let host = url_origin(&net.rpc_url);
        out.insert("status".into(), json!("expired-unseen"));
        out.insert("asOf".into(), json!({"block": head.number, "blockTime": send::expires_at(head.timestamp), "host": host}));
        out.insert("hint".into(), json!(format!("this node returned no record of it as of block {} on {host}; another node may have it. {SECOND_PAYMENT}", head.number)));
    }
    Ok(out)
}

pub async fn lookup_tx(
    net: &Net,
    hash: &str,
    expiration: Option<u64>,
) -> Result<Map<String, Value>> {
    let hash = parse_hash(hash)?;
    let c = rpc::connect(net).await?;
    observe(&c, net, &hash, expiration).await
}

/// A single deadline covers connection, every HTTP body and polling sleep, including the
/// final observation. Keep the last completed observation when the deadline fires.
pub async fn wait(
    net: &Net,
    hash: &str,
    expiration: Option<u64>,
    timeout: Duration,
) -> Result<Map<String, Value>> {
    let hash = parse_hash(hash)?;
    let deadline = tokio::time::Instant::now() + timeout;
    let mut last = json!({"chain": net.chain.key, "txHash": hash, "status": "unknown", "found": null,
        "hint": format!("the first observation did not finish before the wait deadline; look it up with `zwallet tx --chain tron {hash}`. {SECOND_PAYMENT}")}).as_object().unwrap().clone();
    let c = match tokio::time::timeout_at(deadline, rpc::connect(net)).await {
        Ok(c) => c?,
        Err(_) => return Ok(last),
    };
    loop {
        match tokio::time::timeout_at(deadline, observe(&c, net, &hash, expiration)).await {
            Ok(result) => last = result?,
            Err(_) => return Ok(last),
        }
        if last["status"] == "mined" || last["status"] == "expired-unseen" {
            return Ok(last);
        }
        let next = (tokio::time::Instant::now() + Duration::from_secs(2)).min(deadline);
        tokio::time::sleep_until(next).await;
        if tokio::time::Instant::now() >= deadline {
            return Ok(last);
        }
    }
}

pub async fn broadcast(net: &Net, signed: &SignedTx, record: &SendRecord) -> Result<Broadcast> {
    let before = |e| before_send_error(signed, e);
    let expires = record
        .entry
        .get("expiresAt")
        .and_then(Value::as_str)
        .and_then(send::parse_expiration)
        .ok_or_else(|| {
            before(WalletError::validation(
                "Tron send record has no valid expiresAt",
            ))
        })?;
    let c = rpc::connect(net).await.map_err(before)?;
    if expires <= send::now_ms() {
        return Err(before(WalletError::validation(
            "the Tron transaction expired before broadcast; build a new transaction",
        )));
    }
    record.write().map_err(before)?;
    let result = c
        .post("broadcasthex", &json!({"transaction": signed.raw}))
        .await;
    let mut rpc_hash = None;
    let why = match result {
        Ok(reply) => {
            if let Some(other) = reply.get("txid").and_then(Value::as_str)
                && other != signed.tx_hash
            {
                rpc_hash = Some(ascii_inert(&net.redact(other), 64));
            }
            if reply.get("result") == Some(&json!(true)) {
                if rpc_hash.is_none() {
                    return Ok(Broadcast::Accepted);
                }
                // A successful send remains accepted. Only the computed id is ever queried.
                let _ = observe(&c, net, &signed.tx_hash, Some(expires)).await;
                return Ok(Broadcast::KnownDespite { warning: "the node accepted the broadcast but answered a different txid; looked up only the computed txID".into(), rpc_tx_hash: rpc_hash });
            }
            let code = reply
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("OTHER_ERROR");
            if code == "DUP_TRANSACTION_ERROR" {
                return Ok(Broadcast::KnownDespite {
                    warning: "the node already knows this transaction (DUP_TRANSACTION_ERROR)"
                        .into(),
                    rpc_tx_hash: rpc_hash,
                });
            }
            let message = reply
                .get("message")
                .and_then(Value::as_str)
                .map(|s| {
                    hex::decode(s).ok().map_or_else(
                        || s.to_string(),
                        |b| String::from_utf8_lossy(&b).into_owned(),
                    )
                })
                .unwrap_or_default();
            let why = ascii_inert(&net.redact(&format!("{code}: {message}")), PROSE_MAX);
            if [
                "SIGERROR",
                "CONTRACT_VALIDATE_ERROR",
                "CONTRACT_EXE_ERROR",
                "BANDWITH_ERROR",
                "TAPOS_ERROR",
                "TOO_BIG_TRANSACTION_ERROR",
                "TRANSACTION_EXPIRATION_ERROR",
            ]
            .contains(&code)
            {
                return Err(WalletError::rpc(format!(
                    "the node rejected the transaction; nothing was broadcast: {why}"
                ))
                .with_detail("txHash", signed.tx_hash.as_str())
                .with_detail("from", signed.signer.as_str())
                .with_detail("expiresAt", send::expires_at(expires))
                .with_detail("journal", record.journal.display()));
            }
            why
        }
        Err(e) => e.message,
    };
    match observe(&c, net, &signed.tx_hash, Some(expires)).await {
        Ok(found) if found.get("found") == Some(&json!(true)) => Ok(Broadcast::KnownDespite { warning: format!("{why}; the node reports this transaction by its computed txID"), rpc_tx_hash: rpc_hash }),
        _ => Err(WalletError::new(ErrorCode::BroadcastUncertain, format!("{why}; look it up with `zwallet tx --chain tron {}`; it can still be included until {}. {SECOND_PAYMENT}", signed.tx_hash, send::expires_at(expires)))
            .with_detail("txHash", signed.tx_hash.as_str()).with_detail("from", signed.signer.as_str()).with_detail("expiresAt", send::expires_at(expires)).with_detail("journal", record.journal.display())),
    }
}
