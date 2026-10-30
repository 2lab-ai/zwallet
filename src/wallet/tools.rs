//! Contract reads and immutable message/invoke reviews, shared by CLI and TUI.
//! No terminal input, printing, or unreviewed signature bytes leave these operations.
use super::{CheckedList, KeyRow};
use crate::chains::evm::{self, Evm, abi, policy, rpc};
use crate::chains::{self, Balance, Chain, ChainFamily, Net};
use crate::core::{
    error::{Result, WalletError},
    protocol::{self, Request, SIGN_TIMEOUT},
    text::{PROSE_MAX, ascii_inert, human_capped, inert_json},
    whitelist,
};
use alloy::hex;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct CallInput {
    pub from: Option<String>,
    pub to: String,
    pub sig: String,
    pub args: Vec<String>,
}

impl CallInput {
    /// Validate before the CLI asks for its optional wallet address, as it historically did.
    pub fn validate(&self) -> Result<()> {
        evm::parse_address(&self.to)?;
        let function = abi::parse_function(&self.sig)?;
        abi::encode_call(&function, &self.args)?;
        self.from.as_deref().map(evm::parse_address).transpose()?;
        Ok(())
    }
}

#[derive(Debug)]
pub struct CallOutput {
    /// Complete machine-readable CLI output. Never draw these untrusted fields directly.
    pub fields: Value,
    /// One isolated TUI result block: printable ASCII, at most 512 characters.
    pub rendered_result: String,
    /// A separate view, never wrapped alongside the result block.
    pub raw: String,
}
impl CallOutput {
    /// Existing CLI rendering keeps machine output complete and bounds the human raw field.
    pub fn cli_text(&self) -> String {
        let mut shown = self.fields.clone();
        shown["raw"] = json!(self.raw);
        crate::cli::render_human(&shown)
    }
}

pub async fn call(net: &Net, input: &CallInput) -> Result<CallOutput> {
    chains::require_contract_calls(net.chain)?;
    let to = evm::parse_address(&input.to)?;
    let from = input.from.as_deref().map(evm::parse_address).transpose()?;
    let function = abi::parse_function(&input.sig)?;
    let data = abi::encode_call(&function, &input.args)?;
    let provider = rpc::connect(net).await?;
    let bytes = rpc::eth_call(&provider, from, to, data).await?;
    let result = if function.outputs.is_empty() {
        Value::Null
    } else {
        Value::Array(abi::decode_output(&function, &bytes)?)
    };
    let sig = if function.outputs.is_empty() {
        function.signature()
    } else {
        function.signature_with_outputs()
    };
    let raw = format!("0x{}", hex::encode(bytes));
    Ok(CallOutput {
        rendered_result: human_capped(&inert_json(&result), PROSE_MAX),
        raw: ascii_inert(&raw, PROSE_MAX),
        fields: json!({"chain":net.chain.key,"from":from.map(|a| a.to_checksum(None)),
            "to":to.to_checksum(None),"sig":sig,"args":input.args,"result":result,"raw":raw}),
    })
}

/// An arbitrary token read, with no registry membership or transfer affordance implied.
/// ERC-20 metadata already escapes and bounds an outside symbol to 32 characters.
pub async fn token_balance(net: &Net, owner: &str, token: &str) -> Result<Balance> {
    chains::require_contract_calls(net.chain)?;
    Evm.token_balance(net, owner, token).await
}

/// Every byte in this review is shown verbatim between brackets. Only a password-pinned key
/// (from `Opened::keys`) is suitable; a cache or daemon status is not proof of that address.
#[derive(Debug)]
pub struct MessageReview {
    home: PathBuf,
    chain: &'static Chain,
    checked: Box<CheckedList>,
    key: KeyRow,
    address: String,
    message: String,
}
impl MessageReview {
    pub fn fingerprint(&self) -> &whitelist::Fingerprint {
        &self.checked.fingerprint
    }
    pub fn bytes(&self) -> &[u8] {
        self.message.as_bytes()
    }
    pub fn key(&self) -> &KeyRow {
        &self.key
    }
    pub fn address(&self) -> &str {
        &self.address
    }
    pub fn chain(&self) -> &'static Chain {
        self.chain
    }
    /// Brackets delimit spaces exactly; no escaping, trimming or elision of reviewed bytes.
    pub fn rows(&self) -> Vec<String> {
        if self.message.is_empty() {
            return vec!["[]".into()];
        }
        self.message
            .as_bytes()
            .chunks(76)
            .map(|row| format!("[{}]", std::str::from_utf8(row).expect("review is ASCII")))
            .collect()
    }
    pub fn metadata(&self) -> String {
        let leading = self.bytes().iter().take_while(|b| **b == b' ').count();
        let trailing = self
            .bytes()
            .iter()
            .rev()
            .take_while(|b| **b == b' ')
            .count();
        format!(
            "{} bytes; leading spaces {leading}, trailing spaces {trailing}",
            self.bytes().len()
        )
    }
}

