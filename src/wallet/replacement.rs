//! Fixed-nonce replacements. A journal claim alone can never construct an original intent.
use super::send::{RetainedTransfer, SendOptions, SendOutput, send_prepared};
use super::{CheckedList, KeyRow};
use crate::chains::evm::{self, Evm, policy, rpc, tx};
use crate::chains::{self, Chain, Family, JournalRow, Net, PreparedTx, TransferRequest};
use crate::core::amount::format_units;
use crate::core::error::{Result, WalletError};
use crate::core::whitelist::{self, EntryKind};
use crate::history::State;
use alloy::primitives::{Address, B256, Bytes, U256};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

pub const CANCEL_RACE: &str = "a cancel replaces nonce N only if it is mined first; if the original is mined first, the payment stands and the cancel fails";
pub const RETRY_WARNING: &str = "the node does not know this hash; that is no proof the send failed. A new send is a new payment at a new nonce: it can pay twice, also without d (the repeat check looks back 10 minutes, in a journal any process of this user can edit). Look the hash up first.";
const DISAGREE: &str = "the journal and the node disagree about the original transaction; look it up with zwallet tx; nothing was signed";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Transfer,
    Token,
    Invoke,
}
impl Kind {
    fn label(self) -> &'static str {
        match self {
            Self::Transfer => "transfer",
            Self::Token => "token-transfer",
            Self::Invoke => "invoke",
        }
    }
}

/// An immutable original, constructed only from this session's review or a verified node
/// transaction matched to a journal row. It holds no password, secret key or raw signed bytes.
#[derive(Clone, Debug)]
pub struct OriginalIntent {
    chain: &'static Chain,
    key_name: String,
    index: u32,
    from: Address,
    decoded: tx::DecodedUnsigned,
    kind: Kind,
    /// None is a retained in-session review. Some binds a recovered node original to its hash.
    node_hash: Option<B256>,
}

impl OriginalIntent {
    pub fn from_transfer(review: &RetainedTransfer) -> Result<Self> {
        let kind = match review.decoded().action() {
            tx::Action::NativeTransfer { .. } => Kind::Transfer,
            tx::Action::TransferCall { .. } => Kind::Token,
            _ => {
                return Err(WalletError::validation(
                    "retained transfer has another action",
                ));
            }
        };
        Ok(Self {
            chain: review.chain(),
            key_name: review.key_name().into(),
            index: review.index(),
            from: evm::parse_address(review.from())?,
            decoded: review.decoded().clone(),
            kind,
            node_hash: None,
        })
    }

    /// Only the invoke review can construct this retained provenance-bearing value.
    pub fn from_invoke(review: &super::tools::RetainedInvoke) -> Result<Self> {
        Ok(Self {
            chain: review.chain(),
            key_name: review.key_name().to_string(),
            index: review.index(),
            from: evm::parse_address(review.from())?,
            decoded: review.decoded().clone(),
            kind: Kind::Invoke,
            node_hash: None,
        })
    }
    pub fn chain(&self) -> &'static Chain {
        self.chain
    }
    pub fn key_name(&self) -> &str {
        &self.key_name
    }
    pub fn index(&self) -> u32 {
        self.index
    }
    pub fn from(&self) -> String {
        self.from.to_checksum(None)
    }
    pub fn nonce(&self) -> u64 {
        self.decoded.nonce()
    }
    pub fn kind(&self) -> &'static str {
        self.kind.label()
    }
    pub fn decoded(&self) -> &tx::DecodedUnsigned {
        &self.decoded
    }
    pub fn is_retained(&self) -> bool {
        self.node_hash.is_none()
    }
    /// An authenticated zero native transfer to the pinned owner is a cancellation intent.
    /// This survives another speed-up or uncertain retry without inventing a payee entry.
    pub fn is_cancel(&self) -> bool {
        self.kind == Kind::Transfer
            && self.decoded.to() == self.from
            && self.decoded.value().is_zero()
            && self.decoded.input().is_empty()
    }
}

fn pinned_address(key: &KeyRow) -> Result<Address> {
    key.address
        .as_deref()
        .ok_or_else(|| WalletError::validation("needs: V (a password-pinned own address)"))
        .and_then(evm::parse_address)
}

fn evm_chain(net: &Net) -> Result<()> {
    if !matches!(net.chain.family, Family::Evm(_)) {
        return Err(WalletError::validation(
            "fixed-nonce replacement is available only on EVM chains",
        ));
    }
    Ok(())
}

