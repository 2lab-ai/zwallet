//! EVM JSON-RPC: balances, contract reads, transaction preparation, broadcast, receipts.
//! Runs in the CLI process only; the daemon never reaches this module.

use super::tx::{
    EvmFees, IERC20, IGasPriceOracle, UnsignedEvmTx, encoded_for_signing, erc20_transfer_calldata,
    gas_with_margin,
};
use super::{config, parse_address};
use crate::chains::{Net, PreparedTx, TransferRequest};
use crate::core::amount::{format_units, parse_units};
use crate::core::error::{ErrorCode, Result, WalletError};
use alloy::eips::BlockNumberOrTag;
use alloy::network::{Ethereum, TransactionBuilder};
use alloy::primitives::{Address, B256, Bytes, U128, U256, address};
use alloy::providers::{Provider, RootProvider};
use alloy::rpc::types::TransactionRequest;
use alloy::sol_types::SolCall;
use serde_json::{Map, Value, json};
use std::fmt::Display;
use std::future::IntoFuture;
use std::time::{Duration, Instant};

const RPC_TIMEOUT: Duration = Duration::from_secs(30);
const RECEIPT_POLL: Duration = Duration::from_secs(3);
/// OP-stack GasPriceOracle predeploy.
const GAS_PRICE_ORACLE: Address = address!("0x420000000000000000000000000000000000000F");
/// Gas limit used for a token-transfer dry-run when the node cannot estimate (no token balance).
pub const TOKEN_FALLBACK_GAS: u64 = 100_000;

pub type EvmProvider = RootProvider<Ethereum>;

async fn rpc<T, E: Display>(
    what: &str,
    call: impl IntoFuture<Output = std::result::Result<T, E>>,
) -> Result<T> {
    match tokio::time::timeout(RPC_TIMEOUT, call.into_future()).await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(WalletError::rpc(format!("{what} failed: {e}"))),
        Err(_) => Err(WalletError::rpc(format!(
            "{what} timed out after {}s",
            RPC_TIMEOUT.as_secs()
        ))),
    }
}

/// Opens the endpoint and checks it serves the chain we think it does.
pub async fn connect(net: &Net) -> Result<EvmProvider> {
    let url = net
        .rpc_url
        .parse()
        .map_err(|_| WalletError::validation(format!("invalid RPC URL '{}'", net.rpc_url)))?;
    let p = RootProvider::<Ethereum>::new_http(url);
    let got = rpc("eth_chainId", p.get_chain_id()).await?;
    let want = config(net.chain).chain_id;
    if got != want {
        return Err(WalletError::rpc(format!(
            "RPC {} serves chainId {got}, but {} is chainId {want}",
            net.rpc_url, net.chain.key
        )));
    }
    Ok(p)
}

pub async fn native_balance(p: &EvmProvider, owner: Address) -> Result<U256> {
    rpc("eth_getBalance", p.get_balance(owner)).await
}