pub fn review_message(
    home: &Path,
    chain: &'static Chain,
    checked: &CheckedList,
    key: &KeyRow,
    message: &str,
) -> Result<MessageReview> {
    whitelist::check_unchanged(home, &checked.fingerprint)?;
    chains::require_contract_calls(chain)?;
    if message.len() > 512 || !message.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
        return Err(WalletError::validation(
            "the message review accepts at most 512 printable ASCII bytes; use zwallet sign-message for other text",
        ));
    }
    policy::check_message(&policy::ListView::Usable(&checked.list))?;
    let address = pinned_address(key)?;
    Ok(MessageReview {
        home: home.to_path_buf(),
        chain,
        checked: Box::new(checked.clone()),
        key: key.clone(),
        address,
        message: message.to_string(),
    })
}

fn pinned_address(key: &KeyRow) -> Result<String> {
    let address = key
        .address
        .as_deref()
        .ok_or_else(|| WalletError::validation("the password-checked key has no pinned address"))?;
    Ok(evm::parse_address(address)?.to_checksum(None))
}

/// Consume a review. No new message, key, chain, or daemon policy claim is accepted at confirm.
pub async fn sign_message(review: MessageReview) -> Result<Value> {
    whitelist::check_unchanged(&review.home, &review.checked.fingerprint)?;
    policy::check_message(&policy::ListView::Usable(&review.checked.list))?;
    sign_message_bytes(
        &review.home,
        review.chain,
        review.key.index,
        &review.key.name,
        &review.address,
        &review.message,
    )
    .await
}

/// CLI keeps its existing arbitrary UTF-8/multiline input; only the interactive review restricts
/// the display alphabet. Both routes verify the exact sent bytes and recovered signer.
pub(crate) async fn sign_message_bytes(
    home: &Path,
    chain: &Chain,
    index: u32,
    key_name: &str,
    address: &str,
    message: &str,
) -> Result<Value> {
    // No Tron message is ever signed (`.prd/05-tron.md` §Non-goals); refused before the daemon.
    if !matches!(chain.family, chains::Family::Evm(_)) {
        return Err(WalletError::new(
            ErrorCode::CapabilityUnsupported,
            format!(
                "{} ({} family) does not support message signing",
                chain.key,
                chain.family.name()
            ),
        ));
    }
    let signed = protocol::send_unlocked(
        home,
        &Request::SignMessage {
            chain: chain.key.into(),
            index,
            message_hex: hex::encode(message.as_bytes()),
        },
        SIGN_TIMEOUT,
    )
    .await?;
    Evm.verify_message(message.as_bytes(), &signed, address)?;
    Ok(
        json!({"chain":chain.key,"key":key_name,"address":address,"message":message,
        "signature":signed["signature"],"digest":signed["digest"]}),
    )
}

use super::send::{SendOptions, SendOutput, TransferOptions, send_prepared, send_summary};
use crate::chains::evm::tx;
use crate::chains::{PreparedTx, TransferRequest};
use crate::core::amount::{format_units, parse_units};
use crate::core::error::ErrorCode;
use alloy::primitives::{Bytes, U256};
use serde_json::Map;

/// A contract chosen from the checked list, with its allowed selectors visible to the form.
#[derive(Debug, Clone)]
pub struct CheckedContract {
    nickname: String,
    address: String,
    chain: &'static str,
    selectors: Vec<String>,
    fingerprint: whitelist::Fingerprint,
}
impl CheckedContract {
    pub fn nickname(&self) -> &str {
        &self.nickname
    }
    pub fn address(&self) -> &str {
        &self.address
    }
    pub fn selectors(&self) -> &[String] {
        &self.selectors
    }
}