fn matches_journal(
    net: &Net,
    hash: B256,
    from: Address,
    decoded: &tx::DecodedUnsigned,
    row: &JournalRow,
) -> Result<Kind> {
    let addr = |s: &str| evm::parse_address(s).ok();
    let common = row.chain == net.chain.key
        && row.tx_hash.parse::<B256>().ok() == Some(hash)
        && addr(&row.from) == Some(from)
        && Into::<Option<u64>>::into(row.nonce) == Some(decoded.nonce());
    let kind = match (row.kind, decoded.action()) {
        ("transfer", tx::Action::NativeTransfer { to, value })
            if row.token.is_none()
                && addr(&row.to) == Some(*to)
                && row.amount_raw == Some(*value) =>
        {
            Kind::Transfer
        }
        ("token-transfer", tx::Action::TransferCall { token, to, amount })
            if decoded.value().is_zero()
                && row.token.as_deref().and_then(addr) == Some(*token)
                && addr(&row.to) == Some(*to)
                && row.amount_raw == Some(*amount)
                && evm::registry::lookup(evm::config(net.chain).chain_id, token).is_some() =>
        {
            Kind::Token
        }
        ("invoke", _)
            if row.token.is_none()
                && addr(&row.to) == Some(decoded.to())
                && row.amount_raw == Some(decoded.value()) =>
        {
            Kind::Invoke
        }
        _ => return Err(WalletError::validation(DISAGREE)),
    };
    if !common {
        return Err(WalletError::validation(DISAGREE));
    }
    Ok(kind)
}

/// Restart path. The key must be the caller's password-pinned key, never a cache/status row.
/// All node fields are reconstructed and authenticated before the journal is compared.
pub async fn replaced_intent(
    net: &Net,
    hash: &str,
    key: &KeyRow,
    nonce: u64,
    row: &JournalRow,
) -> Result<OriginalIntent> {
    evm_chain(net)?;
    let hash = evm::parse_tx_hash(hash)?;
    let from = pinned_address(key)?;
    let provider = rpc::connect(net).await?;
    let decoded = rpc::pending_original(&provider, net, hash, from, nonce)
        .await?
        .ok_or_else(|| WalletError::validation(RETRY_WARNING))?;
    let kind = matches_journal(net, hash, from, &decoded, row)?;
    if rpc::tx_state(&provider, net, hash, Some(row)).await? != State::Pending {
        return Err(WalletError::validation(
            "original transaction is no longer pending; nothing was signed",
        ));
    }
    Ok(OriginalIntent {
        chain: net.chain,
        key_name: key.name.clone(),
        index: key.index,
        from,
        decoded,
        kind,
        node_hash: Some(hash),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    SpeedUp,
    Cancel,
    Retry,
}

#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Ten-thousandths, 1.1 through 3.0. The default is 1.25.
    pub bump: u128,
    pub max_fee_wei: Option<u128>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            bump: chains::DEFAULT_BUMP,
            max_fee_wei: None,
        }
    }
}

/// The exact review binds the account, chain, original hash/nonce, purpose and fee maximum.
#[derive(Debug)]
pub struct ReplacementReview {
    home: PathBuf,
    net: Net,
    checked: CheckedList,
    original: OriginalIntent,
    hash: B256,
    action: Action,
    options: Options,
    confirm: String,
    request: rpc::Intent,
    prepared: PreparedTx,
    decoded: tx::DecodedUnsigned,
    verdict: Result<()>,
    sufficient_funds: bool,
    max_total_fee: U256,
    fields: Map<String, Value>,
}
impl ReplacementReview {
    pub fn fields(&self) -> &Map<String, Value> {
        &self.fields
    }
    pub fn intent(&self) -> &tx::DecodedUnsigned {
        &self.decoded
    }
    pub fn original(&self) -> &OriginalIntent {
        &self.original
    }
    /// Retain the newly reviewed replacement itself for a later retry/replacement of its hash.
    pub fn retained_intent(&self) -> OriginalIntent {
        OriginalIntent {
            decoded: self.decoded.clone(),
            kind: if self.action == Action::Cancel {
                Kind::Transfer
            } else {
                self.original.kind
            },
            node_hash: None,
            ..self.original.clone()
        }
    }
    pub fn replaced_hash(&self) -> String {
        format!("{:#x}", self.hash)
    }
    pub fn action(&self) -> Action {
        self.action
    }
    pub fn is_cancel(&self) -> bool {
        self.action == Action::Cancel || self.original.is_cancel()
    }
    pub fn options(&self) -> Options {
        self.options
    }
    pub fn confirmation(&self) -> &str {
        &self.confirm
    }
    pub fn verdict(&self) -> &Result<()> {
        &self.verdict
    }
    pub fn sufficient_funds(&self) -> bool {
        self.sufficient_funds
    }
    pub fn max_total_fee(&self) -> U256 {
        self.max_total_fee
    }
    pub fn max_gas_cost(&self) -> U256 {
        self.prepared.max_gas_cost
    }
    pub fn l1_fee(&self) -> Option<U256> {
        self.prepared.l1_fee
    }
    pub fn fee_cap_wei(&self) -> Option<U256> {
        self.checked.list.policy.max_fee_wei(self.net.chain.key)
    }
    fn same_intent(&self, next: &Self) -> bool {
        self.hash == next.hash
            && self.action == next.action
            && self.confirm == next.confirm
            && self.original.from == next.original.from
            && self.decoded.chain_id() == next.decoded.chain_id()
            && self.decoded.nonce() == next.decoded.nonce()
            && self.decoded.to() == next.decoded.to()
            && self.decoded.value() == next.decoded.value()
            && self.decoded.input() == next.decoded.input()
            && self.verdict == next.verdict
            && self.sufficient_funds == next.sufficient_funds
            && next.max_total_fee <= self.max_total_fee
    }
}

