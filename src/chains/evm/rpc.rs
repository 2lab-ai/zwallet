//! EVM JSON-RPC: balances, contract reads, transaction preparation, broadcast, receipts.
//! Runs in the CLI process only; the daemon never reaches this module.

use super::tx::{
    EvmFees, IERC20, IGasPriceOracle, UnsignedEvmTx, encoded_for_signing, erc20_transfer_calldata,
    gas_with_margin,
};
use super::{config, parse_address};
use crate::chains::{Net, PreparedTx, TransferRequest, redact_url};
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
use std::ops::Deref;
use std::time::{Duration, Instant};

const RPC_TIMEOUT: Duration = Duration::from_secs(30);
const RECEIPT_POLL: Duration = Duration::from_secs(3);
/// After a failed send, how often and how far apart to ask the node for the transaction.
const KNOWN_POLLS: u32 = 5;
const KNOWN_POLL_INTERVAL: Duration = Duration::from_secs(2);
const GWEI: u128 = 1_000_000_000;
/// Fee ceiling (see [`check_fee_ceiling`]): multiples of the reference price.
const TIP_CEILING_MULT: u128 = 2;
const PRICE_CEILING_MULT: u128 = 6;
/// Floor of the reference price, so near-zero L2 and testnet base fees do not reject every tip.
const REFERENCE_FLOOR: u128 = GWEI;
/// OP-stack GasPriceOracle predeploy.
const GAS_PRICE_ORACLE: Address = address!("0x420000000000000000000000000000000000000F");
/// Gas limit used for a token-transfer dry-run when the node cannot estimate (no token balance).
pub const TOKEN_FALLBACK_GAS: u64 = 100_000;
/// A replacement (`--nonce` below the pending nonce) pays this multiple of the estimated fees,
/// `NUM / DEN`, rounded up. Nodes drop a replacement that does not outbid the pending transaction
/// by their bump rule (geth and most clients: +10% on both the tip and the fee cap).
const REPLACEMENT_BUMP_NUM: u128 = 5;
const REPLACEMENT_BUMP_DEN: u128 = 4;
const REPLACEMENT_BUMP: &str = "1.25x";
/// `eth_sendRawTransaction` errors in which the node refuses the transaction itself (geth's
/// txpool wording, which most clients share), matched case-insensitively: it never entered the
/// node's pool, so nothing was broadcast.
const SEND_REJECTIONS: &[&str] = &[
    "replacement transaction underpriced",
    "nonce too low",
    "insufficient funds",
    "intrinsic gas too low",
    "exceeds block gas limit",
    "max fee per gas less than block base fee",
];
/// `eth_sendRawTransaction` errors meaning the node already holds this transaction.
const SEND_KNOWN: &[&str] = &["already known", "known transaction"];

/// A JSON-RPC client for one endpoint (derefs to the alloy provider). Errors become text only
/// through its `rpc` method, which scrubs the endpoint's URL and API key ([`Net::redact`]).
/// No `Debug`: the inner provider holds the URL.
pub struct EvmProvider {
    root: RootProvider<Ethereum>,
    net: Net,
}

impl Deref for EvmProvider {
    type Target = RootProvider<Ethereum>;

    fn deref(&self) -> &Self::Target {
        &self.root
    }
}

impl EvmProvider {
    /// No I/O: the HTTP client connects on the first request.
    fn new(net: &Net) -> Result<Self> {
        let url = net.rpc_url.parse().map_err(|_| {
            WalletError::validation(format!("invalid RPC URL '{}'", redact_url(&net.rpc_url)))
        })?;
        Ok(Self {
            root: RootProvider::<Ethereum>::new_http(url),
            net: net.clone(),
        })
    }

    async fn rpc<T, E: Display>(
        &self,
        what: &str,
        call: impl IntoFuture<Output = std::result::Result<T, E>>,
    ) -> Result<T> {
        match tokio::time::timeout(RPC_TIMEOUT, call.into_future()).await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => Err(self.failed(what, &e)),
            Err(_) => Err(timed_out(what)),
        }
    }

    /// Transport errors quote the request URL, and an HTTP error body or a node's error message
    /// can quote the API key on its own; both are scrubbed.
    fn failed(&self, what: &str, e: &dyn Display) -> WalletError {
        WalletError::rpc(self.net.redact(&format!("{what} failed: {e}")))
    }
}

