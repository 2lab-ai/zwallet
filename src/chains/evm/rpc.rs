//! EVM JSON-RPC: balances, contract reads, transaction preparation, broadcast, receipts.
//! Runs in the CLI process only; the daemon never reaches this module.

use super::tx::{
    EvmFees, IERC20, IGasPriceOracle, UnsignedEvmTx, encoded_for_signing, erc20_transfer_calldata,
    gas_with_margin,
};
use super::{config, parse_address, parse_tx_hash};
use crate::chains::{
    BUMP_SCALE, DEFAULT_BUMP, Net, PreparedTx, SendJournal, TransferRequest, bump_label,
    nonce_label, redact_url, url_origin,
};
use crate::core::amount::{format_units, parse_units};
use crate::core::error::{ErrorCode, Result, WalletError};
use alloy::eips::BlockNumberOrTag;
use alloy::network::{Ethereum, TransactionBuilder};
use alloy::primitives::{Address, B256, Bytes, U128, U256, address};
use alloy::providers::{Provider, RootProvider};
use alloy::rpc::types::TransactionRequest;
use alloy::sol_types::SolCall;
use alloy::transports::{RpcError, TransportError, TransportResult};
use serde_json::{Map, Value, json};
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
/// `eth_sendRawTransaction` errors in which the node refuses the transaction itself (geth's
/// txpool wording, which most clients share), matched case-insensitively: it never entered the
/// node's pool, so nothing was broadcast. "transaction underpriced" is a price below the pool's
/// minimum; "replacement transaction underpriced" is checked first ([`SEND_NONCE_TAKEN`]).
const SEND_REJECTIONS: &[&str] = &[
    "insufficient funds",
    "intrinsic gas too low",
    "exceeds block gas limit",
    "max fee per gas less than block base fee",
    "transaction underpriced",
];
/// `eth_sendRawTransaction` errors meaning the nonce is already used, by another transaction or
/// by this very one (resent after it went in); only the lookup by hash can tell which.
const SEND_NONCE_TAKEN: &[&str] = &["nonce too low", "replacement transaction underpriced"];
/// `eth_sendRawTransaction` errors meaning the node already holds this transaction: "already
/// known" as a phrase, or a message that starts with "known transaction" (never "unknown
/// transaction").
const SEND_KNOWN_PHRASE: &str = "already known";
const SEND_KNOWN_PREFIX: &str = "known transaction";

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

    async fn rpc<T>(
        &self,
        what: &str,
        call: impl IntoFuture<Output = TransportResult<T>>,
    ) -> Result<T> {
        match tokio::time::timeout(RPC_TIMEOUT, call.into_future()).await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => Err(self.failed(what, &e)),
            Err(_) => Err(timed_out(what)),
        }
    }

    /// A non-2xx answer is printed as its status and the endpoint's origin only: its body is a
    /// gateway's text, which can quote the API key in any encoding. A 2xx answer that is not
    /// JSON-RPC, or not the type expected, is a fixed text with the serde error's category and
    /// position: serde quotes the offending value escaped (`\"`, `\\`), which no scrub of the
    /// key as written would catch. Transport errors quote the request URL, and a node's JSON-RPC
    /// error message can quote the key on its own; both are scrubbed ([`Net::redact`]). The
    /// error's `data` is kept only when it is a JSON string (revert data), and decoded first.
    fn failed(&self, what: &str, e: &TransportError) -> WalletError {
        let why = match e {
            RpcError::Transport(kind) => match kind.as_http_error() {
                Some(http) => format!(
                    "HTTP {} from {}",
                    http.status,
                    url_origin(&self.net.rpc_url)
                ),
                None => self.net.redact(&kind.to_string()),
            },
            RpcError::DeserError { err, .. } => deser_text(err),
            RpcError::ErrorResp(payload) => {
                let data = payload
                    .data
                    .as_ref()
                    .and_then(|d| serde_json::from_str::<String>(d.get()).ok())
                    .map(|d| format!(", data: {d}"))
                    .unwrap_or_default();
                self.net.redact(&format!(
                    "server returned an error response: error code {}: {}{data}",
                    payload.code, payload.message
                ))
            }
            other => self.net.redact(&other.to_string()),
        };
        WalletError::rpc(format!("{what} failed: {why}"))
    }
}

/// A serde error as printable text: its category and position only, never its message.
fn deser_text(err: &serde_json::Error) -> String {
    use serde_json::error::Category;
    let kind = match err.classify() {
        Category::Io => "an I/O error",
        Category::Syntax => "a syntax error",
        Category::Data => "a data type error",
        Category::Eof => "an unexpected end",
    };
    format!(
        "the endpoint's answer is not valid JSON-RPC ({kind} at line {} column {})",
        err.line(),
        err.column()
    )
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

/// What a replacement (`--nonce` below the pending nonce, or `--replace`) or an explicit
/// `--bump` must outbid: `bump` ([`crate::chains::parse_bump`]) times the estimate, and times the
/// fees of the transaction it replaces when those are known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outbid {
    pub bump: u128,
    pub original: Option<OriginalFees>,
}

/// Fee cap and tip of the transaction being replaced (both its `gasPrice` if it is legacy).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OriginalFees {
    pub max_fee: u128,
    pub tip: u128,
}