fn destination(original: &OriginalIntent) -> Address {
    match (original.kind, original.decoded.action()) {
        (Kind::Token, tx::Action::TransferCall { to, .. }) => *to,
        _ => original.decoded.to(),
    }
}

fn confirmation(
    checked: &CheckedList,
    original: &OriginalIntent,
    action: Action,
) -> Result<String> {
    if action == Action::Cancel || original.is_cancel() {
        return Ok("cancel".into());
    }
    let expected = if original.kind == Kind::Invoke {
        EntryKind::Contract
    } else {
        EntryKind::Address
    };
    let to = destination(original);
    if to.is_zero() {
        return Err(WalletError::validation(
            "the zero address is never a replacement payee",
        ));
    }
    let entry = checked
        .list
        .entries
        .iter()
        .find(|e| {
            e.kind == expected
                && e.covers_chain(original.chain.key)
                && evm::parse_address(&e.address).ok() == Some(to)
        })
        .ok_or_else(|| {
            WalletError::validation(
                "the current checked list does not allow this replacement's payee or contract",
            )
        })?;
    if original.kind == Kind::Invoke {
        let selector = original
            .decoded
            .input()
            .get(..4)
            .map(|s| format!("0x{}", alloy::hex::encode(s)));
        if !selector.is_some_and(|s| {
            entry
                .selectors
                .as_ref()
                .is_some_and(|sels| sels.contains(&s))
        }) {
            return Err(WalletError::validation(
                "the current checked contract does not allow this selector",
            ));
        }
    }
    Ok(entry.nickname.clone())
}

fn fees(decoded: &tx::DecodedUnsigned) -> rpc::OriginalFees {
    match decoded.fees() {
        tx::EvmFees::Eip1559 {
            max_fee_per_gas,
            max_priority_fee_per_gas,
        } => rpc::OriginalFees {
            max_fee: max_fee_per_gas.to(),
            tip: max_priority_fee_per_gas.to(),
        },
        tx::EvmFees::Legacy { gas_price } => rpc::OriginalFees {
            max_fee: gas_price.to(),
            tip: gas_price.to(),
        },
    }
}

fn request(
    home: &Path,
    original: &OriginalIntent,
    hash: B256,
    action: Action,
    options: Options,
) -> Result<rpc::Intent> {
    let token = match (action, original.kind, original.decoded.action()) {
        (Action::Cancel, _, _) => None,
        (_, Kind::Token, tx::Action::TransferCall { token, amount, .. }) => {
            let meta = evm::registry::lookup(evm::config(original.chain).chain_id, token)
                .ok_or_else(|| {
                    WalletError::validation("replacement token is not in this chain's registry")
                })?;
            Some(rpc::TokenDebit {
                token: *token,
                amount: *amount,
                symbol: meta.symbol.into(),
                decimals: meta.decimals,
            })
        }
        _ => None,
    };
    Ok(rpc::Intent {
        from: original.from,
        to: if action == Action::Cancel {
            original.from
        } else {
            original.decoded.to()
        },
        value: if action == Action::Cancel {
            U256::ZERO
        } else {
            original.decoded.value()
        },
        input: if action == Action::Cancel {
            Bytes::new()
        } else {
            original.decoded.input().clone()
        },
        token,
        fallback_gas: Some(original.decoded.gas_limit()),
        dry_run: true,
        nonce: Some(original.nonce()),
        bump: Some(options.bump),
        replace: Some(hash),
        max_fee_per_gas: options.max_fee_wei,
        allow_pending: false,
        journal: Some(chains::SendJournal::new(
            home,
            original.chain,
            &original.from(),
        )),
    })
}

