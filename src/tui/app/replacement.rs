//! Fixed-nonce review/confirmation. Effects are single-use job ids; reads cannot become sends.
use super::history::{DetailRequest, TransactionDetail};
use super::transfer::{ReceiptState, TransferMsg};
use super::*;
use crate::chains;
use crate::core::whitelist;
use crate::tui::actions::ActionId;
use crate::tui::form::{Edit, Field, Input, Kind};
use crate::wallet::replacement::{
    self as backend, Action, Options, OriginalIntent, ReplacementReview, SendReplacementOutcome,
};
use crate::wallet::{self, KeyRow};
use std::ops::ControlFlow;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stage {
    #[default]
    Idle,
    Preparing,
    Review,
    Confirm,
    Bump,
    Sending,
    Failed,
}

#[derive(Clone, Debug)]
pub struct Context {
    net: Net,
    key: KeyRow,
    checked: CheckedList,
    hash: String,
    nonce: u64,
    action: Action,
    options: Options,
    original: Option<OriginalIntent>,
}

#[derive(Default)]
pub struct Replacement {
    pub stage: Stage,
    pub review: Option<ReplacementReview>,
    pub field: Option<Field>,
    pub notice: Option<Outside>,
    pub scroll: usize,
    context: Option<Context>,
    counter: u64,
    active: Option<u64>,
    external_send: Option<u64>,
    inflight: Option<OriginalIntent>,
    retained: BTreeMap<String, OriginalIntent>,
    cancel_hashes: std::collections::BTreeSet<String>,
    jobs: BTreeMap<u64, Job>,
    proof_id: Option<u64>,
    proof: Option<(DetailRequest, Pane<OriginalIntent>)>,
}

#[derive(Debug)]
pub enum Event {
    Proven(Box<DetailRequest>, Result<Box<OriginalIntent>>),
    Reviewed(Result<Box<ReplacementReview>>),
    Sent {
        transfer_id: u64,
        result: Result<SendReplacementOutcome>,
    },
}

pub enum Job {
    Prove(Box<Context>, Box<DetailRequest>),
    Prepare(Box<Context>),
    Send {
        review: Box<ReplacementReview>,
        typed: String,
        transfer_id: u64,
        net: Net,
    },
}

impl Job {
    pub async fn run(self, id: u64, home: &Path, sender: &UnboundedSender<Msg>) {
        let event = match self {
            Self::Prove(context, request) => {
                Event::Proven(request, prove(home, &context).await.map(Box::new))
            }
            Self::Prepare(context) => {
                let result = async {
                    whitelist::check_unchanged(home, &context.checked.fingerprint)?;
                    let original = match &context.original {
                        Some(original) => original.clone(),
                        None => {
                            if context.action == Action::Retry {
                                return Err(WalletError::validation(backend::RETRY_WARNING));
                            }
                            prove(home, &context).await?
                        }
                    };
                    backend::review_replacement(
                        home,
                        &context.net,
                        &context.checked,
                        &original,
                        &context.hash,
                        context.action,
                        context.options,
                    )
                    .await
                    .map(Box::new)
                }
                .await;
                Event::Reviewed(result)
            }
            Self::Send {
                review,
                typed,
                transfer_id,
                net,
            } => {
                let nonce = review.intent().nonce();
                let journal = chains::SendJournal::new(home, net.chain, &review.original().from())
                    .path()
                    .to_path_buf();
                let tx = sender.clone();
                let observer = Arc::new(move |line: &str| {
                    if let Some(hash) = line
                        .strip_prefix("broadcasting ")
                        .and_then(|s| s.split_whitespace().next())
                        && hash.parse::<alloy::primitives::B256>().is_ok()
                    {
                        let _ = tx.send(Msg::Transfer(
                            transfer_id,
                            TransferMsg::Progress {
                                hash: hash.into(),
                                nonce,
                                journal: journal.clone(),
                            },
                        ));
                    }
                });
                Event::Sent {
                    transfer_id,
                    result: backend::send_replacement(*review, &typed, Some(observer)).await,
                }
            }
        };
        let _ = sender.send(Msg::Replacement(id, event));
    }
}

