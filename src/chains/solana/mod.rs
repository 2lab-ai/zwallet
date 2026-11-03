//! Native SOL support. A shared BIP39 seed derives an Ed25519 key at m/44'/501'/i'/0'.
//! Only a single canonical legacy SystemProgram transfer is signed. There are no arbitrary
//! programs, SPL tokens, priority-fee instructions, address lookups or durable-nonce support.
pub mod rpc;
pub mod tx;
use crate::chains::{
    self, Balance, Broadcast, Chain, ChainFamily, Family, FamilyKeys, Net, PreparedTx, SendRecord,
    SignedTx, TokenBalance, TransferRequest, policy::ListView,
};
use crate::core::{
    amount::{format_units, parse_units},
    error::{ErrorCode, Result, WalletError},
    seed::Seed,
};
use alloy::primitives::U256;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Map, Value, json};
use std::time::Duration;

/// Solana Labs SDK ClusterType::MainnetBeta, sdk/src/genesis_config.rs, and
/// https://beta.docs.solanalabs.com/clusters/available (verified 2026-11-02).
pub const MAINNET_GENESIS: &str = "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d";
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SolanaChain {
    pub default_rpc: &'static str,
    pub explorer: &'static str,
    pub genesis: &'static str,
}
pub struct Solana;
pub fn config(chain: &Chain) -> Result<&SolanaChain> {
    match &chain.family {
        Family::Solana(c) => Ok(c),
        _ => Err(WalletError::validation("not a Solana chain")),
    }
}
pub fn derivation_path(index: u32) -> String {
    format!("m/44'/501'/{index}'/0'")
}
pub fn parse_address(input: &str) -> Result<[u8; 32]> {
    let s = input.trim();
    if !(32..=44).contains(&s.len()) {
        return Err(WalletError::invalid_address(
            "Solana address must encode exactly 32 bytes in base58",
        ));
    }
    let bytes = bs58::decode(s)
        .into_vec()
        .map_err(|_| WalletError::invalid_address("invalid Solana base58 address"))?;
    let a: [u8; 32] = bytes
        .try_into()
        .map_err(|_| WalletError::invalid_address("Solana address must encode exactly 32 bytes"))?;
    if bs58::encode(a).into_string() != s {
        return Err(WalletError::invalid_address(
            "Solana address is not canonical base58",
        ));
    }
    Ok(a)
}
pub fn normalize_address(input: &str) -> Result<String> {
    Ok(bs58::encode(parse_address(input)?).into_string())
}
pub fn policy_address(input: &str) -> Result<String> {
    Ok(format!(
        "solana:{}",
        alloy::hex::encode(parse_address(input)?)
    ))
}
pub fn address_from_policy(input: &str) -> Result<String> {
    let s = input
        .strip_prefix("solana:")
        .filter(|s| s.len() == 64)
        .ok_or_else(|| WalletError::invalid_address("invalid Solana policy identity"))?;
    let b = alloy::hex::decode(s)
        .map_err(|_| WalletError::invalid_address("invalid Solana policy identity"))?;
    let a: [u8; 32] = b
        .try_into()
        .map_err(|_| WalletError::invalid_address("invalid Solana policy identity"))?;
    let out = bs58::encode(a).into_string();
    if policy_address(&out)? != input {
        return Err(WalletError::invalid_address(
            "noncanonical Solana policy identity",
        ));
    }
    Ok(out)
}
fn unsupported(what: &str) -> WalletError {
    WalletError::new(
        ErrorCode::CapabilityUnsupported,
        format!("solana supports native SOL transfers only; {what} is unsupported"),
    )
}
pub fn inspect_unsigned(chain: &Chain, unsigned: &Value) -> Result<Value> {
    config(chain)?;
    Ok(tx::decode(&tx::unsigned_bytes(unsigned)?)?.view())
}
pub fn check_unsigned(
    chain: &Chain,
    from: &str,
    unsigned: &Value,
    list: &ListView<'_>,
) -> Result<()> {
    config(chain)?;
    let t = tx::decode(&tx::unsigned_bytes(unsigned)?)?;
    if t.from != parse_address(from)? {
        return Err(WalletError::validation(
            "Solana transfer sender is not this key",
        ));
    }
    chains::ed25519::check_native(
        list,
        chain,
        from,
        &bs58::encode(t.to).into_string(),
        U256::from(t.lamports),
        U256::from(tx::BASE_FEE),
    )
}
pub fn verify_signed_payload(signed: &SignedTx, unsigned: &Value) -> Result<()> {
    tx::verify_signed_payload(signed, unsigned)
}
pub async fn validate_rpc(net: &Net) -> Result<()> {
    rpc::connect(net).await.map(|_| ())
}