pub async fn eth_call(
    p: &EvmProvider,
    from: Option<Address>,
    to: Address,
    data: Bytes,
) -> Result<Bytes> {
    let mut req = TransactionRequest::default().with_to(to).with_input(data);
    if let Some(from) = from {
        req = req.with_from(from);
    }
    rpc("eth_call", p.call(req)).await
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenMeta {
    pub symbol: String,
    pub decimals: u8,
}

pub async fn erc20_meta(p: &EvmProvider, net: &Net, token: Address) -> Result<TokenMeta> {
    let not_token = || {
        WalletError::invalid_address(format!(
            "{} is not an ERC-20 contract on {} (decimals() returned no usable data)",
            token.to_checksum(None),
            net.chain.key
        ))
    };
    let raw = eth_call(p, None, token, IERC20::decimalsCall {}.abi_encode().into())
        .await
        .map_err(|_| not_token())?;
    let decimals = IERC20::decimalsCall::abi_decode_returns(&raw).map_err(|_| not_token())?;
    let raw = eth_call(p, None, token, IERC20::symbolCall {}.abi_encode().into())
        .await
        .unwrap_or_default();
    let symbol = IERC20::symbolCall::abi_decode_returns(&raw)
        .ok()
        .or_else(|| {
            // Some old tokens (e.g. MKR) return bytes32.
            (raw.len() == 32).then(|| {
                String::from_utf8_lossy(&raw)
                    .trim_end_matches('\0')
                    .to_string()
            })
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "?".to_string());
    Ok(TokenMeta { symbol, decimals })
}

pub async fn erc20_balance(p: &EvmProvider, token: Address, owner: Address) -> Result<U256> {
    let raw = eth_call(
        p,
        None,
        token,
        IERC20::balanceOfCall { owner }.abi_encode().into(),
    )
    .await?;
    IERC20::balanceOfCall::abi_decode_returns(&raw)
        .map_err(|e| WalletError::rpc(format!("balanceOf returned malformed data: {e}")))
}

/// EIP-1559 fees when the chain has a base fee, legacy `eth_gasPrice` otherwise.
pub async fn fees(p: &EvmProvider) -> Result<EvmFees> {
    let block = rpc(
        "eth_getBlockByNumber",
        p.get_block_by_number(BlockNumberOrTag::Latest),
    )
    .await?
    .ok_or_else(|| WalletError::rpc("latest block is not available"))?;
    match block.header.base_fee_per_gas {
        Some(base) if base > 0 => {
            let est = rpc("eth_feeHistory", p.estimate_eip1559_fees()).await?;
            let priority = est.max_priority_fee_per_gas;
            // 2x base fee + tip survives several full blocks of base-fee growth.
            let floor = u128::from(base).saturating_mul(2).saturating_add(priority);
            let max = est.max_fee_per_gas.max(floor);
            Ok(EvmFees::Eip1559 {
                max_fee_per_gas: U128::from(max),
                max_priority_fee_per_gas: U128::from(priority),
            })
        }
        _ => Ok(EvmFees::Legacy {
            gas_price: U128::from(rpc("eth_gasPrice", p.get_gas_price()).await?),
        }),
    }
}

/// A token the transaction debits besides native value (for the balance pre-check).
#[derive(Debug, Clone)]
pub struct TokenDebit {
    pub token: Address,
    pub amount: U256,
    pub symbol: String,
    pub decimals: u8,
}

/// What a transaction should do; `prepare` fills in nonce, fees and gas from the chain.
#[derive(Debug, Clone)]
pub struct Intent {
    pub from: Address,
    pub to: Address,
    pub value: U256,
    pub input: Bytes,
    pub token: Option<TokenDebit>,
    /// Gas limit to use in a dry-run when the node refuses to estimate an unfunded tx.
    pub fallback_gas: Option<u64>,
    pub dry_run: bool,
}

async fn estimate(p: &EvmProvider, intent: &Intent, value: U256) -> Result<u64> {
    // No fee fields: the node then does not demand that the account can pay for gas.
    let req = TransactionRequest::default()
        .with_from(intent.from)
        .with_to(intent.to)
        .with_value(value)
        .with_input(intent.input.clone());
    rpc("eth_estimateGas", p.estimate_gas(req)).await
}

fn shortfall_entry(asset: &str, decimals: u8, required: U256, available: U256) -> Value {
    json!({
        "asset": asset,
        "required": format_units(required, decimals),
        "available": format_units(available, decimals),
        "missing": format_units(required.saturating_sub(available), decimals),
    })
}

fn insufficient(shortfall: &[Value]) -> WalletError {
    let parts: Vec<String> = shortfall
        .iter()
        .map(|s| {
            format!(
                "need {} {} but have {}",
                s["required"].as_str().unwrap_or("?"),
                s["asset"].as_str().unwrap_or("?"),
                s["available"].as_str().unwrap_or("?")
            )
        })
        .collect();
    WalletError::new(
        ErrorCode::InsufficientFunds,
        format!("insufficient funds: {}", parts.join("; ")),
    )
}

/// Shared by `transfer` and `invoke`: nonce (pending), fees, gas estimate with margin, balance
/// pre-check. Outside dry-run a shortfall is `INSUFFICIENT_FUNDS` before anything is signed.
pub async fn prepare(p: &EvmProvider, net: &Net, intent: Intent) -> Result<PreparedTx> {
    let chain = net.chain;
    let native = native_balance(p, intent.from).await?;
    let token_balance = match &intent.token {
        Some(t) => Some(erc20_balance(p, t.token, intent.from).await?),
        None => None,
    };
    let token_short = match (&intent.token, token_balance) {
        (Some(t), Some(bal)) if bal < t.amount => {
            Some(shortfall_entry(&t.symbol, t.decimals, t.amount, bal))
        }
        _ => None,
    };
    let value_short = native < intent.value;
    if !intent.dry_run && (token_short.is_some() || value_short) {
        let mut s: Vec<Value> = token_short.into_iter().collect();
        if value_short {
            s.push(shortfall_entry(
                chain.symbol,
                chain.decimals,
                intent.value,
                native,
            ));
        }
        return Err(insufficient(&s));
    }

    let nonce = rpc(
        "eth_getTransactionCount",
        p.get_transaction_count(intent.from).pending(),
    )
    .await?;
    let fees = fees(p).await?;
    let (gas_estimate, gas_source) = match estimate(p, &intent, intent.value).await {
        Ok(g) => (g, "estimate"),
        // The node will not estimate a tx the account cannot fund. In a dry-run, estimate
        // without the value, or fall back to a fixed limit, so an empty wallet can still be
        // dry-run.
        Err(e) if intent.dry_run && (token_short.is_some() || value_short) => {
            let without_value = if intent.value.is_zero() {
                None
            } else {
                estimate(p, &intent, U256::ZERO).await.ok()
            };
            match (without_value, intent.fallback_gas) {
                (Some(g), _) => (g, "estimate-without-value"),
                (None, Some(g)) => (g, "fallback"),
                (None, None) => return Err(e),
            }
        }
        Err(e) => return Err(e),
    };
    let gas_limit = gas_with_margin(gas_estimate);
    let unsigned = UnsignedEvmTx {
        chain_id: config(chain).chain_id,
        nonce,
        gas_limit,
        fees,
        to: intent.to,
        value: intent.value,
        input: intent.input.clone(),
    };
    // OP-stack L1 data fee. Best effort: without it the node still rejects an unfundable tx.
    let l1_fee = if config(chain).op_stack {
        op_l1_fee(p, &unsigned).await.ok()
    } else {
        None
    };
    let max_gas_cost = U256::from(gas_limit) * U256::from(fees.max_price());
    let native_needed = intent
        .value
        .saturating_add(max_gas_cost)
        .saturating_add(l1_fee.unwrap_or_default());

    let mut shortfall: Vec<Value> = token_short.into_iter().collect();
    if native < native_needed {
        shortfall.push(shortfall_entry(
            chain.symbol,
            chain.decimals,
            native_needed,
            native,
        ));
    }
    if !intent.dry_run && !shortfall.is_empty() {
        return Err(insufficient(&shortfall));
    }

    let mut fields = Map::new();
    fields.insert("chainId".into(), json!(unsigned.chain_id));
    fields.insert("nonce".into(), json!(nonce));
    fields.insert("gasLimit".into(), json!(gas_limit));
    fields.insert("gasEstimate".into(), json!(gas_estimate));
    fields.insert("gasLimitSource".into(), json!(gas_source));
    match fees {
        EvmFees::Eip1559 {
            max_fee_per_gas,
            max_priority_fee_per_gas,
        } => {
            fields.insert("type".into(), json!("eip1559"));
            fields.insert("maxFeePerGas".into(), json!(max_fee_per_gas.to_string()));
            fields.insert(
                "maxPriorityFeePerGas".into(),
                json!(max_priority_fee_per_gas.to_string()),
            );
            fields.insert("gasPrice".into(), Value::Null);
        }
        EvmFees::Legacy { gas_price } => {
            fields.insert("type".into(), json!("legacy"));
            fields.insert("maxFeePerGas".into(), Value::Null);
            fields.insert("maxPriorityFeePerGas".into(), Value::Null);
            fields.insert("gasPrice".into(), json!(gas_price.to_string()));
        }
    }
    fields.insert(
        "maxGasCost".into(),
        json!(format_units(max_gas_cost, chain.decimals)),
    );
    fields.insert(
        "l1Fee".into(),
        l1_fee.map_or(Value::Null, |f| json!(format_units(f, chain.decimals))),
    );
    fields.insert(
        "nativeBalance".into(),
        json!(format_units(native, chain.decimals)),
    );
    fields.insert("sufficientFunds".into(), json!(shortfall.is_empty()));
    fields.insert("shortfall".into(), Value::Array(shortfall));
    Ok(PreparedTx {
        unsigned: serde_json::to_value(&unsigned)
            .map_err(|e| WalletError::validation(format!("cannot encode tx: {e}")))?,
        fields,
    })
}

async fn op_l1_fee(p: &EvmProvider, tx: &UnsignedEvmTx) -> Result<U256> {
    let data = IGasPriceOracle::getL1FeeCall {
        data: encoded_for_signing(tx)?.into(),
    }
    .abi_encode();
    let raw = eth_call(p, None, GAS_PRICE_ORACLE, data.into()).await?;
    IGasPriceOracle::getL1FeeCall::abi_decode_returns(&raw)
        .map_err(|e| WalletError::rpc(format!("getL1Fee returned malformed data: {e}")))
}

/// Native or ERC-20 transfer. Addresses and the native amount are validated before any RPC.
pub async fn prepare_transfer(net: &Net, req: &TransferRequest) -> Result<PreparedTx> {
    let chain = net.chain;
    let from = parse_address(&req.from)?;
    let to = parse_address(&req.to)?;
    let token = req.token.as_deref().map(parse_address).transpose()?;
    let native_amount = match token {
        None => Some(parse_units(&req.amount, chain.decimals)?),
        Some(_) => None,
    };
    let p = connect(net).await?;
    let (intent, mut extra) = match (token, native_amount) {
        (Some(token), _) => {
            let meta = erc20_meta(&p, net, token).await?;
            let amount = parse_units(&req.amount, meta.decimals)?;
            let mut extra = Map::new();
            extra.insert("token".into(), json!(token.to_checksum(None)));
            extra.insert("symbol".into(), json!(meta.symbol));
            extra.insert("decimals".into(), json!(meta.decimals));
            extra.insert("amount".into(), json!(format_units(amount, meta.decimals)));
            extra.insert("amountRaw".into(), json!(amount.to_string()));
            let intent = Intent {
                from,
                to: token,
                value: U256::ZERO,
                input: erc20_transfer_calldata(to, amount),
                token: Some(TokenDebit {
                    token,
                    amount,
                    symbol: meta.symbol,
                    decimals: meta.decimals,
                }),
                fallback_gas: Some(TOKEN_FALLBACK_GAS),
                dry_run: req.dry_run,
            };
            (intent, extra)
        }
        (None, amount) => {
            let amount = amount.unwrap_or_default();
            let mut extra = Map::new();
            extra.insert("symbol".into(), json!(chain.symbol));
            extra.insert("decimals".into(), json!(chain.decimals));
            extra.insert("amount".into(), json!(format_units(amount, chain.decimals)));
            extra.insert("amountRaw".into(), json!(amount.to_string()));
            let intent = Intent {
                from,
                to,
                value: amount,
                input: Bytes::new(),
                token: None,
                fallback_gas: None,
                dry_run: req.dry_run,
            };
            (intent, extra)
        }
    };
    let mut prepared = prepare(&p, net, intent).await?;
    extra.append(&mut prepared.fields);
    prepared.fields = extra;
    Ok(prepared)
}

pub async fn broadcast(p: &EvmProvider, raw: &[u8]) -> Result<B256> {
    let pending = rpc("eth_sendRawTransaction", p.send_raw_transaction(raw)).await?;
    Ok(*pending.tx_hash())
}

/// Polls for the receipt until `timeout`; `Ok(None)` if it has not been mined by then.
pub async fn wait_receipt(p: &EvmProvider, hash: B256, timeout: Duration) -> Result<Option<Value>> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(r) = rpc("eth_getTransactionReceipt", p.get_transaction_receipt(hash)).await? {
            return Ok(Some(json!({
                "status": if r.status() { "success" } else { "reverted" },
                "blockNumber": r.block_number,
                "gasUsed": r.gas_used,
            })));
        }
        if Instant::now() + RECEIPT_POLL > deadline {
            return Ok(None);
        }
        tokio::time::sleep(RECEIPT_POLL).await;
    }
}
