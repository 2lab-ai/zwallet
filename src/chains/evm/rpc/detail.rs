//! Shaped, read-only transaction detail. Node responses are claims; this module never supplies
//! replacement intent or claims that a returned `from` is a recovered signing identity.
use super::{EvmProvider, fetch_by_hash, fetch_receipt, quantity, receipt_success};
use crate::chains::{JournalRow, Net};
use crate::core::error::{Result, WalletError};
use crate::history::{State, TokenMovement};
use alloy::primitives::{Address, B256, U256, keccak256};
use alloy::providers::Provider;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MovementsState {
    Confirmed,
    Reverted,
    Pending,
    Unknown,
}
impl MovementsState {
    pub fn note(&self) -> &'static str {
        match self {
            Self::Confirmed => "token movements from this node's receipt",
            Self::Reverted => "no token moved: transaction reverted",
            Self::Pending => "token movements unknown until it is mined",
            Self::Unknown => "token movements unknown: execution outcome unavailable",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TxDetail {
    pub chain: &'static str,
    pub tx_hash: String,
    pub state: State,
    pub success: Option<bool>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub contract_creation: bool,
    pub nonce: Option<u64>,
    /// The local row's nonce, labelled separately when this node cannot provide one.
    /// It remains an untrusted claim and never authorizes a replacement.
    pub journal_nonce: Option<u64>,
    pub value: Option<U256>,
    pub block_number: Option<u64>,
    pub gas_used: Option<U256>,
    pub effective_gas_price: Option<U256>,
    pub l1_fee: Option<U256>,
    pub fee: Option<U256>,
    pub confirmations: Option<u64>,
    pub movements: Vec<TokenMovement>,
    pub movements_state: MovementsState,
    pub hidden_movements: usize,
    pub hint: &'static str,
}

const NOT_FOUND: &str = "this node does not know the transaction; it may still reach the network or be held by another node. Look up the original hash and nonce before another payment";
const DROPPED: &str = "this node reports that the nonce is consumed without a receipt for this hash; another transaction may have replaced it. This does not prove whether the payment happened";

struct Snapshot {
    tx: Option<Value>,
    receipt: Option<Value>,
    state: State,
}
fn addr(v: Option<&Value>) -> Option<Address> {
    let text = v?.as_str()?;
    if text.len() != 42 || !text.starts_with("0x") {
        return None;
    }
    text.parse().ok()
}
fn number(v: Option<&Value>) -> Option<U256> {
    let s = v?.as_str()?;
    if s.len() > 66
        || !s.starts_with("0x")
        || s.len() < 3
        || !s[2..].bytes().all(|b| b.is_ascii_hexdigit())
    {
        return None;
    }
    quantity(Some(s))
}
fn small(v: Option<&Value>) -> Option<u64> {
    u64::try_from(number(v)?).ok()
}
fn block(tx: Option<&Value>, receipt: Option<&Value>) -> Option<u64> {
    receipt
        .and_then(|r| small(r.get("blockNumber")))
        .or_else(|| tx.and_then(|t| small(t.get("blockNumber"))))
}
async fn snapshot(
    p: &EvmProvider,
    net: &Net,
    hash: B256,
    journal: Option<&JournalRow>,
) -> Result<Snapshot> {
    if p.net.chain != net.chain || p.net.rpc_url != net.rpc_url {
        return Err(WalletError::validation(
            "transaction provider does not match the selected chain and endpoint",
        ));
    }
    let tx = fetch_by_hash(p, "eth_getTransactionByHash", "hash", hash).await?;
    let receipt = fetch_receipt(p, hash).await?;
    // If both replies say which block included this hash, a disagreement is not a coherent
    // mined snapshot. Do not combine its fee, logs and confirmations into a false detail.
    if let (Some(a), Some(b)) = (
        tx.as_ref().and_then(|v| small(v.get("blockNumber"))),
        receipt.as_ref().and_then(|v| small(v.get("blockNumber"))),
    ) && a != b
    {
        return Err(WalletError::rpc(
            "transaction and receipt disagree about the mined block",
        ));
    }
    if let (Some(a), Some(b)) = (
        tx.as_ref().and_then(|v| v.get("blockHash")).and_then(word),
        receipt
            .as_ref()
            .and_then(|v| v.get("blockHash"))
            .and_then(word),
    ) && a != b
    {
        return Err(WalletError::rpc(
            "transaction and receipt disagree about the mined block hash",
        ));
    }
    let mut state = if receipt.is_some() || block(tx.as_ref(), None).is_some() {
        State::Mined
    } else if tx.is_some() {
        State::Pending
    } else {
        State::NotFound
    };
    if receipt.is_none() && state != State::Mined {
        let claimed = tx
            .as_ref()
            .and_then(|v| Some((addr(v.get("from"))?, small(v.get("nonce"))?)));
        let saved = journal
            .filter(|j| {
                j.chain == net.chain.key && j.tx_hash.eq_ignore_ascii_case(&format!("{hash:#x}"))
            })
            .and_then(|j| Some((j.from.parse::<Address>().ok()?, j.nonce?)));
        if let (Some(a), Some(b)) = (claimed, saved)
            && a != b
        {
            return Err(WalletError::rpc(
                "transaction disagrees with the journal's sender or nonce; neither supplies a replacement intent",
            ));
        }
        if let Some((from, nonce)) = claimed.or(saved) {
            let call = p
                .client()
                .request::<_, String>("eth_getTransactionCount", (from, "latest"));
            let latest = p.rpc("eth_getTransactionCount", call).await?;
            let latest = small(Some(&Value::String(latest))).ok_or_else(|| {
                WalletError::rpc("eth_getTransactionCount returned an invalid quantity")
            })?;
            if latest > nonce {
                state = State::DroppedReplaced;
            }
        }
    }
    Ok(Snapshot { tx, receipt, state })
}
/// The node's state, with a shaped journal row only as an informational nonce fallback.
/// It does not establish the intent or signing identity needed to replace this transaction.
pub async fn tx_state(
    p: &EvmProvider,
    net: &Net,
    hash: B256,
    journal: Option<&JournalRow>,
) -> Result<State> {
    Ok(snapshot(p, net, hash, journal).await?.state)
}

pub async fn tx_detail(
    p: &EvmProvider,
    net: &Net,
    hash: B256,
    wallet: Address,
    journal: Option<&JournalRow>,
) -> Result<TxDetail> {
    let Snapshot { tx, receipt, state } = snapshot(p, net, hash, journal).await?;
    let tx = tx.as_ref();
    let receipt = receipt.as_ref();
    let block_number = block(tx, receipt);
    let success = receipt.and_then(receipt_success);
    let from = tx
        .and_then(|v| addr(v.get("from")))
        .or_else(|| receipt.and_then(|v| addr(v.get("from"))));
    let to = tx
        .and_then(|v| addr(v.get("to")))
        .or_else(|| receipt.and_then(|v| addr(v.get("to"))));
    let gas_used = receipt.and_then(|v| number(v.get("gasUsed")));
    let effective_gas_price = receipt.and_then(|v| number(v.get("effectiveGasPrice")));
    let op_stack = matches!(net.chain.key, "base" | "optimism");
    let l1_fee = if op_stack {
        receipt.and_then(|v| number(v.get("l1Fee")))
    } else {
        None
    };
    let fee = match (gas_used, effective_gas_price) {
        (Some(gas), Some(price)) => {
            let gas_fee = gas
                .checked_mul(price)
                .ok_or_else(|| WalletError::rpc("receipt fee overflows 256 bits"))?;
            if op_stack {
                l1_fee
                    .map(|l1| {
                        gas_fee
                            .checked_add(l1)
                            .ok_or_else(|| WalletError::rpc("receipt fee overflows 256 bits"))
                    })
                    .transpose()?
            } else {
                Some(gas_fee)
            }
        }
        _ => None,
    };
    let confirmations = if let Some(block) = block_number {
        let head = p.rpc("eth_blockNumber", p.get_block_number()).await?;
        // Reorg/racing/incorrect head below the receipt's block is unknown, never zero/mined.
        head.checked_sub(block).and_then(|n| n.checked_add(1))
    } else {
        None
    };
    let movements_state = match (receipt, success, state) {
        (Some(_), Some(true), _) => MovementsState::Confirmed,
        (Some(_), Some(false), _) => MovementsState::Reverted,
        (None, _, State::Pending) => MovementsState::Pending,
        _ => MovementsState::Unknown,
    };
    let (movements, hidden_movements) = if movements_state == MovementsState::Confirmed {
        movements(net, wallet, hash, receipt.expect("confirmed receipt"))?
    } else {
        (Vec::new(), 0)
    };
    Ok(TxDetail {
        chain: net.chain.key,
        tx_hash: format!("{hash:#x}"),
        state,
        success,
        from: from.map(|a| a.to_checksum(None)),
        to: to.map(|a| a.to_checksum(None)),
        contract_creation: tx.is_some_and(|tx| tx.get("to") == Some(&Value::Null)),
        nonce: tx.and_then(|v| small(v.get("nonce"))),
        journal_nonce: journal
            .filter(|j| {
                j.chain == net.chain.key && j.tx_hash.eq_ignore_ascii_case(&format!("{hash:#x}"))
            })
            .and_then(|j| j.nonce),
        value: tx.and_then(|v| number(v.get("value"))),
        block_number,
        gas_used,
        effective_gas_price,
        l1_fee,
        fee,
        confirmations,
        movements,
        movements_state,
        hidden_movements,
        hint: match state {
            State::NotFound => NOT_FOUND,
            State::DroppedReplaced => DROPPED,
            _ => "",
        },
    })
}
fn word(v: &Value) -> Option<[u8; 32]> {
    let text = v.as_str()?.strip_prefix("0x")?;
    if text.len() != 64 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    alloy::hex::decode_to_array(text).ok()
}
fn topic_address(v: &Value) -> Option<Address> {
    let word = word(v)?;
    if word[..12].iter().any(|&b| b != 0) {
        return None;
    }
    Some(Address::from_slice(&word[12..]))
}
fn movements(
    net: &Net,
    wallet: Address,
    hash: B256,
    receipt: &Value,
) -> Result<(Vec<TokenMovement>, usize)> {
    let logs = receipt
        .get("logs")
        .and_then(Value::as_array)
        .ok_or_else(|| WalletError::rpc("receipt has no valid logs array"))?;
    if logs.len() > 2048 {
        return Err(WalletError::rpc(
            "receipt exceeds the 2048-log detail limit",
        ));
    }
    let signature = keccak256("Transfer(address,address,uint256)");
    let mut shown = Vec::new();
    let mut hidden = 0;
    for log in logs {
        let topics = log.get("topics").and_then(Value::as_array);
        // Other events are irrelevant; unknown/malformed Transfer-like logs are counted.
        let Some(topics) = topics else {
            hidden += 1;
            continue;
        };
        if topics.first().and_then(word).map(B256::from) != Some(signature) {
            continue;
        }
        let parsed = || -> Option<TokenMovement> {
            if topics.len() != 3 || log.get("removed").is_some_and(|v| v != &Value::Bool(false)) {
                return None;
            }
            if let Some(id) = log.get("transactionHash")
                && word(id).map(B256::from) != Some(hash)
            {
                return None;
            }
            let token = addr(log.get("address"))?;
            let known =
                crate::chains::evm::registry::lookup(net.chain.chain_id_json().as_u64()?, &token)?;
            let (from, to) = (topic_address(&topics[1])?, topic_address(&topics[2])?);
            let amount = U256::from_be_bytes(word(log.get("data")?)?);
            if amount.is_zero() || (from != wallet && to != wallet) {
                return None;
            }
            Some(TokenMovement {
                token: token.to_checksum(None),
                symbol: known.symbol,
                decimals: known.decimals,
                from: from.to_checksum(None),
                to: to.to_checksum(None),
                amount,
                log_index: u32::try_from(small(log.get("logIndex"))?).ok()?,
            })
        };
        match parsed() {
            Some(m) => {
                if shown
                    .iter()
                    .any(|old: &TokenMovement| old.log_index == m.log_index)
                {
                    hidden += 1;
                } else {
                    shown.push(m);
                }
            }
            None => hidden += 1,
        }
    }
    shown.sort_by_key(|m| m.log_index);
    Ok((shown, hidden))
}
