//! Immutable review/confirm for TRX and registry USDT. No nonce, replacement or resend API.
use super::{
    CheckedList,
    send::{SendOptions, SendOutput, send_prepared, send_summary},
};
use crate::{
    chains::{
        self, Family, Net, TransferRequest,
        policy::ListView,
        tron::{
            self, address, registry,
            tx::{self, Contract, DecodedTron},
        },
    },
    core::{
        amount::parse_units,
        error::{Result, WalletError},
        whitelist::{self, EntryKind, Fingerprint},
    },
};
use alloy::{hex, primitives::U256};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

/// An address from a password-checked list whose scope is exactly `["tron"]`.
#[derive(Clone, Debug)]
pub struct TronCheckedPayee {
    nickname: String,
    address: String,
    fingerprint: Fingerprint,
}
impl TronCheckedPayee {
    pub fn nickname(&self) -> &str {
        &self.nickname
    }
    pub fn address(&self) -> &str {
        &self.address
    }
}
/// Call `check_unchanged` before showing choices. Contracts and `*` entries are never offered.
pub fn tron_transfer_payees(checked: &CheckedList) -> Vec<TronCheckedPayee> {
    checked
        .list
        .entries
        .iter()
        .filter(|e| e.kind == EntryKind::Address && e.chains == ["tron"])
        .filter_map(|e| {
            let body: [u8; 20] = hex::decode(e.address.strip_prefix("0x")?)
                .ok()?
                .try_into()
                .ok()?;
            (body != [0; 20]).then(|| TronCheckedPayee {
                nickname: e.nickname.clone(),
                address: address::from_body(&body),
                fingerprint: checked.fingerprint,
            })
        })
        .collect()
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TronAsset {
    Trx,
    Usdt,
}
impl TronAsset {
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Trx => "TRX",
            Self::Usdt => "USDT",
        }
    }
}
/// `index` and `from` must come from this session's password-derived key pins, never a fresh
/// metadata row or daemon status. Confirm consumes these held values and accepts no new key.
#[derive(Clone, Debug)]
pub struct TronTransferInput {
    /// Advisory local task attribution, never signer authority.
    pub task_id: Option<String>,
    pub key_name: String,
    pub index: u32,
    pub from: String,
    pub payee: TronCheckedPayee,
    pub asset: TronAsset,
    pub amount: String,
    pub allow_duplicate: bool,
}
#[derive(Debug)]
pub struct TronTransferReview {
    home: PathBuf,
    net: Net,
    checked: CheckedList,
    input: TronTransferInput,
    request: TransferRequest,
    prepared: tron::send::PreparedTronTransfer,
    decoded: DecodedTron,
    verdict: Result<()>,
    fields: Map<String, Value>,
}
impl TronTransferReview {
    pub fn fields(&self) -> &Map<String, Value> {
        &self.fields
    }
    pub fn intent(&self) -> &DecodedTron {
        &self.decoded
    }
    pub fn payee(&self) -> &TronCheckedPayee {
        &self.input.payee
    }
    pub fn from(&self) -> &str {
        &self.input.from
    }
    pub fn key_name(&self) -> &str {
        &self.input.key_name
    }
    pub fn index(&self) -> u32 {
        self.input.index
    }
    pub fn asset(&self) -> TronAsset {
        self.input.asset
    }
    pub fn allow_duplicate(&self) -> bool {
        self.input.allow_duplicate
    }
    pub fn fingerprint(&self) -> Fingerprint {
        self.checked.fingerprint
    }
    pub fn verdict(&self) -> &Result<()> {
        &self.verdict
    }
    pub fn sufficient_funds(&self) -> bool {
        self.prepared.balance_sun >= self.prepared.max_cost_sun
    }
    pub fn max_fee_sun(&self) -> U256 {
        self.prepared.max_fee_sun
    }
    pub fn max_cost_sun(&self) -> U256 {
        self.prepared.max_cost_sun
    }
    pub fn estimated_cost_sun(&self) -> U256 {
        self.prepared.estimated_cost_sun
    }
    pub fn trx_left_sun(&self) -> U256 {
        self.prepared
            .balance_sun
            .saturating_sub(self.prepared.max_cost_sun)
    }
    pub fn fee_cap_sun(&self) -> Option<U256> {
        self.checked.list.policy.max_fee_wei("tron")
    }
    fn matches(&self, next: &Self) -> bool {
        self.input.task_id == next.input.task_id
            && self.decoded.owner == next.decoded.owner
            && self.decoded.contract == next.decoded.contract
            && self.input.asset == next.input.asset
            && self.input.from == next.input.from
            && self.input.index == next.input.index
            && self.input.key_name == next.input.key_name
            && self.verdict == next.verdict
            && self.sufficient_funds() == next.sufficient_funds()
            && next.max_fee_sun() <= self.max_fee_sun()
            && next.max_cost_sun() <= self.max_cost_sun()
    }
}
#[derive(Debug)]
pub enum SendTronTransferOutcome {
    Sent(SendOutput),
    ReviewChanged(Box<TronTransferReview>),
}
fn kind(asset: TronAsset) -> &'static str {
    if asset == TronAsset::Trx {
        "transfer"
    } else {
        "token-transfer"
    }
}
fn fields(input: &TronTransferInput) -> Map<String, Value> {
    let mut fields = json!({"chain":"tron","key":input.key_name,"from":input.from,"to":input.payee.address,"toNickname":input.payee.nickname}).as_object().unwrap().clone();
    if let Some(task) = &input.task_id {
        fields.insert("taskId".into(), task.clone().into());
    }
    fields
}
fn decode(unsigned: &Value) -> Result<DecodedTron> {
    let raw = unsigned
        .as_str()
        .and_then(|s| s.strip_prefix("0x"))
        .and_then(|s| hex::decode(s).ok())
        .ok_or_else(|| WalletError::validation("missing Tron intent bytes"))?;
    tx::decode_raw(&raw)
}
fn assemble(
    home: PathBuf,
    net: Net,
    checked: CheckedList,
    input: TronTransferInput,
    request: TransferRequest,
    prepared: tron::send::PreparedTronTransfer,
) -> Result<TronTransferReview> {
    let decoded = decode(&prepared.prepared.unsigned)?;
    let owner = address::parse(&input.from)?;
    let to = address::parse(input.payee.address())?;
    let amount = parse_units(&input.amount, 6)?;
    let same = match (&decoded.contract, input.asset) {
        (
            Contract::Transfer {
                to: actual,
                amount_sun,
            },
            TronAsset::Trx,
        ) => *actual == to && U256::from(*amount_sun) == amount,
        (
            Contract::TriggerSmart {
                contract_address, ..
            },
            TronAsset::Usdt,
        ) => registry::is_token(contract_address) && decoded.transfer_call() == Some((to, amount)),
        _ => false,
    };
    if decoded.owner != owner || !same {
        return Err(WalletError::validation(
            "prepared Tron transaction differs from the held owner, payee, registry asset or amount; nothing signed",
        ));
    }
    let verdict = tron::policy::check_tx(
        &ListView::Usable(&checked.list),
        owner,
        &decoded,
        tron::send::now_ms(),
    );
    let mut fields = fields(&input);
    fields.extend(prepared.prepared.fields.clone());
    fields.insert(
        "trxLeftSun".into(),
        json!(
            prepared
                .balance_sun
                .saturating_sub(prepared.max_cost_sun)
                .to_string()
        ),
    );
    let window = checked.list.policy.duplicate_window_secs();
    let journal =
        chains::SendJournal::new(&home, net.chain, &input.from).with_duplicate_window(window)?;
    fields.insert("duplicateGuard".into(), json!({"windowSecs":window,"journalTailBytes":1 << 20,"advisory":true,"policySource":"password-checked-list"}));
    let duplicates = journal.recent_duplicates(
        &send_summary(&fields, net.chain, kind(input.asset)),
        crate::core::time::unix_secs(std::time::SystemTime::now()),
    )?;
    fields.insert(
        "duplicateSends".into(),
        duplicates
            .iter()
            .map(crate::core::text::inert_json)
            .collect(),
    );
    fields.insert(
        "whitelist".into(),
        match &verdict {
            Ok(()) => json!({"allowed":true}),
            Err(e) => json!({"allowed":false,"reason":e.message,"code":e.code.as_str()}),
        },
    );
    Ok(TronTransferReview {
        home,
        net,
        checked,
        input,
        request,
        prepared,
        decoded,
        verdict,
        fields,
    })
}
/// Uses only the password-checked list. Fingerprints bracket every RPC preparation; no signing.
pub async fn review_tron_transfer(
    home: &Path,
    net: &Net,
    checked: &CheckedList,
    input: TronTransferInput,
) -> Result<TronTransferReview> {
    if !matches!(net.chain.family, Family::Tron(_)) {
        return Err(WalletError::validation(
            "Tron transfer requires the tron chain",
        ));
    }
    whitelist::check_unchanged(home, &checked.fingerprint)?;
    if let Some(task) = &input.task_id {
        super::tasks::validate_binding_async(home, task, net.chain, &input.from).await?;
    }
    if input.payee.fingerprint != checked.fingerprint
        || !tron_transfer_payees(checked)
            .iter()
            .any(|p| p.nickname == input.payee.nickname && p.address == input.payee.address)
    {
        return Err(WalletError::validation(
            "payee is not from this password-checked Tron list",
        ));
    }
    address::parse(&input.from)?;
    let request = TransferRequest {
        from: input.from.clone(),
        to: input.payee.address.clone(),
        amount: input.amount.clone(),
        token: (input.asset == TronAsset::Usdt).then(|| "usdt".into()),
        dry_run: true,
        nonce: None,
        bump: None,
        replace: None,
        max_fee_per_gas: None,
        force: false,
        allow_pending: false,
        journal: Some(chains::SendJournal::new(home, net.chain, &input.from)),
    };
    let prepared = tron::send::prepare_review(net, &request).await?;
    whitelist::check_unchanged(home, &checked.fingerprint)?;
    assemble(
        home.into(),
        net.clone(),
        checked.clone(),
        input,
        request,
        prepared,
    )
}
/// Type the held nickname exactly, then refresh under the send lock. The block reference and
/// expiration may advance; the payment must be identical and the integer maximum cannot rise.
pub async fn send_tron_transfer(
    review: TronTransferReview,
    typed_nickname: &str,
    progress: Option<chains::SendProgress>,
) -> Result<SendTronTransferOutcome> {
    if typed_nickname != review.payee().nickname() {
        return Err(WalletError::validation(
            "type the payee nickname exactly; nothing signed",
        ));
    }
    whitelist::check_unchanged(&review.home, &review.checked.fingerprint)?;
    let lock = chains::SendLock::acquire(&review.home, review.net.chain, &review.input.from)?;
    let prepared = tron::send::prepare_review(&review.net, &review.request).await?;
    whitelist::check_unchanged(&review.home, &review.checked.fingerprint)?;
    let next = assemble(
        review.home.clone(),
        review.net.clone(),
        review.checked.clone(),
        review.input.clone(),
        review.request.clone(),
        prepared,
    )?;
    if !review.matches(&next) {
        return Ok(SendTronTransferOutcome::ReviewChanged(Box::new(next)));
    }
    next.verdict?;
    let options = SendOptions {
        dry_run: false,
        nonce: None,
        allow_duplicate: next.input.allow_duplicate,
        wait: false,
        wait_timeout: None,
        duplicate_window_secs: Some(next.checked.list.policy.duplicate_window_secs()),
    };
    let out = send_prepared(
        &tron::Tron,
        &next.net,
        &next.home,
        next.input.index,
        &next.input.from,
        next.prepared.prepared,
        &options,
        fields(&next.input),
        Some(lock),
        kind(next.input.asset),
        progress,
    )
    .await?;
    Ok(SendTronTransferOutcome::Sent(out))
}
