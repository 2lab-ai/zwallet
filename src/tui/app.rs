//! The TUI's state: a pure state machine, [`App`] plus a [`Msg`] gives the next [`App`] and the
//! work it asks for ([`Effect`]). Drawing is `ui`'s, reading the terminal and running the work is
//! the loop's (`tui::run`). What comes in from outside is made [`Outside`] here, where it arrives.

mod addresses;
pub mod create;
pub mod history;
mod navigation;
pub mod replacement;
pub mod tools;
pub mod transfer;
pub mod whitelist;
use create::{CreateEffect, CreateMsg, CreateUi};
use history::{DetailRead, DetailRequest, HistoryRead, HistoryScope, HistoryTarget, HistoryView};
pub use navigation::{FieldPurpose, Navigation, Popup, RpcChoice, UserText};

use crate::chains::{Chain, Net};
use crate::core::error::{Result, WalletError};
use crate::core::time::unix_secs;
use crate::tui::actions::Tab;
use crate::tui::text::Outside;
use crate::wallet::{CheckedList, Keys, Opened, StatusView, Trust};
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

/// A key the human pressed, as the TUI reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    Backspace,
    Delete,
    Tab,
    BackTab,
    Enter,
    Esc,
    Char(char),
    /// Ctrl-C, which raw mode delivers as a key instead of a signal.
    Interrupt,
}

/// Everything that changes the state: a key, the refresh timer, or the result of work.
#[derive(Debug)]
pub enum Msg {
    WhitelistRead(u64, Result<whitelist::Read>),
    WhitelistPrepared(u64, Result<String>),
    WhitelistApplied(u64, whitelist::Applied),
    Tools(u64, tools::ToolMsg),
    Replacement(u64, replacement::Event),
    Transfer(u64, transfer::TransferMsg),
    HistoryOwner(HistoryTarget, Result<(String, Trust)>),
    History(HistoryScope, Result<HistoryRead>),
    Transaction(Box<DetailRequest>, Result<DetailRead>),
    HistoryList(u64, crate::core::whitelist::Fingerprint, Result<()>),
    Create(CreateMsg),
    Key(Key),
    Paste(UserText),
    Verified(
        Result<(
            crate::wallet::PasswordCheck,
            std::result::Result<CheckedList, String>,
        )>,
    ),
    Balances(u64, String, Result<crate::wallet::ChainBalances>),
    EveryBalances(u64, String, Result<crate::wallet::EveryBalance>),
    Addresses(u64, String, Result<crate::wallet::Addresses>),
    RpcChecked(u64, Result<RpcChoice>),
    Prepared(Result<()>),
    PasswordLength(usize, bool),
    PasswordSubmitted,
    PasswordCancelled,
    Checked(Result<(Opened, std::result::Result<CheckedList, String>)>),
    Address(u64, String, Result<(String, Trust)>),
    /// The terminal changed size; the next frame is drawn for the new one.
    Resize,
    /// The refresh timer (`tui::REFRESH`): read the lock state again.
    Tick,
    /// A key read, with the generation it was started in ([`Effect::LoadKeys`]).
    Keys(u64, Result<Keys>),
    /// A status read, with the generation it was started in ([`Effect::LoadStatus`]).
    Status(u64, Result<StatusView>),
    /// What `protocol::stop_all_daemons` did (`L`).
    Locked(Result<Vec<i32>>),
    /// What `wallet::set_active` did (`s`, confirmed): the key's address and what verifies it.
    Activated(Result<(String, Trust)>),
}