fn timed_out(what: &str) -> WalletError {
    WalletError::rpc(format!("{what} timed out after {}s", RPC_TIMEOUT.as_secs()))
}

/// Opens the endpoint and checks it serves the chain we think it does.
pub async fn connect(net: &Net) -> Result<EvmProvider> {
    let p = EvmProvider::new(net)?;
    let got = p.rpc("eth_chainId", p.get_chain_id()).await?;
    let want = config(net.chain).chain_id;
    if got != want {
        return Err(WalletError::rpc(format!(
            "RPC {} serves chainId {got}, but {} is chainId {want}",
            redact_url(&net.rpc_url),
            net.chain.key
        )));
    }
    Ok(p)
}

pub async fn native_balance(p: &EvmProvider, owner: Address) -> Result<U256> {
    p.rpc("eth_getBalance", p.get_balance(owner)).await
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
    p.rpc("eth_call", p.call(req)).await
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

/// EIP-1559 fees when the chain has a base fee, legacy `eth_gasPrice` otherwise, raised 1.25x when
/// `replacing` a pending transaction, then checked against [`check_fee_ceiling`] (`explicit` =
/// `--max-fee-gwei` in wei).
pub async fn fees(p: &EvmProvider, explicit: Option<u128>, replacing: bool) -> Result<EvmFees> {
    let block = p
        .rpc(
            "eth_getBlockByNumber",
            p.get_block_by_number(BlockNumberOrTag::Latest),
        )
        .await?
        .ok_or_else(|| WalletError::rpc("latest block is not available"))?;
    let gas_price = p.rpc("eth_gasPrice", p.get_gas_price()).await?;
    let (fees, base) = match block.header.base_fee_per_gas {
        Some(base) if base > 0 => {
            let base = u128::from(base);
            let est = p.rpc("eth_feeHistory", p.estimate_eip1559_fees()).await?;
            let priority = est.max_priority_fee_per_gas;
            // 2x base fee + tip survives several full blocks of base-fee growth.
            let floor = base.saturating_mul(2).saturating_add(priority);
            let max = est.max_fee_per_gas.max(floor);
            let fees = EvmFees::Eip1559 {
                max_fee_per_gas: U128::from(max),
                max_priority_fee_per_gas: U128::from(priority),
            };
            (fees, base)
        }
        _ => {
            let fees = EvmFees::Legacy {
                gas_price: U128::from(gas_price),
            };
            (fees, 0)
        }
    };
    if !replacing {
        check_fee_ceiling(&fees, base, gas_price, explicit)?;
        return Ok(fees);
    }
    let fees = replacement_fees(fees);
    check_fee_ceiling(&fees, base, gas_price, explicit).map_err(|e| {
        WalletError::new(
            e.code,
            format!(
                "{} (this replaces a pending transaction, so the fees include the \
                 {REPLACEMENT_BUMP} replacement bump)",
                e.message
            ),
        )
    })?;
    Ok(fees)
}

/// Every fee field times [`REPLACEMENT_BUMP`], rounded up (so a 1 wei tip still rises).
fn replacement_fees(fees: EvmFees) -> EvmFees {
    let up = |v: U128| {
        U128::from(
            v.to::<u128>()
                .saturating_mul(REPLACEMENT_BUMP_NUM)
                .div_ceil(REPLACEMENT_BUMP_DEN),
        )
    };
    match fees {
        EvmFees::Eip1559 {
            max_fee_per_gas,
            max_priority_fee_per_gas,
        } => EvmFees::Eip1559 {
            max_fee_per_gas: up(max_fee_per_gas),
            max_priority_fee_per_gas: up(max_priority_fee_per_gas),
        },
        EvmFees::Legacy { gas_price } => EvmFees::Legacy {
            gas_price: up(gas_price),
        },
    }
}

/// Refuses node-suggested fees above a ceiling before anything is signed. Every fee input comes
/// from the RPC endpoint, so a broken or hostile one could ask for any price per gas:
///
/// ```text
/// reference     = max(latest block baseFeePerGas, eth_gasPrice, 1 gwei)
/// tip ceiling   = 2 x reference   (maxPriorityFeePerGas)
/// price ceiling = 6 x reference   (maxFeePerGas, or gasPrice on a legacy chain)
/// ```
///
/// The normal estimate, maxFee = 2 x base fee + tip, stays well inside it, and so does a
/// replacement's 1.25x of it. `eth_gasPrice` (base fee plus the node's suggested tip) and the
/// 1 gwei floor keep chains whose tip exceeds the base fee working (Polygon, OP Mainnet, Sepolia).
/// On a legacy chain the price *is* `eth_gasPrice`, so only an explicit ceiling can reject it
/// there.
///
/// `explicit` (`--max-fee-gwei`) replaces both ceilings with one price, which may be higher or
/// lower. The reference comes from the same node, so this stops one bad estimate, not a node that
/// lies consistently; an absolute limit belongs in the daemon-side signing policy.
pub fn check_fee_ceiling(
    fees: &EvmFees,
    base_fee: u128,
    gas_price: u128,
    explicit: Option<u128>,
) -> Result<()> {
    let gwei = |wei: u128| format!("{} gwei", format_units(U256::from(wei), 9));
    let reference = base_fee.max(gas_price).max(REFERENCE_FLOOR);
    let (tip_cap, price_cap, rule) = match explicit {
        Some(max) => (max, max, format!("--max-fee-gwei {}", gwei(max))),
        None => (
            reference.saturating_mul(TIP_CEILING_MULT),
            reference.saturating_mul(PRICE_CEILING_MULT),
            format!(
                "zwallet's ceiling ({PRICE_CEILING_MULT} x max(baseFee {}, eth_gasPrice {}, 1 gwei), \
                 tip {TIP_CEILING_MULT} x)",
                gwei(base_fee),
                gwei(gas_price)
            ),
        ),
    };
    let (name, value, cap) = match *fees {
        EvmFees::Eip1559 {
            max_priority_fee_per_gas,
            ..
        } if max_priority_fee_per_gas.to::<u128>() > tip_cap => (
            "maxPriorityFeePerGas",
            max_priority_fee_per_gas.to::<u128>(),
            tip_cap,
        ),
        EvmFees::Eip1559 {
            max_fee_per_gas, ..
        } => ("maxFeePerGas", max_fee_per_gas.to::<u128>(), price_cap),
        EvmFees::Legacy { gas_price } => ("gasPrice", gas_price.to::<u128>(), price_cap),
    };
    if value <= cap {
        return Ok(());
    }
    let hint = if explicit.is_some() {
        "raise --max-fee-gwei if this price is intended"
    } else {
        "if fees really are this high, pass --max-fee-gwei <gwei> to allow up to that price per gas"
    };
    Err(WalletError::validation(format!(
        "the RPC's fee {name} {} is above {rule} = {}; nothing was signed. {hint}",
        gwei(value),
        gwei(cap)
    )))
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
    /// `--nonce`: replace a pending transaction (or name the next nonce) instead of `pending`.
    pub nonce: Option<u64>,
    /// `--max-fee-gwei` in wei: the explicit fee ceiling.
    pub max_fee_per_gas: Option<u128>,
}

async fn estimate(p: &EvmProvider, intent: &Intent, value: U256) -> Result<u64> {
    // No fee fields: the node then does not demand that the account can pay for gas.
    let req = TransactionRequest::default()
        .with_from(intent.from)
        .with_to(intent.to)
        .with_value(value)
        .with_input(intent.input.clone());
    p.rpc("eth_estimateGas", p.estimate_gas(req)).await
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

    let pending = p
        .rpc(
            "eth_getTransactionCount",
            p.get_transaction_count(intent.from).pending(),
        )
        .await?;
    let nonce = match intent.nonce {
        None => pending,
        Some(n) => {
            let mined = p
                .rpc(
                    "eth_getTransactionCount",
                    p.get_transaction_count(intent.from).latest(),
                )
                .await?;
            check_nonce(n, mined, pending)?
        }
    };
    let replacing = nonce < pending;
    let fees = fees(p, intent.max_fee_per_gas, replacing).await?;
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
    if intent.nonce.is_some() {
        fields.insert("replacesPending".into(), json!(replacing));
        let bump = replacing.then_some(REPLACEMENT_BUMP);
        fields.insert("feeBump".into(), json!(bump));
    }
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
        // The daemon gets the signing payload bytes only and decodes them itself.
        unsigned: json!(format!(
            "0x{}",
            alloy::hex::encode(encoded_for_signing(&unsigned))
        )),
        fields,
    })
}