/// EIP-1559 fees when the chain has a base fee, legacy `eth_gasPrice` otherwise, raised by
/// `outbid` when replacing a pending transaction or bumping, then checked against [`check_fee_ceiling`]
/// (`explicit` = `--max-fee-gwei` in wei).
pub async fn fees(
    p: &EvmProvider,
    explicit: Option<u128>,
    outbid: Option<Outbid>,
) -> Result<EvmFees> {
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
    let Some(outbid) = outbid else {
        check_fee_ceiling(&fees, base, gas_price, explicit)?;
        return Ok(fees);
    };
    let fees = replacement_fees(fees, outbid);
    check_fee_ceiling(&fees, base, gas_price, explicit).map_err(|e| {
        WalletError::new(
            e.code,
            format!(
                "{} (the fees include the {} bump{})",
                e.message,
                bump_label(outbid.bump),
                if outbid.original.is_some() {
                    " over the higher of the estimate and the original's fees"
                } else {
                    ""
                }
            ),
        )
    })?;
    Ok(fees)
}

/// Every fee field times the bump, rounded up (so a 1 wei tip still rises), and at least the bump
/// times the original's matching fee. The tip never exceeds the fee cap.
fn replacement_fees(fees: EvmFees, outbid: Outbid) -> EvmFees {
    let up = |v: u128| v.saturating_mul(outbid.bump).div_ceil(BUMP_SCALE);
    let (cap, tip) = outbid.original.map_or((0, 0), |o| (o.max_fee, o.tip));
    match fees {
        EvmFees::Eip1559 {
            max_fee_per_gas,
            max_priority_fee_per_gas,
        } => {
            let tip = up(max_priority_fee_per_gas.to()).max(up(tip));
            let max = up(max_fee_per_gas.to()).max(up(cap)).max(tip);
            EvmFees::Eip1559 {
                max_fee_per_gas: U128::from(max),
                max_priority_fee_per_gas: U128::from(tip),
            }
        }
        EvmFees::Legacy { gas_price } => EvmFees::Legacy {
            gas_price: U128::from(up(gas_price.to()).max(up(cap))),
        },
    }
}

/// The fees of the transaction `--replace` names, read as plain JSON. It must be `from`'s
/// transaction at `nonce` and not yet mined. A node that does not know it cannot tell its fees;
/// what to do then depends on whether this node holds another transaction at `nonce` (`nonce <
/// pending`): if so the hash is the wrong one, and the error names the journal's entries there;
/// if not, the slot is free and a retry at the same nonce without `--replace` is allowed, so the
/// two errors never send the agent back and forth.
async fn original_fees(
    p: &EvmProvider,
    hash: B256,
    from: Address,
    nonce: u64,
    pending: u64,
    journal: Option<&SendJournal>,
) -> Result<OriginalFees> {
    let call = p
        .client()
        .request::<_, Option<Value>>("eth_getTransactionByHash", (hash,));
    let Some(tx) = p.rpc("eth_getTransactionByHash", call).await? else {
        let shown = format!("{hash:#x}");
        if nonce < pending {
            // The hash the agent passed is known not to be the one at `nonce`.
            let pointer = journal
                .map(|j| format!(" {}.", j.pointer(nonce..=nonce, Some(&shown)).0))
                .unwrap_or_default();
            let mut e = WalletError::rpc(format!(
                "the node does not know --replace {shown}, but it holds another pending \
                 transaction from this key at nonce {nonce}; nothing was signed. Look up the \
                 txHash of your send at nonce {nonce} (its own output, or the send journal) and \
                 pass that to --replace.{pointer} {}",
                different_payment(nonce)
            ))
            .with_detail("nonce", nonce);
            if let Some(j) = journal {
                e = e.with_detail("journal", j.display());
            }
            return Err(e);
        }
        return Err(WalletError::rpc(format!(
            "the node does not know --replace {shown} and holds nothing from this key at nonce \
             {nonce}: that transaction was dropped or never reached this node; nothing was \
             signed. To send at nonce {nonce} anyway, retry with --nonce {nonce} and no \
             --replace: it uses the same nonce, so it can take that transaction's place but \
             never add a second one"
        ))
        .with_detail("nonce", nonce));
    };
    let num = |k: &str| {
        tx.get(k)
            .and_then(Value::as_str)
            .and_then(|h| u128::from_str_radix(h.strip_prefix("0x")?, 16).ok())
    };
    let refuse = |why: String| {
        WalletError::validation(format!("--replace {hash}: {why}; nothing was signed"))
    };
    let sender = tx
        .get("from")
        .and_then(Value::as_str)
        .and_then(|a| a.parse::<Address>().ok());
    if sender != Some(from) {
        return Err(refuse(format!(
            "it was sent from {}, not from this key ({})",
            sender.map_or_else(|| "an unknown address".to_string(), |a| a.to_checksum(None)),
            from.to_checksum(None)
        )));
    }
    match num("nonce") {
        Some(n) if n == u128::from(nonce) => {}
        other => {
            return Err(refuse(format!(
                "it has nonce {}, not --nonce {nonce}",
                other.map_or_else(|| "?".to_string(), |n| n.to_string())
            )));
        }
    }
    if tx.get("blockNumber").is_some_and(|b| !b.is_null()) {
        return Err(refuse(format!(
            "it is already mined: if it was your earlier attempt at this payment, the payment \
             went through; look it up before sending anything new ({})",
            crate::chains::lookup_hint(p.net.chain, &format!("{hash:#x}"))
        )));
    }
    match (
        num("maxFeePerGas"),
        num("maxPriorityFeePerGas"),
        num("gasPrice"),
    ) {
        (Some(max_fee), Some(tip), _) => Ok(OriginalFees { max_fee, tip }),
        (_, _, Some(price)) => Ok(OriginalFees {
            max_fee: price,
            tip: price,
        }),
        _ => Err(WalletError::rpc(format!(
            "the node's answer for --replace {hash} has no fee fields; nothing was signed"
        ))),
    }
}