fn display_fields(
    original: &OriginalIntent,
    action: Action,
    confirm: &str,
    request: &rpc::Intent,
) -> Map<String, Value> {
    let mut out = json!({"chain": original.chain.key, "key": original.key_name, "from": original.from(),
        "to": if action == Action::Cancel { original.from() } else { destination(original).to_checksum(None) },
        "toNickname": confirm, "replacementAction": match action { Action::SpeedUp => "speed-up", Action::Cancel => "cancel", Action::Retry => "retry" },
        "data": format!("0x{}", alloy::hex::encode(&request.input))}).as_object().unwrap().clone();
    if action == Action::Cancel {
        out.extend(
            json!({"amount":"0","amountRaw":"0","symbol":original.chain.symbol,"race":CANCEL_RACE})
                .as_object()
                .unwrap()
                .clone(),
        );
    } else if let Some(token) = &request.token {
        out.extend(json!({"amount":format_units(token.amount, token.decimals),"amountRaw":token.amount.to_string(),
            "symbol":token.symbol,"token":token.token.to_checksum(None)}).as_object().unwrap().clone());
    } else {
        let (amount, raw) = if original.kind == Kind::Invoke {
            ("value", "valueRaw")
        } else {
            ("amount", "amountRaw")
        };
        out.insert(
            amount.into(),
            json!(format_units(request.value, original.chain.decimals)),
        );
        out.insert(raw.into(), json!(request.value.to_string()));
        out.insert("symbol".into(), json!(original.chain.symbol));
    }
    out
}

/// Prepare a replacement without signing. A retry is possible only from this session's review;
/// speed-up/cancel require a currently pending, cryptographically checked original as well.
#[allow(clippy::too_many_arguments)]
pub async fn review_replacement(
    home: &Path,
    net: &Net,
    checked: &CheckedList,
    original: &OriginalIntent,
    hash: &str,
    action: Action,
    options: Options,
) -> Result<ReplacementReview> {
    evm_chain(net)?;
    if original.chain.key != net.chain.key
        || original.decoded.chain_id() != evm::config(net.chain).chain_id
    {
        return Err(WalletError::validation(
            "replacement original is on another chain",
        ));
    }
    if !(11_000..=30_000).contains(&options.bump) || options.max_fee_wei == Some(0) {
        return Err(WalletError::validation(
            "replacement bump must be 1.1 through 3 and fee ceiling positive",
        ));
    }
    let hash = evm::parse_tx_hash(hash)?;
    if original.node_hash.is_some_and(|h| h != hash)
        || (action == Action::Retry && !original.is_retained())
    {
        return Err(WalletError::validation(
            "retry requires this session's own immutable review; look the hash up first",
        ));
    }
    whitelist::check_unchanged(home, &checked.fingerprint)?;
    let confirm = confirmation(checked, original, action)?;
    let provider = rpc::connect(net).await?;
    let observed =
        rpc::pending_original(&provider, net, hash, original.from, original.nonce()).await?;
    if let Some(observed) = &observed {
        if observed.to() != original.decoded.to()
            || observed.value() != original.decoded.value()
            || observed.input() != original.decoded.input()
        {
            return Err(WalletError::validation(
                "node transaction differs from the retained intent; nothing was signed",
            ));
        }
        if rpc::tx_state(&provider, net, hash, None).await? != State::Pending {
            return Err(WalletError::validation(
                "already mined (nonce consumed); nothing was signed; look up the original hash with zwallet tx",
            ));
        }
    } else if action != Action::Retry {
        return Err(WalletError::validation(RETRY_WARNING));
    }
    let mut floor = fees(&original.decoded);
    if let Some(observed) = &observed {
        let observed = fees(observed);
        floor.max_fee = floor.max_fee.max(observed.max_fee);
        floor.tip = floor.tip.max(observed.tip);
    }
    let request = request(home, original, hash, action, options)?;
    let prepared = rpc::prepare_replacement(&provider, net, request.clone(), floor).await?;
    whitelist::check_unchanged(home, &checked.fingerprint)?;
    let payload = prepared
        .unsigned
        .as_str()
        .and_then(|v| v.strip_prefix("0x"))
        .and_then(|v| alloy::hex::decode(v).ok())
        .ok_or_else(|| WalletError::validation("replacement payload is not 0x hex"))?;
    let decoded = tx::decode_unsigned(&payload)?;
    if decoded.chain_id() != original.decoded.chain_id()
        || decoded.nonce() != original.nonce()
        || decoded.to() != request.to
        || decoded.value() != request.value
        || decoded.input() != &request.input
    {
        return Err(WalletError::validation(
            "prepared replacement changed the reviewed intent",
        ));
    }
    if evm::config(net.chain).op_stack && prepared.l1_fee.is_none() {
        return Err(WalletError::validation(
            "L1 data fee estimate unavailable; review again before sending",
        ));
    }
    let max_total_fee = prepared
        .max_gas_cost
        .checked_add(prepared.l1_fee.unwrap_or_default())
        .ok_or_else(|| WalletError::validation("total replacement fee is too large"))?;
    let verdict = policy::check_tx(
        &policy::ListView::Usable(&checked.list),
        net.chain,
        &original.from,
        &decoded,
    );
    let sufficient_funds = prepared
        .fields
        .get("sufficientFunds")
        .and_then(Value::as_bool)
        .ok_or_else(|| WalletError::validation("replacement has no funds verdict"))?;
    let mut fields = display_fields(original, action, &confirm, &request);
    fields.extend(prepared.fields.clone());
    fields.insert("decoded".into(), decoded.to_json());
    fields.insert(
        "whitelist".into(),
        match &verdict {
            Ok(()) => json!({"allowed":true}),
            Err(e) => json!({"allowed":false,"reason":e.message}),
        },
    );
    Ok(ReplacementReview {
        home: home.to_path_buf(),
        net: net.clone(),
        checked: checked.clone(),
        original: original.clone(),
        hash,
        action,
        options,
        confirm,
        request,
        prepared,
        decoded,
        verdict,
        sufficient_funds,
        max_total_fee,
        fields,
    })
}

