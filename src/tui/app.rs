//! The TUI's state: a pure state machine, [`App`] plus a [`Msg`] gives the next [`App`] and the
//! work it asks for ([`Effect`]). Drawing is `ui`'s, reading the terminal and running the work is
//! the loop's (`tui::run`). What comes in from outside is made [`Outside`] here, where it arrives.

use crate::chains::{Chain, Net};
use crate::core::error::{Result, WalletError};
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// `wallet::keys`, then [`Msg::Keys`] with this generation: a lock starts a new one, and a
    /// read from an older one is dropped.
    LoadKeys(u64),
    /// `wallet::status`, then [`Msg::Status`] with this generation, as for [`Effect::LoadKeys`].
    LoadStatus(u64),
    /// `protocol::stop_all_daemons`: every zwallet daemon of this user, then [`Msg::Locked`].
    LockAll,
    /// `wallet::set_active` for this key, then [`Msg::Activated`].
    SetActive(String),
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
    /// The generation of reads the panes take ([`Effect::LoadKeys`], [`Effect::LoadStatus`]).
    generation: u64,
    locking: bool,
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
            chain: net.chain,
            rpc_host: Outside::rpc_host(&net.rpc_url),
            warm,
            keys: Pane::Loading,
            status: Pane::Loading,
            selected: 0,
            keys_pending: false,
            status_pending: false,
            generation: 0,
            locking: false,
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
        match msg {
            Msg::Key(key) => {
                self.notice = None;
                self.key(key)
            }
            Msg::Resize => Vec::new(),
            Msg::Tick => self.load_status(),
            Msg::Keys(generation, _) if generation != self.generation => Vec::new(),
            Msg::Keys(_, keys) => {
                self.keys_pending = false;
                self.keys = match keys {
                    Ok(keys) => Pane::Ready(key_list(keys)),
                    Err(e) => Pane::Failed(Outside::error(&e)),
                };
                self.selected = self.selected.min(self.rows().saturating_sub(1));
                Vec::new()
            }
            Msg::Status(generation, _) if generation != self.generation => Vec::new(),
            Msg::Status(_, status) => {
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
                    if self.quit {
                        self.quit_error = Some(e);
                    }
                }
                // A read started before the lock may land after it and say unlocked: the bar
                // says it is checking until a fresh status lands.
                self.status = Pane::Loading;
                self.reload()
            }
            Msg::Activated(Ok(_)) => self.reload(),
            // A cache the daemon's answer contradicts names an address not to fund: the whole
            // error stops the pane (wrapped there), never a line cut before its warning.
            Msg::Activated(Err(e)) => {
                self.keys = Pane::Failed(Outside::error(&e));
                Vec::new()
            }
        }
    }

    /// The key list and the status read again in a new generation: a read started before (a
    /// lock, a new active key) is dropped when it lands.
    fn reload(&mut self) -> Vec<Effect> {
        self.generation += 1;
        self.status_pending = false;
        self.keys_pending = false;
        let mut work = self.load_status();
        work.extend(self.load_keys());
        work
    }

    fn key(&mut self, key: Key) -> Vec<Effect> {
        // Any key but `y` says no; Ctrl-C says no and still quits.
        if let Some(line) = self.confirm.take()
            && key != Key::Interrupt
        {
            return match key {
                Key::Char('y') => vec![Effect::SetActive(line.key)],
                _ => Vec::new(),
            };
        }
        match key {
            Key::Char('q') | Key::Interrupt => self.quit = true,
            Key::Up | Key::Char('k') => self.selected = self.selected.saturating_sub(1),
            Key::Down | Key::Char('j') if self.selected + 1 < self.rows() => {
                self.selected += 1;
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
            Key::Char('s') => {
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

    /// `q` was pressed, and no lock (`L`) is still stopping daemons: the TUI never leaves while
    /// one is, or the stop would be cut off with the process.
    pub fn quitting(&self) -> bool {
        self.quit && !self.locking
    }

    /// `q` waits for the lock to finish ([`App::quitting`]).
    pub fn quit_pending(&self) -> bool {
        self.quit && self.locking
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
            Some(match trust {
                Trust::Session => TrustLine::Session,
                Trust::Locked => TrustLine::Locked,
                Trust::Unverified(None) => TrustLine::NotVerified,
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
