//! Review without signing; confirm the held intent before the shared signing/broadcast tail.
use super::CheckedList;
use crate::chains::evm::{self, Evm, policy, tx};
use crate::chains::{self, Broadcast, Chain, ChainFamily, Net, PreparedTx, TransferRequest};
use crate::commands::whitelist;
use crate::core::error::{Result, WalletError};
use crate::core::protocol::{self, Request, SIGN_TIMEOUT};
use alloy::primitives::U256;
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;

const DEFAULT_WAIT_TIMEOUT: Duration = Duration::from_secs(90);

/// A send's public result. No signature or raw signed transaction escapes this boundary.
#[derive(Debug)]
pub struct SendOutput {
    pub fields: Map<String, Value>,
    pub duplicates_text: Option<String>,
}

pub(crate) struct SendOptions {
    pub dry_run: bool,
    pub nonce: Option<u64>,
    pub allow_duplicate: bool,
    pub wait: bool,
    pub wait_timeout: Option<Duration>,
}

fn payload(unsigned: &Value) -> Result<Vec<u8>> {
    unsigned
        .as_str()
        .and_then(|v| v.strip_prefix("0x"))
        .and_then(|v| alloy::hex::decode(v).ok())
        .ok_or_else(|| WalletError::validation("the unsigned transaction is not 0x hex"))
}