/// An explicit `--nonce n` must reuse a nonce that is still pending (`mined <= n < pending`, the
/// replacement case) or be the next free one (`n == pending`). Below `mined` it is already spent;
/// above `pending` the transaction would wait forever for the gap to fill.
fn check_nonce(n: u64, mined: u64, pending: u64) -> Result<u64> {
    if n < mined {
        return Err(WalletError::validation(format!(
            "--nonce {n} is already used: this account has {mined} mined transaction(s); the \
             next free nonce is {pending}"
        )));
    }
    if n > pending {
        return Err(WalletError::validation(format!(
            "--nonce {n} would leave a gap: the next free nonce is {pending}, and a later nonce \
             is not mined until every earlier one is"
        )));
    }
    Ok(n)
}

async fn op_l1_fee(p: &EvmProvider, tx: &UnsignedEvmTx) -> Result<U256> {
    let data = IGasPriceOracle::getL1FeeCall {
        data: encoded_for_signing(tx).into(),
    }
    .abi_encode();
    let raw = eth_call(p, None, GAS_PRICE_ORACLE, data.into()).await?;
    IGasPriceOracle::getL1FeeCall::abi_decode_returns(&raw)
        .map_err(|e| WalletError::rpc(format!("getL1Fee returned malformed data: {e}")))
}

