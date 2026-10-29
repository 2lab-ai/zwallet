//! The transfer flow holds its reviewed intent until a separate nickname confirmation.
//! Workers own requests; UI effects carry only a unique id, so cloning an effect cannot send twice.
use super::*;
use crate::chains::{self, ChainFamily, evm::Evm};
use crate::core::{amount::format_units, error::ErrorCode, whitelist};
use crate::tui::actions::ActionId;
use crate::tui::form::{Edit, Field, Input, Kind};
use crate::wallet::{
    self, CheckedPayee, SendTransferOutcome, TransferInput, TransferOptions, TransferReview,
};
use alloy::primitives::U256;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;

pub const SEND_WAIT: Duration = Duration::from_secs(90);

#[derive(Clone, Debug)]
pub struct Asset {
    pub symbol: String,
    pub decimals: u8,
    pub raw: U256,
    pub token: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Inactive,
    Loading,
    Assets,
    Payees,
    Amount,
    Review,
    Confirm,
    Fee,
    Sending,
    Status,
    Wait,
    Failed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiptState {
    Pending,
    Success,
    Reverted,
    Unknown,
}
#[derive(Debug)]
pub struct SendStatus {
    pub hash: String,
    pub nonce: u64,
    pub journal: Outside,
    pub state: ReceiptState,
    pub wait: Duration,
    pub note: Option<Outside>,
    pub uncertain: bool,
    pub net: Net,
}
#[derive(Debug)]
pub enum TransferMsg {
    Assets(Result<Vec<Asset>>),
    Payees(Result<Vec<CheckedPayee>>),
    Review(Result<Box<TransferReview>>),
    Progress {
        hash: String,
        nonce: u64,
        journal: PathBuf,
    },
    Sent(Result<SendTransferOutcome>),
    Receipt(Result<Option<Value>>),
}

pub struct Transfer {
    pub stage: Stage,
    pub assets: Vec<Asset>,
    pub payees: Vec<CheckedPayee>,
    pub selected: usize,
    pub asset: Option<Asset>,
    pub draft: Option<TransferInput>,
    pub review: Option<TransferReview>,
    pub field: Option<Field>,
    pub notice: Option<Outside>,
    pub status: Option<SendStatus>,
    pub retained: Option<wallet::send::RetainedTransfer>,
    context: Option<(String, &'static str, String)>,
    counter: u64,
    active: Option<u64>,
    watch: Option<u64>,
    cancel_watch: Option<tokio::sync::watch::Sender<bool>>,
    lock_after_send: bool,
    jobs: BTreeMap<u64, TransferJob>,
}
impl Default for Transfer {
    fn default() -> Self {
        Self {
            stage: Stage::Inactive,
            assets: Vec::new(),
            payees: Vec::new(),
            selected: 0,
            asset: None,
            draft: None,
            review: None,
            field: None,
            notice: None,
            status: None,
            retained: None,
            context: None,
            counter: 0,
            active: None,
            watch: None,
            cancel_watch: None,
            lock_after_send: false,
            jobs: BTreeMap::new(),
        }
    }
}

/// Owned work extracted once by the event loop. `run` is also the mock integration-test seam.
pub enum TransferJob {
    Assets {
        net: Net,
        from: String,
        checked: CheckedList,
    },
    Payees {
        net: Net,
        checked: CheckedList,
    },
    Review {
        net: Net,
        checked: CheckedList,
        input: Box<TransferInput>,
    },
    Send {
        review: Box<TransferReview>,
        net: Net,
        from: String,
    },
    Receipt {
        net: Net,
        hash: String,
        timeout: Duration,
        cancel: tokio::sync::watch::Receiver<bool>,
    },
}
fn unchanged(home: &Path, checked: &CheckedList) -> Result<()> {
    whitelist::check_unchanged(home, &checked.fingerprint)
        .map_err(|e| e.with_detail("checkedListChanged", true))
}
impl TransferJob {
    pub async fn run(self, id: u64, home: &Path, tx: &UnboundedSender<Msg>) {
        let event = match self {
            Self::Assets { net, from, checked } => {
                let result = async {
                    unchanged(home, &checked)?;
                    let (native, tokens) = wallet::read::read_network(net, from).await?;
                    let mut rows = vec![Asset {
                        symbol: native.symbol,
                        decimals: native.decimals,
                        raw: native.raw,
                        token: None,
                    }];
                    rows.extend(tokens.into_iter().filter_map(|token| {
                        let raw = token.raw.ok()?;
                        (!raw.is_zero()).then_some(Asset {
                            symbol: token.symbol,
                            decimals: token.decimals,
                            raw,
                            token: Some(token.address),
                        })
                    }));
                    unchanged(home, &checked)?;
                    Ok(rows)
                }
                .await;
                TransferMsg::Assets(result)
            }
            Self::Payees { net, checked } => TransferMsg::Payees(
                unchanged(home, &checked).map(|()| wallet::transfer_payees(&checked, net.chain)),
            ),
            Self::Review {
                net,
                checked,
                input,
            } => {
                let result = async {
                    unchanged(home, &checked)?;
                    let review = wallet::review_transfer(home, &net, &checked, *input).await?;
                    unchanged(home, &checked)?;
                    Ok(review)
                }
                .await;
                TransferMsg::Review(result.map(Box::new))
            }
            Self::Send { review, net, from } => {
                let nonce = review.intent().nonce();
                let journal = chains::SendJournal::new(home, net.chain, &from)
                    .path()
                    .to_path_buf();
                let sender = tx.clone();
                let observer = Arc::new(move |line: &str| {
                    // SendRecord writes this fixed prefix only after fsync. The hash itself is
                    // computed from independently checked signed bytes, not node metadata.
                    if let Some(hash) = line
                        .strip_prefix("broadcasting ")
                        .and_then(|s| s.split_whitespace().next())
                        && hash.parse::<alloy::primitives::B256>().is_ok()
                    {
                        let _ = sender.send(Msg::Transfer(
                            id,
                            TransferMsg::Progress {
                                hash: hash.into(),
                                nonce,
                                journal: journal.clone(),
                            },
                        ));
                    }
                });
                TransferMsg::Sent(wallet::send_transfer(*review, Some(observer)).await)
            }
            Self::Receipt {
                net,
                hash,
                timeout,
                mut cancel,
            } => {
                // The deadline includes connecting to the RPC, not only the polling loop.
                let deadline = tokio::time::Instant::now().checked_add(timeout);
                let poll = async {
                    let future = Evm.wait_for_receipt(&net, &hash, timeout);
                    match deadline {
                        Some(deadline) => tokio::time::timeout_at(deadline, future)
                            .await
                            .unwrap_or(Ok(None)),
                        None => future.await,
                    }
                };
                let result = tokio::select! {
                    _ = cancel.changed() => return,
                    result = poll => result,
                };
                // timeout_at polls a ready inner future before its timer. A delayed task
                // can therefore resume with a receipt after the connect-inclusive deadline,
                // even while the RPC helper's later, post-connect deadline has time left.
                // The same postcheck applies to the initial wait and every typed extension.
                let result = if deadline.is_some_and(|end| tokio::time::Instant::now() >= end) {
                    Ok(None)
                } else {
                    result
                };
                TransferMsg::Receipt(result)
            }
        };
        let _ = tx.send(Msg::Transfer(id, event));
    }
}

impl App {
    /// A confirmed invoke can use the same non-cancellable send/status flow. Its worker sends
    /// TransferMsg::Progress and TransferMsg::Sent(Ok(Sent(output))) with this id. The caller
    /// performs its own immutable invoke comparison before requesting a signature.
    pub fn begin_external_send(&mut self) -> u64 {
        self.transfer.counter += 1;
        let id = self.transfer.counter;
        self.transfer.active = Some(id);
        self.transfer.watch = None;
        if let Some(cancel) = self.transfer.cancel_watch.take() {
            let _ = cancel.send(true);
        }
        self.transfer.stage = Stage::Sending;
        self.transfer.review = None;
        self.transfer.retained = None;
        self.transfer.field = None;
        self.transfer.status = None;
        self.transfer.notice = None;
        self.nav.tab = Tab::Send;
        self.transfer.context = Some((
            self.open_key.clone().unwrap_or_default(),
            self.chain.key,
            self.net().rpc_url.clone(),
        ));
        id
    }
    /// Return to an invoke review that changed before signing. No broadcast was started.
    pub fn return_external_review(&mut self, id: u64) {
        if self.transfer.active == Some(id) && self.transfer.status.is_none() {
            self.transfer.active = None;
            self.transfer.stage = Stage::Inactive;
            self.nav.tab = Tab::Tools;
        }
    }
    pub fn transfer(&self) -> &Transfer {
        &self.transfer
    }
    pub fn take_transfer_job(&mut self, id: u64) -> Option<TransferJob> {
        self.transfer.jobs.remove(&id)
    }
    pub fn sending(&self) -> bool {
        self.transfer.stage == Stage::Sending
    }
    fn transfer_job(&mut self, job: TransferJob, watch: bool) -> Vec<Effect> {
        self.transfer.counter += 1;
        let id = self.transfer.counter;
        if watch {
            self.transfer.watch = Some(id);
        } else {
            self.transfer.active = Some(id);
        }
        self.transfer.jobs.insert(id, job);
        vec![Effect::Transfer(id)]
    }
    fn transfer_ready(&self) -> Result<(String, u32, String, CheckedList)> {
        if self.locking || self.activating || self.password != PasswordState::Idle {
            return Err(WalletError::validation(
                "another wallet operation is in progress",
            ));
        }
        let key = self.verified_key()?;
        let from = key.address.clone().expect("verified key has an address");
        if !matches!(&self.status,Pane::Ready(bar) if matches!(bar.lock,LockLine::Open(_))) {
            return Err(WalletError::locked());
        }
        let checked = self
            .checked_list
            .as_ref()
            .ok_or_else(|| WalletError::validation("needs: V; whitelist not verified"))?;
        Ok((key.name, key.index, from, checked.clone()))
    }
    pub(super) fn show_transfer(&mut self) -> Vec<Effect> {
        let context = (
            self.open_key.clone().unwrap_or_default(),
            self.chain.key,
            self.net().rpc_url.clone(),
        );
        if self.transfer.context.as_ref() == Some(&context) {
            match self.transfer.stage {
                Stage::Status | Stage::Sending | Stage::Wait | Stage::Loading => return Vec::new(),
                Stage::Review => return self.review_draft(),
                Stage::Payees => {
                    let (_, _, _, checked) = match self.transfer_ready() {
                        Ok(v) => v,
                        Err(e) => {
                            self.transfer_error(e);
                            return Vec::new();
                        }
                    };
                    self.transfer.stage = Stage::Loading;
                    return self.transfer_job(
                        TransferJob::Payees {
                            net: self.net().clone(),
                            checked,
                        },
                        false,
                    );
                }
                _ => {}
            }
        }
        self.transfer.context = Some(context);
        self.transfer.review = None;
        self.transfer.draft = None;
        self.transfer.field = None;
        self.transfer.status = None;
        self.transfer.retained = None;
        self.transfer.watch = None;
        if let Some(cancel) = self.transfer.cancel_watch.take() {
            let _ = cancel.send(true);
        }
        self.transfer.active = None;
        self.transfer.notice = None;
        self.transfer.selected = 0;
        let (_, _, from, checked) = match self.transfer_ready() {
            Ok(v) => v,
            Err(e) => {
                self.transfer_error(e);
                return Vec::new();
            }
        };
        self.transfer.stage = Stage::Loading;
        self.transfer_job(
            TransferJob::Assets {
                net: self.net().clone(),
                from,
                checked,
            },
            false,
        )
    }
    fn transfer_error(&mut self, error: WalletError) {
        if error.details.get("checkedListChanged") == Some(&Value::Bool(true))
            || error.message.contains("changed after it was shown")
        {
            self.checked_list = None;
            self.transfer.payees.clear();
            self.transfer.review = None;
            self.transfer.draft = None;
            self.list_reason = Some(Outside::detail(
                "not verified: whitelist changed; V checks it again",
            ));
        }
        self.transfer.notice = Some(Outside::error(&error));
        self.transfer.stage = Stage::Failed;
        self.transfer.active = None;
    }
    fn review_draft(&mut self) -> Vec<Effect> {
        let (_, _, _, checked) = match self.transfer_ready() {
            Ok(v) => v,
            Err(e) => {
                self.transfer_error(e);
                return Vec::new();
            }
        };
        let Some(input) = self.transfer.draft.clone() else {
            return Vec::new();
        };
        self.transfer.review = None;
        self.transfer.field = None;
        self.transfer.notice = None;
        self.transfer.stage = Stage::Loading;
        self.transfer_job(
            TransferJob::Review {
                net: self.net().clone(),
                checked,
                input: Box::new(input),
            },
            false,
        )
    }
    pub(super) fn transfer_update(&mut self, msg: Msg) -> std::ops::ControlFlow<Msg, Vec<Effect>> {
        let Msg::Transfer(id, event) = msg else {
            if let Msg::Paste(text) = &msg
                && self.password == PasswordState::Idle
                && self.nav.popup.is_none()
                && self.nav.tab == Tab::Send
                && self.transfer.field.is_some()
            {
                return std::ops::ControlFlow::Continue(self.transfer_input(Input::Paste(&text.0)));
            }
            return std::ops::ControlFlow::Break(msg);
        };
        if matches!(event, TransferMsg::Receipt(_)) {
            if self.transfer.watch != Some(id) {
                return std::ops::ControlFlow::Continue(Vec::new());
            }
            self.transfer.watch = None;
        } else if self.transfer.active != Some(id) {
            return std::ops::ControlFlow::Continue(Vec::new());
        }
        let send_finished = matches!(&event, TransferMsg::Sent(_));
        let mut work = match event {
            TransferMsg::Assets(Ok(rows)) => {
                self.transfer.active = None;
                self.transfer.assets = rows;
                self.transfer.stage = Stage::Assets;
                Vec::new()
            }
            TransferMsg::Payees(Ok(rows)) => {
                self.transfer.active = None;
                self.transfer.payees = rows;
                self.transfer.selected = 0;
                self.transfer.stage = Stage::Payees;
                Vec::new()
            }
            TransferMsg::Review(Ok(review)) => {
                self.transfer.active = None;
                self.transfer.review = Some(*review);
                self.transfer.stage = Stage::Review;
                Vec::new()
            }
            TransferMsg::Assets(Err(e))
            | TransferMsg::Payees(Err(e))
            | TransferMsg::Review(Err(e)) => {
                self.transfer_error(e);
                Vec::new()
            }
            TransferMsg::Progress {
                hash,
                nonce,
                journal,
            } => {
                self.transfer.status = Some(SendStatus {
                    hash,
                    nonce,
                    journal: Outside::home(&journal.to_string_lossy()),
                    state: ReceiptState::Pending,
                    wait: SEND_WAIT,
                    note: None,
                    uncertain: false,
                    net: self.net().clone(),
                });
                Vec::new()
            }
            TransferMsg::Sent(result) => {
                self.transfer.active = None;
                // q waits for this result: a failed send must leave a failing exit status,
                // including recovery details when its durable journal was already reported.
                if self.quit
                    && let Err(error) = &result
                {
                    self.quit_error.get_or_insert_with(|| error.clone());
                }
                match result {
                    Ok(SendTransferOutcome::ReviewChanged(review)) => {
                        self.transfer.review = Some(*review);
                        self.transfer.stage = Stage::Review;
                        self.transfer.field = None;
                        self.transfer.notice = Some(Outside::detail(
                            "review changed; nothing signed; confirm the new values",
                        ));
                        Vec::new()
                    }
                    Ok(SendTransferOutcome::Sent(_)) => {
                        self.transfer.stage = Stage::Status;
                        self.start_receipt_wait(SEND_WAIT)
                    }
                    Err(error) if self.transfer.status.is_some() => {
                        self.transfer.stage = Stage::Status;
                        if let Some(status) = &mut self.transfer.status {
                            status.state = ReceiptState::Unknown;
                            status.uncertain = error.code == ErrorCode::BroadcastUncertain;
                            status.note = Some(Outside::error(&error));
                        }
                        // An uncertain broadcast must still be watched by its computed hash.
                        if error.code == ErrorCode::BroadcastUncertain {
                            self.start_receipt_wait(SEND_WAIT)
                        } else {
                            Vec::new()
                        }
                    }
                    Err(error) => {
                        self.transfer_error(error);
                        Vec::new()
                    }
                }
            }
            TransferMsg::Receipt(result) => {
                if let Some(status) = &mut self.transfer.status {
                    status.state = match result {
                        Ok(Some(receipt)) => {
                            match receipt.get("success").and_then(Value::as_bool) {
                                Some(true) => ReceiptState::Success,
                                Some(false) => ReceiptState::Reverted,
                                None => ReceiptState::Unknown,
                            }
                        }
                        Ok(None) => ReceiptState::Unknown,
                        Err(error) => {
                            status.note = Some(Outside::error(&error));
                            ReceiptState::Unknown
                        }
                    };
                }
                Vec::new()
            }
        };
        if send_finished && std::mem::take(&mut self.transfer.lock_after_send) {
            work.push(Effect::LockAll);
        }
        std::ops::ControlFlow::Continue(work)
    }
    fn start_receipt_wait(&mut self, timeout: Duration) -> Vec<Effect> {
        let Some(status) = &mut self.transfer.status else {
            return Vec::new();
        };
        status.wait = timeout;
        status.state = ReceiptState::Pending;
        if let Some(cancel) = self.transfer.cancel_watch.take() {
            let _ = cancel.send(true);
        }
        let (cancel, receiver) = tokio::sync::watch::channel(false);
        self.transfer.cancel_watch = Some(cancel);
        let job = TransferJob::Receipt {
            net: status.net.clone(),
            hash: status.hash.clone(),
            timeout,
            cancel: receiver,
        };
        self.transfer_job(job, true)
    }
    pub(super) fn transfer_action(&mut self, action: ActionId) -> Option<Vec<Effect>> {
        let key = match action {
            ActionId::Transfer | ActionId::TokenTransfer | ActionId::DryRun => {
                self.nav.tab = Tab::Send;
                if self.transfer.stage == Stage::Status {
                    self.transfer.context = None;
                }
                return Some(self.show_transfer());
            }
            ActionId::AllowPending => Key::Char('p'),
            ActionId::AllowDuplicate => Key::Char('d'),
            ActionId::GasCap => Key::Char('g'),
            ActionId::Wait => Key::Char('w'),
            _ => return None,
        };
        self.nav.tab = Tab::Send;
        Some(self.transfer_key(key).unwrap_or_default())
    }
    pub(super) fn transfer_key(&mut self, key: Key) -> Option<Vec<Effect>> {
        if self.sending() {
            if key == Key::Char('L') {
                self.locking = true;
                self.transfer.lock_after_send = true;
            }
            if matches!(key, Key::Char('q') | Key::Interrupt) {
                self.quit = true;
            }
            return Some(Vec::new());
        }
        if self.nav.tab != Tab::Send || self.nav.popup.is_some() {
            return None;
        }
        if self.transfer.field.is_some() {
            return Some(self.transfer_input(super::navigation::input(key)));
        }
        self.transfer.notice = None;
        let work = match (self.transfer.stage, key) {
            (Stage::Assets | Stage::Payees, Key::Up | Key::Char('k')) => {
                self.transfer.selected = self.transfer.selected.saturating_sub(1);
                Vec::new()
            }
            (Stage::Assets | Stage::Payees, Key::Down | Key::Char('j')) => {
                let len = if self.transfer.stage == Stage::Assets {
                    self.transfer.assets.len()
                } else {
                    self.transfer.payees.len()
                };
                self.transfer.selected = (self.transfer.selected + 1).min(len.saturating_sub(1));
                Vec::new()
            }
            (Stage::Assets, Key::Enter) => {
                let Some(asset) = self.transfer.assets.get(self.transfer.selected).cloned() else {
                    return Some(Vec::new());
                };
                self.transfer.asset = Some(asset);
                let (_, _, _, checked) = match self.transfer_ready() {
                    Ok(v) => v,
                    Err(e) => {
                        self.transfer_error(e);
                        return Some(Vec::new());
                    }
                };
                self.transfer.stage = Stage::Loading;
                self.transfer_job(
                    TransferJob::Payees {
                        net: self.net().clone(),
                        checked,
                    },
                    false,
                )
            }
            (Stage::Payees, Key::Enter) => {
                let Some(payee) = self.transfer.payees.get(self.transfer.selected).cloned() else {
                    return Some(Vec::new());
                };
                let (key, index, from, _) = match self.transfer_ready() {
                    Ok(v) => v,
                    Err(e) => {
                        self.transfer_error(e);
                        return Some(Vec::new());
                    }
                };
                let token = self.transfer.asset.as_ref().and_then(|a| a.token.clone());
                self.transfer.draft = Some(TransferInput {
                    index,
                    key_name: key,
                    from,
                    payee,
                    amount: String::new(),
                    token,
                    options: TransferOptions::default(),
                });
                self.transfer.field = Some(Field::new(Kind::Amount));
                self.transfer.stage = Stage::Amount;
                Vec::new()
            }
            (Stage::Review, Key::Enter) => {
                if let Some(error) = self
                    .transfer
                    .review
                    .as_ref()
                    .and_then(|r| r.verdict().as_ref().err())
                    .cloned()
                {
                    self.transfer_error(error);
                    return Some(Vec::new());
                }
                if let Err(e) = self.transfer_ready() {
                    self.transfer_error(e);
                    return Some(Vec::new());
                }
                self.transfer.field = Some(Field::new(Kind::Nickname));
                self.transfer.stage = Stage::Confirm;
                Vec::new()
            }
            (Stage::Review | Stage::Failed, Key::Char('p') | Key::Char('d')) => {
                if let Some(draft) = &mut self.transfer.draft {
                    if key == Key::Char('p') {
                        draft.options.allow_pending = !draft.options.allow_pending;
                    } else {
                        draft.options.allow_duplicate = !draft.options.allow_duplicate;
                    }
                }
                self.review_draft()
            }
            (Stage::Review | Stage::Failed, Key::Char('g')) => {
                self.transfer.field = Some(Field::new(Kind::Amount));
                self.transfer.stage = Stage::Fee;
                Vec::new()
            }
            (Stage::Status, Key::Char('n')) => {
                self.transfer.context = None;
                self.show_transfer()
            }
            (Stage::Status, Key::Char('w')) => {
                self.transfer.field = Some(Field::new(Kind::Duration));
                self.transfer.stage = Stage::Wait;
                Vec::new()
            }
            (Stage::Review | Stage::Failed, Key::Char('r')) if self.transfer.draft.is_some() => {
                self.review_draft()
            }
            (Stage::Failed, Key::Char('r')) => {
                self.transfer.context = None;
                self.show_transfer()
            }
            (Stage::Review, Key::Esc) => {
                self.transfer.review = None;
                self.transfer.field = Some(Field::new(Kind::Amount));
                self.transfer.stage = Stage::Amount;
                Vec::new()
            }
            (Stage::Payees, Key::Esc) => {
                self.transfer.stage = Stage::Assets;
                self.transfer.selected = 0;
                Vec::new()
            }
            (Stage::Status, Key::Char('s') | Key::Char('x') | Key::Char('R')) => {
                self.transfer.notice = Some(Outside::detail(
                    "replacements are in a later version; use zwallet tx first",
                ));
                Vec::new()
            }
            (Stage::Payees, Key::Char(c)) if !matches!(c, 'q' | '?' | ':') => Vec::new(),
            _ => return None,
        };
        Some(work)
    }
    fn transfer_input(&mut self, input: Input<'_>) -> Vec<Effect> {
        let Some(mut field) = self.transfer.field.take() else {
            return Vec::new();
        };
        self.transfer.notice = None;
        match field.edit(input) {
            Edit::Submit => match self.transfer.stage {
                Stage::Amount => {
                    if let Some(draft) = &mut self.transfer.draft {
                        draft.amount = field.value().into();
                    }
                    return self.review_draft();
                }
                Stage::Fee => match wallet::parse_fee_ceiling(field.value()) {
                    Ok(fee) => {
                        if let Some(draft) = &mut self.transfer.draft {
                            draft.options.max_fee_wei = Some(fee);
                        }
                        return self.review_draft();
                    }
                    Err(error) => self.transfer.notice = Some(Outside::error(&error)),
                },
                Stage::Confirm => {
                    if let Err(e) = self.transfer_ready() {
                        self.transfer_error(e);
                        return Vec::new();
                    }
                    if self
                        .transfer
                        .review
                        .as_ref()
                        .is_some_and(|r| r.payee().nickname() == field.value())
                    {
                        let review = self.transfer.review.take().expect("confirmed review");
                        self.transfer.retained = Some(review.retained_intent());
                        let from = review.from().to_string();
                        self.transfer.stage = Stage::Sending;
                        self.transfer.status = None;
                        return self.transfer_job(
                            TransferJob::Send {
                                review: Box::new(review),
                                net: self.net().clone(),
                                from,
                            },
                            false,
                        );
                    }
                    self.transfer.notice = Some(Outside::detail("type the exact payee nickname"));
                }
                Stage::Wait => match crate::core::time::parse_duration(field.value()) {
                    Ok(timeout) => {
                        self.transfer.stage = Stage::Status;
                        return self.start_receipt_wait(timeout);
                    }
                    Err(error) => self.transfer.notice = Some(Outside::error(&error)),
                },
                _ => {}
            },
            Edit::Cancel => {
                self.transfer.stage = match self.transfer.stage {
                    Stage::Amount => Stage::Payees,
                    Stage::Wait => Stage::Status,
                    _ => Stage::Review,
                };
                return Vec::new();
            }
            Edit::Interrupt => {
                self.quit = true;
                return Vec::new();
            }
            Edit::Rejected(reason) => self.transfer.notice = Some(Outside::detail(reason)),
            _ => {}
        }
        self.transfer.field = Some(field);
        Vec::new()
    }
    pub fn transfer_hint(&self) -> &'static str {
        match self.transfer.stage {
            Stage::Amount | Stage::Fee | Stage::Confirm | Stage::Wait => {
                "type value  Enter accept  Esc back  Ctrl-C quit"
            }
            Stage::Loading => "reading...  q quit  ? actions",
            Stage::Sending => "sending... q quits when done; do not resend  ? actions",
            Stage::Review => "Enter confirm  p allow pending  d allow repeat  g fee  ? actions",
            Stage::Payees | Stage::Assets => {
                "arrows choose  Enter select  Esc back  q quit  ? actions"
            }
            Stage::Status => "w wait longer  n new payment  2 wallet  q quit  ? actions",
            _ => "V verify  u warm-up  r retry  q quit  ? actions",
        }
    }
    pub fn asset_label(asset: &Asset) -> String {
        format!(
            "{}  balance {}",
            asset.symbol,
            format_units(asset.raw, asset.decimals)
        )
    }
}

#[cfg(test)]
mod quit_error_tests {
    use super::*;