/// Work the state asks for; its result comes back as a [`Msg`].
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    ReadWhitelist(u64, Option<CheckedList>),
    PrepareWhitelist(whitelist::EditRequest),
    ApplyWhitelist(u64),
    DiscardWhitelist(u64),
    Tools(u64),
    Replacement(u64),
    Transfer(u64),
    HistoryOwner(HistoryTarget),
    LoadHistory(HistoryScope),
    LoadTransaction(Box<DetailRequest>),
    CheckHistoryList(u64, crate::core::whitelist::Fingerprint),
    Create(CreateEffect),
    /// `wallet::keys`, then [`Msg::Keys`] with this generation: a lock starts a new one, and a
    /// read from an older one is dropped.
    LoadKeys(u64),
    /// `wallet::status`, then [`Msg::Status`] with this generation, as for [`Effect::LoadKeys`].
    LoadStatus(u64),
    /// `protocol::stop_all_daemons`: every zwallet daemon of this user, then [`Msg::Locked`].
    LockAll,
    /// `wallet::set_active` for this key, then [`Msg::Activated`].
    SetActive(String),
    PrepareUnlock,
    PrepareRead,
    PrepareDuration(Duration),
    CheckReadPassword {
        key: String,
        all: bool,
        repair: bool,
    },
    LoadBalances(u64, String),
    LoadEveryBalances(u64, String),
    LoadAddresses(u64, String),
    CheckRpc(u64, RpcChoice),
    CheckPassword {
        key: String,
        start_session: bool,
    },
    LoadAddress(u64, String),
}

/// A pane's data: being read, read, or the error that stopped it.
#[derive(Clone, Debug)]
pub enum Pane<T> {
    Loading,
    Ready(T),
    Failed(Outside),
}

/// One row of the key list.
#[derive(Clone, Debug)]
pub struct KeyLine {
    /// The key's name in wallet.json, for the work on it; drawn only as `name`.
    pub key: String,
    pub name: Outside,
    pub index: u32,
    pub address: Option<Outside>,
    /// The globally active key.
    pub active: bool,
}

/// The key list as the keys pane draws it.
#[derive(Clone, Debug)]
pub struct KeyList {
    pub rows: Vec<KeyLine>,
    pub warning: Option<Outside>,
}

/// What the status bar says about the active key's address (03-tui.md §Address trust).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrustLine {
    Password,
    /// This build's key daemon of this home derived it ([`Trust::Session`]).
    Session,
    /// Locked: the cache.
    Locked,
    /// Unlocked, but no session check passed (a cached address, or Linux, where it never does).
    NotVerified,
    /// Another process answered on `d.sock`: what `PeerCheck::warning` says about it.
    Impostor(Outside),
}

/// The lock state as the bar draws it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LockLine {
    Locked,
    Unknown(Outside),
    /// Unlocked: the session's expiry, when a daemon of the home reported one.
    Open(Option<Outside>),
}