pub fn invoke_contracts(checked: &CheckedList, chain: &'static Chain) -> Vec<CheckedContract> {
    checked
        .list
        .entries
        .iter()
        .filter(|e| e.kind == whitelist::EntryKind::Contract && e.covers_chain(chain.key))
        .filter_map(|e| {
            let address = evm::parse_address(&e.address).ok()?;
            (!address.is_zero()).then(|| CheckedContract {
                nickname: e.nickname.clone(),
                address: address.to_checksum(None),
                chain: chain.key,
                selectors: e.selectors.clone().unwrap_or_default(),
                fingerprint: checked.fingerprint,
            })
        })
        .collect()
}

#[derive(Debug, Clone)]
pub enum InvokeData {
    Signature { sig: String, args: Vec<String> },
    Raw(String),
}
impl InvokeData {
    /// The same ABI/parser path as the CLI, including exact string argument bytes.
    pub fn encode(&self) -> Result<(Bytes, Option<String>)> {
        match self {
            Self::Signature { sig, args } => {
                let function = abi::parse_function(sig)?;
                Ok((
                    abi::encode_call(&function, args)?,
                    Some(function.signature()),
                ))
            }
            Self::Raw(data) => Ok((abi::parse_calldata(data)?, None)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct InvokeInput {
    pub key: KeyRow,
    pub contract: CheckedContract,
    pub call: InvokeData,
    /// Native-unit decimal value, as the CLI's --value; zero by default at the form.
    pub value: String,
    pub options: TransferOptions,
}

/// A trusted in-session invoke original. Only a completed immutable review can create it.
#[derive(Clone, Debug)]
pub struct RetainedInvoke {
    task_id: Option<String>,
    chain: &'static Chain,
    key: KeyRow,
    decoded: tx::DecodedUnsigned,
}
impl RetainedInvoke {
    pub fn task_id(&self) -> Option<&str> {
        self.task_id.as_deref()
    }
    pub fn chain(&self) -> &'static Chain {
        self.chain
    }
    pub fn key_name(&self) -> &str {
        &self.key.name
    }
    pub fn index(&self) -> u32 {
        self.key.index
    }
    pub fn from(&self) -> &str {
        self.key.address.as_deref().expect("review key is pinned")
    }
    pub fn decoded(&self) -> &tx::DecodedUnsigned {
        &self.decoded
    }
}

/// Holds the exact locally decoded intent and the checked policy. Editing requires a new one.
#[derive(Debug)]
pub struct InvokeReview {
    home: PathBuf,
    net: Net,
    checked: CheckedList,
    input: InvokeInput,
    request: rpc::Intent,
    prepared: PreparedTx,
    decoded: tx::DecodedUnsigned,
    fields: Map<String, Value>,
    verdict: Result<()>,
    sufficient_funds: bool,
    max_total_fee: U256,
}
impl InvokeReview {
    pub fn retained_intent(&self) -> RetainedInvoke {
        RetainedInvoke {
            task_id: self.input.options.task_id.clone(),
            chain: self.net.chain,
            key: self.input.key.clone(),
            decoded: self.decoded.clone(),
        }
    }
    pub fn fingerprint(&self) -> &whitelist::Fingerprint {
        &self.checked.fingerprint
    }
    pub fn fields(&self) -> &Map<String, Value> {
        &self.fields
    }
    pub fn intent(&self) -> &tx::DecodedUnsigned {
        &self.decoded
    }
    pub fn contract(&self) -> &CheckedContract {
        &self.input.contract
    }
    pub fn key(&self) -> &KeyRow {
        &self.input.key
    }
    pub fn chain(&self) -> &'static Chain {
        self.net.chain
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
        self.input.options.task_id == next.input.options.task_id
            && self.decoded.chain_id() == next.decoded.chain_id()
            && self.request.from == next.request.from
            && self.decoded.to() == next.decoded.to()
            && self.decoded.input() == next.decoded.input()
            && self.decoded.value() == next.decoded.value()
            && self.decoded.nonce() == next.decoded.nonce()
            && self.decoded.action() == next.decoded.action()
            && self.sufficient_funds == next.sufficient_funds
            && self.verdict == next.verdict
            && next.max_total_fee <= self.max_total_fee
    }
}

#[derive(Debug)]
pub enum SendInvokeOutcome {
    Sent(SendOutput),
    ReviewChanged(Box<InvokeReview>),
}

fn invoke_fields(
    net: &Net,
    input: &InvokeInput,
    request: &rpc::Intent,
) -> Result<Map<String, Value>> {
    let (_, sig) = input.call.encode()?;
    let args = match &input.call {
        InvokeData::Signature { args, .. } => args.clone(),
        _ => Vec::new(),
    };
    let mut fields = json!({"chain":net.chain.key,"key":input.key.name,"from":request.from.to_checksum(None),
        "to":request.to.to_checksum(None),"toNickname":input.contract.nickname,"sig":sig,"args":args,
        "data":format!("0x{}",hex::encode(&request.input)),"value":format_units(request.value,net.chain.decimals),
        "valueRaw":request.value.to_string()}).as_object().unwrap().clone();
    if let Some(task) = &input.options.task_id {
        fields.insert("taskId".into(), task.clone().into());
    }
    Ok(fields)
}

fn assemble_invoke(
    home: PathBuf,
    net: Net,
    checked: CheckedList,
    input: InvokeInput,
    request: rpc::Intent,
    prepared: PreparedTx,
) -> Result<InvokeReview> {
    let bytes = prepared
        .unsigned
        .as_str()
        .and_then(|s| s.strip_prefix("0x"))
        .and_then(|s| hex::decode(s).ok())
        .ok_or_else(|| WalletError::validation("prepared invoke is not 0x hex"))?;
    let decoded = tx::decode_unsigned(&bytes)?;
    if decoded.chain_id() != evm::config(net.chain)?.chain_id
        || decoded.to() != request.to
        || decoded.value() != request.value
        || decoded.input() != &request.input
    {
        return Err(WalletError::validation(
            "prepared invoke differs from the reviewed contract, value or calldata",
        ));
    }
    if evm::config(net.chain)?.op_stack && prepared.l1_fee.is_none() {
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
        &request.from,
        &decoded,
    );
    let sufficient_funds = prepared
        .fields
        .get("sufficientFunds")
        .and_then(Value::as_bool)
        .ok_or_else(|| WalletError::validation("prepared invoke has no funds verdict"))?;
    let mut fields = invoke_fields(&net, &input, &request)?;
    fields.extend(prepared.fields.clone());
    fields.insert("decoded".into(), decoded.to_json());
    fields.insert(
        "whitelist".into(),
        match &verdict {
            Ok(()) => json!({"allowed":true}),
            Err(e) => json!({"allowed":false,"reason":e.message,"code":e.code.as_str()}),
        },
    );
    if request.nonce.is_none() {
        let window = checked.list.policy.duplicate_window_secs();
        let journal = chains::SendJournal::new(&home, net.chain, &request.from.to_checksum(None))
            .with_duplicate_window(window)?;
        fields.insert("duplicateGuard".into(), json!({"windowSecs":window,"journalTailBytes":1 << 20,"advisory":true,"policySource":"password-checked-list"}));
        let duplicates = journal.recent_duplicates(
            &send_summary(&fields, net.chain, "invoke"),
            crate::core::time::unix_secs(std::time::SystemTime::now()),
        )?;
        fields.insert(
            "duplicateSends".into(),
            duplicates.iter().map(inert_json).collect(),
        );
    }
    Ok(InvokeReview {
        home,
        net,
        checked,
        input,
        request,
        prepared,
        decoded,
        fields,
        verdict,
        sufficient_funds,
        max_total_fee,
    })
}

pub async fn review_invoke(
    home: &Path,
    net: &Net,
    checked: &CheckedList,
    input: InvokeInput,
) -> Result<InvokeReview> {
    whitelist::check_unchanged(home, &checked.fingerprint)?;
    chains::require_contract_calls(net.chain)?;
    if let Some(task) = &input.options.task_id {
        super::tasks::validate_binding_async(home, task, net.chain, &pinned_address(&input.key)?)
            .await?;
    }
    if input.contract.chain != net.chain.key
        || input.contract.fingerprint != checked.fingerprint
        || !invoke_contracts(checked, net.chain).iter().any(|c| {
            c.address == input.contract.address
                && c.nickname == input.contract.nickname
                && c.selectors == input.contract.selectors
        })
    {
        return Err(WalletError::validation(
            "contract is not from this checked list and chain",
        ));
    }
    let (data, _) = input.call.encode()?;
    let selector = data.get(..4).map(|s| format!("0x{}", hex::encode(s)));
    if !selector
        .as_ref()
        .is_some_and(|s| input.contract.selectors.contains(s))
    {
        return Err(WalletError::new(
            ErrorCode::WhitelistDenied,
            format!(
                "{} allows only these selectors: {}",
                input.contract.nickname,
                input.contract.selectors.join(", ")
            ),
        ));
    }
    if input.options.max_fee_wei == Some(0) {
        return Err(WalletError::validation(
            "--max-fee-gwei must be greater than zero",
        ));
    }
    // Replacement intent reconstruction belongs to wallet::replaced_intent. An arbitrary nonce
    // or replacement hash must not turn this new-invoke form into an unverified replacement.
    if input.options.nonce.is_some()
        || input.options.bump.is_some()
        || input.options.replace.is_some()
    {
        return Err(WalletError::validation(
            "invoke replacements require a reviewed replacement intent",
        ));
    }
    let request = rpc::Intent {
        from: evm::parse_address(&pinned_address(&input.key)?)?,
        to: evm::parse_address(&input.contract.address)?,
        value: parse_units(&input.value, net.chain.decimals)?,
        input: data,
        token: None,
        fallback_gas: None,
        dry_run: true,
        nonce: None,
        bump: None,
        replace: None,
        max_fee_per_gas: input.options.max_fee_wei,
        allow_pending: input.options.allow_pending,
        journal: Some(chains::SendJournal::new(
            home,
            net.chain,
            &pinned_address(&input.key)?,
        )),
    };
    let provider = rpc::connect(net).await?;
    let prepared = rpc::prepare(&provider, net, request.clone()).await?;
    whitelist::check_unchanged(home, &checked.fingerprint)?;
    assemble_invoke(
        home.to_path_buf(),
        net.clone(),
        checked.clone(),
        input,
        request,
        prepared,
    )
}

/// Consume the review, check the typed nickname and current list, then refresh nonce/fees/funds
/// under the account lock. A changed review produces no signature. This handles new invokes;
/// replacement reconstruction is a separate API and cannot be bypassed by options here.
pub async fn send_invoke(
    review: InvokeReview,
    typed_nickname: &str,
    progress: Option<chains::SendProgress>,
) -> Result<SendInvokeOutcome> {
    if typed_nickname != review.input.contract.nickname {
        return Err(WalletError::validation(
            "type the reviewed contract nickname exactly",
        ));
    }
    whitelist::check_unchanged(&review.home, &review.checked.fingerprint)?;
    let from = review.request.from.to_checksum(None);
    let lock = chains::SendLock::acquire(&review.home, review.net.chain, &from)?;
    let provider = rpc::connect(&review.net).await?;
    let prepared = rpc::prepare(&provider, &review.net, review.request.clone()).await?;
    whitelist::check_unchanged(&review.home, &review.checked.fingerprint)?;
    let next = assemble_invoke(
        review.home.clone(),
        review.net.clone(),
        review.checked.clone(),
        review.input.clone(),
        review.request.clone(),
        prepared,
    )?;
    if !review.same_intent(&next) {
        return Ok(SendInvokeOutcome::ReviewChanged(Box::new(next)));
    }
    next.verdict?;
    // The common guard only uses pending overrides/journal and the prepared funds verdict.
    // Keep one implementation for transfer and invoke, as the CLI's rpc::prepare does.
    rpc::check_transfer_sendable(
        &next.net,
        &TransferRequest {
            from: from.clone(),
            to: next.input.contract.address.clone(),
            amount: next.input.value.clone(),
            token: None,
            dry_run: false,
            nonce: None,
            bump: None,
            replace: None,
            max_fee_per_gas: next.input.options.max_fee_wei,
            force: false,
            allow_pending: next.input.options.allow_pending,
            journal: next.request.journal.clone(),
        },
        &next.prepared,
    )?;
    let fields = invoke_fields(&next.net, &next.input, &next.request)?;
    let out = send_prepared(
        &Evm,
        &next.net,
        &next.home,
        next.input.key.index,
        &from,
        next.prepared,
        &SendOptions {
            dry_run: false,
            nonce: None,
            allow_duplicate: next.input.options.allow_duplicate,
            wait: false,
            wait_timeout: None,
            duplicate_window_secs: Some(next.checked.list.policy.duplicate_window_secs()),
        },
        fields,
        Some(lock),
        "invoke",
        progress,
    )
    .await?;
    Ok(SendInvokeOutcome::Sent(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_tron_message_is_refused_before_any_daemon_request() {
        let home = tempfile::tempdir().unwrap();
        let tron = chains::find("tron").unwrap();
        let e = sign_message_bytes(
            home.path(),
            tron,
            0,
            "main",
            "TPjjvMwjPoDC32V2dGDYTkLH4E5LAtBZ6C",
            "hello",
        )
        .await
        .unwrap_err();
        assert_eq!(e.code, ErrorCode::CapabilityUnsupported, "{e}");
        assert!(e.message.contains("message signing"), "{e}");
    }
}