/// Refuses node-suggested fees above a ceiling before anything is signed. Every fee input comes
/// from the RPC endpoint, so a broken or hostile one could ask for any price per gas:
///
/// ```text
/// EIP-1559: reference = max(latest block baseFeePerGas, eth_gasPrice, 1 gwei)
///           tip ceiling   = 2 x reference   (maxPriorityFeePerGas)
///           price ceiling = 6 x reference   (maxFeePerGas)
/// legacy:   reference = max(latest block baseFeePerGas, 1 gwei)
///           price ceiling = 6 x reference   (gasPrice)
/// ```
///
/// The normal estimate, maxFee = 2 x base fee + tip, stays well inside it, and so does a
/// replacement's bump of it. On an EIP-1559 chain `eth_gasPrice` (base fee plus the node's
/// suggested tip) and the 1 gwei floor keep chains whose tip exceeds the base fee working
/// (Polygon, OP Mainnet, Sepolia). On a legacy chain the price *is* `eth_gasPrice`, so it is left
/// out of its own reference: BSC (base fee 0) caps at 6 gwei unless `--max-fee-gwei` says more.
///
/// `explicit` (`--max-fee-gwei`) replaces both ceilings with one price, which may be higher or
/// lower. The reference comes from the same node, so this stops one bad estimate, not a node that
/// lies consistently.
pub fn check_fee_ceiling(
    fees: &EvmFees,
    base_fee: u128,
    gas_price: u128,
    explicit: Option<u128>,
) -> Result<()> {
    // Not here: an absolute fee budget that does not depend on the RPC at all. That belongs in
    // the daemon-side signing policy, which sees the decoded fees of every transaction it signs.
    let gwei = |wei: u128| format!("{} gwei", format_units(U256::from(wei), 9));
    let legacy = matches!(fees, EvmFees::Legacy { .. });
    let reference = if legacy {
        base_fee.max(REFERENCE_FLOOR)
    } else {
        base_fee.max(gas_price).max(REFERENCE_FLOOR)
    };
    let (tip_cap, price_cap, rule) = match explicit {
        Some(max) => (max, max, format!("--max-fee-gwei {}", gwei(max))),
        None if legacy => (
            reference,
            reference.saturating_mul(PRICE_CEILING_MULT),
            format!(
                "zwallet's legacy ceiling ({PRICE_CEILING_MULT} x max(baseFee {}, 1 gwei))",
                gwei(base_fee)
            ),
        ),
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
    /// `--bump` in ten-thousandths; applied whenever given (default [`DEFAULT_BUMP`] when
    /// replacing).
    pub bump: Option<u128>,
    /// `--replace`: the pending transaction being replaced, whose fees are outbid too.
    pub replace: Option<B256>,
    /// `--max-fee-gwei` in wei: the explicit fee ceiling.
    pub max_fee_per_gas: Option<u128>,
    /// `--allow-pending`: without `nonce`, send even while a transaction from `from` is pending.
    pub allow_pending: bool,
    /// The account's send journal, for naming earlier transactions in errors.
    pub journal: Option<SendJournal>,
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

    // Mined first: a transaction mined between the two reads then shows as pending (a refusal
    // to look it up), never the other way round.
    let mined = p
        .rpc(
            "eth_getTransactionCount",
            p.get_transaction_count(intent.from).latest(),
        )
        .await?;
    let pending = p
        .rpc(
            "eth_getTransactionCount",
            p.get_transaction_count(intent.from).pending(),
        )
        .await?;
    let in_flight = (pending > mined).then(|| mined..=pending - 1);
    let journal = intent.journal.as_ref();
    let nonce = match intent.nonce {
        None => {
            // A dry-run reports them (`pendingNonces`) instead, as it does a shortfall.
            if let Some(nonces) = &in_flight
                && !intent.allow_pending
                && !intent.dry_run
            {
                return Err(pending_guard_error(net, nonces.clone(), journal));
            }
            pending
        }
        Some(n) => {
            let n =
                check_nonce(n, mined, pending).map_err(|e| with_nonce_pointer(e, n, journal))?;
            if n < pending && intent.replace.is_none() {
                return Err(needs_replace_error(n, journal));
            }
            n
        }
    };
    let replacing = nonce < pending;
    let original = match intent.replace {
        Some(hash) => Some(original_fees(p, hash, intent.from, nonce, pending, journal).await?),
        None => None,
    };
    // `--replace` outbids the named transaction even where this node no longer counts it, and
    // an explicit `--bump` always applies: the agent asked for it, whatever this node holds.
    let outbid = (replacing || original.is_some() || intent.bump.is_some()).then(|| Outbid {
        bump: intent.bump.unwrap_or(DEFAULT_BUMP),
        original,
    });
    let fees = fees(p, intent.max_fee_per_gas, outbid).await?;
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
    for (k, v) in pending_fields(in_flight.as_ref()) {
        fields.insert(k.into(), v);
    }
    if intent.nonce.is_some() {
        fields.insert("replacesPending".into(), json!(replacing));
        let bump = outbid.map(|o| bump_label(o.bump));
        fields.insert("feeBump".into(), json!(bump));
    }
    if let Some(hash) = intent.replace {
        fields.insert("replacesTx".into(), json!(format!("{hash:#x}")));
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

/// Most pending nonces `pendingNonces` lists.
const PENDING_LISTED: usize = 64;

/// The nonces pending between the mined count and the next free nonce (`in_flight`), as output
/// fields: `pendingNonces` (the first [`PENDING_LISTED`]), `pendingRange` (`[first, last]`, or
/// null) and `pendingCount`. Both counts come from the node, which can claim any range, so the
/// range is never enumerated past the listed few.
fn pending_fields(in_flight: Option<&std::ops::RangeInclusive<u64>>) -> [(&'static str, Value); 3] {
    let Some(r) = in_flight else {
        return [
            ("pendingNonces", json!([])),
            ("pendingRange", Value::Null),
            ("pendingCount", json!(0)),
        ];
    };
    let listed: Vec<u64> = r.clone().take(PENDING_LISTED).collect();
    let count = (r.end() - r.start()).saturating_add(1);
    [
        ("pendingNonces", json!(listed)),
        ("pendingRange", json!([r.start(), r.end()])),
        ("pendingCount", json!(count)),
    ]
}

/// A send without `--nonce` while transactions from this key are pending at `nonces`: refused,
/// because the usual reason is a re-run of a send that was cut off (the harness killed it, the
/// output was lost), and sent again it would land at the next nonce as a second payment.
fn pending_guard_error(
    net: &Net,
    nonces: std::ops::RangeInclusive<u64>,
    journal: Option<&SendJournal>,
) -> WalletError {
    let at = nonce_label(&nonces);
    let (pointer, entries) = match journal {
        Some(j) => {
            let (text, entries) = j.pointer(nonces.clone(), None);
            (format!(" {text}."), entries)
        }
        None => (String::new(), Vec::new()),
    };
    let n = if nonces.start() == nonces.end() {
        nonces.start().to_string()
    } else {
        "<its nonce>".to_string()
    };
    let mut e = WalletError::validation(format!(
        "a transaction from this key is still pending on {} at {at}; nothing was \
         signed.{pointer} If this command repeats a payment sent earlier (for example a run that \
         was cut off), do not run it again: look that txHash up, and to speed it up or replace \
         it use --nonce {n} --replace <its txHash>. Only for a different payment, pass \
         --allow-pending to send it after the pending one",
        net.chain.key
    ))
    .with_detail("pendingSends", Value::Array(entries));
    for (k, v) in pending_fields(Some(&nonces)) {
        e = e.with_detail(k, v);
    }
    if let Some(j) = journal {
        e = e.with_detail("journal", j.display());
    }
    e
}

/// `--nonce n` below the pending nonce without `--replace`: it would replace the transaction
/// pending at `n` without saying which, so the agent must name it.
fn needs_replace_error(n: u64, journal: Option<&SendJournal>) -> WalletError {
    let (pointer, entries) = match journal {
        Some(j) => {
            let (text, entries) = j.pointer(n..=n, None);
            (format!(". {text}"), entries)
        }
        None => (String::new(), Vec::new()),
    };
    let mut e = WalletError::validation(format!(
        "nonce {n} belongs to a pending transaction; pass --replace <its txHash> to replace it \
         (see the send journal); nothing was signed{pointer}. {}",
        different_payment(n)
    ))
    .with_detail("nonce", n)
    .with_detail("pendingSends", Value::Array(entries));
    if let Some(j) = journal {
        e = e.with_detail("journal", j.display());
    }
    e
}

/// The transaction at nonce `n` may be another payment than the one being retried (the agent may
/// have the wrong nonce): replacing it would cancel that payment and still not send this one.
fn different_payment(n: u64) -> String {
    format!(
        "If the transaction at nonce {n} is a different payment than the one you are retrying, \
         do not replace it; that payment and yours are separate"
    )
}

/// `e` from [`check_nonce`] with the journal's entries at nonce `n` (each with how to look it
/// up), or where else to look.
fn with_nonce_pointer(e: WalletError, n: u64, journal: Option<&SendJournal>) -> WalletError {
    let Some(j) = journal else {
        return e;
    };
    let (text, _) = j.pointer(n..=n, None);
    WalletError {
        message: format!("{}. {text}", e.message),
        ..e
    }
    .with_detail("journal", j.display())
}

/// An explicit `--nonce n` must reuse a nonce that is still pending (`mined <= n < pending`, the
/// replacement case) or be the next free one (`n == pending`). Below `mined` it is already spent;
/// above `pending` the transaction would wait forever for the gap to fill.
///
/// `--nonce` is how a retry is made safe, so neither message names another nonce to use: an
/// agent that took one would turn its retry into a second send.
fn check_nonce(n: u64, mined: u64, pending: u64) -> Result<u64> {
    if n < mined {
        return Err(WalletError::validation(format!(
            "the transaction at nonce {n} is already mined; nothing was signed. If it was your \
             earlier attempt at this payment, the payment went through; look it up before \
             sending anything new. {}",
            different_payment(n)
        ))
        .with_detail("nonce", n));
    }
    if n > pending {
        return Err(WalletError::validation(format!(
            "--nonce {n} would leave a gap: it is past this account's next unused nonce on this \
             node, and a nonce is not mined until every earlier one is; nothing was signed. If \
             you are retrying a send that used nonce {n}, do not send it again under another \
             nonce: look up its txHash first (this node may be behind)"
        ))
        .with_detail("nonce", n));
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
    let replace = req.replace.as_deref().map(parse_tx_hash).transpose()?;
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
                bump: req.bump,
                replace,
                max_fee_per_gas: req.max_fee_per_gas,
                allow_pending: req.allow_pending,
                journal: req.journal.clone(),
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
                bump: req.bump,
                replace,
                max_fee_per_gas: req.max_fee_per_gas,
                allow_pending: req.allow_pending,
                journal: req.journal.clone(),
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
    /// The call failed, timed out, or answered another hash (`rpc_hash`), but the node reports
    /// the transaction by its own hash: it was sent. `why` says what went wrong.
    KnownDespite { why: String, rpc_hash: Option<B256> },
    /// The node answered with some other hash (a broken or dishonest endpoint), and does not
    /// report the transaction by its own hash.
    HashMismatch(B256),
    /// The call failed or timed out (the text), and the node does not report the transaction.
    Unknown(String),
    /// The node says the nonce is already used (`why`; see [`SEND_NONCE_TAKEN`]) and does not
    /// report this transaction: another transaction, pending or mined, holds the nonce. It may be
    /// an earlier send of the same payment. `underpriced`: the node's raw answer was "replacement
    /// transaction underpriced".
    NonceTaken { why: String, underpriced: bool },
    /// The node refused the transaction itself (`why`; see [`SEND_REJECTIONS`]): nothing was
    /// broadcast. `underpriced`: the node's raw answer was "transaction underpriced" (a price
    /// below the pool's minimum).
    Rejected { why: String, underpriced: bool },
}

/// How a JSON-RPC error message from `eth_sendRawTransaction` reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SendError {
    /// The node refused the transaction ([`SEND_REJECTIONS`]): nothing was broadcast.
    Rejected,
    /// The nonce is used ([`SEND_NONCE_TAKEN`]), perhaps by this very transaction.
    NonceTaken,
    /// The node already holds it ([`SEND_KNOWN`]): the lookup by hash then reports it as sent.
    Known,
    /// Anything else: only the lookup by hash can tell.
    Other,
}

/// Does `phrase` occur in `text` as whole words (no letter or digit right before or after)?
fn has_phrase(text: &str, phrase: &str) -> bool {
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    text.match_indices(phrase).any(|(i, _)| {
        !word(text[..i].chars().last()) && !word(text[i + phrase.len()..].chars().next())
    })
}

fn classify_send_error(message: &str) -> SendError {
    let m = message.to_lowercase();
    let has = |list: &[&str]| list.iter().any(|s| m.contains(s));
    let known = has_phrase(&m, SEND_KNOWN_PHRASE)
        || m.trim_start()
            .strip_prefix(SEND_KNOWN_PREFIX)
            .is_some_and(|rest| !rest.starts_with(|c: char| c.is_alphanumeric()));
    if known {
        SendError::Known
    } else if has(SEND_NONCE_TAKEN) {
        SendError::NonceTaken
    } else if has(SEND_REJECTIONS) {
        SendError::Rejected
    } else {
        SendError::Other
    }
}

/// Sends `raw`, whose hash the caller computed from the bytes. The node's reply is only checked
/// against that hash, never used in its place. A JSON-RPC error that refuses the transaction
/// itself is [`SendOutcome::Rejected`]. Every other outcome short of the node answering that
/// hash (a used nonce, another hash, "already known", a timeout, a transport error) asks the
/// node for the transaction by `hash` before giving up.
pub async fn send_raw(p: &EvmProvider, raw: &[u8], hash: B256) -> SendOutcome {
    const WHAT: &str = "eth_sendRawTransaction";
    let call = p
        .client()
        .request::<_, B256>(WHAT, (alloy::hex::encode_prefixed(raw),));
    let found = |why: String, rpc_hash: Option<B256>| SendOutcome::KnownDespite { why, rpc_hash };
    let why = match tokio::time::timeout(RPC_TIMEOUT, call.into_future()).await {
        Ok(Ok(h)) if h == hash => return SendOutcome::Accepted,
        Ok(Ok(h)) => {
            return if node_has_tx(p, hash).await {
                let why = format!(
                    "the RPC answered {WHAT} with hash {h}, not the transaction's own hash"
                );
                found(why, Some(h))
            } else {
                SendOutcome::HashMismatch(h)
            };
        }
        Ok(Err(e)) => {
            // `why` is the scrubbed text for printing; the classification and every hint come
            // from the node's raw message, which scrubbing could have cut into.
            let why = p.failed(WHAT, &e).message;
            // Only a JSON-RPC error object (HTTP 200, or a 4xx carrying one) is the node's
            // answer; a bare transport error says nothing about the transaction.
            let raw = e.as_error_resp().map(|r| r.message.to_lowercase());
            let underpriced = raw.as_deref().is_some_and(|m| m.contains("underpriced"));
            match raw.as_deref().map_or(SendError::Other, classify_send_error) {
                SendError::Rejected => return SendOutcome::Rejected { why, underpriced },
                SendError::NonceTaken if node_has_tx(p, hash).await => return found(why, None),
                SendError::NonceTaken => return SendOutcome::NonceTaken { why, underpriced },
                SendError::Known | SendError::Other => why,
            }
        }
        Err(_) => timed_out(WHAT).message,
    };
    if node_has_tx(p, hash).await {
        found(why, None)
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

/// `zwallet tx`: `hash` as this node reports it, from `eth_getTransactionByHash` and
/// `eth_getTransactionReceipt`, both read as plain JSON (so chain-specific transaction and receipt
/// types cannot fail the lookup). Mined when there is a receipt or the transaction names its
/// block, pending when the node holds it unmined, not found otherwise. An answer whose hash is
/// not `hash` is `RPC_ERROR`: it says nothing about this transaction, and calling it "not found"
/// could send the agent into a second payment. "Pending" and "not found" add a `hint` that keeps
/// a retry on the transaction's own nonce.
pub async fn lookup_tx(p: &EvmProvider, net: &Net, hash: B256) -> Result<Map<String, Value>> {
    let chain = net.chain;
    let shown = format!("{hash:#x}");
    let get = |method: &'static str| p.client().request::<_, Option<Value>>(method, (hash,));
    let tx = p
        .rpc("eth_getTransactionByHash", get("eth_getTransactionByHash"))
        .await?;
    let receipt = p
        .rpc(
            "eth_getTransactionReceipt",
            get("eth_getTransactionReceipt"),
        )
        .await?;
    for (answer, key, method) in [
        (&tx, "hash", "eth_getTransactionByHash"),
        (&receipt, "transactionHash", "eth_getTransactionReceipt"),
    ] {
        let Some(v) = answer else { continue };
        let got = v.get(key).and_then(Value::as_str);
        if got.and_then(|h| h.parse::<B256>().ok()) != Some(hash) {
            return Err(WalletError::rpc(format!(
                "{method} for {shown} answered about transaction {}, so the node says nothing \
                 about this one; look it up on {}",
                got.unwrap_or("<none>"),
                chain.explorer_tx_url(&shown)
            )));
        }
    }
    let field = |k: &str| {
        tx.as_ref()
            .and_then(|t| t.get(k))
            .or_else(|| receipt.as_ref().and_then(|r| r.get(k)))
            .and_then(Value::as_str)
    };
    let quantity = |v: Option<&str>| {
        v.and_then(|h| h.strip_prefix("0x"))
            .and_then(|h| U256::from_str_radix(h, 16).ok())
    };
    let address = |k: &str| {
        field(k)
            .and_then(|a| a.parse::<Address>().ok())
            .map(|a| a.to_checksum(None))
    };
    let nonce = tx
        .as_ref()
        .and_then(|t| quantity(t.get("nonce").and_then(Value::as_str)))
        .and_then(|n| u64::try_from(n).ok());
    let value = tx
        .as_ref()
        .and_then(|t| quantity(t.get("value").and_then(Value::as_str)));
    let block = quantity(field("blockNumber")).and_then(|n| u64::try_from(n).ok());
    let found = tx.is_some() || receipt.is_some();
    let status = match (found, receipt.is_some() || block.is_some()) {
        (false, _) => "not found",
        (true, true) => "mined",
        (true, false) => "pending",
    };
    let mut out = Map::new();
    out.insert("chain".into(), json!(chain.key));
    out.insert("txHash".into(), json!(shown));
    out.insert("found".into(), json!(found));
    out.insert("status".into(), json!(status));
    out.insert("from".into(), json!(address("from")));
    out.insert("to".into(), json!(address("to")));
    out.insert("nonce".into(), json!(nonce));
    out.insert(
        "value".into(),
        json!(value.map(|v| format_units(v, chain.decimals))),
    );
    out.insert("valueRaw".into(), json!(value.map(|v| v.to_string())));
    out.insert("symbol".into(), json!(chain.symbol));
    out.insert("explorerUrl".into(), json!(chain.explorer_tx_url(&shown)));
    if status == "mined" {
        out.insert("blockNumber".into(), json!(block));
    }
    if let Some(r) = &receipt {
        let used =
            quantity(r.get("gasUsed").and_then(Value::as_str)).and_then(|g| u64::try_from(g).ok());
        out.insert("gasUsed".into(), json!(used));
        if let Some(ok) = r.get("status").and_then(Value::as_str) {
            out.insert(
                "success".into(),
                json!(quantity(Some(ok)) == Some(U256::from(1))),
            );
        }
    }
    let hint = match (status, nonce) {
        ("pending", Some(n)) => Some(crate::chains::pending_hint("still pending", n, &shown)),
        ("pending", None) => Some(
            "still pending; to speed up or cancel it, retry with --nonce <its nonce> --replace \
             <its txHash>; a run without --nonce is a second payment"
                .to_string(),
        ),
        ("not found", _) => Some(
            "this node does not know the transaction; that does not mean it was never sent: it \
             may still reach the network, be held by other nodes, or have been dropped. Do not \
             send this payment again without --nonce: retry it only with --nonce <the nonce it \
             was signed with> (in its own output or the send journal), so that the retry can \
             take its place but never add a second payment"
                .to_string(),
        ),
        _ => None,
    };
    if let Some(h) = hint {
        out.insert("hint".into(), json!(h));
    }
    Ok(out)
}

/// Polls for the receipt until `timeout`; `Ok(None)` if it has not been mined by then. A timeout
/// past what `Instant` can hold waits without end.
pub async fn wait_receipt(p: &EvmProvider, hash: B256, timeout: Duration) -> Result<Option<Value>> {
    let deadline = Instant::now().checked_add(timeout);
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
        if deadline.is_some_and(|d| Instant::now() + RECEIPT_POLL > d) {
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
    fn legacy_ceiling_does_not_use_the_price_it_checks() {
        // On a legacy chain the price *is* eth_gasPrice, so it cannot be its own reference: the
        // reference is max(base fee, 1 gwei), and BSC (base fee 0) caps at 6 gwei.
        let legacy = |gwei: u128| EvmFees::Legacy {
            gas_price: U128::from(gwei * G),
        };
        assert!(check_fee_ceiling(&legacy(6), 0, 6 * G, None).is_ok());
        let e = check_fee_ceiling(&legacy(7), 0, 7 * G, None).unwrap_err();
        assert!(e.message.contains("gasPrice 7 gwei"), "{}", e.message);
        assert!(
            e.message.contains("6 x max(baseFee 0 gwei, 1 gwei)"),
            "{}",
            e.message
        );
        assert!(!e.message.contains("eth_gasPrice"), "{}", e.message);
        assert!(check_fee_ceiling(&legacy(7), 0, 7 * G, Some(8 * G)).is_ok());
        // An EIP-1559 chain still counts eth_gasPrice in its reference.
        let price = EvmFees::Eip1559 {
            max_fee_per_gas: U128::from(7 * G),
            max_priority_fee_per_gas: U128::from(G),
        };
        assert!(check_fee_ceiling(&price, G, 2 * G, None).is_ok());
    }

    #[test]
    fn explicit_nonce_must_be_pending_or_next() {
        // 5 mined, 7 = next free: 5 and 6 replace pending transactions, 7 is new.
        for n in [5, 6, 7] {
            assert_eq!(check_nonce(n, 5, 7).unwrap(), n);
        }
        let mined = check_nonce(4, 5, 7).unwrap_err().message;
        assert_eq!(
            mined,
            "the transaction at nonce 4 is already mined; nothing was signed. If it was your \
             earlier attempt at this payment, the payment went through; look it up before \
             sending anything new. If the transaction at nonce 4 is a different payment than the \
             one you are retrying, do not replace it; that payment and yours are separate"
        );
        for e in [mined, check_nonce(8, 5, 7).unwrap_err().message] {
            // Neither names a nonce to use instead.
            assert!(!e.contains('5') && !e.contains('7'), "{e}");
        }
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

    const QUARTER: Outbid = Outbid {
        bump: DEFAULT_BUMP,
        original: None,
    };

    #[test]
    fn replacement_fees_rise_by_a_quarter_rounded_up() {
        let bumped = replacement_fees(eip1559(G, G / 10), QUARTER);
        assert_eq!(
            bumped,
            EvmFees::Eip1559 {
                // (2 x 1 + 0.1) gwei x 1.25, and 0.1 gwei x 1.25.
                max_fee_per_gas: U128::from(2_625_000_000u64),
                max_priority_fee_per_gas: U128::from(125_000_000u64),
            }
        );
        // A 1 wei tip (Arbitrum, OP Mainnet) must still rise, or the node sees no bump.
        let tiny = replacement_fees(
            EvmFees::Eip1559 {
                max_fee_per_gas: U128::from(3u64),
                max_priority_fee_per_gas: U128::from(1u64),
            },
            QUARTER,
        );
        assert_eq!(
            tiny,
            EvmFees::Eip1559 {
                max_fee_per_gas: U128::from(4u64),
                max_priority_fee_per_gas: U128::from(2u64),
            }
        );
        let legacy = replacement_fees(
            EvmFees::Legacy {
                gas_price: U128::from(3 * G),
            },
            QUARTER,
        );
        assert_eq!(legacy.max_price(), 3_750_000_000);
        // The minimum 1.1x still lifts a 1 wei tip.
        let min = Outbid {
            bump: 11_000,
            original: None,
        };
        assert_eq!(
            replacement_fees(eip1559(0, 1), min),
            EvmFees::Eip1559 {
                max_fee_per_gas: U128::from(2u64),
                max_priority_fee_per_gas: U128::from(2u64),
            }
        );
    }

    #[test]
    fn replacement_outbids_the_original_when_its_fees_are_higher() {
        let original = OriginalFees {
            max_fee: 10 * G,
            tip: 2 * G,
        };
        let outbid = Outbid {
            bump: 15_000,
            original: Some(original),
        };
        // Estimate 2.1 gwei / 0.1 gwei: the original's fees win, times 1.5.
        assert_eq!(
            replacement_fees(eip1559(G, G / 10), outbid),
            EvmFees::Eip1559 {
                max_fee_per_gas: U128::from(15 * G),
                max_priority_fee_per_gas: U128::from(3 * G),
            }
        );
        // A legacy estimate outbids the original's fee cap.
        let legacy = EvmFees::Legacy {
            gas_price: U128::from(G),
        };
        assert_eq!(replacement_fees(legacy, outbid).max_price(), 15 * G);
        // A higher estimate wins over a low original.
        let low = Outbid {
            bump: 15_000,
            original: Some(OriginalFees { max_fee: 1, tip: 1 }),
        };
        assert_eq!(
            replacement_fees(eip1559(4 * G, G), low).max_price(),
            (9 * G * 3) / 2
        );
        // A node that reports tip > cap for the original cannot make the tip exceed the cap.
        let odd = Outbid {
            bump: DEFAULT_BUMP,
            original: Some(OriginalFees {
                max_fee: G,
                tip: 8 * G,
            }),
        };
        match replacement_fees(eip1559(G, G / 10), odd) {
            EvmFees::Eip1559 {
                max_fee_per_gas,
                max_priority_fee_per_gas,
            } => assert!(max_priority_fee_per_gas <= max_fee_per_gas),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn send_errors_are_classified_case_insensitively() {
        for m in [
            "INSUFFICIENT FUNDS for gas * price + value",
            "intrinsic gas too low",
            "tx exceeds block gas limit",
            "max fee per gas less than block base fee: address 0xabc",
        ] {
            assert_eq!(classify_send_error(m), SendError::Rejected, "{m}");
        }
        for m in [
            "replacement transaction underpriced",
            "Nonce too low: address 0xabc, tx: 1 state: 2",
        ] {
            assert_eq!(classify_send_error(m), SendError::NonceTaken, "{m}");
        }
        for m in [
            "already known",
            "ALREADY KNOWN",
            "Known transaction: 0xabc",
            "known transaction",
            "tx already known: 0xabc",
        ] {
            assert_eq!(classify_send_error(m), SendError::Known, "{m}");
        }
        for m in ["internal error", "rate limited", "", "unknown transaction"] {
            assert_eq!(classify_send_error(m), SendError::Other, "{m}");
        }
        // "unknown transaction" is not "known transaction": the rest of the text decides.
        assert_eq!(
            classify_send_error("unknown transaction type; insufficient funds"),
            SendError::Rejected
        );
        assert_eq!(
            classify_send_error("unknown transaction: nonce too low"),
            SendError::NonceTaken
        );
        // A price below the pool's minimum is refused outright; a replacement that does not
        // outbid the pending one is a used nonce (looked up first).
        for m in [
            "transaction underpriced",
            "Transaction underpriced: gas tip cap 1, minimum needed 25000000",
        ] {
            assert_eq!(classify_send_error(m), SendError::Rejected, "{m}");
        }
        assert_eq!(
            classify_send_error("REPLACEMENT transaction underpriced"),
            SendError::NonceTaken
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn serde_errors_never_print_the_value_they_quote() {
        use alloy::transports::{RpcError, TransportError};
        // A key with a quote and a backslash in it: serde quotes a mistyped value escaped
        // (`\"`, `\\`), which no scrub of the key as written would match.
        let key = r#"SEC"RET\KEY99"#;
        let net = Net {
            chain: crate::chains::find("ethereum").unwrap(),
            rpc_url: "https://eth.example.com/?apikey=SEC%22RET%5CKEY99".into(),
        };
        let p = EvmProvider::new(&net).unwrap();
        let echoed = serde_json::to_string(&format!("invalid key {key}")).unwrap();
        // A real type mismatch: a string where a number was expected.
        let err = serde_json::from_str::<u64>(&echoed).unwrap_err();
        assert!(err.to_string().contains(r#"SEC\"RET\\KEY99"#), "{err}");
        let err: TransportError = RpcError::DeserError {
            err,
            text: echoed.clone(),
        };
        let e = p
            .rpc("eth_chainId", std::future::ready(Err::<(), _>(err)))
            .await
            .unwrap_err();
        for part in ["SEC", "RET", "KEY99", "invalid key"] {
            assert!(!e.message.contains(part), "{part}: {}", e.message);
        }
        assert_eq!(
            e.message,
            "eth_chainId failed: the endpoint's answer is not valid JSON-RPC (a data type error \
             at line 1 column 29)"
        );
        // A JSON-RPC error's `data`: a string is decoded (then scrubbed), anything else dropped.
        let resp = |data: &str| -> TransportError {
            RpcError::ErrorResp(
                serde_json::from_str(&format!(
                    r#"{{"code":3,"message":"execution reverted","data":{data}}}"#
                ))
                .unwrap(),
            )
        };
        for data in [
            serde_json::to_string(&format!("key {key}")).unwrap(),
            format!(r#"{{"note":{echoed}}}"#),
        ] {
            let e = p
                .rpc("eth_call", std::future::ready(Err::<(), _>(resp(&data))))
                .await
                .unwrap_err();
            for part in ["RET", "KEY99"] {
                assert!(!e.message.contains(part), "{part}: {}", e.message);
            }
        }
        let e = p
            .rpc(
                "eth_call",
                std::future::ready(Err::<(), _>(resp(r#""0x08c379a0""#))),
            )
            .await
            .unwrap_err();
        assert!(e.message.ends_with("data: 0x08c379a0"), "{}", e.message);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn http_error_origin_redacts_a_key_like_host_label() {
        use alloy::transports::TransportErrorKind;
        let net = Net {
            chain: crate::chains::find("ethereum").unwrap(),
            rpc_url: "https://deadbeef0123.eth.example.com".into(),
        };
        let p = EvmProvider::new(&net).unwrap();
        let e = p
            .rpc(
                "eth_call",
                std::future::ready(Err::<(), _>(TransportErrorKind::http_error(
                    403,
                    "no".into(),
                ))),
            )
            .await
            .unwrap_err();
        assert_eq!(
            e.message,
            "eth_call failed: HTTP 403 from https://<redacted>.eth.example.com"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn rpc_error_text_never_prints_the_endpoint_key() {
        use alloy::transports::{RpcError, TransportError, TransportErrorKind};
        let net = Net {
            chain: crate::chains::find("ethereum").unwrap(),
            rpc_url: "https://eth.example.com/v3/0123456789abcdef?apikey=SECRETKEY99&chain=1"
                .into(),
        };
        let p = EvmProvider::new(&net).unwrap();
        // A non-2xx body: only the status and the endpoint's origin are printed.
        let body = "{\"error\":\"invalid project id 0123456789abcdef\",\"path\":\
                    \"/v3/0123456789abcdef?apikey=SECRETKEY99&chain=1\"} (apikey SECRETKEY99)";
        let e = p
            .rpc(
                "eth_call",
                std::future::ready(Err::<(), _>(TransportErrorKind::http_error(
                    401,
                    body.into(),
                ))),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::RpcError);
        assert_eq!(
            e.message,
            "eth_call failed: HTTP 401 from https://eth.example.com"
        );
        // A JSON-RPC error that quotes the key, URL-decoded or not.
        let err: TransportError = RpcError::ErrorResp(
            serde_json::from_str(
                r#"{"code":-32000,"message":"key 0123456789abcdef or SECRETKEY99 rejected at block 1"}"#,
            )
            .unwrap(),
        );
        let e = p
            .rpc("eth_call", std::future::ready(Err::<(), _>(err)))
            .await
            .unwrap_err();
        for key in ["0123456789abcdef", "SECRETKEY99"] {
            assert!(!e.message.contains(key), "{key}: {}", e.message);
        }
        assert!(e.message.contains("error code -32000"), "{}", e.message);
        // A 200 answer that is not JSON-RPC: its text is the endpoint's, so it is dropped.
        let err: TransportError = RpcError::DeserError {
            err: serde_json::from_str::<u64>("x").unwrap_err(),
            text: "<html>gateway for key SECRETKEY99</html>".into(),
        };
        let e = p
            .rpc("eth_call", std::future::ready(Err::<(), _>(err)))
            .await
            .unwrap_err();
        assert!(!e.message.contains("SECRETKEY99"), "{}", e.message);
        assert!(!e.message.contains("html"), "{}", e.message);
    }
}