impl FamilyKeys for Solana {
    fn derivation_path(&self, index: u32) -> String {
        derivation_path(index)
    }
    fn derive_address(&self, seed: &Seed, index: u32) -> Result<String> {
        let signer = chains::ed25519::signer_at(seed, 501, index, Some(0))?;
        Ok(bs58::encode(signer.verifying_key().to_bytes()).into_string())
    }
    fn sign_message(
        &self,
        _seed: &Seed,
        _index: u32,
        _chain: &Chain,
        _message: &[u8],
        _list: &ListView<'_>,
    ) -> Result<Value> {
        Err(unsupported("message signing"))
    }
    fn sign_tx(
        &self,
        seed: &Seed,
        index: u32,
        chain: &Chain,
        unsigned: &Value,
        list: &ListView<'_>,
    ) -> Result<Value> {
        let key = chains::ed25519::signer_at(seed, 501, index, Some(0))?;
        let from = bs58::encode(key.verifying_key().to_bytes()).into_string();
        check_unsigned(chain, &from, unsigned, list)?;
        tx::sign(&tx::unsigned_bytes(unsigned)?, &key)
    }
}
fn value_u64(value: &Value, key: &str) -> Result<u64> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| WalletError::rpc(format!("Solana RPC {key} is not a u64")))
}
fn hash(value: &Value) -> Result<[u8; 32]> {
    value
        .as_str()
        .ok_or_else(|| WalletError::rpc("Solana blockhash is not a string"))
        .and_then(|s| {
            parse_address(s).map_err(|_| WalletError::rpc("Solana blockhash is not 32-byte base58"))
        })
}
fn signature(input: &str) -> Result<()> {
    if input.len() > 88 {
        return Err(WalletError::validation(
            "Solana transaction id must be a base58 Ed25519 signature",
        ));
    }
    let b = bs58::decode(input)
        .into_vec()
        .map_err(|_| WalletError::validation("invalid Solana transaction id"))?;
    if b.len() != 64 || bs58::encode(&b).into_string() != input {
        return Err(WalletError::validation("invalid Solana transaction id"));
    }
    Ok(())
}
async fn balance_with(client: &rpc::Client, owner: &str) -> Result<u64> {
    let v = client
        .call("getBalance", json!([owner,{"commitment":"confirmed"}]))
        .await?;
    value_u64(&v, "value")
}
/// Validate RPC execution evidence before treating it as a definite success or failure.
/// Unknown future shapes remain unknown, so malformed metadata cannot invite a second payment.
fn execution_success(err: &Value) -> Result<bool> {
    let enum_name = |v: &Value| {
        v.as_str().is_some_and(|s| {
            !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric())
        })
    };
    if err.is_null() {
        return Ok(true);
    }
    let valid = if enum_name(err) {
        true
    } else if let Some(object) = err.as_object().filter(|o| o.len() == 1) {
        let (kind, value) = object.iter().next().unwrap();
        match kind.as_str() {
            "InstructionError" => value.as_array().is_some_and(|a| {
                a.len() == 2
                    && a[0].as_u64().is_some_and(|i| i == 0)
                    && (enum_name(&a[1])
                        || a[1].as_object().filter(|o| o.len() == 1).is_some_and(|o| {
                            o.get("Custom")
                                .and_then(Value::as_u64)
                                .is_some_and(|n| u32::try_from(n).is_ok())
                                || o.get("BorshIoError")
                                    .and_then(Value::as_str)
                                    .is_some_and(|s| s.len() <= 256)
                        }))
            }),
            "DuplicateInstruction" => value.as_u64().is_some_and(|i| i == 0),
            "InsufficientFundsForRent" | "ProgramExecutionTemporarilyRestricted" => {
                value.as_object().filter(|o| o.len() == 1).is_some_and(|o| {
                    o.get("account_index")
                        .and_then(Value::as_u64)
                        .is_some_and(|i| i < 3)
                })
            }
            _ => false,
        }
    } else {
        false
    };
    if valid {
        Ok(false)
    } else {
        Err(WalletError::rpc(
            "Solana transaction execution outcome has an unsupported shape",
        ))
    }
}