/// Recipients that are almost always a mistake: the zero address, or the token contract itself
/// (most tokens cannot send back what they hold). `--force` allows them.
fn check_recipient(to: Address, token: Option<Address>, force: bool) -> Result<()> {
    if force {
        return Ok(());
    }
    if to == Address::ZERO {
        return Err(WalletError::invalid_address(
            "refusing to send to the zero address: nothing can ever spend from it; pass --force \
             if this burn is intended",
        ));
    }
    if token == Some(to) {
        return Err(WalletError::invalid_address(format!(
            "refusing to send tokens to the token contract itself ({}): most tokens cannot send \
             them back; pass --force if this is intended",
            to.to_checksum(None)
        )));
    }
    Ok(())
}

/// Native or ERC-20 transfer. Addresses, the recipient and the native amount are validated
/// before any RPC.
pub async fn prepare_transfer(net: &Net, req: &TransferRequest) -> Result<PreparedTx> {
    let chain = net.chain;
    let from = parse_address(&req.from)?;
    let to = parse_address(&req.to)?;
    let token = req.token.as_deref().map(parse_address).transpose()?;
    check_recipient(to, token, req.force)?;
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
                nonce: req.nonce,
                max_fee_per_gas: req.max_fee_per_gas,
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
                nonce: req.nonce,
                max_fee_per_gas: req.max_fee_per_gas,
            };
            (intent, extra)
        }
    };
    let mut prepared = prepare(&p, net, intent).await?;
    extra.append(&mut prepared.fields);
    prepared.fields = extra;
    Ok(prepared)
}

/// What became of `eth_sendRawTransaction` once the request went out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendOutcome {
    /// The node answered with the transaction's own hash.
    Accepted,
    /// The node answered with some other hash: a broken or dishonest endpoint.
    HashMismatch(B256),
    /// The call failed or timed out (the text), but the node reports the transaction.
    KnownDespite(String),
    /// The call failed or timed out (the text), and the node does not report the transaction.
    Unknown(String),
    /// The node refused the transaction itself (the text; see `SEND_REJECTIONS`): nothing was
    /// broadcast.
    Rejected(String),
}