/// The status as the bar draws it.
#[derive(Clone, Debug)]
pub struct Bar {
    pub configured: bool,
    pub key: Option<Outside>,
    pub address: Option<Outside>,
    pub trust: Option<TrustLine>,
    pub lock: LockLine,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasswordState {
    Idle,
    Preparing,
    /// Preparation failed: consume typeahead until Enter/Esc dismisses the error.
    Failed,
    Input {
        characters: usize,
        rejected: bool,
    },
    Checking,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasswordPurpose {
    WhitelistVerify,
    WhitelistEdit,
    Warm,
    Verify { all: bool },
    Repair,
    Unlock(Duration),
}

pub struct App {
    whitelist: whitelist::WhitelistUi,
    tools: tools::Tools,
    replacement: replacement::Replacement,
    transfer: transfer::Transfer,
    create: CreateUi,
    nav: Navigation,
    history: HistoryView,
    password_purpose: PasswordPurpose,
    activating: bool,
    chain: &'static Chain,
    rpc_host: Outside,
    warm: Duration,
    keys: Pane<KeyList>,
    status: Pane<Bar>,
    selected: usize,
    open_key: Option<String>,
    password: PasswordState,
    pending_key: Option<String>,
    pins: BTreeMap<String, String>,
    verified_indices: BTreeMap<String, u32>,
    checked_list: Option<CheckedList>,
    list_reason: Option<Outside>,
    keys_pending: bool,
    status_pending: bool,
    /// The generation of reads the panes take ([`Effect::LoadKeys`], [`Effect::LoadStatus`]).
    generation: u64,
    locking: bool,
    lock_after_check: bool,
    /// An error to show in place of the key hints until the next key.
    notice: Option<Outside>,
    /// `s` asks before the key becomes the globally active one: the next key says.
    confirm: Option<KeyLine>,
    quit: bool,
    /// The error of a lock that failed while a quit waited for it, for the CLI to print.
    quit_error: Option<WalletError>,
}

impl App {
    /// The TUI on `net`'s chain and endpoint; `warm` is how long a warm-up unlocks for.
    pub fn new(net: &Net, warm: Duration) -> Self {
        Self {
            whitelist: whitelist::WhitelistUi::default(),
            tools: tools::Tools::default(),
            replacement: replacement::Replacement::default(),
            transfer: transfer::Transfer::default(),
            nav: Navigation::new(net),
            history: HistoryView::default(),
            create: CreateUi::default(),
            password_purpose: PasswordPurpose::Warm,
            activating: false,
            chain: net.chain,
            rpc_host: Outside::rpc_host(&net.rpc_url),
            warm,
            keys: Pane::Loading,
            status: Pane::Loading,
            selected: 0,
            open_key: None,
            password: PasswordState::Idle,
            pending_key: None,
            pins: BTreeMap::new(),
            verified_indices: BTreeMap::new(),
            checked_list: None,
            list_reason: None,
            keys_pending: false,
            status_pending: false,
            generation: 0,
            locking: false,
            lock_after_check: false,
            notice: None,
            confirm: None,
            quit: false,
            quit_error: None,
        }
    }

    /// The work to start with: the key list and the status.
    pub fn start(&mut self) -> Vec<Effect> {
        let mut work = self.load_keys();
        work.extend(self.load_status());
        work
    }

    pub fn update(&mut self, msg: Msg) -> Vec<Effect> {
        let msg = match self.create_update(msg) {
            std::ops::ControlFlow::Continue(work) => return work,
            std::ops::ControlFlow::Break(msg) => msg,
        };
        let msg = match self.whitelist_update(msg) {
            std::ops::ControlFlow::Continue(work) => return work,
            std::ops::ControlFlow::Break(msg) => msg,
        };
        let msg = match self.tools_update(msg) {
            std::ops::ControlFlow::Continue(work) => return work,
            std::ops::ControlFlow::Break(msg) => msg,
        };
        let msg = match self.replacement_update(msg) {
            std::ops::ControlFlow::Continue(work) => return work,
            std::ops::ControlFlow::Break(msg) => msg,
        };
        let msg = match self.transfer_update(msg) {
            std::ops::ControlFlow::Continue(effects) => return effects,
            std::ops::ControlFlow::Break(msg) => msg,
        };
        let msg = match self.history_update(msg) {
            std::ops::ControlFlow::Continue(work) => return work,
            std::ops::ControlFlow::Break(msg) => msg,
        };
        let msg = match self.navigation_update(msg) {
            std::ops::ControlFlow::Continue(work) => return work,
            std::ops::ControlFlow::Break(msg) => msg,
        };
        match msg {
            Msg::Key(key) => {
                self.notice = None;
                self.key(key)
            }
            Msg::Resize => Vec::new(),
            Msg::Tick => {
                let mut work = self.load_status();
                work.extend(self.history_tick());
                work
            }
            Msg::Prepared(Ok(())) => {
                self.password = PasswordState::Input {
                    characters: 0,
                    rejected: false,
                };
                Vec::new()
            }
            Msg::Prepared(Err(e)) => {
                self.password = PasswordState::Failed;
                self.pending_key = None;
                self.fail_pane(e);
                Vec::new()
            }
            Msg::PasswordLength(characters, rejected) => {
                if matches!(self.password, PasswordState::Input { .. }) {
                    self.password = PasswordState::Input {
                        characters,
                        rejected,
                    };
                }
                Vec::new()
            }
            Msg::PasswordCancelled => {
                self.password = PasswordState::Idle;
                self.pending_key = None;
                Vec::new()
            }
            Msg::PasswordSubmitted => {
                if matches!(self.password, PasswordState::Input { .. })
                    && let Some(key) = self.pending_key.clone()
                {
                    self.password = PasswordState::Checking;
                    self.checked_list = None;
                    return vec![match self.password_purpose {
                        PasswordPurpose::Warm | PasswordPurpose::Unlock(_) => {
                            Effect::CheckPassword {
                                key,
                                start_session: true,
                            }
                        }
                        PasswordPurpose::WhitelistEdit => {
                            unreachable!("whitelist submission owns its prepared plan")
                        }
                        PasswordPurpose::WhitelistVerify => Effect::CheckReadPassword {
                            key,
                            all: false,
                            repair: false,
                        },
                        PasswordPurpose::Verify { all } => Effect::CheckReadPassword {
                            key,
                            all,
                            repair: false,
                        },
                        PasswordPurpose::Repair => Effect::CheckReadPassword {
                            key,
                            all: true,
                            repair: true,
                        },
                    }];
                }
                Vec::new()
            }
            Msg::Checked(result) => {
                self.password = PasswordState::Idle;
                let key = self.pending_key.take();
                // A later password check may read a replaced wallet or edited key index. The
                // original derivations stay authoritative for this TUI's whole lifetime.
                let result = result.and_then(|(opened, checked)| {
                    for row in &opened.keys {
                        if self.verified_indices.get(&row.name).is_some_and(|index| *index != row.index) {
                            return Err(WalletError::validation("the key index changed after the password check; close and verify this wallet again"));
                        }
                        if let Some(address) = &row.address {
                            self.check_pin(&row.name, address)?;
                        }
                    }
                    Ok((opened, checked))
                });
                let mut work = match result {
                    Ok((opened, checked)) => {
                        let opened_key = key
                            .filter(|key| !key.is_empty())
                            .unwrap_or_else(|| opened.active.clone());
                        for k in opened.keys {
                            self.verified_indices
                                .entry(k.name.clone())
                                .or_insert(k.index);
                            if let Some(address) = k.address {
                                self.pins.insert(k.name, address);
                            }
                        }
                        if matches!(
                            self.password_purpose,
                            PasswordPurpose::Warm | PasswordPurpose::Unlock(_)
                        ) {
                            self.open_key = Some(opened_key);
                            self.nav.tab = Tab::Wallet;
                        }
                        match checked {
                            Ok(list) => {
                                self.checked_list = Some(list);
                                self.list_reason = None;
                            }
                            Err(reason) => {
                                self.checked_list = None;
                                self.list_reason = Some(Outside::warning(&reason));
                            }
                        }
                        self.reload()
                    }
                    Err(e) => {
                        if self.quit {
                            self.quit_error = Some(e.clone());
                        }
                        self.fail_pane(e);
                        if matches!(self.password_purpose, PasswordPurpose::Verify { .. }) {
                            self.notice = Some(Outside::detail(
                                "A on 1 keys repairs cached addresses with the password",
                            ));
                        }
                        self.load_status()
                    }
                };
                if std::mem::take(&mut self.lock_after_check) {
                    work.push(Effect::LockAll);
                }
                work
            }
            Msg::Address(generation, _, _) if generation != self.generation => Vec::new(),
            Msg::Address(_, key, result) => {
                let selected = self.open_key.as_ref().or_else(|| {
                    self.nav
                        .session
                        .as_ref()
                        .and_then(|status| status.active.as_ref())
                });
                if selected != Some(&key) {
                    return Vec::new();
                }
                match result.and_then(|(address, trust)| {
                    self.observe_family_address(&key, self.chain, &address, trust.clone())?;
                    if self.history.scope.as_ref().is_some_and(|scope| scope.target.key == key && scope.owner != address) {
                        return Err(WalletError::validation("selected key's address changed during the history read; verify it again"));
                    }
                    Ok((address, trust))
                }) {
                    Ok((address, trust)) => {
                        let pinned = self.family_pinned(&key, self.chain);
                        if let Pane::Ready(bar) = &mut self.status {
                            bar.key = Some(Outside::key_name(&key));
                            bar.address = Some(Outside::short_address(&address));
                            bar.trust = Some(if matches!(&trust, Trust::Unverified(Some(_))) {
                                trust_line(trust)
                            } else if pinned {
                                TrustLine::Password
                            } else {
                                trust_line(trust)
                            });
                        }
                    }
                    Err(e) => self.fail_pane(e),
                }
                Vec::new()
            }
            Msg::Keys(generation, _) if generation != self.generation => Vec::new(),
            Msg::Keys(_, keys) => {
                self.keys_pending = false;
                let checked = keys.and_then(|keys| {
                    keys.rows.iter().try_for_each(|k| {
                        k.address
                            .as_deref()
                            .map_or(Ok(()), |a| self.check_pin(&k.name, a))
                    })?;
                    Ok(keys)
                });
                match checked {
                    Ok(keys) => self.keys = Pane::Ready(key_list(keys)),
                    Err(e) => self.fail_pane(e),
                }
                self.selected = self.selected.min(self.rows().saturating_sub(1));
                Vec::new()
            }
            Msg::Status(generation, _) if generation != self.generation => Vec::new(),
            Msg::Status(_, status) => {
                self.status_pending = false;
                match status {
                    Ok(status) => {
                        for (_, trust) in self.nav.family_reads.values_mut() {
                            if let Some((_, Trust::Unverified(Some(warning)))) = &status.address {
                                *trust = Trust::Unverified(Some(warning.clone()));
                            } else if *trust == Trust::Session {
                                // Fresh negative session evidence revokes a previous read's
                                // session claim. Password-derived pins are stored separately.
                                if status.locked
                                    || matches!(&status.address, Some((_, Trust::Locked)))
                                {
                                    *trust = Trust::Locked;
                                } else if status.serving.is_none()
                                    || matches!(&status.address, Some((_, Trust::Unverified(None))))
                                {
                                    *trust = Trust::Unverified(None);
                                }
                            }
                        }
                        // An address refusal must not hide fresh process-table lock evidence.
                        // Retain the error until the new status and expiry have replaced the bar.
                        let observation_error = if let (Some(key), Some((Some(address), trust))) =
                            (&status.active, &status.address)
                        {
                            let primary = crate::chains::find(crate::chains::PRIMARY_CHAIN)
                                .expect("primary chain");
                            self.observe_family_address(key, primary, address, trust.clone())
                                .err()
                        } else {
                            None
                        };
                        self.nav.session = Some(status.clone());
                        if self.nav.tab == Tab::Session {
                            self.render_session();
                        }
                        let mismatch =
                            observation_error.or_else(|| match (&status.active, &status.address) {
                                (Some(key), Some((Some(address), _))) => {
                                    self.check_pin(key, address).err()
                                }
                                _ => None,
                            });
                        let shown_key = self.open_key.clone().or_else(|| status.active.clone());
                        let mut next = bar(status);
                        if let Some(key) = shown_key {
                            next.key = Some(Outside::key_name(&key));
                            if let Some(address) = self.family_pin(&key, self.chain) {
                                next.address = Some(Outside::short_address(address));
                                if !matches!(&next.trust, Some(TrustLine::Impostor(_))) {
                                    next.trust = Some(TrustLine::Password);
                                }
                            } else if self.open_key.is_some() {
                                next.address = None;
                                if !matches!(&next.trust, Some(TrustLine::Impostor(_))) {
                                    next.trust = None;
                                }
                            }
                        }
                        // Pinning an address never pins lock certainty: fresh process-table
                        // evidence and expiry still replace the previous lock line on refusal.
                        self.status = Pane::Ready(next);
                        if let Some(e) = mismatch {
                            self.fail_pane(e);
                            return Vec::new();
                        }
                    }
                    Err(e) => {
                        let pinned = self
                            .open_key
                            .as_ref()
                            .is_some_and(|key| self.family_pinned(key, self.chain))
                            || (self.open_key.is_none()
                                && matches!(&self.status, Pane::Ready(bar)
                                if bar.key.as_ref().is_some_and(|key| self.family_pinned(key.text(), self.chain))));
                        if pinned && let Pane::Ready(bar) = &mut self.status {
                            bar.lock = LockLine::Unknown(Outside::error(&e));
                        } else {
                            self.status = Pane::Failed(Outside::error(&e));
                        }
                        self.fail_pane(e);
                        return Vec::new();
                    }
                }
                self.open_key
                    .clone()
                    .or_else(|| {
                        matches!(self.chain.family, crate::chains::Family::Tron(_))
                            .then(|| {
                                self.nav
                                    .session
                                    .as_ref()
                                    .and_then(|status| status.active.clone())
                            })
                            .flatten()
                    })
                    .map(|key| vec![Effect::LoadAddress(self.generation, key)])
                    .unwrap_or_default()
            }
            Msg::Locked(stopped) => {
                self.locking = false;
                if let Err(e) = stopped {
                    self.notice = Some(Outside::error(&e));
                    if self.quit {
                        // A deferred stop must not replace an earlier send failure and its
                        // hash/nonce recovery details while the same quit is waiting.
                        self.quit_error.get_or_insert(e);
                    }
                }
                // A read started before the lock may land after it and say unlocked: the bar
                // says it is checking until a fresh status lands.
                self.status = Pane::Loading;
                self.reload()
            }
            Msg::Activated(Ok(_)) => {
                self.activating = false;
                self.reload()
            }
            // A cache the daemon's answer contradicts names an address not to fund: the whole
            // error stops the pane (wrapped there), never a line cut before its warning.
            Msg::Activated(Err(e)) => {
                self.activating = false;
                if self.quit {
                    self.quit_error = Some(e.clone());
                }
                self.fail_pane(e);
                Vec::new()
            }
            // The navigation dispatcher consumes these variants before this match.
            Msg::Tools(..)
            | Msg::Transfer(..)
            | Msg::HistoryOwner(..)
            | Msg::History(..)
            | Msg::Transaction(..)
            | Msg::HistoryList(..)
            | Msg::WhitelistRead(..)
            | Msg::WhitelistPrepared(..)
            | Msg::WhitelistApplied(..)
            | Msg::Create(_)
            | Msg::Paste(_)
            | Msg::Verified(_)
            | Msg::Balances(..)
            | Msg::EveryBalances(..)
            | Msg::Addresses(..)
            | Msg::RpcChecked(..)
            | Msg::Replacement(..) => unreachable!("navigation or replacement message"),
        }
    }

    /// The key list and the status read again in a new generation: a read started before (a
    /// lock, a new active key) is dropped when it lands.
    fn invalidate_reads(&mut self) {
        self.nav.family_reads.clear();
        self.history = HistoryView::default();
        self.generation += 1;
        self.status_pending = false;
        self.keys_pending = false;
    }

    fn fail_pane(&mut self, error: WalletError) {
        self.invalidate_reads();
        self.checked_list = None;
        self.nav.lines = Pane::Failed(Outside::error(&error));
        self.keys = Pane::Failed(Outside::error(&error));
        if self.quit {
            // Reads can finish while a submitted operation waits for its deferred lock.
            // Keep that operation's original error and recovery details for the exit.
            self.quit_error.get_or_insert(error);
        }
    }

    fn reload(&mut self) -> Vec<Effect> {
        self.invalidate_reads();
        let mut work = self.load_status();
        work.extend(self.load_keys());
        work.extend(self.load_view());
        work
    }

    fn key(&mut self, key: Key) -> Vec<Effect> {
        if self.quit {
            return Vec::new();
        }
        if self.password != PasswordState::Idle {
            if self.password == PasswordState::Checking && key == Key::Char('L') {
                // Starting a daemon is already in progress. Stop it after that operation has
                // finished, so an immediate stop cannot be undone by its later completion.
                self.locking = true;
                self.lock_after_check = true;
            }
            if key == Key::Interrupt
                || key == Key::Char('q') && self.password == PasswordState::Checking
            {
                self.quit = true;
            }
            return Vec::new();
        }
        // Any key but `y` says no; Ctrl-C says no and still quits.
        if let Some(line) = self.confirm.take()
            && key != Key::Interrupt
        {
            return match key {
                Key::Char('y') => {
                    self.activating = true;
                    vec![Effect::SetActive(line.key)]
                }
                _ => Vec::new(),
            };
        }
        if let Some(work) = self.replacement_key(key) {
            return work;
        }
        if let Some(work) = self.transfer_key(key) {
            return work;
        }
        if let Some(work) = self.navigation_key(key) {
            return work;
        }
        match key {
            Key::Char('q') | Key::Interrupt => self.quit = true,
            Key::Up | Key::Char('k') => self.selected = self.selected.saturating_sub(1),
            Key::Down | Key::Char('j') if self.selected + 1 < self.rows() => {
                self.selected += 1;
            }
            Key::Char('r') => {
                return self.reload();
            }
            Key::Char('L') if !self.locking => {
                self.locking = true;
                return vec![Effect::LockAll];
            }
            Key::Enter if !self.locking && !self.activating => {
                if let Pane::Ready(list) = &self.keys
                    && let Some(row) = list.rows.get(self.selected)
                {
                    let key = row.key.clone();
                    if matches!(
                        &self.status,
                        Pane::Ready(Bar {
                            lock: LockLine::Locked,
                            ..
                        })
                    ) {
                        self.invalidate_reads();
                        self.pending_key = Some(key);
                        self.password_purpose = PasswordPurpose::Warm;
                        self.password = PasswordState::Preparing;
                        return vec![Effect::PrepareUnlock];
                    }
                    self.open_key = Some(key.clone());
                    self.nav.tab = Tab::Wallet;
                    let mut work = vec![Effect::LoadAddress(self.generation, key)];
                    work.extend(self.load_view());
                    return work;
                }
            }
            Key::Char('s') if !self.activating => {
                if let Pane::Ready(list) = &self.keys {
                    self.confirm = list.rows.get(self.selected).cloned();
                }
            }
            _ => {}
        }
        Vec::new()
    }

    /// [`Effect::LoadKeys`], unless a read of this generation is running already.
    fn load_keys(&mut self) -> Vec<Effect> {
        if std::mem::replace(&mut self.keys_pending, true) {
            return Vec::new();
        }
        vec![Effect::LoadKeys(self.generation)]
    }

    /// [`Effect::LoadStatus`], unless a read of this generation is running already.
    fn load_status(&mut self) -> Vec<Effect> {
        if std::mem::replace(&mut self.status_pending, true) {
            return Vec::new();
        }
        vec![Effect::LoadStatus(self.generation)]
    }

    fn rows(&self) -> usize {
        match &self.keys {
            Pane::Ready(list) => list.rows.len(),
            _ => 0,
        }
    }

    /// A submitted operation must report its outcome before terminal failure or quit can
    /// release its runtime. Later forms extend this predicate for their submitted workers;
    /// `Checking` includes creation, cache repair and session startup.
    pub fn mutation_pending(&self) -> bool {
        self.locking
            || self.activating
            || self.sending()
            || self.whitelist_busy()
            || self.tools.stage == tools::Stage::Signing
            || self.password == PasswordState::Checking
    }

    /// `q` waits for every submitted mutation, including a deferred daemon stop.
    pub fn quitting(&self) -> bool {
        self.quit && !self.mutation_pending() && self.password != PasswordState::Preparing
    }

    pub fn quit_pending(&self) -> bool {
        self.quit && (self.mutation_pending() || self.password == PasswordState::Preparing)
    }

    /// The error of a lock that failed while a quit waited for it.
    pub fn take_quit_error(&mut self) -> Option<WalletError> {
        self.quit_error.take()
    }

    /// The generation a read must carry for a pane to take it ([`Effect::LoadKeys`],
    /// [`Effect::LoadStatus`]).
    pub fn generation(&self) -> u64 {
        self.generation
    }

    fn check_pin(&self, key: &str, address: &str) -> Result<()> {
        if let Some(pinned) = self.pins.get(key)
            && !pinned.eq_ignore_ascii_case(address)
        {
            return Err(WalletError::validation(format!(
                "the ethereum address for key '{key}' ({address}) does not match the address this TUI derived from the seed ({pinned}); the session or cache changed. Do not send funds to either address. `zwallet address --verify` checks the cache from the seed"
            )));
        }
        Ok(())
    }

    pub fn password_state(&self) -> PasswordState {
        self.password
    }
    pub fn pinned(&self, key: &str) -> bool {
        self.pins.contains_key(key)
    }
    pub fn checked_list(&self) -> Option<&CheckedList> {
        self.checked_list.as_ref()
    }
    /// The current open key from this session's password-derived address and index pins.
    /// Fresh public metadata must still agree; it never supplies the index for a signature.
    pub fn verified_key(&self) -> Result<crate::wallet::KeyRow> {
        let key = self
            .open_key
            .as_ref()
            .ok_or_else(|| WalletError::validation("needs: an open key"))?;
        let index = self.verified_indices.get(key).ok_or_else(|| {
            WalletError::validation("needs: V; verify the key index with the password")
        })?;
        let address = self.family_pin(key, self.chain).ok_or_else(|| {
            WalletError::validation("needs: V; verify the address with the password")
        })?;
        let row = match &self.keys {
            Pane::Ready(keys) => keys.rows.iter().find(|row| &row.key == key),
            _ => None,
        }
        .ok_or_else(|| {
            WalletError::validation(
                "key metadata is unavailable; refresh and verify before signing",
            )
        })?;
        if row.index != *index {
            return Err(WalletError::validation(
                "the key index changed after the password check; nothing signed",
            ));
        }
        Ok(crate::wallet::KeyRow {
            name: key.clone(),
            index: *index,
            address: Some(address.clone()),
            active: row.active,
        })
    }
    pub fn open_key(&self) -> Option<&str> {
        self.open_key.as_deref()
    }

    pub fn chain(&self) -> &'static Chain {
        self.chain
    }

    pub fn rpc_host(&self) -> &Outside {
        &self.rpc_host
    }

    /// How long a warm-up unlocks for (`--warm`).
    pub fn warm(&self) -> Duration {
        self.warm
    }

    pub fn keys(&self) -> &Pane<KeyList> {
        &self.keys
    }

    pub fn status(&self) -> &Pane<Bar> {
        &self.status
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    /// `L` was pressed and the daemons are being stopped.
    pub fn locking(&self) -> bool {
        self.locking
    }
    pub fn activating(&self) -> bool {
        self.activating
    }
    pub fn password_purpose(&self) -> PasswordPurpose {
        self.password_purpose
    }

    pub fn notice(&self) -> Option<&Outside> {
        self.notice.as_ref()
    }

    /// The key `s` asks to make active, until the next key answers.
    pub fn confirming(&self) -> Option<&KeyLine> {
        self.confirm.as_ref()
    }
}

fn key_list(keys: Keys) -> KeyList {
    KeyList {
        rows: keys
            .rows
            .iter()
            .map(|k| KeyLine {
                key: k.name.clone(),
                name: Outside::key_name(&k.name),
                index: k.index,
                address: k.address.as_deref().map(Outside::address),
                active: k.active,
            })
            .collect(),
        warning: keys.trust.warning().map(Outside::warning),
    }
}

fn bar(status: StatusView) -> Bar {
    let lock = if status.locked {
        LockLine::Locked
    } else {
        let now = unix_secs(SystemTime::now());
        LockLine::Open(
            status
                .serving
                .and_then(|s| s.expires_at)
                .map(|e| Outside::expiry(&e, now)),
        )
    };
    let (address, trust) = match status.address {
        Some((address, trust)) => (
            address.as_deref().map(Outside::short_address),
            Some(trust_line(trust)),
        ),
        None => (None, None),
    };
    Bar {
        configured: status.configured,
        key: status.active.as_deref().map(Outside::key_name),
        address,
        trust,
        lock,
    }
}

fn trust_line(trust: Trust) -> TrustLine {
    match trust {
        Trust::Session => TrustLine::Session,
        Trust::Locked => TrustLine::Locked,
        Trust::Unverified(None) => TrustLine::NotVerified,
        Trust::Unverified(Some(w)) => TrustLine::Impostor(Outside::warning(&w)),
    }
}
