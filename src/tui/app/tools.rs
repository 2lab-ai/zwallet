//! Tool forms retain every reviewed byte. Worker effects hold only single-use job ids.
use super::*;
use crate::chains::{self, Balance};
use crate::core::{error::ErrorCode, whitelist};
use crate::tui::actions::ActionId;
use crate::tui::form::{Edit, Field, Input, Kind};
use crate::wallet::{
    self, CallInput, CallOutput, CheckedContract, InvokeData, InvokeInput, InvokeReview,
    MessageReview, SendInvokeOutcome, TransferOptions,
};
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Call,
    Invoke,
    RawInvoke,
    Message,
    Token,
}

impl Tool {
    pub const ALL: [Self; 5] = [
        Self::Call,
        Self::Invoke,
        Self::RawInvoke,
        Self::Message,
        Self::Token,
    ];
    pub fn title(self) -> &'static str {
        match self {
            Self::Call => "call",
            Self::Invoke => "invoke",
            Self::RawInvoke => "raw invoke",
            Self::Message => "sign message",
            Self::Token => "token balance",
        }
    }
    pub fn action(self) -> ActionId {
        match self {
            Self::Call => ActionId::Call,
            Self::Invoke => ActionId::Invoke,
            Self::RawInvoke => ActionId::RawInvoke,
            Self::Message => ActionId::SignMessage,
            Self::Token => ActionId::TokenBalance,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Menu,
    Contracts,
    Field,
    Loading,
    MessageReview,
    MessageConfirm,
    InvokeReview,
    Confirm,
    Signing,
    Result,
    Error,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    Address,
    Signature,
    Argument(usize),
    Data,
    Value,
    Message,
    Token,
    Fee,
    Task,
    Confirm,
    SignConfirm,
}

#[derive(Debug)]
pub struct RetainedToolSend {
    pub intent: wallet::RetainedInvoke,
    pub uncertain: bool,
}

pub struct Tools {
    pub retained: BTreeMap<String, RetainedToolSend>,
    pending_intent: Option<wallet::RetainedInvoke>,
    send_id: Option<u64>,
    last_hash: Option<String>,
    pub stage: Stage,
    pub tool: Tool,
    pub selected: usize,
    pub review_scroll: usize,
    pub field: Option<Field>,
    pub purpose: Purpose,
    pub notice: Option<Outside>,
    pub contracts: Vec<CheckedContract>,
    pub contract: Option<CheckedContract>,
    pub message: Option<MessageReview>,
    pub invoke: Option<InvokeReview>,
    pub result: Option<ToolResult>,
    pub alternate: bool,
    pub options: TransferOptions,
    lock_after: bool,
    to: String,
    sig: String,
    args: Vec<String>,
    data: String,
    value: String,
    original_message: String,
    argument_count: usize,
    counter: u64,
    active: Option<u64>,
    context: Option<(String, &'static str, String)>,
    jobs: BTreeMap<u64, ToolJob>,
}
impl Default for Tools {
    fn default() -> Self {
        Self {
            retained: BTreeMap::new(),
            pending_intent: None,
            send_id: None,
            last_hash: None,
            stage: Stage::Menu,
            tool: Tool::Call,
            selected: 0,
            review_scroll: 0,
            field: None,
            purpose: Purpose::Address,
            notice: None,
            contracts: vec![],
            contract: None,
            message: None,
            invoke: None,
            result: None,
            alternate: false,
            options: TransferOptions::default(),
            lock_after: false,
            to: String::new(),
            sig: String::new(),
            args: vec![],
            data: String::new(),
            value: "0".into(),
            original_message: String::new(),
            argument_count: 0,
            counter: 0,
            active: None,
            context: None,
            jobs: BTreeMap::new(),
        }
    }
}
#[derive(Debug)]
pub enum ToolResult {
    Call(CallOutput),
    Token {
        balance: Balance,
        owner: String,
        token: String,
    },
    Signature(Value),
}
#[derive(Debug)]
pub enum ToolMsg {
    Contracts(Result<Vec<CheckedContract>>),
    Call(Result<CallOutput>),
    Token(Result<(Balance, String, String)>),
    MessageReview(Result<MessageReview>),
    InvokeReview(Result<Box<InvokeReview>>),
    Signature(Result<Value>),
    InvokeChanged(u64, Box<InvokeReview>),
    Sent,
}
pub enum ToolJob {
    Contracts {
        net: Net,
        checked: CheckedList,
    },
    Call {
        net: Net,
        input: CallInput,
    },
    Token {
        net: Net,
        owner: String,
        token: String,
    },
    MessageReview {
        net: Net,
        checked: CheckedList,
        key: wallet::KeyRow,
        text: String,
    },
    InvokeReview {
        net: Net,
        checked: CheckedList,
        input: Box<InvokeInput>,
    },
    SignMessage(MessageReview),
    SendInvoke {
        review: Box<InvokeReview>,
        net: Net,
        from: String,
        nickname: String,
        send_id: u64,
    },
}
impl ToolJob {
    pub async fn run(self, id: u64, home: &Path, tx: &UnboundedSender<Msg>) {
        let event = match self {
            Self::Contracts { net, checked } => ToolMsg::Contracts(
                whitelist::check_unchanged(home, &checked.fingerprint)
                    .map(|()| wallet::invoke_contracts(&checked, net.chain)),
            ),
            Self::Call { net, input } => ToolMsg::Call(wallet::call(&net, &input).await),
            Self::Token { net, owner, token } => ToolMsg::Token(
                wallet::token_balance(&net, &owner, &token)
                    .await
                    .map(|b| (b, owner, token)),
            ),
            Self::MessageReview {
                net,
                checked,
                key,
                text,
            } => ToolMsg::MessageReview(wallet::review_message(
                home, net.chain, &checked, &key, &text,
            )),
            Self::InvokeReview {
                net,
                checked,
                input,
            } => ToolMsg::InvokeReview(
                wallet::review_invoke(home, &net, &checked, *input)
                    .await
                    .map(Box::new),
            ),
            Self::SignMessage(review) => ToolMsg::Signature(wallet::sign_message(review).await),
            Self::SendInvoke {
                review,
                net,
                from,
                nickname,
                send_id,
            } => {
                let nonce = review.intent().nonce();
                let journal = chains::SendJournal::new(home, net.chain, &from)
                    .path()
                    .to_path_buf();
                let sender = tx.clone();
                let progress = Arc::new(move |line: &str| {
                    if let Some(hash) = line
                        .strip_prefix("broadcasting ")
                        .and_then(|s| s.split_whitespace().next())
                        && hash.parse::<alloy::primitives::B256>().is_ok()
                    {
                        let _ = sender.send(Msg::Transfer(
                            send_id,
                            super::transfer::TransferMsg::Progress {
                                hash: hash.into(),
                                nonce,
                                journal: journal.clone(),
                            },
                        ));
                    }
                });
                match wallet::send_invoke(*review, &nickname, Some(progress)).await {
                    Ok(SendInvokeOutcome::ReviewChanged(next)) => {
                        ToolMsg::InvokeChanged(send_id, next)
                    }
                    result => {
                        let result = result.map(|v| match v {
                            SendInvokeOutcome::Sent(out) => wallet::SendTransferOutcome::Sent(out),
                            _ => unreachable!(),
                        });
                        let _ = tx.send(Msg::Transfer(
                            send_id,
                            super::transfer::TransferMsg::Sent(result),
                        ));
                        ToolMsg::Sent
                    }
                }
            }
        };
        let _ = tx.send(Msg::Tools(id, event));
    }
}

impl App {
    pub fn tools(&self) -> &Tools {
        &self.tools
    }
    pub fn take_tool_job(&mut self, id: u64) -> Option<ToolJob> {
        self.tools.jobs.remove(&id)
    }
    fn tool_job(&mut self, job: ToolJob, signing: bool) -> Vec<Effect> {
        self.tools.counter += 1;
        let id = self.tools.counter;
        self.tools.active = Some(id);
        self.tools.jobs.insert(id, job);
        self.tools.stage = if signing {
            Stage::Signing
        } else {
            Stage::Loading
        };
        vec![Effect::Tools(id)]
    }
    pub(super) fn tools_reset(&mut self) {
        let counter = self.tools.counter;
        let retained = std::mem::take(&mut self.tools.retained);
        self.tools = Tools::default();
        self.tools.retained = retained;
        self.tools.counter = counter;
        self.tools.context = Some((
            self.open_key.clone().unwrap_or_default(),
            self.chain.key,
            self.net().rpc_url.clone(),
        ));
    }
    pub(super) fn show_tools(&mut self) -> Vec<Effect> {
        let context = (
            self.open_key.clone().unwrap_or_default(),
            self.chain.key,
            self.net().rpc_url.clone(),
        );
        if self.tools.context.as_ref() != Some(&context) {
            let dropped = self.tools.stage != Stage::Menu;
            self.tools_reset();
            if dropped {
                // Never a silent dead end: the review belonged to another key, chain or RPC.
                self.tools.notice = Some(Outside::detail(
                    "the open review was for another key, chain or RPC; nothing was signed",
                ));
            }
        }
        Vec::new()
    }
    fn tools_error(&mut self, e: WalletError) {
        if self.quit {
            self.quit_error.get_or_insert_with(|| e.clone());
        }
        if e.message.contains("changed after it was shown") {
            self.checked_list = None;
            self.list_reason = Some(Outside::detail(
                "not verified: whitelist changed; V checks it again",
            ));
            self.tools.contracts.clear();
            self.tools.contract = None;
        }
        self.tools.message = None;
        self.tools.invoke = None;
        self.tools.field = None;
        self.tools.notice = Some(Outside::error(&e));
        self.tools.stage = Stage::Error;
    }
    fn tool_signing_context(&self) -> Result<(wallet::KeyRow, CheckedList)> {
        if self.locking || self.activating {
            return Err(WalletError::validation(
                "wait for the lock or metadata change to finish before signing",
            ));
        }
        let key = self.verified_key()?;
        if !matches!(&self.status,Pane::Ready(bar) if matches!(bar.lock,LockLine::Open(_))) {
            return Err(WalletError::locked());
        }
        let checked = self.checked_list.clone().ok_or_else(|| {
            WalletError::new(
                ErrorCode::WhitelistInvalid,
                "needs: V; the whitelist is not password-checked",
            )
        })?;
        Ok((key, checked))
    }
    pub(super) fn start_tool(&mut self, tool: Tool) -> Vec<Effect> {
        self.close_tasks();
        self.nav.tab = Tab::Tools;
        self.nav.popup = None;
        self.tools_reset();
        self.tools.tool = tool;
        if let Err(e) = chains::require_contract_calls(self.chain) {
            self.tools_error(e);
            return vec![];
        }
        match tool {
            Tool::Call => {
                self.tool_field(Purpose::Address, Kind::EvmAddress, "");
                vec![]
            }
            Tool::Token => match self.verified_key() {
                Ok(_) => {
                    self.tool_field(Purpose::Token, Kind::EvmAddress, "");
                    vec![]
                }
                Err(e) => {
                    self.tools_error(e);
                    vec![]
                }
            },
            Tool::Message | Tool::Invoke | Tool::RawInvoke => {
                let (_, checked) = match self.tool_signing_context() {
                    Ok(v) => v,
                    Err(e) => {
                        self.tools_error(e);
                        return vec![];
                    }
                };
                if tool == Tool::Message {
                    self.tool_field(Purpose::Message, Kind::Message, "");
                    vec![]
                } else {
                    self.tool_job(
                        ToolJob::Contracts {
                            net: self.net().clone(),
                            checked,
                        },
                        false,
                    )
                }
            }
        }
    }
    fn tool_field(&mut self, purpose: Purpose, kind: Kind, value: &str) {
        self.tools.purpose = purpose;
        self.tools.field = Some(Field::with_value(kind, value).expect("validated field value"));
        self.tools.stage = if purpose == Purpose::Confirm {
            Stage::Confirm
        } else if purpose == Purpose::SignConfirm {
            Stage::MessageConfirm
        } else {
            Stage::Field
        };
        self.tools.notice = None;
    }
    pub(super) fn tools_update(&mut self, msg: Msg) -> std::ops::ControlFlow<Msg, Vec<Effect>> {
        use std::ops::ControlFlow::{Break, Continue};
        if let Msg::Transfer(id, event) = &msg
            && Some(*id) == self.tools.send_id
        {
            match event {
                super::transfer::TransferMsg::Progress { hash, .. } => {
                    if let Some(intent) = &self.tools.pending_intent {
                        self.tools.retained.insert(
                            hash.clone(),
                            RetainedToolSend {
                                intent: intent.clone(),
                                uncertain: false,
                            },
                        );
                        self.tools.last_hash = Some(hash.clone());
                    }
                }
                super::transfer::TransferMsg::Sent(Err(e))
                    if e.code == ErrorCode::BroadcastUncertain =>
                {
                    if let Some(hash) = &self.tools.last_hash
                        && let Some(retained) = self.tools.retained.get_mut(hash)
                    {
                        retained.uncertain = true;
                    }
                }
                _ => {}
            }
        }

        match msg {
            Msg::Tools(id, event) => {
                if self.tools.active != Some(id) {
                    return Continue(vec![]);
                }
                self.tools.active = None;
                match event {
                    ToolMsg::Contracts(Ok(contracts)) => {
                        self.tools.contracts = contracts;
                        self.tools.selected = 0;
                        self.tools.stage = Stage::Contracts;
                    }
                    ToolMsg::Call(Ok(result)) => {
                        self.tools.result = Some(ToolResult::Call(result));
                        self.tools.stage = Stage::Result;
                    }
                    ToolMsg::Token(Ok((balance, owner, token))) => {
                        self.tools.result = Some(ToolResult::Token {
                            balance,
                            owner,
                            token,
                        });
                        self.tools.stage = Stage::Result;
                    }
                    ToolMsg::MessageReview(Ok(review)) => {
                        self.tools.message = Some(review);
                        self.tools.stage = Stage::MessageReview;
                    }
                    ToolMsg::InvokeReview(Ok(review)) => {
                        self.tools.review_scroll = 0;
                        self.tools.invoke = Some(*review);
                        self.tools.stage = Stage::InvokeReview;
                    }
                    ToolMsg::Signature(Ok(result)) => {
                        self.tools.result = Some(ToolResult::Signature(result));
                        self.tools.stage = Stage::Result;
                    }
                    ToolMsg::InvokeChanged(send_id, review) => {
                        let work = self.return_external_review(send_id);
                        self.tools.review_scroll = 0;
                        self.tools.invoke = Some(*review);
                        self.tools.stage = Stage::InvokeReview;
                        self.tools.notice = Some(Outside::detail(
                            "Nonce, funds or fees changed; review again.",
                        ));
                        return Continue(work);
                    }
                    ToolMsg::InvokeReview(Err(e)) if e.code == ErrorCode::WhitelistDenied => {
                        let (purpose, kind, value) = if self.tools.tool == Tool::RawInvoke {
                            (Purpose::Data, Kind::Calldata, self.tools.data.clone())
                        } else {
                            (Purpose::Signature, Kind::Signature, self.tools.sig.clone())
                        };
                        self.tool_field(purpose, kind, &value);
                        self.tools.notice = Some(Outside::error(&e));
                    }
                    ToolMsg::InvokeReview(Err(e)) => self.tools_error(e),
                    ToolMsg::Sent => {
                        self.tools.stage = Stage::Menu;
                    }
                    ToolMsg::Contracts(Err(e))
                    | ToolMsg::Call(Err(e))
                    | ToolMsg::Token(Err(e))
                    | ToolMsg::MessageReview(Err(e))
                    | ToolMsg::Signature(Err(e)) => self.tools_error(e),
                }
                if std::mem::take(&mut self.tools.lock_after) {
                    Continue(vec![Effect::LockAll])
                } else {
                    Continue(vec![])
                }
            }
            Msg::Key(key)
                if self.tab() == Tab::Tools
                    && self.password == PasswordState::Idle
                    && self.nav.popup.is_none() =>
            {
                match self.tools_key(key) {
                    Some(work) => Continue(work),
                    None => Break(Msg::Key(key)),
                }
            }
            Msg::Paste(text)
                if self.tab() == Tab::Tools
                    && self.password == PasswordState::Idle
                    && self.nav.popup.is_none() =>
            {
                Continue(self.tools_input(Input::Paste(&text.0)))
            }
            other => Break(other),
        }
    }
    pub(super) fn tools_key(&mut self, key: Key) -> Option<Vec<Effect>> {
        if self.tools.field.is_some() {
            return Some(self.tools_input(super::navigation::input(key)));
        }
        // A tab key leaves the review where it is, for the return (as the send, replacement and
        // whitelist reviews stay); confirming re-checks everything before any signature.
        if self.tools.stage != Stage::Signing
            && super::navigation::switches_tab(&super::navigation::input(key))
        {
            return None;
        }
        if self.tools.stage != Stage::Signing
            && matches!(
                key,
                Key::Char('L' | 'q' | '?' | 'u' | 'V' | 'c') | Key::Interrupt
            )
        {
            self.tools_reset();
            return None;
        }
        match self.tools.stage {
            Stage::Menu => match key {
                Key::Up => self.tools.selected = self.tools.selected.saturating_sub(1),
                Key::Down => self.tools.selected = (self.tools.selected + 1).min(Tool::ALL.len()),
                Key::Enter if self.tools.selected == Tool::ALL.len() => {
                    return Some(self.start_tasks(ActionId::TaskList));
                }
                Key::Enter => return Some(self.start_tool(Tool::ALL[self.tools.selected])),
                Key::Char('a') => return Some(self.start_tasks(ActionId::TaskList)),
                Key::Char('C') => return Some(self.start_tool(Tool::Call)),
                Key::Char('i') => return Some(self.start_tool(Tool::Invoke)),
                Key::Char('d') => return Some(self.start_tool(Tool::RawInvoke)),
                Key::Char('m') => return Some(self.start_tool(Tool::Message)),
                Key::Char('t') => return Some(self.start_tool(Tool::Token)),
                _ => return None,
            },
            Stage::Contracts => match key {
                Key::Up => self.tools.selected = self.tools.selected.saturating_sub(1),
                Key::Down => {
                    self.tools.selected =
                        (self.tools.selected + 1).min(self.tools.contracts.len().saturating_sub(1))
                }
                Key::Enter => {
                    if let Some(contract) = self.tools.contracts.get(self.tools.selected).cloned() {
                        self.tools.contract = Some(contract);
                        if self.tools.tool == Tool::RawInvoke {
                            self.tool_field(Purpose::Data, Kind::Calldata, "");
                        } else {
                            self.tool_field(Purpose::Signature, Kind::Signature, "");
                        }
                    }
                }
                Key::Esc => self.tools_reset(),
                Key::Interrupt => return None,
                _ => {}
            },
            Stage::MessageReview => match key {
                Key::Char('e') => {
                    self.tools.message = None;
                    let text = self.tools.original_message.clone();
                    self.tool_field(Purpose::Message, Kind::Message, &text);
                }
                Key::Enter => {
                    if let Err(e) = self.ensure_tool_review_key() {
                        self.tools_error(e);
                        return Some(vec![]);
                    }
                    self.tool_field(Purpose::SignConfirm, Kind::Nickname, "");
                }
                Key::Esc => self.tools_reset(),
                Key::Interrupt => return None,
                _ => {}
            },
            Stage::InvokeReview => match key {
                Key::Char('t') => {
                    let value = self.tools.options.task_id.clone().unwrap_or_default();
                    self.tool_field(Purpose::Task, Kind::OptionalTaskId, &value);
                }
                Key::Down | Key::Char('j') => {
                    self.tools.review_scroll = self.tools.review_scroll.saturating_add(1);
                }
                Key::Up | Key::Char('k') => {
                    self.tools.review_scroll = self.tools.review_scroll.saturating_sub(1);
                }
                Key::Home => self.tools.review_scroll = 0,
                Key::Enter => {
                    if let Err(e) = self.ensure_tool_review_key() {
                        self.tools_error(e);
                        return Some(vec![]);
                    }
                    if let Some(review) = &self.tools.invoke {
                        if let Err(e) = review.verdict() {
                            self.tools.notice = Some(Outside::error(e));
                        } else if !review.sufficient_funds() {
                            self.tools.notice = Some(Outside::detail(
                                "insufficient funds; edit the value or wait for funds",
                            ));
                        } else {
                            self.tool_field(Purpose::Confirm, Kind::Nickname, "");
                        }
                    }
                }
                Key::Char('e') => {
                    self.tools.invoke = None;
                    let data = self.tools.data.clone();
                    let sig = self.tools.sig.clone();
                    if self.tools.tool == Tool::RawInvoke {
                        self.tool_field(Purpose::Data, Kind::Calldata, &data);
                    } else {
                        self.tool_field(Purpose::Signature, Kind::Signature, &sig);
                    }
                }
                Key::Char('p') => {
                    self.tools.options.allow_pending = !self.tools.options.allow_pending;
                    return Some(self.prepare_invoke());
                }
                Key::Char('d') => {
                    self.tools.options.allow_duplicate = !self.tools.options.allow_duplicate;
                    return Some(self.prepare_invoke());
                }
                Key::Char('g') => {
                    self.tools.invoke = None;
                    self.tool_field(Purpose::Fee, Kind::Amount, "");
                }
                Key::Esc => self.tools_reset(),
                Key::Interrupt => return None,
                _ => {}
            },
            Stage::Result => match key {
                Key::Char('r') => self.tools.alternate = !self.tools.alternate,
                Key::Esc | Key::Enter => self.tools_reset(),
                Key::Interrupt => return None,
                _ => {}
            },
            Stage::Error => match key {
                Key::Esc | Key::Enter => self.tools_reset(),
                _ => return None,
            },
            Stage::Loading => match key {
                Key::Esc => {
                    self.tools.active = None;
                    self.tools_reset();
                }
                Key::Interrupt => return None,
                _ => {}
            },
            Stage::Signing => {
                if matches!(key, Key::Char('q') | Key::Interrupt) {
                    self.quit = true;
                }
                if key == Key::Char('L') {
                    self.locking = true;
                    self.tools.lock_after = true;
                }
                self.tools.notice = Some(Outside::detail(
                    "Waiting for the signature result; the request is already in progress.",
                ));
            }
            Stage::Field | Stage::Confirm | Stage::MessageConfirm => {}
        }
        Some(vec![])
    }
    fn tools_input(&mut self, input: Input<'_>) -> Vec<Effect> {
        let Some(field) = self.tools.field.as_mut() else {
            return vec![];
        };
        let edit = field.edit(input);
        match edit {
            Edit::Submit => {
                let value = field.value().to_string();
                return self.submit_tool_field(value);
            }
            Edit::Cancel => {
                self.tools_reset();
            }
            Edit::Interrupt => {
                self.tools_reset();
                return self.key(Key::Interrupt);
            }
            Edit::Rejected(reason) => {
                self.tools.notice =
                    Some(Outside::detail(if self.tools.purpose == Purpose::Message {
                        "Use zwallet sign-message for non-ASCII text or more than 512 bytes."
                    } else {
                        reason
                    }));
            }
            _ => {}
        }
        vec![]
    }
    fn submit_tool_field(&mut self, value: String) -> Vec<Effect> {
        self.tools.notice = None;
        match self.tools.purpose {
            Purpose::Task => {
                self.tools.options.task_id = (!value.is_empty()).then_some(value);
                return self.prepare_invoke();
            }
            Purpose::Address => {
                if let Err(e) = crate::chains::evm::parse_address(&value) {
                    self.tools.notice = Some(Outside::error(&e));
                    return vec![];
                }
                self.tools.to = value;
                self.tool_field(Purpose::Signature, Kind::Signature, "");
            }
            Purpose::Signature => match crate::chains::evm::abi::parse_function(&value) {
                Ok(function) => {
                    self.tools.sig = value;
                    self.tools.argument_count = function.inputs.len();
                    self.tools.args.clear();
                    if self.tools.argument_count == 0 {
                        return self.arguments_done();
                    }
                    self.tool_field(Purpose::Argument(0), Kind::Arguments, "");
                }
                Err(e) => self.tools.notice = Some(Outside::error(&e)),
            },
            Purpose::Argument(i) => {
                if i < self.tools.args.len() {
                    self.tools.args[i] = value;
                } else {
                    self.tools.args.push(value);
                }
                if i + 1 == self.tools.argument_count {
                    return self.arguments_done();
                }
                self.tool_field(Purpose::Argument(i + 1), Kind::Arguments, "");
            }
            Purpose::Data => {
                self.tools.data = value;
                let old = self.tools.value.clone();
                self.tool_field(Purpose::Value, Kind::Amount, &old);
            }
            Purpose::Value => {
                self.tools.value = value;
                return self.prepare_invoke();
            }
            Purpose::Fee => match wallet::parse_fee_ceiling(&value) {
                Ok(fee) => {
                    self.tools.options.max_fee_wei = Some(fee);
                    return self.prepare_invoke();
                }
                Err(e) => self.tools.notice = Some(Outside::error(&e)),
            },
            Purpose::Message => {
                let (key, checked) = match self.tool_signing_context() {
                    Ok(v) => v,
                    Err(e) => {
                        self.tools_error(e);
                        return vec![];
                    }
                };
                self.tools.original_message = value.clone();
                self.tools.field = None;
                return self.tool_job(
                    ToolJob::MessageReview {
                        net: self.net().clone(),
                        checked,
                        key,
                        text: value,
                    },
                    false,
                );
            }
            Purpose::Token => {
                let key = match self.verified_key() {
                    Ok(v) => v,
                    Err(e) => {
                        self.tools_error(e);
                        return vec![];
                    }
                };
                self.tools.field = None;
                return self.tool_job(
                    ToolJob::Token {
                        net: self.net().clone(),
                        owner: key.address.unwrap(),
                        token: value,
                    },
                    false,
                );
            }
            Purpose::SignConfirm => {
                if value != "sign" {
                    self.tools.notice = Some(Outside::detail(
                        "Type sign exactly to sign these reviewed bytes.",
                    ));
                    return vec![];
                }
                if let Err(e) = self.ensure_tool_review_key() {
                    self.tools_error(e);
                    return vec![];
                }
                let review = self
                    .tools
                    .message
                    .take()
                    .expect("key check requires a review");
                self.tools.field = None;
                return self.tool_job(ToolJob::SignMessage(review), true);
            }
            Purpose::Confirm => {
                if let Err(e) = self.ensure_tool_review_key() {
                    self.tools_error(e);
                    return vec![];
                }
                if self
                    .tools
                    .invoke
                    .as_ref()
                    .is_none_or(|r| r.contract().nickname() != value)
                {
                    self.tools.notice = Some(Outside::detail(
                        "Type the reviewed contract nickname exactly.",
                    ));
                    return vec![];
                }
                let review = self.tools.invoke.take().unwrap();
                let from = review.key().address.clone().unwrap();
                let net = self.net().clone();
                self.tools.field = None;
                self.tools.pending_intent = Some(review.retained_intent());
                let send_id = self.begin_external_send();
                self.tools.send_id = Some(send_id);
                return self.tool_job(
                    ToolJob::SendInvoke {
                        review: Box::new(review),
                        net,
                        from,
                        nickname: value,
                        send_id,
                    },
                    true,
                );
            }
        }
        vec![]
    }
    fn arguments_done(&mut self) -> Vec<Effect> {
        if self.tools.tool == Tool::Call {
            let input = CallInput {
                from: self.verified_key().ok().and_then(|k| k.address),
                to: self.tools.to.clone(),
                sig: self.tools.sig.clone(),
                args: self.tools.args.clone(),
            };
            if let Err(e) = input.validate() {
                self.tools.notice = Some(Outside::error(&e));
                return vec![];
            }
            self.tools.field = None;
            return self.tool_job(
                ToolJob::Call {
                    net: self.net().clone(),
                    input,
                },
                false,
            );
        }
        let old = self.tools.value.clone();
        self.tool_field(Purpose::Value, Kind::Amount, &old);
        vec![]
    }
    fn prepare_invoke(&mut self) -> Vec<Effect> {
        let (key, checked) = match self.tool_signing_context() {
            Ok(v) => v,
            Err(e) => {
                self.tools_error(e);
                return vec![];
            }
        };
        let Some(contract) = self.tools.contract.clone() else {
            self.tools_error(WalletError::validation("choose a checked contract"));
            return vec![];
        };
        let call = if self.tools.tool == Tool::RawInvoke {
            InvokeData::Raw(self.tools.data.clone())
        } else {
            InvokeData::Signature {
                sig: self.tools.sig.clone(),
                args: self.tools.args.clone(),
            }
        };
        let input = InvokeInput {
            key,
            contract,
            call,
            value: self.tools.value.clone(),
            options: self.tools.options.clone(),
        };
        self.tools.invoke = None;
        self.tools.review_scroll = 0;
        self.tools.field = None;
        self.tool_job(
            ToolJob::InvokeReview {
                net: self.net().clone(),
                checked,
                input: Box::new(input),
            },
            false,
        )
    }
    fn ensure_tool_review_key(&self) -> Result<()> {
        let (key, checked) = self.tool_signing_context()?;
        let reviewed = self
            .tools
            .message
            .as_ref()
            .map(MessageReview::key)
            .or_else(|| self.tools.invoke.as_ref().map(InvokeReview::key))
            .ok_or_else(|| WalletError::validation("review again before signing"))?;
        if key.name != reviewed.name
            || key.index != reviewed.index
            || key.address != reviewed.address
        {
            return Err(WalletError::validation(
                "the password-checked key changed; review again",
            ));
        }
        let fingerprint = self
            .tools
            .message
            .as_ref()
            .map(MessageReview::fingerprint)
            .or_else(|| self.tools.invoke.as_ref().map(InvokeReview::fingerprint));
        if fingerprint != Some(&checked.fingerprint) {
            return Err(WalletError::validation(
                "the checked list changed; review again",
            ));
        }
        Ok(())
    }
    pub fn tools_hint(&self) -> &'static str {
        match self.tools.stage {
            Stage::Menu => {
                "Enter open  C call  i invoke  d raw  m message  t token  a tasks  ? actions"
            }
            Stage::Field | Stage::Confirm | Stage::MessageConfirm => {
                "Enter continue  arrows edit  Esc cancel  Ctrl-C quit"
            }
            Stage::Contracts => "arrows choose checked contract  Enter continue  Esc back",
            Stage::MessageReview => "Enter confirm  e edit (drops review)  Esc cancel",
            Stage::InvokeReview => {
                "Enter confirm  j/k scroll  e edit  p pending  d repeat  g fee  t task  Esc back"
            }
            Stage::Loading => "reading...  Esc cancel",
            Stage::Signing => "signing reviewed bytes...",
            Stage::Result => "r alternate result/raw or signature/digest  Enter/Esc tools",
            Stage::Error => "Enter/Esc tools  V verify  u warm-up",
        }
    }
}

#[cfg(test)]
mod completion_tests {
    use super::*;

    fn submitted() -> App {
        let net = Net {
            chain: chains::find("ethereum").unwrap(),
            rpc_url: "http://127.0.0.1:1".into(),
        };
        let mut app = App::new(&net, Duration::from_secs(60));
        app.nav.tab = Tab::Tools;
        app.tools.stage = Stage::Signing;
        app.tools.active = Some(42);
        app
    }
    fn original_error() -> WalletError {
        WalletError::new(ErrorCode::WhitelistDenied, "original signature failure")
            .with_detail("operation", "sign_message")
            .with_detail("fixture", 42)
    }
    fn failed(id: u64) -> Msg {
        Msg::Tools(id, ToolMsg::Signature(Err(original_error())))
    }
    #[test]
    fn quitting_signature_preserves_error_through_deferred_lock_and_ignores_stale_replies() {
        for lock in [false, true] {
            let mut app = submitted();
            app.update(Msg::Key(Key::Char('q')));
            if lock {
                app.update(Msg::Key(Key::Char('L')));
            }
            assert!(app.mutation_pending());
            assert!(!app.quitting());
            assert!(app.update(failed(41)).is_empty());
            assert!(app.take_quit_error().is_none());
            let work = app.update(failed(42));
            if lock {
                assert_eq!(work, vec![Effect::LockAll]);
                assert!(!app.quitting());
                app.update(Msg::Locked(Err(WalletError::validation(
                    "later lock failure",
                ))));
            }
            assert!(app.quitting());
            assert_eq!(app.take_quit_error().unwrap(), original_error());
        }
    }
    #[test]
    fn broken_terminal_drains_signature_and_deferred_lock_but_never_consumes_more_input() {
        use crate::tui::{terminal_frame, terminal_update};
        for quit in [false, true] {
            let mut app = submitted();
            if quit {
                app.update(Msg::Key(Key::Char('q')));
            }
            app.update(Msg::Key(Key::Char('L')));
            let mut failure = None;
            assert!(
                terminal_frame(&app, &mut failure, || Err(std::io::Error::other(
                    "original terminal failure"
                )))
                .is_ok()
            );
            assert!(terminal_update(&mut app, &failure, Msg::Key(Key::Char('q'))).is_empty());
            assert_eq!(
                app.quit_pending(),
                quit,
                "failed frame cannot consume queued input"
            );
            assert_eq!(
                terminal_update(&mut app, &failure, failed(42)),
                vec![Effect::LockAll]
            );
            assert!(terminal_frame(&app, &mut failure, || panic!("no redraw")).is_ok());
            terminal_update(&mut app, &failure, Msg::Locked(Ok(vec![])));
            assert_eq!(
                terminal_frame(&app, &mut failure, || panic!("no redraw"))
                    .unwrap_err()
                    .message,
                "terminal: original terminal failure"
            );
            assert_eq!(app.take_quit_error(), quit.then(original_error));
        }
    }
}