/// How a JSON-RPC error message from `eth_sendRawTransaction` reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SendError {
    /// The node refused the transaction ([`SEND_REJECTIONS`]): nothing was broadcast.
    Rejected,
    /// The node already holds it ([`SEND_KNOWN`]): the lookup by hash then reports it as sent.
    Known,
    /// Anything else: only the lookup by hash can tell.
    Other,
}

fn classify_send_error(message: &str) -> SendError {
    let m = message.to_lowercase();
    let has = |list: &[&str]| list.iter().any(|s| m.contains(s));
    if has(SEND_KNOWN) {
        SendError::Known
    } else if has(SEND_REJECTIONS) {
        SendError::Rejected
    } else {
        SendError::Other
    }
}

/// Sends `raw`, whose hash the caller computed from the bytes. The node's reply is only checked
/// against that hash, never used in its place. A JSON-RPC error that refuses the transaction
/// itself is [`SendOutcome::Rejected`]. Any other failure after the request may have gone out
/// (including "already known", a timeout, or a transport error) asks the node for the
/// transaction by `hash` before giving up.
pub async fn send_raw(p: &EvmProvider, raw: &[u8], hash: B256) -> SendOutcome {
    const WHAT: &str = "eth_sendRawTransaction";
    let call = p
        .client()
        .request::<_, B256>(WHAT, (alloy::hex::encode_prefixed(raw),));
    let why = match tokio::time::timeout(RPC_TIMEOUT, call.into_future()).await {
        Ok(Ok(h)) if h == hash => return SendOutcome::Accepted,
        Ok(Ok(h)) => return SendOutcome::HashMismatch(h),
        Ok(Err(e)) => {
            let why = p.failed(WHAT, &e).message;
            // Only a JSON-RPC error object (HTTP 200, or a 4xx carrying one) is the node's
            // answer; a bare transport error says nothing about the transaction.
            let kind = e
                .as_error_resp()
                .map_or(SendError::Other, |r| classify_send_error(&r.message));
            if kind == SendError::Rejected {
                return SendOutcome::Rejected(why);
            }
            why
        }
        Err(_) => timed_out(WHAT).message,
    };
    if node_has_tx(p, hash).await {
        SendOutcome::KnownDespite(why)
    } else {
        SendOutcome::Unknown(why)
    }
}

/// Asks for the transaction by hash a few times: a load-balanced endpoint may answer from a
/// backend that has not seen it yet. The reply is read as plain JSON (any non-null object whose
/// `hash` matches), so chain-specific transaction types cannot fail the lookup.
async fn node_has_tx(p: &EvmProvider, hash: B256) -> bool {
    for attempt in 0..KNOWN_POLLS {
        if attempt > 0 {
            tokio::time::sleep(KNOWN_POLL_INTERVAL).await;
        }
        let call = p
            .client()
            .request::<_, Option<Value>>("eth_getTransactionByHash", (hash,));
        if let Ok(Some(tx)) = p.rpc("eth_getTransactionByHash", call).await
            && tx
                .get("hash")
                .and_then(Value::as_str)
                .and_then(|h| h.parse::<B256>().ok())
                == Some(hash)
        {
            return true;
        }
    }
    false
}

