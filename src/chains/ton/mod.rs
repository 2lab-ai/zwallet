//! TON mainnet V4R2 native-coin wallet using the existing BIP39 seed and hardened SLIP-0010
//! path m/44'/607'/{index}'. This is NOT TON-native mnemonic derivation. Workchain, contract
//! version and wallet id are fixed recovery parameters; unsupported capabilities fail closed.
mod address;
mod cell;
mod rpc;
#[cfg(test)]
mod tests;
mod tx;
use crate::{
    chains::{
        self, Balance, Broadcast, Chain, ChainFamily, Family, FamilyKeys, Net, PreparedTx,
        SendRecord, SignedTx, TokenBalance, TransferRequest, policy::ListView,
    },
    core::{
        amount::{format_units, parse_units},
        error::{ErrorCode, Result, WalletError},
        seed::Seed,
    },
};
use alloy::primitives::U256;
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, VerifyingKey};
use serde_json::{Map, Value, json};
use std::time::{Duration, SystemTime};

pub const TON_WALLET_ID: u32 = 698_983_191;
pub const MAX_LIFETIME: u64 = 300;
/// Acceptance reserve only: V4R2 does not encode a protocol max fee. The daemon requires a
/// signed cap at least this large but cannot guarantee actual chain fees stay below it.
pub const FEE_RESERVE: u64 = 50_000_000;
pub const FEE_WARNING: &str = "TON fees are an RPC estimate/reserve, not a protocol maximum. Non-bounceable native transfer; sender execution does not prove downstream contract settlement. V4R2 signatures do not bind a network genesis.";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TonChain {
    pub default_rpc: &'static str,
    pub explorer: &'static str,
    pub global_id: i32,
}
pub struct Ton;
pub fn config(chain: &Chain) -> Result<&TonChain> {
    match &chain.family {
        Family::Ton(c) => Ok(c),
        _ => Err(WalletError::validation("not a TON chain")),
    }
}
pub fn normalize_address(input: &str) -> Result<String> {
    Ok(address::raw(&address::parse(input)?))
}
pub fn policy_address(input: &str) -> Result<String> {
    Ok(format!("ton:{}", normalize_address(input)?))
}
pub fn derivation_path(index: u32) -> String {
    format!("m/44'/607'/{index}'")
}
fn unsupported(what: &str) -> WalletError {
    WalletError::new(
        ErrorCode::CapabilityUnsupported,
        format!("ton supports native TON transfers only; {what} is unsupported"),
    )
}
fn now() -> u64 {
    crate::core::time::unix_secs(SystemTime::now())
}
pub fn inspect_unsigned(chain: &Chain, unsigned: &Value) -> Result<Value> {
    config(chain)?;
    Ok(tx::from_value(unsigned)?.1.fields())
}
pub fn check_unsigned(
    chain: &Chain,
    from: &str,
    unsigned: &Value,
    view: &ListView<'_>,
) -> Result<()> {
    if config(chain)?.global_id != -239 {
        return Err(unsupported("networks other than mainnet"));
    }
    let (_, intent) = tx::from_value(unsigned)?;
    intent.fresh(now())?;
    chains::ed25519::check_native(
        view,
        chain,
        &normalize_address(from)?,
        &address::raw(&intent.to),
        U256::from(intent.amount),
        U256::from(FEE_RESERVE),
    )
}
pub async fn validate_rpc(net: &Net) -> Result<()> {
    rpc::connect(net).await.map(|_| ())
}
impl FamilyKeys for Ton {
    fn derivation_path(&self, index: u32) -> String {
        derivation_path(index)
    }
    fn derive_address(&self, seed: &Seed, index: u32) -> Result<String> {
        let key = chains::ed25519::signer_at(seed, 607, index, None)?;
        Ok(tx::wallet_address(&key.verifying_key().to_bytes()))
    }
    fn sign_message(
        &self,
        _seed: &Seed,
        _index: u32,
        _chain: &Chain,
        _message: &[u8],
        _list: &ListView<'_>,
    ) -> Result<Value> {
        Err(unsupported("arbitrary message signing"))
    }
    fn sign_tx(
        &self,
        seed: &Seed,
        index: u32,
        chain: &Chain,
        unsigned: &Value,
        list: &ListView<'_>,
    ) -> Result<Value> {
        let key = chains::ed25519::signer_at(seed, 607, index, None)?;
        let from = tx::wallet_address(&key.verifying_key().to_bytes());
        check_unsigned(chain, &from, unsigned, list)?;
        let (cell, intent) = tx::from_value(unsigned)?;
        let signed = tx::sign(&key, &cell);
        let inspected = tx::inspect(&signed)?;
        Ok(
            json!({"rawTx":format!("0x{}",alloy::hex::encode(cell::encode(&signed))),"txHash":alloy::hex::encode(inspected.body.hash()),"hashKind":"ton-signed-body","signer":from,"decoded":intent.fields()}),
        )
    }
}
fn raw_signed(s: &str) -> Result<(std::sync::Arc<cell::Cell>, tx::Inspected)> {
    let bytes = s
        .strip_prefix("0x")
        .filter(|s| s.len() <= cell::MAX_BOC * 2)
        .and_then(|s| alloy::hex::decode(s).ok())
        .ok_or_else(|| {
            WalletError::validation("TON signed transaction is not bounded 0x BOC bytes")
        })?;
    let external = cell::decode(&bytes)?;
    if cell::encode(&external) != bytes {
        return Err(WalletError::validation("TON signed BOC is not canonical"));
    }
    let i = tx::inspect(&external)?;
    Ok((external, i))
}
pub fn verify_signed_payload(signed: &SignedTx, unsigned: &Value) -> Result<()> {
    let (_, i) = raw_signed(&signed.raw)?;
    let (expected, _) = tx::from_value(unsigned)?;
    if *i.unsigned != *expected
        || signed.tx_hash != alloy::hex::encode(i.body.hash())
        || signed.signer != tx::wallet_address(&i.public)
    {
        return Err(WalletError::validation(
            "TON signed transaction differs from the reviewed payload or signer",
        ));
    }
    Ok(())
}
fn fields(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}
impl ChainFamily for Ton {
    fn normalize_address(&self, input: &str) -> Result<String> {
        normalize_address(input)
    }
    fn verify_message(&self, _message: &[u8], _signed: &Value, _expected: &str) -> Result<()> {
        Err(unsupported("arbitrary message signing"))
    }
    fn inspect_signed(&self, chain: &Chain, value: &Value) -> Result<SignedTx> {
        config(chain)?;
        let raw = value["rawTx"]
            .as_str()
            .ok_or_else(|| WalletError::validation("TON signer returned no raw transaction"))?;
        let (_, i) = raw_signed(raw)?;
        let tx_hash = alloy::hex::encode(i.body.hash());
        if value.get("txHash").is_some_and(|v| v != &tx_hash) {
            return Err(WalletError::validation(
                "TON signer returned an inconsistent body hash",
            ));
        }
        Ok(SignedTx {
            raw: raw.into(),
            tx_hash,
            signer: tx::wallet_address(&i.public),
            nonce: None,
        })
    }
    async fn native_balance(&self, net: &Net, owner: &str) -> Result<Balance> {
        let c = rpc::connect(net).await?;
        let a = rpc::account(&c, owner).await?;
        Ok(Balance {
            symbol: "TON".into(),
            decimals: 9,
            raw: U256::from(a.balance),
        })
    }
    async fn token_balance(&self, _net: &Net, _owner: &str, _token: &str) -> Result<Balance> {
        Err(unsupported("Jetton balances"))
    }
    async fn wallet_balances(
        &self,
        net: &Net,
        owner: &str,
        limit: Duration,
    ) -> Result<(Balance, Vec<TokenBalance>)> {
        tokio::time::timeout(limit, self.native_balance(net, owner))
            .await
            .map_err(|_| WalletError::rpc("TON balance deadline expired"))?
            .map(|b| (b, Vec::new()))
    }
    async fn prepare_transfer(&self, net: &Net, req: &TransferRequest) -> Result<PreparedTx> {
        config(net.chain)?;
        if req.token.is_some()
            || req.nonce.is_some()
            || req.bump.is_some()
            || req.replace.is_some()
            || req.max_fee_per_gas.is_some()
            || req.allow_pending
            || req.force
        {
            return Err(unsupported(
                "tokens, EVM nonce/replacement/fee flags or force overrides",
            ));
        }
        let from = normalize_address(&req.from)?;
        let to = address::parse(&req.to)?;
        if to == [0; 32] {
            return Err(WalletError::validation("TON zero recipient is refused"));
        }
        let amount = u64::try_from(parse_units(&req.amount, 9)?)
            .ok()
            .filter(|v| *v > 0)
            .ok_or_else(|| {
                WalletError::validation("TON amount must be positive and fit in 64-bit nanotons")
            })?;
        let c = rpc::connect(net).await?;
        let account = rpc::account(&c, &from).await?;
        let intent = tx::Intent {
            to,
            amount,
            seqno: account.seqno,
            valid_until: u32::try_from(now().saturating_add(120))
                .map_err(|_| WalletError::validation("TON timestamp exceeds supported range"))?,
        };
        let unsigned = tx::unsigned(&intent);
        let (fee, fee_source) = rpc::estimate(&c, &from, &unsigned, account.active).await?;
        let sufficient = amount
            .checked_add(fee)
            .is_some_and(|cost| cost <= account.balance);
        if !sufficient && !req.dry_run {
            return Err(WalletError::new(
                ErrorCode::InsufficientFunds,
                "TON balance does not cover amount and the displayed fee reserve",
            ));
        }
        let mut out = fields(intent.fields());
        out.extend(fields(json!({"from":from,"to":address::raw(&to),"amount":format_units(U256::from(amount),9),"amountRaw":amount.to_string(),"symbol":"TON","decimals":9,"balanceRaw":account.balance.to_string(),"sufficientFunds":sufficient,"feeEstimateRaw":fee.to_string(),"feeEstimate":format_units(U256::from(fee),9),"feeEstimateSource":fee_source,"warning":FEE_WARNING,"activated":account.active,"walletVersion":"v4r2","hashKind":"ton-signed-body"})));
        Ok(PreparedTx {
            max_gas_cost: U256::from(fee),
            l1_fee: None,
            unsigned: json!(format!("0x{}", alloy::hex::encode(cell::encode(&unsigned)))),
            fields: out,
        })
    }
    async fn broadcast(
        &self,
        net: &Net,
        signed: &SignedTx,
        _explicit_nonce: bool,
        record: &SendRecord,
    ) -> Result<Broadcast> {
        if _explicit_nonce {
            return Err(unsupported("EVM explicit nonce retries"));
        }
        let (external, i) = raw_signed(&signed.raw)?;
        if signed.signer != tx::wallet_address(&i.public)
            || signed.tx_hash != alloy::hex::encode(i.body.hash())
        {
            return Err(WalletError::validation("TON signed identity mismatch"));
        }
        i.intent.fresh(now())?;
        let c = rpc::connect(net).await?;
        let a = rpc::account(&c, &signed.signer).await?;
        if a.seqno != i.intent.seqno {
            return Err(WalletError::validation(
                "TON seqno changed since review; nothing was broadcast; inspect earlier sends before retrying",
            ));
        }
        record.write()?;
        match c.call("sendBoc",json!({"boc":STANDARD.encode(cell::encode(&external))})).await{
            Ok(v) if v["@type"]=="ok"=>Ok(Broadcast::Accepted),
            _=>Err(WalletError::new(ErrorCode::BroadcastUncertain,"TON broadcast outcome is uncertain; inspect the signed-body hash before sending again; a retry can pay twice").with_detail("txHash",signed.tx_hash.clone()).with_detail("hashKind","ton-signed-body").with_detail("from",signed.signer.clone()).with_detail("journal",record.journal.display()))
        }
    }
    async fn wait_for_receipt(
        &self,
        net: &Net,
        hash: &str,
        timeout: Duration,
    ) -> Result<Option<Value>> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let v = tokio::time::timeout_at(deadline, self.lookup_tx(net, hash)).await;
            match v {
                Err(_) => return Ok(None),
                Ok(Err(e)) => return Err(e),
                Ok(Ok(v))
                    if v.get("status") == Some(&json!("mined"))
                        && v.get("success").is_some_and(Value::is_boolean) =>
                {
                    return Ok(Some(Value::Object(v)));
                }
                _ => {}
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok(None);
            }
            tokio::time::sleep_until(
                (tokio::time::Instant::now() + Duration::from_secs(3)).min(deadline),
            )
            .await;
        }
    }
    async fn lookup_tx(&self, net: &Net, hash: &str) -> Result<Map<String, Value>> {
        lookup(net, hash).await
    }
}
async fn lookup(net: &Net, hash: &str) -> Result<Map<String, Value>> {
    let wanted = rpc::hash(&json!(hash))?;
    let hash = alloy::hex::encode(wanted);
    let c = rpc::connect(net).await?;
    let response = c.by_body_hash(&hash).await?;
    let rows = response["transactions"]
        .as_array()
        .filter(|v| v.len() <= 2)
        .ok_or_else(|| {
            WalletError::rpc("TON index returned malformed or excessive transactions")
        })?;
    let mut out = fields(
        json!({"chain":"ton","txHash":hash,"hashKind":"ton-signed-body","found":false,"status":"not found","hint":"A missing index row does not prove the message failed; do not resend blindly.","warning":FEE_WARNING}),
    );
    if rows.is_empty() {
        return Ok(out);
    }
    if rows.len() != 1 {
        return Err(WalletError::rpc("TON signed-body lookup is ambiguous"));
    }
    let row = &rows[0];
    let incoming = &row["in_msg"];
    let body_text = incoming["message_content"]["body"]
        .as_str()
        .filter(|s| s.len() <= cell::MAX_BOC * 2)
        .ok_or_else(|| WalletError::rpc("TON index did not include the signed message body"))?;
    let bytes = STANDARD
        .decode(body_text)
        .map_err(|_| WalletError::rpc("TON indexed body is not base64"))?;
    let body = cell::decode(&bytes)?;
    if body.hash() != wanted || rpc::hash(&incoming["message_content"]["hash"])? != wanted {
        return Err(WalletError::rpc(
            "TON index answered for another message body",
        ));
    }
    let (unsigned, intent, signature) = tx::inspect_body(&body)?;
    let from = normalize_address(
        row["account"]
            .as_str()
            .ok_or_else(|| WalletError::rpc("TON transaction has no source account"))?,
    )?;
    if !rpc::same_address(&incoming["destination"], &from)
        || !matches!(incoming.get("source"), None | Some(Value::Null))
            && incoming.get("source") != Some(&Value::String(String::new()))
    {
        return Err(WalletError::rpc(
            "TON body lookup is not an external message to the source wallet",
        ));
    }
    let account = rpc::account(&c, &from).await?;
    let public = account
        .public
        .ok_or_else(|| WalletError::rpc("TON indexed source wallet is not active"))?;
    VerifyingKey::from_bytes(&public)
        .map_err(|_| WalletError::rpc("TON source public key is invalid"))?
        .verify_strict(&unsigned.hash(), &Signature::from_bytes(&signature))
        .map_err(|_| WalletError::rpc("TON indexed signature does not match source wallet"))?;
    let tx_hash = rpc::hash(&row["hash"])?;
    let to = address::raw(&intent.to);
    let emulated = match row.get("emulated") {
        None => false,
        Some(Value::Bool(b)) => *b,
        _ => return Err(WalletError::rpc("TON index emulated flag is malformed")),
    };
    let actual_block = row["mc_block_seqno"].as_u64().is_some_and(|v| v > 0) && !emulated;
    let outgoing = row["out_msgs"]
        .as_array()
        .ok_or_else(|| WalletError::rpc("TON index omitted outgoing messages"))?;
    let emitted = outgoing.len() == 1
        && rpc::same_address(&outgoing[0]["source"], &from)
        && rpc::same_address(&outgoing[0]["destination"], &to)
        && rpc::number(&outgoing[0]["value"]).ok() == Some(intent.amount);
    let d = &row["description"];
    let success = actual_block
        && d["aborted"] == false
        && d["compute_ph"]["success"] == true
        && d["action"]["success"] == true
        && emitted;
    let failed = actual_block
        && (d["aborted"] == true
            || d["compute_ph"]["success"] == false
            || d["action"]["success"] == false);
    out.extend(fields(json!({"found":true,"status":if actual_block{"mined"}else{"pending"},"success":if success{Some(true)}else if failed{Some(false)}else{None},"from":from,"to":to,"valueRaw":intent.amount.to_string(),"value":format_units(U256::from(intent.amount),9),"symbol":"TON","seqno":intent.seqno,"transactionHash":alloy::hex::encode(tx_hash),"explorerUrl":format!("{}/transaction/{}",config(net.chain)?.explorer,alloy::hex::encode(tx_hash)),"executionEvidence":if success{"outgoing-message-emitted"}else{"incomplete-or-failed"},"provenance":"rpc-index-observation"})));
    if let Ok(fee) = rpc::number(&row["total_fees"]) {
        out.insert("feeRaw".into(), json!(fee.to_string()));
    }
    if let Some(block) = row["mc_block_seqno"].as_u64() {
        out.insert("blockNumber".into(), json!(block));
    }
    Ok(out)
}