fn reject_options(req: &TransferRequest) -> Result<()> {
    if req.token.is_some() {
        return Err(unsupported("SPL tokens"));
    }
    if req.nonce.is_some()
        || req.bump.is_some()
        || req.replace.is_some()
        || req.max_fee_per_gas.is_some()
        || req.allow_pending
        || req.force
    {
        return Err(unsupported("EVM nonce/replacement/gas/force options"));
    }
    Ok(())
}
impl ChainFamily for Solana {
    fn normalize_address(&self, input: &str) -> Result<String> {
        normalize_address(input)
    }
    fn verify_message(&self, _message: &[u8], _signed: &Value, _expected: &str) -> Result<()> {
        Err(unsupported("message signing"))
    }
    fn inspect_signed(&self, chain: &Chain, signed: &Value) -> Result<SignedTx> {
        config(chain)?;
        tx::inspect(signed)
    }
    async fn native_balance(&self, net: &Net, owner: &str) -> Result<Balance> {
        let owner = normalize_address(owner)?;
        let client = rpc::connect(net).await?;
        Ok(Balance {
            symbol: "SOL".into(),
            decimals: 9,
            raw: U256::from(balance_with(&client, &owner).await?),
        })
    }
    async fn token_balance(&self, _net: &Net, _owner: &str, _token: &str) -> Result<Balance> {
        Err(unsupported("SPL tokens"))
    }
    async fn wallet_balances(
        &self,
        net: &Net,
        owner: &str,
        limit: Duration,
    ) -> Result<(Balance, Vec<TokenBalance>)> {
        tokio::time::timeout(limit, self.native_balance(net, owner))
            .await
            .map_err(|_| WalletError::rpc("Solana balance deadline elapsed"))?
            .map(|b| (b, Vec::new()))
    }
    async fn prepare_transfer(&self, net: &Net, req: &TransferRequest) -> Result<PreparedTx> {
        reject_options(req)?;
        let from = parse_address(&req.from)?;
        let to = parse_address(&req.to)?;
        let amount = parse_units(&req.amount, 9)?;
        let lamports = u64::try_from(amount)
            .map_err(|_| WalletError::validation("SOL amount exceeds u64 lamports"))?;
        if from == to || to == [0; 32] || lamports == 0 {
            return Err(WalletError::validation(
                "SOL transfer requires a positive amount and a different non-system recipient",
            ));
        }
        let client = rpc::connect(net).await?;
        let latest = client
            .call("getLatestBlockhash", json!([{"commitment":"confirmed"}]))
            .await?;
        let slot = latest
            .pointer("/context/slot")
            .and_then(Value::as_u64)
            .ok_or_else(|| WalletError::rpc("Solana latest blockhash has no context slot"))?;
        let blockhash = hash(&latest["value"]["blockhash"])?;
        let last_valid = value_u64(&latest["value"], "lastValidBlockHeight")?;
        let message = tx::encode(&tx::Transfer {
            from,
            to,
            blockhash,
            lamports,
        })?;
        let fee = client
            .call(
                "getFeeForMessage",
                json!([STANDARD.encode(&message),{"commitment":"confirmed","minContextSlot":slot}]),
            )
            .await?;
        if fee.get("value").and_then(Value::as_u64) != Some(tx::BASE_FEE) {
            return Err(WalletError::rpc(
                "Solana base fee is unavailable or differs from the supported 5000-lamport single-signature fee",
            ));
        }
        let balance = balance_with(&client, &req.from).await?;
        let cost = amount + U256::from(tx::BASE_FEE);
        let sufficient = U256::from(balance) >= cost;
        if !sufficient && !req.dry_run {
            return Err(WalletError::new(
                ErrorCode::InsufficientFunds,
                "not enough SOL for the transfer and 5000-lamport fee",
            ));
        }
        // Simulation carries no valid signature; preflight is repeated on the signed transaction.
        // An insufficient-funds dry-run still explains the deficit without a misleading RPC error.
        if sufficient {
            let simulated=client.call("simulateTransaction",json!([tx::simulation_wire(&message)?,{"encoding":"base64","sigVerify":false,"replaceRecentBlockhash":false,"commitment":"confirmed","minContextSlot":slot}])).await?;
            if simulated.pointer("/value/err") != Some(&Value::Null) {
                return Err(WalletError::rpc(
                    "Solana transfer simulation failed or omitted its result; nothing was signed",
                ));
            }
        }
        let mut fields = Map::new();
        for (k, v) in [
            ("amount", json!(format_units(amount, 9))),
            ("amountRaw", json!(lamports.to_string())),
            ("symbol", json!("SOL")),
            ("balanceRaw", json!(balance.to_string())),
            ("sufficientFunds", json!(sufficient)),
            ("maxFeeRaw", json!(tx::BASE_FEE.to_string())),
            ("feeEstimateRaw", json!(tx::BASE_FEE.to_string())),
            ("estimatedFee", json!("0.000005")),
            (
                "recentBlockhash",
                json!(bs58::encode(blockhash).into_string()),
            ),
            ("lastValidBlockHeight", json!(last_valid)),
            ("contextSlot", json!(slot)),
            ("from", json!(bs58::encode(from).into_string())),
            ("to", json!(bs58::encode(to).into_string())),
            (
                "feeAssumption",
                json!(
                    "one signature, no priority fee; 5000 lamports must match the RPC; the protocol base fee is not encoded as a maximum in the message"
                ),
            ),
        ] {
            fields.insert(k.into(), v);
        }
        Ok(PreparedTx {
            max_gas_cost: U256::from(tx::BASE_FEE),
            l1_fee: None,
            unsigned: json!(format!("0x{}", alloy::hex::encode(message))),
            fields,
        })
    }
    async fn broadcast(
        &self,
        net: &Net,
        signed: &SignedTx,
        explicit_nonce: bool,
        record: &SendRecord,
    ) -> Result<Broadcast> {
        if explicit_nonce {
            return Err(chains::before_send_error(
                signed,
                unsupported("nonce override"),
            ));
        }
        let (verified, transfer) =
            tx::inspect_raw(&signed.raw).map_err(|e| chains::before_send_error(signed, e))?;
        if &verified != signed {
            return Err(chains::before_send_error(
                signed,
                WalletError::validation("Solana signed identity mismatch"),
            ));
        }
        let client = rpc::connect(net)
            .await
            .map_err(|e| chains::before_send_error(signed, e))?;
        let valid = client
            .call(
                "isBlockhashValid",
                json!([bs58::encode(transfer.blockhash).into_string(),{"commitment":"confirmed"}]),
            )
            .await
            .map_err(|e| chains::before_send_error(signed, e))?;
        if valid.get("value").and_then(Value::as_bool) != Some(true) {
            return Err(chains::before_send_error(
                signed,
                WalletError::validation(
                    "Solana recent blockhash is no longer valid; review a fresh transaction",
                ),
            ));
        }
        let path = record
            .write()
            .map_err(|e| chains::before_send_error(signed, e))?;
        let result=client.call("sendTransaction",json!([signed.raw,{"encoding":"base64","skipPreflight":false,"preflightCommitment":"confirmed","maxRetries":3}])).await;
        if result.as_ref().ok().and_then(Value::as_str) == Some(signed.tx_hash.as_str()) {
            return Ok(Broadcast::Accepted);
        }
        // Even an RPC error can follow acceptance (or be a dishonest reply). Never suggest a
        // blind resend; a new recent blockhash creates a distinct, payable transaction.
        let reason = if result.is_err() {
            "Solana send request failed after submission"
        } else {
            "Solana RPC returned a different transaction id"
        };
        Err(
            chains::after_send_error(ErrorCode::BroadcastUncertain, net.chain, signed, reason)
                .with_detail("journal", path),
        )
    }
    async fn wait_for_receipt(
        &self,
        net: &Net,
        tx_hash: &str,
        timeout: Duration,
    ) -> Result<Option<Value>> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let result = tokio::time::timeout_at(deadline, self.lookup_tx(net, tx_hash)).await;
            let row = match result {
                Err(_) => return Ok(None),
                Ok(result) => result?,
            };
            if row.get("status").and_then(Value::as_str) == Some("mined") {
                let mut out = row;
                out.insert(
                    "status".into(),
                    json!(
                        if out.get("success").and_then(Value::as_bool) == Some(true) {
                            "success"
                        } else {
                            "reverted"
                        }
                    ),
                );
                return Ok(Some(Value::Object(out)));
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok(None);
            }
            tokio::time::sleep_until(
                (tokio::time::Instant::now() + Duration::from_millis(500)).min(deadline),
            )
            .await;
        }
    }
    async fn lookup_tx(&self, net: &Net, tx_hash: &str) -> Result<Map<String, Value>> {
        signature(tx_hash)?;
        let client = rpc::connect(net).await?;
        let v=client.call("getTransaction",json!([tx_hash,{"encoding":"base64","commitment":"confirmed","maxSupportedTransactionVersion":0}])).await?;
        let mut out=json!({"chain":net.chain.key,"txHash":tx_hash,"explorerUrl":net.chain.explorer_tx_url(tx_hash),"found":false,"status":"not found","hint":"absence from one RPC does not prove expiry; do not resend blindly with a fresh blockhash"}).as_object().unwrap().clone();
        if v.is_null() {
            let statuses = client
                .call(
                    "getSignatureStatuses",
                    json!([[tx_hash],{"searchTransactionHistory":true}]),
                )
                .await?;
            let rows = statuses
                .get("value")
                .and_then(Value::as_array)
                .filter(|a| a.len() == 1)
                .ok_or_else(|| {
                    WalletError::rpc("Solana signature status response has wrong shape")
                })?;
            if !rows[0].is_null() {
                if !rows[0].is_object() || rows[0].get("slot").and_then(Value::as_u64).is_none() {
                    return Err(WalletError::rpc("Solana signature status is malformed"));
                }
                out.insert("found".into(), json!(true));
                out.insert("status".into(), json!("pending"));
                out.insert("hint".into(),json!("RPC reports the signature but its transaction bytes are unavailable; confirmation and amount remain unverified"));
            }
            return Ok(out);
        }
        let wire = v
            .get("transaction")
            .and_then(Value::as_array)
            .filter(|a| a.len() == 2 && a[1] == "base64")
            .and_then(|a| a[0].as_str())
            .ok_or_else(|| {
                WalletError::rpc("Solana transaction response omitted its base64 wire bytes")
            })?;
        let (signed, t) = tx::inspect_raw(wire).map_err(|_| {
            WalletError::rpc("Solana transaction is not a valid supported signed transfer")
        })?;
        if signed.tx_hash != tx_hash {
            return Err(WalletError::rpc("Solana RPC returned another transaction"));
        }
        let slot = value_u64(&v, "slot")?;
        let meta = v
            .get("meta")
            .filter(|m| m.is_object())
            .ok_or_else(|| WalletError::rpc("Solana transaction has no metadata"))?;
        let fee = value_u64(meta, "fee")?;
        let err = meta
            .get("err")
            .ok_or_else(|| WalletError::rpc("Solana transaction has no execution outcome"))?;
        let success = execution_success(err)?;
        out.remove("hint");
        out.extend(json!({"found":true,"status":"mined","from":signed.signer,"to":bs58::encode(t.to).into_string(),"value":format_units(U256::from(t.lamports),9),"valueRaw":t.lamports.to_string(),"symbol":"SOL","blockNumber":slot,"feeRaw":fee.to_string(),"fee":format_units(U256::from(fee),9),"success":success,"confirmation":"confirmed","evidence":"rpc-reported; transaction signature verified"}).as_object().unwrap().clone());
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{
        seed::TEST_MNEMONIC,
        whitelist::{Entry, Whitelist},
    };
    use zeroize::Zeroizing;
    #[test]
    fn independent_web3js_fixture_derivation_serialization_and_signature() {
        let f: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/solana/native-transfer.json"
        ))
        .unwrap();
        let seed = Seed::from_phrase(Zeroizing::new(TEST_MNEMONIC.into())).unwrap();
        for i in 0..2 {
            assert_eq!(
                Solana.derive_address(&seed, i).unwrap(),
                f["addresses"][i as usize]
            );
            assert_eq!(derivation_path(i), f["paths"][i as usize]);
        }
        let bytes = alloy::hex::decode(f["messageHex"].as_str().unwrap()).unwrap();
        let t = tx::decode(&bytes).unwrap();
        assert_eq!(
            t.from,
            parse_address(f["addresses"][0].as_str().unwrap()).unwrap()
        );
        assert_eq!(
            t.to,
            parse_address(f["addresses"][1].as_str().unwrap()).unwrap()
        );
        assert_eq!(t.lamports, 1_000_000);
        assert_eq!(tx::encode(&t).unwrap(), bytes);
        let key = chains::ed25519::signer_at(&seed, 501, 0, Some(0)).unwrap();
        let signed = tx::sign(&bytes, &key).unwrap();
        assert_eq!(signed["rawTx"], f["rawBase64"]);
        assert_eq!(signed["txHash"], f["signatureBase58"]);
    }
    #[test]
    fn daemon_policy_checks_decoded_native_intent() {
        let f: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/solana/native-transfer.json"
        ))
        .unwrap();
        let chain = chains::find("solana").unwrap();
        let from = f["addresses"][0].as_str().unwrap();
        let to = f["addresses"][1].as_str().unwrap();
        let unsigned = json!(format!("0x{}", f["messageHex"].as_str().unwrap()));
        let mut list = Whitelist::empty(&chains::WHITELIST_RULES);
        list.generation = 1;
        list.add(Entry::new_address(
            "recipient",
            &policy_address(to).unwrap(),
            &["solana"],
        ))
        .unwrap();
        check_unsigned(chain, from, &unsigned, &ListView::Usable(&list)).unwrap();
        assert!(check_unsigned(chain, from, &unsigned, &ListView::NoList).is_err());
        assert!(check_unsigned(chain, to, &unsigned, &ListView::Usable(&list)).is_err());
        list.entries[0].chains = vec!["*".into()];
        assert!(check_unsigned(chain, from, &unsigned, &ListView::Usable(&list)).is_err());
    }
    #[test]
    fn transaction_id_parser_accepts_leading_zero_encodings_but_not_invalid_widths() {
        for zeros in [16, 32, 63, 64] {
            let mut bytes = [1u8; 64];
            bytes[..zeros].fill(0);
            let encoded = bs58::encode(bytes).into_string();
            if zeros >= 32 {
                assert!(encoded.len() < 80);
            }
            signature(&encoded).unwrap();
        }
        for bytes in [vec![0u8; 63], vec![0u8; 65]] {
            assert!(signature(&bs58::encode(bytes).into_string()).is_err());
        }
        assert!(signature("").is_err());
        assert!(signature(&"1".repeat(89)).is_err());
        assert!(signature(&"0".repeat(64)).is_err());
        // Identifier parsing permits a query, not a cryptographic assertion about a signature.
        let f: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/solana/native-transfer.json"
        ))
        .unwrap();
        let mut wire = STANDARD.decode(f["rawBase64"].as_str().unwrap()).unwrap();
        wire[1..65].fill(0);
        assert!(tx::inspect_raw(&STANDARD.encode(wire)).is_err());
    }
    #[test]
    fn address_shapes_and_policy_roundtrip() {
        let a = bs58::encode([8; 32]).into_string();
        assert_eq!(
            address_from_policy(&policy_address(&a).unwrap()).unwrap(),
            a
        );
        for bad in ["0x1234", "111", "O0Il", &"1".repeat(45)] {
            assert!(normalize_address(bad).is_err());
        }
        assert_eq!(
            normalize_address("11111111111111111111111111111111").unwrap(),
            "11111111111111111111111111111111"
        );
    }
}