#[derive(Debug)]
pub enum SendReplacementOutcome {
    Sent(SendOutput),
    ReviewChanged(Box<ReplacementReview>),
}

pub async fn send_replacement(
    review: ReplacementReview,
    typed: &str,
    progress: Option<chains::SendProgress>,
) -> Result<SendReplacementOutcome> {
    if typed != review.confirm {
        return Err(WalletError::validation(
            "type the reviewed nickname or cancel exactly",
        ));
    }
    whitelist::check_unchanged(&review.home, &review.checked.fingerprint)?;
    let lock = chains::SendLock::acquire(&review.home, review.net.chain, &review.original.from())?;
    let next = review_replacement(
        &review.home,
        &review.net,
        &review.checked,
        &review.original,
        &review.replaced_hash(),
        review.action,
        review.options,
    )
    .await?;
    if !review.same_intent(&next) {
        return Ok(SendReplacementOutcome::ReviewChanged(Box::new(next)));
    }
    next.verdict.as_ref().map_err(Clone::clone)?;
    rpc::check_transfer_sendable(
        &next.net,
        &TransferRequest {
            from: next.original.from(),
            to: next.request.to.to_checksum(None),
            amount: String::new(),
            token: None,
            dry_run: false,
            nonce: Some(next.original.nonce()),
            bump: Some(next.options.bump),
            replace: Some(next.replaced_hash()),
            max_fee_per_gas: next.options.max_fee_wei,
            force: false,
            allow_pending: false,
            journal: next.request.journal.clone(),
        },
        &next.prepared,
    )?;
    let kind = if next.action == Action::Cancel {
        "transfer"
    } else {
        next.original.kind()
    };
    let fields = display_fields(&next.original, next.action, &next.confirm, &next.request);
    let output = send_prepared(
        &Evm,
        &next.net,
        &next.home,
        next.original.index,
        &next.original.from(),
        next.prepared,
        &SendOptions {
            dry_run: false,
            nonce: Some(next.original.nonce()),
            allow_duplicate: false,
            wait: false,
            wait_timeout: None,
        },
        fields,
        Some(lock),
        kind,
        progress,
    )
    .await?;
    Ok(SendReplacementOutcome::Sent(output))
}
