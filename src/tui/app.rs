//! The TUI's state: a pure state machine, [`App`] plus a [`Msg`] gives the next [`App`] and the
//! work it asks for ([`Effect`]). Drawing is `ui`'s, reading the terminal and running the work is
//! the loop's (`tui::run`). What comes in from outside is made [`Outside`] here, where it arrives.

use crate::chains::{Chain, Net};
use crate::core::error::Result;
use crate::core::time::unix_secs;
use crate::tui::text::Outside;
use crate::wallet::{Keys, StatusView, Trust};
use std::time::{Duration, SystemTime};

/// A key the human pressed, as the TUI reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Enter,
    Esc,
    Char(char),
    /// Ctrl-C, which raw mode delivers as a key instead of a signal.
    Interrupt,
}

/// Everything that changes the state: a key, the refresh timer, or the result of work.
#[derive(Debug)]
pub enum Msg {
    Key(Key),
    /// The terminal changed size; the next frame is drawn for the new one.
    Resize,
    /// The refresh timer (`tui::REFRESH`): read the lock state again.
    Tick,
    Keys(Result<Keys>),
    Status(Result<StatusView>),
    /// What `protocol::stop_all_daemons` did (`L`).
    Locked(Result<Vec<i32>>),
}

/// Work the state asks for; its result comes back as a [`Msg`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// `wallet::keys`, then [`Msg::Keys`].
    LoadKeys,
    /// `wallet::status`, then [`Msg::Status`].
    LoadStatus,
    /// `protocol::stop_all_daemons`: every zwallet daemon of this user, then [`Msg::Locked`].
    LockAll,
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
    /// This build's key daemon of this home derived it ([`Trust::Session`]).
    Session,
    /// The cache, and no session checked it.
    NotVerified,
    /// Another process answered on `d.sock`: what `PeerCheck::warning` says about it.
    Impostor(Outside),
}

/// The lock state as the bar draws it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LockLine {
    Locked,
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

pub struct App {
    chain: &'static Chain,
    rpc_host: Outside,
    warm: Duration,
    keys: Pane<KeyList>,
    status: Pane<Bar>,
    selected: usize,
    keys_pending: bool,
    status_pending: bool,
    locking: bool,
    /// An error to show in place of the key hints until the next key.
    notice: Option<Outside>,
    quit: bool,
}

impl App {
    /// The TUI on `net`'s chain and endpoint; `warm` is how long a warm-up unlocks for.
    pub fn new(net: &Net, warm: Duration) -> Self {
        Self {
            chain: net.chain,
            rpc_host: Outside::rpc_host(&net.rpc_url),
            warm,
            keys: Pane::Loading,
            status: Pane::Loading,
            selected: 0,
            keys_pending: false,
            status_pending: false,
            locking: false,
            notice: None,
            quit: false,
        }
    }

    /// The work to start with: the key list and the status.
    pub fn start(&mut self) -> Vec<Effect> {
        let mut work = self.load_keys();
        work.extend(self.load_status());
        work
    }

    pub fn update(&mut self, msg: Msg) -> Vec<Effect> {
        match msg {
            Msg::Key(key) => {
                self.notice = None;
                self.key(key)
            }
            Msg::Resize => Vec::new(),
            Msg::Tick => self.load_status(),
            Msg::Keys(keys) => {
                self.keys_pending = false;
                self.keys = match keys {
                    Ok(keys) => Pane::Ready(key_list(keys)),
                    Err(e) => Pane::Failed(Outside::error(&e)),
                };
                self.selected = self.selected.min(self.rows().saturating_sub(1));
                Vec::new()
            }
            Msg::Status(status) => {
                self.status_pending = false;
                self.status = match status {
                    Ok(status) => Pane::Ready(bar(status)),
                    Err(e) => Pane::Failed(Outside::error(&e)),
                };
                Vec::new()
            }
            Msg::Locked(stopped) => {
                self.locking = false;
                if let Err(e) = stopped {
                    self.notice = Some(Outside::error(&e));
                }
                let mut work = self.load_status();
                work.extend(self.load_keys());
                work
            }
        }
    }

    fn key(&mut self, key: Key) -> Vec<Effect> {
        match key {
            Key::Char('q') | Key::Interrupt => self.quit = true,
            Key::Up | Key::Char('k') => self.selected = self.selected.saturating_sub(1),
            Key::Down | Key::Char('j') => {
                if self.selected + 1 < self.rows() {
                    self.selected += 1;
                }
            }
            Key::Char('r') => {
                let mut work = self.load_keys();
                work.extend(self.load_status());
                return work;
            }
            Key::Char('L') if !self.locking => {
                self.locking = true;
                return vec![Effect::LockAll];
            }
            _ => {}
        }
        Vec::new()
    }

    /// [`Effect::LoadKeys`], unless a read is running already.
    fn load_keys(&mut self) -> Vec<Effect> {
        if std::mem::replace(&mut self.keys_pending, true) {
            return Vec::new();
        }
        vec![Effect::LoadKeys]
    }

    /// [`Effect::LoadStatus`], unless a read is running already.
    fn load_status(&mut self) -> Vec<Effect> {
        if std::mem::replace(&mut self.status_pending, true) {
            return Vec::new();
        }
        vec![Effect::LoadStatus]
    }

    fn rows(&self) -> usize {
        match &self.keys {
            Pane::Ready(list) => list.rows.len(),
            _ => 0,
        }
    }

    pub fn quitting(&self) -> bool {
        self.quit
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

    pub fn notice(&self) -> Option<&Outside> {
        self.notice.as_ref()
    }
}

fn key_list(keys: Keys) -> KeyList {
    KeyList {
        rows: keys
            .rows
            .iter()
            .map(|k| KeyLine {
                name: Outside::key_name(&k.name),
                index: k.index,
                address: k.address.as_deref().map(Outside::address),
                active: k.active,
            })
            .collect(),
        warning: keys.warning.as_deref().map(Outside::warning),
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
            Some(match trust {
                Trust::Session => TrustLine::Session,
                Trust::Locked | Trust::Unverified(None) => TrustLine::NotVerified,
                Trust::Unverified(Some(w)) => TrustLine::Impostor(Outside::warning(&w)),
            }),
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