/// Polls for the receipt until `timeout`; `Ok(None)` if it has not been mined by then.
pub async fn wait_receipt(p: &EvmProvider, hash: B256, timeout: Duration) -> Result<Option<Value>> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(r) = p
            .rpc("eth_getTransactionReceipt", p.get_transaction_receipt(hash))
            .await?
        {
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

#[cfg(test)]
mod tests {
    use super::*;

    const G: u128 = GWEI;

    /// What zwallet asks for on a node: alloy's estimate (2 x base + median 20th-percentile
    /// tip), floored at 2 x base + tip.
    fn eip1559(base: u128, tip: u128) -> EvmFees {
        EvmFees::Eip1559 {
            max_fee_per_gas: U128::from(2 * base + tip),
            max_priority_fee_per_gas: U128::from(tip),
        }
    }

    #[test]
    fn fee_ceiling_accepts_what_public_nodes_quoted_on_2026_10_20() {
        // (chain, latest baseFeePerGas, eth_gasPrice, median 20th-percentile tip), in wei, read
        // from the default publicnode.com endpoints on 2026-10-20.
        for (chain, base, gas_price, tip) in [
            ("ethereum", 72_766_367, 72_766_367, 100_000),
            ("base", 5_000_000, 6_000_000, 312_872),
            ("arbitrum", 20_000_000, 20_072_000, 1),
            ("optimism", 19_579, 1_019_574, 1),
            ("polygon", 253_547_540_214, 281_072_289_569, 126_168_000_000),
            ("avalanche", 2_500_000_000, 2_500_000_150, 1_245_891_731),
            ("sepolia", 15, 22, 12),
            // Outliers seen in the same samples: a Sepolia block whose tips were ~1 gwei, an
            // Avalanche block with a 28 gwei tip at a 2.5 gwei base fee.
            ("sepolia-busy", 15, 22, 999_999_985),
            (
                "avalanche-spike",
                2_500_000_000,
                2_500_000_150,
                28_114_905_753,
            ),
        ] {
            let fees = eip1559(base, tip);
            // Avalanche's spike is above the 2x tip ceiling: refused, then allowed explicitly.
            if chain == "avalanche-spike" {
                assert!(check_fee_ceiling(&fees, base, gas_price, None).is_err());
                assert!(check_fee_ceiling(&fees, base, gas_price, Some(40 * G)).is_ok());
                continue;
            }
            check_fee_ceiling(&fees, base, gas_price, None)
                .unwrap_or_else(|e| panic!("{chain}: {}", e.message));
        }
        // BSC is a legacy chain (base fee 0): gasPrice = eth_gasPrice = 0.05 gwei.
        let bsc = EvmFees::Legacy {
            gas_price: U128::from(50_000_000u64),
        };
        check_fee_ceiling(&bsc, 0, 50_000_000, None).unwrap();
    }

    #[test]
    fn fee_ceiling_refuses_inflated_tips_and_prices() {
        // 1 gwei base fee: reference 1 gwei (or eth_gasPrice if higher).
        let tip = check_fee_ceiling(&eip1559(G, 3 * G), G, G, None).unwrap_err();
        assert!(
            tip.message.contains("maxPriorityFeePerGas 3 gwei"),
            "{}",
            tip.message
        );
        assert!(tip.message.contains("--max-fee-gwei"), "{}", tip.message);
        let price = EvmFees::Eip1559 {
            max_fee_per_gas: U128::from(7 * G),
            max_priority_fee_per_gas: U128::from(G),
        };
        let e = check_fee_ceiling(&price, G, G, None).unwrap_err();
        assert!(e.message.contains("maxFeePerGas 7 gwei"), "{}", e.message);
        assert!(check_fee_ceiling(&price, G, 2 * G, None).is_ok());
        // Explicit ceiling: replaces the automatic one, both up and down.
        assert!(check_fee_ceiling(&price, G, G, Some(7 * G)).is_ok());
        assert!(check_fee_ceiling(&eip1559(G, G / 10), G, G, Some(2 * G)).is_err());
        let legacy = EvmFees::Legacy {
            gas_price: U128::from(3 * G),
        };
        assert!(check_fee_ceiling(&legacy, 0, 3 * G, None).is_ok());
        let e = check_fee_ceiling(&legacy, 0, 3 * G, Some(2 * G)).unwrap_err();
        assert!(e.message.contains("gasPrice 3 gwei"), "{}", e.message);
    }

    #[test]
    fn explicit_nonce_must_be_pending_or_next() {
        // 5 mined, 7 = next free: 5 and 6 replace pending transactions, 7 is new.
        for n in [5, 6, 7] {
            assert_eq!(check_nonce(n, 5, 7).unwrap(), n);
        }
        assert!(
            check_nonce(4, 5, 7)
                .unwrap_err()
                .message
                .contains("already used")
        );
        assert!(check_nonce(8, 5, 7).unwrap_err().message.contains("gap"));
    }

    #[test]
    fn recipient_checks() {
        let token: Address = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"
            .parse()
            .unwrap();
        let other: Address = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
            .parse()
            .unwrap();
        assert_eq!(
            check_recipient(Address::ZERO, None, false)
                .unwrap_err()
                .code,
            ErrorCode::InvalidAddress
        );
        assert!(check_recipient(Address::ZERO, Some(token), false).is_err());
        assert!(check_recipient(token, Some(token), false).is_err());
        assert!(check_recipient(token, None, false).is_ok());
        assert!(check_recipient(other, Some(token), false).is_ok());
        assert!(check_recipient(Address::ZERO, None, true).is_ok());
        assert!(check_recipient(token, Some(token), true).is_ok());
    }

    #[test]
    fn replacement_fees_rise_by_a_quarter_rounded_up() {
        let bumped = replacement_fees(eip1559(G, G / 10));
        assert_eq!(
            bumped,
            EvmFees::Eip1559 {
                // (2 x 1 + 0.1) gwei x 1.25, and 0.1 gwei x 1.25.
                max_fee_per_gas: U128::from(2_625_000_000u64),
                max_priority_fee_per_gas: U128::from(125_000_000u64),
            }
        );
        // A 1 wei tip (Arbitrum, OP Mainnet) must still rise, or the node sees no bump.
        let tiny = replacement_fees(EvmFees::Eip1559 {
            max_fee_per_gas: U128::from(3u64),
            max_priority_fee_per_gas: U128::from(1u64),
        });
        assert_eq!(
            tiny,
            EvmFees::Eip1559 {
                max_fee_per_gas: U128::from(4u64),
                max_priority_fee_per_gas: U128::from(2u64),
            }
        );
        let legacy = replacement_fees(EvmFees::Legacy {
            gas_price: U128::from(3 * G),
        });
        assert_eq!(legacy.max_price(), 3_750_000_000);
    }

    #[test]
    fn send_errors_are_classified_case_insensitively() {
        for m in [
            "replacement transaction underpriced",
            "Nonce too low: address 0xabc, tx: 1 state: 2",
            "INSUFFICIENT FUNDS for gas * price + value",
            "intrinsic gas too low",
            "tx exceeds block gas limit",
            "max fee per gas less than block base fee: address 0xabc",
        ] {
            assert_eq!(classify_send_error(m), SendError::Rejected, "{m}");
        }
        for m in ["already known", "Known transaction: 0xabc"] {
            assert_eq!(classify_send_error(m), SendError::Known, "{m}");
        }
        for m in ["internal error", "rate limited", ""] {
            assert_eq!(classify_send_error(m), SendError::Other, "{m}");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn rpc_error_text_never_prints_the_endpoint_key() {
        let net = Net {
            chain: crate::chains::find("ethereum").unwrap(),
            rpc_url: "https://eth.example.com/v3/0123456789abcdef?apikey=SECRETKEY99&chain=1"
                .into(),
        };
        let p = EvmProvider::new(&net).unwrap();
        // An HTTP error body that quotes the key bare, inside the path and query, and the URL.
        let body = "HTTP error 401 with body: {\"error\":\"invalid project id 0123456789abcdef\",\
                    \"path\":\"/v3/0123456789abcdef?apikey=SECRETKEY99&chain=1\",\"url\":\
                    \"https://eth.example.com/v3/0123456789abcdef\"} (apikey SECRETKEY99)";
        let e = p
            .rpc("eth_call", std::future::ready(Err::<(), _>(body)))
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::RpcError);
        for key in ["0123456789abcdef", "SECRETKEY99", "apikey=", "/v3/"] {
            assert!(!e.message.contains(key), "{key}: {}", e.message);
        }
        assert!(
            e.message.starts_with("eth_call failed: HTTP error 401"),
            "{}",
            e.message
        );
    }
}