/// What `out` (the command's fields merged with `prepare`'s) says the send does: the recipient
/// (the contract for an invoke), the amount with its symbol (`amount` and the asset's `symbol`
/// for a transfer, the native `value` for an invoke), the same amount in base units
/// (`amountRaw` for a transfer, `valueRaw` for an invoke), and the token contract of a token
/// transfer. The duplicate check keys on the base units, never on the symbol the node reported.
pub(crate) fn send_summary(
    out: &Map<String, Value>,
    chain: &Chain,
    kind: &str,
) -> chains::SendSummary {
    let text = |k: &str| out.get(k).and_then(Value::as_str);
    chains::SendSummary {
        kind: kind.to_string(),
        to: text("to").unwrap_or("?").to_string(),
        value: format!(
            "{} {}",
            text("amount").or_else(|| text("value")).unwrap_or("?"),
            text("symbol").unwrap_or(chain.symbol)
        ),
        amount_raw: text("amountRaw")
            .or_else(|| text("valueRaw"))
            .map(str::to_string),
        token: text("token").map(str::to_string),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn send_prepared<F: ChainFamily>(
    fam: &F,
    net: &Net,
    home: &Path,
    index: u32,
    from: &str,
    prepared: PreparedTx,
    send: &SendOptions,
    mut out: Map<String, Value>,
    _lock: Option<chains::SendLock>,
    kind: &str,
    progress: Option<chains::SendProgress>,
) -> Result<SendOutput> {
    let PreparedTx {
        unsigned, fields, ..
    } = prepared;
    for (k, v) in fields {
        out.entry(k).or_insert(v);
    }
    let journal = chains::SendJournal::new(home, net.chain, from);
    let summary = send_summary(&out, net.chain, kind);
    let mut duplicates_text = None;
    if send.nonce.is_none() {
        let now = crate::core::time::unix_secs(std::time::SystemTime::now());
        let duplicates = journal.recent_duplicates(&summary, now)?;
        if !duplicates.is_empty() && !send.dry_run && !send.allow_duplicate {
            return Err(journal.duplicate_error(duplicates));
        }
        // The journal's own lines, which anyone of this user can write: escaped in `--json`,
        // and for a human each field only in the shape zwallet writes it, within one budget.
        out.insert(
            "duplicateSends".into(),
            duplicates
                .iter()
                .map(crate::core::text::inert_json)
                .collect(),
        );
        if !duplicates.is_empty() {
            duplicates_text = Some(journal.entries_text(&duplicates));
        }
    }
    // A dry-run the whitelist on disk refuses is explained without asking the daemon to sign.
    let checked = if send.dry_run {
        whitelist::dry_run_check(home, net.chain, from, &unsigned)?
    } else {
        whitelist::DryRunCheck::NotChecked
    };
    if let whitelist::DryRunCheck::Denied(denied) = checked {
        out.insert("dryRun".into(), json!(true));
        out.insert("whitelist".into(), denied);
        return Ok(SendOutput {
            fields: out,
            duplicates_text,
        });
    }
    let signed_value = protocol::send_unlocked(
        home,
        &Request::SignTx {
            chain: net.chain.key.into(),
            index,
            tx: unsigned.clone(),
        },
        SIGN_TIMEOUT,
    )
    .await?;
    let signed = fam.inspect_signed(net.chain, &signed_value)?;
    tx::verify_signed_payload(&signed.raw, &payload(&unsigned)?)?;
    if !signed.signer.eq_ignore_ascii_case(from) {
        return Err(WalletError::validation(format!(
            "the key daemon signed as {}, expected {from}; not broadcasting",
            signed.signer
        )));
    }
    // The nonce the signed bytes actually consume.
    if let Some(n) = signed.nonce {
        out.insert("nonce".into(), json!(n));
    }
    out.insert("dryRun".into(), json!(send.dry_run));
    // What the key holder decoded from the bytes it signed (not what this process intended).
    if let Some(view) = signed_value.get("decoded") {
        out.insert(
            "signerDecoded".into(),
            crate::commands::signer_decoded(view),
        );
    }
    // The hash is computed from the signed bytes, never taken from the node.
    out.insert("txHash".into(), json!(signed.tx_hash));
    if send.dry_run {
        // Checked here against the list on disk, then by the daemon against the sealed one.
        if matches!(checked, whitelist::DryRunCheck::Allowed) {
            out.insert("whitelist".into(), json!({ "allowed": true }));
        }
        // No rawTx: a dry-run must not hand out a transaction that can be broadcast without
        // zwallet.
        out.insert("recoveredFrom".into(), json!(signed.signer));
        return Ok(SendOutput {
            fields: out,
            duplicates_text,
        });
    }
    let record = chains::SendRecord::new(journal, &signed, &summary).with_progress(progress);
    let outcome = fam
        .broadcast(net, &signed, send.nonce.is_some(), &record)
        .await?;
    out.insert(
        "explorerUrl".into(),
        json!(net.chain.explorer_tx_url(&signed.tx_hash)),
    );
    out.insert("journal".into(), json!(record.journal.display()));
    if let Broadcast::KnownDespite {
        warning,
        rpc_tx_hash,
    } = outcome
    {
        out.insert(
            "broadcastWarning".into(),
            json!(format!(
                "{warning}; the node reports the transaction by its own hash, so it was sent"
            )),
        );
        if let Some(h) = rpc_tx_hash {
            out.insert("rpcTxHash".into(), json!(h));
        }
    }
    if send.wait {
        let timeout = send.wait_timeout.unwrap_or(DEFAULT_WAIT_TIMEOUT);
        let receipt = fam.wait_for_receipt(net, &signed.tx_hash, timeout).await;
        chains::record_receipt(&mut out, net.chain, receipt);
    }
    Ok(SendOutput {
        fields: out,
        duplicates_text,
    })
}

/// A payee selected from the password-checked list, never a free-form address.
#[derive(Debug, Clone)]
pub struct CheckedPayee {
    nickname: String,
    address: String,
    chain: &'static str,
    fingerprint: crate::core::whitelist::Fingerprint,
}

impl CheckedPayee {
    pub fn nickname(&self) -> &str {
        &self.nickname
    }
    pub fn address(&self) -> &str {
        &self.address
    }
}

/// Address entries on this chain only. Invalid/zero addresses and contracts are not choices.
/// The caller checks the fingerprint before presenting the picker; this pure operation uses
/// only the list retained by the password check.
pub fn transfer_payees(checked: &CheckedList, chain: &'static Chain) -> Vec<CheckedPayee> {
    checked
        .list
        .entries
        .iter()
        .filter(|e| {
            e.kind == crate::core::whitelist::EntryKind::Address && e.covers_chain(chain.key)
        })
        .filter_map(|e| {
            let address = evm::parse_address(&e.address).ok()?;
            (!address.is_zero()).then(|| CheckedPayee {
                nickname: e.nickname.clone(),
                address: address.to_checksum(None),
                chain: chain.key,
                fingerprint: checked.fingerprint,
            })
        })
        .collect()
}

/// Choices held by a review. A changed choice requires another review.
#[derive(Debug, Clone, Default)]
pub struct TransferOptions {
    pub allow_pending: bool,
    pub allow_duplicate: bool,
    /// Price per gas unit in wei, equivalent to the CLI's --max-fee-gwei.
    pub max_fee_wei: Option<u128>,
    pub nonce: Option<u64>,
    pub bump: Option<u128>,
    pub replace: Option<String>,
}

/// Parse the same decimal gwei fee ceiling as the CLI, before any RPC or signature.
pub fn parse_fee_ceiling(gwei: &str) -> Result<u128> {
    let bad = |why: String| WalletError::validation(format!("--max-fee-gwei: {why}"));
    let wei = crate::core::amount::parse_units(gwei, 9).map_err(|e| bad(e.message))?;
    if wei.is_zero() {
        return Err(bad("must be greater than zero".into()));
    }
    u128::try_from(wei).map_err(|_| bad(format!("'{gwei}' is too large")))
}

/// The caller supplies the key's password-pinned address and index. Cached metadata or daemon
/// status is not sufficient provenance for `from`. The payee can only come from the picker.
#[derive(Debug, Clone)]
pub struct TransferInput {
    pub index: u32,
    pub key_name: String,
    pub from: String,
    pub payee: CheckedPayee,
    pub amount: String,
    pub token: Option<String>,
    pub options: TransferOptions,
}

/// An immutable review. Its request, checked list, RPC, key and exact decoded intent stay
/// together; confirm accepts no replacement address, amount, overrides or RPC URL.
#[derive(Debug)]
pub struct TransferReview {
    home: PathBuf,
    net: Net,
    checked: CheckedList,
    input: TransferInput,
    request: TransferRequest,
    prepared: PreparedTx,
    decoded: tx::DecodedUnsigned,
    verdict: Result<()>,
    sufficient_funds: bool,
    max_total_fee: U256,
    fields: Map<String, Value>,
}

/// A session's own immutable transfer review retained for a later fixed-nonce replacement.
/// Only a reviewed transfer can construct it; journal text and node fields cannot.
#[derive(Clone, Debug)]
pub struct RetainedTransfer {
    chain: &'static Chain,
    input: TransferInput,
    decoded: tx::DecodedUnsigned,
    max_total_fee: U256,
}
impl RetainedTransfer {
    pub fn chain(&self) -> &'static Chain {
        self.chain
    }
    pub fn from(&self) -> &str {
        &self.input.from
    }
    pub fn index(&self) -> u32 {
        self.input.index
    }
    pub fn key_name(&self) -> &str {
        &self.input.key_name
    }
    pub fn decoded(&self) -> &tx::DecodedUnsigned {
        &self.decoded
    }
    pub fn options(&self) -> &TransferOptions {
        &self.input.options
    }
    pub fn max_total_fee(&self) -> U256 {
        self.max_total_fee
    }
}

impl TransferReview {
    pub fn retained_intent(&self) -> RetainedTransfer {
        RetainedTransfer {
            chain: self.net.chain,
            input: self.input.clone(),
            decoded: self.decoded.clone(),
            max_total_fee: self.max_total_fee,
        }
    }
    pub fn fields(&self) -> &Map<String, Value> {
        &self.fields
    }
    pub fn intent(&self) -> &tx::DecodedUnsigned {
        &self.decoded
    }
    pub fn from(&self) -> &str {
        &self.input.from
    }
    pub fn payee(&self) -> &CheckedPayee {
        &self.input.payee
    }
    pub fn options(&self) -> &TransferOptions {
        &self.input.options
    }
    pub fn verdict(&self) -> &Result<()> {
        &self.verdict
    }
    pub fn sufficient_funds(&self) -> bool {
        self.sufficient_funds
    }
    /// The checked list's execution-fee cap (OP L1 data fees are outside this signing cap).
    pub fn fee_cap_wei(&self) -> Option<U256> {
        self.checked.list.policy.max_fee_wei(self.net.chain.key)
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

    fn same_intent(&self, next: &Self) -> bool {
        self.decoded.chain_id() == next.decoded.chain_id()
            && self.input.from == next.input.from
            && self.decoded.to() == next.decoded.to()
            && self.decoded.value() == next.decoded.value()
            && self.decoded.input() == next.decoded.input()
            && self.decoded.action() == next.decoded.action()
            && self.decoded.nonce() == next.decoded.nonce()
            && self.sufficient_funds == next.sufficient_funds
            && self.verdict == next.verdict
            && next.max_total_fee <= self.max_total_fee
    }
}

/// Review again when fees, nonce or another reviewed fact changed. No SignTx was sent in that
/// case. Consuming the review avoids accidental reuse of a confirmed review in UI state.
#[derive(Debug)]
pub enum SendTransferOutcome {
    Sent(SendOutput),
    ReviewChanged(Box<TransferReview>),
}

fn transfer_kind(request: &TransferRequest) -> &'static str {
    if request.token.is_some() {
        "token-transfer"
    } else {
        "transfer"
    }
}

fn base_fields(chain: &Chain, input: &TransferInput) -> Map<String, Value> {
    json!({"chain":chain.key, "key":input.key_name, "from":input.from,
        "to":input.payee.address, "toNickname":input.payee.nickname})
    .as_object()
    .unwrap()
    .clone()
}

fn assemble_review(
    home: PathBuf,
    net: Net,
    checked: CheckedList,
    input: TransferInput,
    request: TransferRequest,
    prepared: PreparedTx,
) -> Result<TransferReview> {
    let decoded = tx::decode_unsigned(&payload(&prepared.unsigned)?)?;
    if decoded.chain_id() != evm::config(net.chain).chain_id {
        return Err(WalletError::validation(
            "prepared transaction has a different chainId",
        ));
    }
    // A registry asset's units come from the registry. An RPC cannot silently change token
    // decimals and turn the human's amount into a different decoded payment.
    let payee = evm::parse_address(&input.payee.address)?;
    let expected = if let Some(token) = &input.token {
        let token = evm::parse_address(token)?;
        let registry = evm::registry::lookup(evm::config(net.chain).chain_id, &token)
            .ok_or_else(|| WalletError::validation("the token is not in this chain's registry"))?;
        let amount = crate::core::amount::parse_units(&input.amount, registry.decimals)?;
        tx::Action::TransferCall {
            token,
            to: payee,
            amount,
        }
    } else {
        tx::Action::NativeTransfer {
            to: payee,
            value: crate::core::amount::parse_units(&input.amount, net.chain.decimals)?,
        }
    };
    if decoded.action() != &expected {
        return Err(WalletError::validation(
            "prepared transfer differs from the requested registry asset, recipient or amount; nothing signed",
        ));
    }
    // An absent OP estimate cannot establish a reviewed upper bound. Never silently count it
    // as zero; a retry may obtain it. This is an estimate, not a cap on the eventual L1 charge.
    if evm::config(net.chain).op_stack && prepared.l1_fee.is_none() {
        return Err(WalletError::validation(
            "L1 data fee estimate unavailable; review again before sending",
        ));
    }
    let max_total_fee = prepared
        .max_gas_cost
        .checked_add(prepared.l1_fee.unwrap_or_default())
        .ok_or_else(|| WalletError::validation("total fee estimate is too large"))?;
    let verdict = policy::check_tx(
        &policy::ListView::Usable(&checked.list),
        net.chain,
        &evm::parse_address(&input.from)?,
        &decoded,
    );
    let sufficient_funds = prepared
        .fields
        .get("sufficientFunds")
        .and_then(Value::as_bool)
        .ok_or_else(|| WalletError::validation("prepared transaction has no funds verdict"))?;
    let mut fields = base_fields(net.chain, &input);
    fields.extend(prepared.fields.clone());
    let journal = chains::SendJournal::new(&home, net.chain, &input.from);
    if request.nonce.is_none() {
        let duplicates = journal.recent_duplicates(
            &send_summary(&fields, net.chain, transfer_kind(&request)),
            crate::core::time::unix_secs(std::time::SystemTime::now()),
        )?;
        fields.insert(
            "duplicateSends".into(),
            duplicates
                .iter()
                .map(crate::core::text::inert_json)
                .collect(),
        );
    }
    fields.insert(
        "whitelist".into(),
        match &verdict {
            Ok(()) => json!({"allowed": true}),
            Err(e) => json!({"allowed":false,"reason":e.message,"code":e.code.as_str()}),
        },
    );
    Ok(TransferReview {
        home,
        net,
        checked,
        input,
        request,
        prepared,
        decoded,
        verdict,
        sufficient_funds,
        max_total_fee,
        fields,
    })
}

/// Prepare and check against the password-verified in-memory list. No send lock, file-list
/// read, daemon call or signature occurs. Pending and duplicate sends remain visible to review.
pub async fn review_transfer(
    home: &Path,
    net: &Net,
    checked: &CheckedList,
    mut input: TransferInput,
) -> Result<TransferReview> {
    if input.payee.chain != net.chain.key
        || input.payee.fingerprint != checked.fingerprint
        || !transfer_payees(checked, net.chain)
            .iter()
            .any(|p| p.nickname == input.payee.nickname && p.address == input.payee.address)
    {
        return Err(WalletError::validation(
            "payee is not from this checked list and chain",
        ));
    }
    input.from = evm::parse_address(&input.from)?.to_checksum(None);
    if input.options.max_fee_wei == Some(0) {
        return Err(WalletError::validation(
            "--max-fee-gwei must be greater than zero",
        ));
    }
    if let Some(token) = &input.token {
        let address = evm::parse_address(token)?;
        if evm::registry::lookup(evm::config(net.chain).chain_id, &address).is_none() {
            return Err(WalletError::validation(
                "the token is not in this chain's registry",
            ));
        }
    }
    let request = TransferRequest {
        from: input.from.clone(),
        to: input.payee.address.clone(),
        amount: input.amount.clone(),
        token: input.token.clone(),
        dry_run: true,
        nonce: input.options.nonce,
        bump: input.options.bump,
        replace: input.options.replace.clone(),
        max_fee_per_gas: input.options.max_fee_wei,
        force: false,
        allow_pending: input.options.allow_pending,
        journal: Some(chains::SendJournal::new(home, net.chain, &input.from)),
    };
    let prepared = Evm.prepare_transfer(net, &request).await?;
    assemble_review(
        home.to_path_buf(),
        net.clone(),
        checked.clone(),
        input,
        request,
        prepared,
    )
}

/// Confirm the exact review using the held address and overrides. Re-read the fingerprint
/// before and after RPC, take the account lock, repeat pending/duplicate checks, compare the
/// decoded intent and integer fee bound, then sign once and journal before broadcast.
pub async fn send_transfer(
    review: TransferReview,
    progress: Option<chains::SendProgress>,
) -> Result<SendTransferOutcome> {
    crate::core::whitelist::check_unchanged(&review.home, &review.checked.fingerprint)?;
    let lock = chains::SendLock::acquire(&review.home, review.net.chain, &review.input.from)?;
    // Keep dry-run preparation so a changed funds verdict can be shown as a new review.
    // Live-send guards run on the fresh result below before any request to sign.
    let request = review.request.clone();
    let prepared = Evm.prepare_transfer(&review.net, &request).await?;
    crate::core::whitelist::check_unchanged(&review.home, &review.checked.fingerprint)?;
    let next = assemble_review(
        review.home.clone(),
        review.net.clone(),
        review.checked.clone(),
        review.input.clone(),
        request,
        prepared,
    )?;
    if !review.same_intent(&next) {
        return Ok(SendTransferOutcome::ReviewChanged(Box::new(next)));
    }
    evm::rpc::check_transfer_sendable(&next.net, &next.request, &next.prepared)?;
    next.verdict?;
    let options = SendOptions {
        dry_run: false,
        nonce: next.input.options.nonce,
        allow_duplicate: next.input.options.allow_duplicate,
        wait: false,
        wait_timeout: None,
    };
    let result = send_prepared(
        &Evm,
        &next.net,
        &next.home,
        next.input.index,
        &next.input.from,
        next.prepared,
        &options,
        base_fields(next.net.chain, &next.input),
        Some(lock),
        transfer_kind(&next.request),
        progress,
    )
    .await?;
    Ok(SendTransferOutcome::Sent(result))
}