    fn app() -> App {
        App::new(
            &Net {
                chain: chains::find("ethereum").unwrap(),
                rpc_url: "http://127.0.0.1:1".into(),
            },
            Duration::from_secs(600),
        )
    }

    #[test]
    fn quitting_send_preserves_original_error_before_and_after_journal_progress() {
        for progress in [false, true] {
            for deferred_lock in [false, true] {
                for code in [ErrorCode::RpcError, ErrorCode::BroadcastUncertain] {
                    let mut app = app();
                    let id = app.begin_external_send();
                    if progress {
                        app.update(Msg::Transfer(
                            id,
                            TransferMsg::Progress {
                                hash: format!("0x{}", "1".repeat(64)),
                                nonce: 7,
                                journal: "/tmp/unused-journal".into(),
                            },
                        ));
                    }
                    if deferred_lock {
                        app.update(Msg::Key(Key::Char('L')));
                    }
                    app.update(Msg::Key(Key::Char('q')));
                    assert!(!app.quitting());
                    let original = WalletError::new(code, "original send failure")
                        .with_detail("nonce", 7)
                        .with_detail("txHash", format!("0x{}", "1".repeat(64)));
                    let work =
                        app.update(Msg::Transfer(id, TransferMsg::Sent(Err(original.clone()))));
                    if deferred_lock {
                        assert!(work.contains(&Effect::LockAll));
                        assert!(!app.quitting());
                        // A later cleanup failure must not erase the send's recovery details.
                        app.update(Msg::Locked(Err(WalletError::validation(
                            "secondary stop failure",
                        ))));
                    }
                    assert!(app.quitting());
                    assert_eq!(
                        app.take_quit_error(),
                        Some(original),
                        "progress={progress}, deferred lock={deferred_lock}"
                    );
                }
            }
        }
    }

    #[test]
    fn stale_or_nonquitting_send_failure_does_not_become_a_quit_error() {
        let mut app = app();
        let id = app.begin_external_send();
        app.update(Msg::Transfer(
            id + 1,
            TransferMsg::Sent(Err(WalletError::rpc("stale worker"))),
        ));
        assert!(app.sending());
        assert!(app.take_quit_error().is_none());
        app.update(Msg::Transfer(
            id,
            TransferMsg::Sent(Err(WalletError::rpc("shown in UI"))),
        ));
        assert!(!app.sending());
        assert!(!app.quitting());
        assert!(app.transfer().notice.is_some());
        assert!(app.take_quit_error().is_none());
    }
}