async fn prove(home: &Path, context: &Context) -> Result<OriginalIntent> {
    whitelist::check_unchanged(home, &context.checked.fingerprint)?;
    let from = context
        .key
        .address
        .as_deref()
        .ok_or_else(|| WalletError::validation("needs: V"))?;
    let row = chains::SendJournal::new(home, context.net.chain, from)
        .recent()?
        .into_iter()
        .find(|row| row.tx_hash.eq_ignore_ascii_case(&context.hash))
        .ok_or_else(|| {
            WalletError::validation(
                "there is no matching local send; look up the hash with zwallet tx",
            )
        })?;
    let original = backend::replaced_intent(
        &context.net,
        &context.hash,
        &context.key,
        context.nonce,
        &row,
    )
    .await?;
    whitelist::check_unchanged(home, &context.checked.fingerprint)?;
    Ok(original)
}

impl App {
    /// A pending detail receives an independent proof before replacement controls appear.
    pub(super) fn prepare_detail_replacement(&mut self) -> Vec<Effect> {
        self.replacement.proof_id = None;
        self.replacement.proof = None;
        let Some((request, Pane::Ready(TransactionDetail::Evm(detail)))) = &self.history.detail
        else {
            return Vec::new();
        };
        if detail.state != crate::history::State::Pending
            || self.replacement_original(&request.hash).is_some()
        {
            return Vec::new();
        }
        let (key, checked) = match self.replacement_ready() {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };
        let Some(nonce) = detail.nonce else {
            return Vec::new();
        };
        let Ok(net) = request.scope.target.net() else {
            return Vec::new();
        };
        let request = request.clone();
        let context = Context {
            net,
            key,
            checked,
            hash: request.hash.clone(),
            nonce,
            action: Action::SpeedUp,
            options: Options::default(),
            original: None,
        };
        if !self.replacement_context_current(&context) {
            return Vec::new();
        }
        self.replacement.counter += 1;
        let id = self.replacement.counter;
        self.replacement.proof_id = Some(id);
        self.replacement.proof = Some((request.clone(), Pane::Loading));
        self.replacement
            .jobs
            .insert(id, Job::Prove(Box::new(context), Box::new(request)));
        vec![Effect::Replacement(id)]
    }
    pub fn history_replacement_note(&self) -> Option<&str> {
        let (request, Pane::Ready(TransactionDetail::Evm(detail))) =
            self.history.detail.as_ref()?
        else {
            return None;
        };
        if matches!(
            detail.state,
            crate::history::State::NotFound | crate::history::State::Unknown
        ) {
            return Some(backend::RETRY_WARNING);
        }
        if detail.state != crate::history::State::Pending {
            return None;
        }
        if self.verified_key().is_err() {
            return Some("needs: V; verify the key before replacing a transaction");
        }
        if let Some((proof_request, pane)) = &self.replacement.proof
            && proof_request == request
        {
            return match pane {
                Pane::Loading => {
                    Some("checking the original hash and recovered signer before offering s/x")
                }
                Pane::Failed(error) => Some(error.text()),
                Pane::Ready(_) => None,
            };
        }
        None
    }
    pub fn history_replacement_hint(&self) -> &'static str {
        if self.on_tron() {
            // Tron has no nonce replacement.
            "s/x/R unavailable   r refresh   Esc history"
        } else if self.replacement_available().0 {
            "s speed up   x cancel   r refresh   Esc history"
        } else if self.verified_key().is_err() {
            "needs: V   r refresh   Esc history"
        } else {
            "r refresh   Esc history"
        }
    }
    fn history_replacement_key(&mut self, key: Key) -> Vec<Effect> {
        if key == Key::Char('R') {
            self.notice = Some(Outside::detail(
                "Retry is only available on this session's uncertain send status; look up the hash first",
            ));
            return Vec::new();
        }
        if self.verified_key().is_err() {
            self.notice = Some(Outside::detail("needs: V"));
            return Vec::new();
        }
        if !self.replacement_available().0 {
            self.notice = Some(Outside::detail(self.history_replacement_note().unwrap_or(
                "no authenticated pending original; look up the hash with zwallet tx",
            )));
            return Vec::new();
        }
        let Some((request, Pane::Ready(TransactionDetail::Evm(detail)))) = &self.history.detail
        else {
            return Vec::new();
        };
        let Some(nonce) = detail.nonce else {
            return Vec::new();
        };
        let Ok(net) = request.scope.target.net() else {
            return Vec::new();
        };
        let hash = request.hash.clone();
        self.start_history_replacement(
            if key == Key::Char('s') {
                Action::SpeedUp
            } else {
                Action::Cancel
            },
            net,
            hash,
            nonce,
        )
    }
    pub fn replacement(&self) -> &Replacement {
        &self.replacement
    }
    pub fn take_replacement_job(&mut self, id: u64) -> Option<Job> {
        self.replacement.jobs.remove(&id)
    }
    pub fn replacement_visible(&self) -> bool {
        self.nav.tab == Tab::Send && !matches!(self.replacement.stage, Stage::Idle | Stage::Sending)
    }
    pub fn register_retained_replacement(&mut self, hash: &str, original: OriginalIntent) {
        if hash.parse::<alloy::primitives::B256>().is_ok() {
            // Bound session metadata. Eviction removes an affordance; it never creates authority.
            if self.replacement.retained.len() >= 64 {
                self.replacement.retained.pop_first();
            }
            self.replacement
                .retained
                .insert(hash.to_ascii_lowercase(), original);
        }
    }
    fn replacement_original(&self, hash: &str) -> Option<OriginalIntent> {
        self.replacement
            .retained
            .get(&hash.to_ascii_lowercase())
            .cloned()
            .or_else(|| {
                self.transfer
                    .status
                    .as_ref()
                    .filter(|s| s.hash.eq_ignore_ascii_case(hash))?;
                OriginalIntent::from_transfer(self.transfer.retained.as_ref()?).ok()
            })
            .or_else(|| {
                self.tools
                    .retained
                    .get(hash)
                    .and_then(|send| OriginalIntent::from_invoke(&send.intent).ok())
            })
            .or_else(|| {
                let (request, Pane::Ready(original)) = self.replacement.proof.as_ref()? else {
                    return None;
                };
                (request.hash.eq_ignore_ascii_case(hash)
                    && self
                        .history
                        .detail
                        .as_ref()
                        .is_some_and(|(r, _)| r == request))
                .then(|| original.clone())
            })
    }
    fn replacement_ready(&self) -> Result<(KeyRow, CheckedList)> {
        if self.locking || self.activating || self.password != PasswordState::Idle || self.sending()
        {
            return Err(WalletError::validation(
                "another wallet operation is in progress",
            ));
        }
        let key = self.verified_key()?;
        if !matches!(&self.status,Pane::Ready(bar) if matches!(bar.lock,LockLine::Open(_))) {
            return Err(WalletError::locked());
        }
        let checked = self
            .checked_list
            .clone()
            .ok_or_else(|| WalletError::validation("needs: V; whitelist not verified"))?;
        Ok((key, checked))
    }
    fn replacement_error(&mut self, error: WalletError) {
        if error.message.contains("changed after it was shown") {
            self.checked_list = None;
            self.list_reason = Some(Outside::detail(
                "not verified: whitelist changed; V checks it again",
            ));
        }
        self.replacement.notice = Some(Outside::error(&error));
        self.replacement.stage = Stage::Failed;
        self.replacement.active = None;
        self.replacement.review = None;
        self.replacement.field = None;
    }
    fn replacement_job(&mut self, job: Job) -> Vec<Effect> {
        self.replacement.counter += 1;
        let id = self.replacement.counter;
        self.replacement.active = Some(id);
        self.replacement.jobs.insert(id, job);
        vec![Effect::Replacement(id)]
    }
    fn replacement_context_current(&self, context: &Context) -> bool {
        self.net().chain.key == context.net.chain.key
            && self.net().rpc_url == context.net.rpc_url
            && self.verified_key().is_ok_and(|key| {
                key.name == context.key.name
                    && key.index == context.key.index
                    && key.address == context.key.address
            })
            && self
                .checked_list
                .as_ref()
                .is_some_and(|list| list.fingerprint == context.checked.fingerprint)
    }
    fn prepare_replacement_again(&mut self) -> Vec<Effect> {
        let Some(context) = self.replacement.context.clone() else {
            return Vec::new();
        };
        if !self.replacement_context_current(&context) {
            self.replacement_error(WalletError::validation(
                "replacement context changed; verify the key and list again",
            ));
            return Vec::new();
        }
        self.replacement.stage = Stage::Preparing;
        self.replacement.review = None;
        self.replacement.field = None;
        self.replacement.notice = None;
        self.replacement_job(Job::Prepare(Box::new(context)))
    }

    /// Detail UI passes only its scoped hash/nonce/net; the worker obtains and verifies the
    /// original separately. A plain transaction-detail `from` never reaches a signing request.
    pub fn start_history_replacement(
        &mut self,
        action: Action,
        net: Net,
        hash: String,
        nonce: u64,
    ) -> Vec<Effect> {
        self.start_replacement(action, net, hash, nonce, false)
    }
    fn start_replacement(
        &mut self,
        action: Action,
        net: Net,
        hash: String,
        nonce: u64,
        uncertain: bool,
    ) -> Vec<Effect> {
        self.nav.tab = Tab::Send;
        let (key, checked) = match self.replacement_ready() {
            Ok(v) => v,
            Err(e) => {
                self.replacement_error(e);
                return Vec::new();
            }
        };
        if net.chain.key != self.net().chain.key || net.rpc_url != self.net().rpc_url {
            self.replacement_error(WalletError::validation(
                "the selected send belongs to another chain or RPC",
            ));
            return Vec::new();
        }
        let original = self.replacement_original(&hash);
        if action == Action::Retry
            && (!uncertain || original.as_ref().is_none_or(|o| !o.is_retained()))
        {
            self.replacement_error(WalletError::validation(backend::RETRY_WARNING));
            return Vec::new();
        }
        if let Some(original) = &original
            && (original.key_name() != key.name
                || original.index() != key.index
                || key.address.as_deref() != Some(original.from().as_str())
                || original.chain().key != net.chain.key
                || original.nonce() != nonce)
        {
            self.replacement_error(WalletError::validation(
                "the retained send belongs to another pinned key or nonce",
            ));
            return Vec::new();
        }
        self.replacement.context = Some(Context {
            net,
            key,
            checked,
            hash,
            nonce,
            action,
            options: Options::default(),
            original,
        });
        self.replacement.scroll = 0;
        self.prepare_replacement_again()
    }
    pub(super) fn replacement_update(&mut self, msg: Msg) -> ControlFlow<Msg, Vec<Effect>> {
        // Capture only the hash emitted after independently checked signing and journal fsync.
        if let Msg::Transfer(id, TransferMsg::Progress { hash, .. }) = &msg {
            let original = if self.replacement.external_send == Some(*id) {
                self.replacement.inflight.clone()
            } else {
                None
            };
            if let Some(original) = original {
                let is_cancel = original.is_cancel();
                self.register_retained_replacement(hash, original);
                if is_cancel {
                    self.replacement
                        .cancel_hashes
                        .insert(hash.to_ascii_lowercase());
                }
            }
        }
        if let Msg::Replacement(id, Event::Proven(request, result)) = msg {
            if self.replacement.proof_id == Some(id)
                && self
                    .history
                    .detail
                    .as_ref()
                    .is_some_and(|(r, _)| r == request.as_ref())
                && self.replacement_ready().is_ok_and(|(key, _)| {
                    key.name == request.scope.target.key
                        && key.address.as_deref() == Some(request.scope.owner.as_str())
                })
                && self.net().chain.key == request.scope.target.endpoint.chain
                && self.net().rpc_url == request.scope.target.endpoint.url
            {
                self.replacement.proof_id = None;
                self.replacement.proof = Some((
                    *request,
                    match result {
                        Ok(original) => Pane::Ready(*original),
                        Err(e) => Pane::Failed(Outside::error(&e)),
                    },
                ));
            }
            return ControlFlow::Continue(Vec::new());
        }
        let (id, event) = match msg {
            Msg::Replacement(id, event) => (id, event),
            Msg::Paste(text) if self.replacement_visible() && self.replacement.field.is_some() => {
                return ControlFlow::Continue(self.replacement_input(Input::Paste(&text.0)));
            }
            other => return ControlFlow::Break(other),
        };
        if self.replacement.active != Some(id) {
            return ControlFlow::Continue(Vec::new());
        }
        self.replacement.active = None;
        let work = match event {
            Event::Proven(..) => unreachable!("proof handled separately"),
            Event::Reviewed(result) => {
                let current = self
                    .replacement
                    .context
                    .as_ref()
                    .is_some_and(|c| self.replacement_context_current(c));
                if !current {
                    self.replacement_error(WalletError::validation(
                        "replacement context changed; nothing was signed",
                    ));
                } else {
                    match result {
                        Ok(review) => {
                            if let Some(context) = &mut self.replacement.context {
                                context.original = Some(review.original().clone());
                            }
                            self.replacement.review = Some(*review);
                            self.replacement.stage = Stage::Review;
                        }
                        Err(e) => self.replacement_error(e),
                    }
                }
                Vec::new()
            }
            Event::Sent {
                transfer_id,
                result,
            } => {
                self.replacement.external_send = None;
                self.replacement.inflight = None;
                match result {
                    Ok(SendReplacementOutcome::ReviewChanged(review)) => {
                        let work = self.return_external_review(transfer_id);
                        self.nav.tab = Tab::Send;
                        self.replacement.review = Some(*review);
                        self.replacement.stage = Stage::Review;
                        self.replacement.field = None;
                        self.replacement.notice = Some(Outside::detail(
                            "review changed; nothing signed; confirm the new values",
                        ));
                        work
                    }
                    result => {
                        self.replacement.stage = Stage::Idle;
                        self.replacement.review = None;
                        self.replacement.field = None;
                        let result = result.map(|out| match out {
                            SendReplacementOutcome::Sent(sent) => {
                                wallet::SendTransferOutcome::Sent(sent)
                            }
                            _ => unreachable!(),
                        });
                        self.update(Msg::Transfer(transfer_id, TransferMsg::Sent(result)))
                    }
                }
            }
        };
        ControlFlow::Continue(work)
    }
    pub(super) fn replacement_action(&mut self, action: ActionId) -> Option<Vec<Effect>> {
        let key = match action {
            ActionId::SpeedUp => Key::Char('s'),
            ActionId::Cancel => Key::Char('x'),
            ActionId::Retry => Key::Char('R'),
            _ => return None,
        };
        if self.nav.tab != Tab::Wallet || self.history.detail.is_none() {
            self.nav.tab = Tab::Send;
        }
        self.replacement_key(key)
    }
    pub(super) fn replacement_key(&mut self, key: Key) -> Option<Vec<Effect>> {
        if self.sending() {
            return None;
        }
        if self.nav.popup.is_none()
            && self.nav.tab == Tab::Wallet
            && self.history.detail.is_some()
            && matches!(key, Key::Char('s' | 'x' | 'R'))
        {
            if self.on_tron() {
                // Tron has no nonce: nothing can replace a broadcast Tron transaction.
                self.notice = Some(Outside::detail(
                    "Tron has no nonce replacement; speed-up, cancel and retry do not apply",
                ));
                return Some(Vec::new());
            }
            return Some(self.history_replacement_key(key));
        }
        if self.nav.popup.is_some() || self.nav.tab != Tab::Send {
            return None;
        }
        if self.replacement.field.is_some() {
            let input = match key {
                Key::Char(c) => Input::Char(c),
                Key::Enter => Input::Enter,
                Key::Esc => Input::Esc,
                Key::Interrupt => Input::Interrupt,
                Key::Backspace => Input::Backspace,
                Key::Delete => Input::Delete,
                Key::Left => Input::Left,
                Key::Right => Input::Right,
                Key::Home => Input::Home,
                Key::End => Input::End,
                Key::Tab => Input::Tab,
                Key::BackTab => Input::BackTab,
                Key::Up => Input::Up,
                Key::Down => Input::Down,
            };
            return Some(self.replacement_input(input));
        }
        if self.replacement.stage != Stage::Idle {
            let work = match key {
                // Not a text field: the tab keys act, and the review stays for the return.
                tab if super::navigation::switches_tab(&super::navigation::input(tab)) => {
                    return None;
                }
                Key::Esc => {
                    self.replacement.stage = Stage::Idle;
                    self.replacement.active = None;
                    self.replacement.review = None;
                    self.replacement.notice = None;
                    Vec::new()
                }
                Key::Char('q') | Key::Interrupt => {
                    self.quit = true;
                    Vec::new()
                }
                Key::Char('L') => {
                    self.replacement.stage = Stage::Idle;
                    self.replacement.active = None;
                    self.replacement.review = None;
                    return None;
                }
                Key::Enter if self.replacement.stage == Stage::Review => {
                    if let Err(e) = self.replacement_ready() {
                        self.replacement_error(e);
                    } else if let Some(review) = &self.replacement.review {
                        if let Err(e) = review.verdict() {
                            self.replacement.notice = Some(Outside::error(e));
                        } else if !review.sufficient_funds() {
                            self.replacement.notice =
                                Some(Outside::detail("insufficient funds; review again"));
                        } else {
                            self.replacement.field = Some(Field::new(Kind::Nickname));
                            self.replacement.stage = Stage::Confirm;
                        }
                    }
                    Vec::new()
                }
                Key::Char('b') if self.replacement.stage == Stage::Review => {
                    self.replacement.field = Some(Field::new(Kind::Bump));
                    self.replacement.stage = Stage::Bump;
                    Vec::new()
                }
                Key::Char('r')
                    if matches!(self.replacement.stage, Stage::Review | Stage::Failed) =>
                {
                    self.prepare_replacement_again()
                }
                Key::Down | Key::Char('j') => {
                    self.replacement.scroll = self.replacement.scroll.saturating_add(1);
                    Vec::new()
                }
                Key::Up | Key::Char('k') => {
                    self.replacement.scroll = self.replacement.scroll.saturating_sub(1);
                    Vec::new()
                }
                _ => Vec::new(),
            };
            return Some(work);
        }
        // Replacement shortcuts belong to the status screen. A typed wait or
        // transfer form owns its characters, even when an earlier status remains.
        if self.transfer.stage != super::transfer::Stage::Status || self.transfer.field.is_some() {
            return None;
        }
        // A Tron status keeps its own handling: Tron has no nonce replacement.
        if self.on_tron() {
            return None;
        }
        let action = match key {
            Key::Char('s') => Action::SpeedUp,
            Key::Char('x') => Action::Cancel,
            Key::Char('R') => Action::Retry,
            _ => return None,
        };
        let status = self.transfer.status.as_ref()?;
        if matches!(status.state, ReceiptState::Success | ReceiptState::Reverted)
            || action != Action::Retry && status.state != ReceiptState::Pending
        {
            self.replacement_error(WalletError::validation(
                "this status does not confirm a pending original; look up the hash; nothing was signed",
            ));
            return Some(Vec::new());
        }
        let Some(nonce) = status.nonce else {
            self.replacement_error(WalletError::validation(
                "this status has no nonce to replace; nothing was signed",
            ));
            return Some(Vec::new());
        };
        Some(self.start_replacement(
            action,
            status.net.clone(),
            status.hash.clone(),
            nonce,
            status.uncertain,
        ))
    }
    fn replacement_input(&mut self, input: Input<'_>) -> Vec<Effect> {
        let Some(mut field) = self.replacement.field.take() else {
            return Vec::new();
        };
        match field.edit(input) {
            Edit::Interrupt => {
                self.quit = true;
                return Vec::new();
            }
            Edit::Cancel => {
                self.replacement.stage = Stage::Review;
                return Vec::new();
            }
            Edit::Submit if self.replacement.stage == Stage::Bump => {
                match chains::parse_bump(field.value()) {
                    Ok(bump) => {
                        if let Some(context) = &mut self.replacement.context {
                            context.options.bump = bump;
                        }
                        return self.prepare_replacement_again();
                    }
                    Err(e) => self.replacement.notice = Some(Outside::error(&e)),
                }
            }
            Edit::Submit => {
                let Some(context) = self.replacement.context.clone() else {
                    return Vec::new();
                };
                if !self.replacement_context_current(&context) || self.replacement_ready().is_err()
                {
                    self.replacement_error(WalletError::validation(
                        "replacement key, session or list changed; nothing signed",
                    ));
                    return Vec::new();
                }
                if let Some(review) = &self.replacement.review {
                    if field.value() != review.confirmation() {
                        self.replacement.notice = Some(Outside::detail(
                            "type the reviewed nickname or cancel exactly",
                        ));
                    } else {
                        let review = self.replacement.review.take().unwrap();
                        self.replacement.inflight = Some(review.retained_intent());
                        let transfer_id = self.begin_external_send();
                        self.replacement.external_send = Some(transfer_id);
                        self.replacement.stage = Stage::Sending;
                        return self.replacement_job(Job::Send {
                            review: Box::new(review),
                            typed: field.value().into(),
                            transfer_id,
                            net: context.net,
                        });
                    }
                }
            }
            Edit::Rejected(reason) => self.replacement.notice = Some(Outside::detail(reason)),
            Edit::Changed => self.replacement.notice = None,
            _ => {}
        }
        self.replacement.field = Some(field);
        Vec::new()
    }
    pub fn replacement_hint(&self) -> &'static str {
        match self.replacement.stage {
            Stage::Preparing => "checking original...   Esc back   q quit",
            Stage::Review => "Enter confirm   b bump   r review again   j/k scroll   Esc back",
            Stage::Confirm | Stage::Bump => "Enter submit   Esc back   Ctrl-C quit",
            Stage::Failed => "r review again   Esc back   q quit",
            _ => match self.replacement_available() {
                (true, true) => "s speed up   x cancel   R retry this uncertain send",
                (true, false) => "s speed up   x cancel",
                (false, true) => "R retry this session's uncertain send",
                _ if self.verified_key().is_err() => "replacements need: V",
                _ => "look up the hash before another payment",
            },
        }
    }
    pub fn replacement_status_notes(&self) -> Vec<&'static str> {
        let Some(status) = self.transfer.status.as_ref() else {
            return Vec::new();
        };
        let mut notes = Vec::new();
        if self
            .replacement
            .cancel_hashes
            .contains(&status.hash.to_ascii_lowercase())
        {
            notes.push(backend::CANCEL_RACE);
        }
        if status.uncertain && matches!(status.state, ReceiptState::Pending | ReceiptState::Unknown)
        {
            notes.push(backend::RETRY_WARNING);
        }
        notes
    }
    pub fn replacement_available(&self) -> (bool, bool) {
        if self.replacement_ready().is_err() {
            return (false, false);
        }
        if self.nav.tab == Tab::Wallet {
            let pending = matches!(&self.history.detail, Some((request, Pane::Ready(TransactionDetail::Evm(d))))
                if d.state == crate::history::State::Pending && self.replacement_original(&request.hash).is_some());
            return (pending, false);
        }
        self.transfer
            .status
            .as_ref()
            .map_or((false, false), |status| {
                let retained = self.replacement_original(&status.hash).is_some();
                (
                    status.state == ReceiptState::Pending && retained,
                    status.uncertain
                        && retained
                        && matches!(status.state, ReceiptState::Pending | ReceiptState::Unknown),
                )
            })
    }
}
