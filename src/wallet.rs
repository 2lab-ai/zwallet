//! The operations the CLI and the TUI share (`.prd/v0-2-tui/flow.md`): plain values in, plain
//! values out. Nothing here reads `/dev/tty` or prints; the CLI command that used the code calls
//! the function here and prints what it returns, and the TUI draws it.

pub mod whitelist;
pub use whitelist::{
    Change, EditSession, WhitelistOutcome, WhitelistPlan, WhitelistUnlock, WhitelistView,
    whitelist_apply, whitelist_plan, whitelist_view,
};

pub mod read;
pub use read::{
    AddressRow, Addresses, ChainBalances, EveryBalance, NetworkBalance, addresses, balance,
    balances, brief, every_balance, every_balance_with, net_with, validate_rpc,
};

pub mod send;
pub use send::{
    CheckedPayee, SendOutput, SendTransferOutcome, TransferInput, TransferOptions, TransferReview,
    parse_fee_ceiling, review_transfer, send_transfer, transfer_payees,
};

use self::whitelist::FIRST_ADD;
use crate::chains::WHITELIST_RULES;
use crate::chains::{self, Chain, Net, PRIMARY_CHAIN};
pub use crate::commands::Trust;
use crate::commands::{
    Daemon, derive_checked, index_warning, inert_whitelist, live_daemons, primary_address,
    session_index_warning, shown,
};
use crate::core::error::{ErrorCode, Result, WalletError};
use crate::core::protocol::{self, DaemonExe};
use crate::core::store::{self, WalletFile, WhitelistAnchor};
use crate::core::whitelist::{
    self as core_whitelist, BaseSource, Fingerprint, ListFiles, NEXT_FILE, WHITELIST_FILE,
    Whitelist,
};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// One key of the master as `zwallet list` shows it. [`keys`] builds it without the password;
/// `list --verify` builds it with the address derived from the seed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyRow {
    pub name: String,
    pub index: u32,
    /// The key's address on [`PRIMARY_CHAIN`]. From [`keys`]: while unlocked the daemon's
    /// derivation, checked against the cache; else the cache; `None` when the cache has none;
    /// never verified. From `list --verify`: derived from the seed, verified by the password.
    pub address: Option<String>,
    /// The globally active key (the one the agent uses by default).
    pub active: bool,
}

/// The key list without the password.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keys {
    pub active: String,
    pub rows: Vec<KeyRow>,
    /// What the rows' addresses come from ([`Trust`]): [`Trust::Session`] when this build's key
    /// daemon of this home derived every one; the warning when whatever answered on `d.sock` is
    /// known not to be it, also when its answers matched the cache
    /// ([`protocol::PeerCheck::warning`]).
    pub trust: Trust,
}

/// The key list: wallet.json's keys with their addresses on [`PRIMARY_CHAIN`]. A cached value
/// that is not an address, or a cache the daemon's answer contradicts, fails the whole list with
/// `VALIDATION_ERROR` (no row shows it).
pub async fn keys(home: &Path) -> Result<Keys> {
    let primary = chains::find(PRIMARY_CHAIN)?;
    let file = store::read_wallet_file(home)?;
    let mut rows = Vec::new();
    // Without the password the list is never verified; asking through `Daemon` only decides how
    // a mismatch is worded, and names an impostor (in that error, or as the `warning`).
    let mut daemon = Daemon::new(home);
    for k in &file.keys {
        let address = match daemon.derive(k.index, primary).await? {
            Some(d) => {
                daemon.check_cache(k, primary, &d)?;
                Some(d)
            }
            None => primary_address(k)?,
        };
        rows.push(KeyRow {
            name: k.name.clone(),
            index: k.index,
            address,
            active: k.name == file.active,
        });
    }
    Ok(Keys {
        active: file.active,
        rows,
        trust: daemon.trust(),
    })
}

/// The session a daemon of the home says it serves: what `status` relays from its reply.
#[derive(Clone, Debug, PartialEq)]
pub struct Serving {
    /// Its pid, which the process table lists for the home.
    pub pid: i32,
    /// Its expiry as whatever answers on `d.sock` wrote it: escaped, at most 64 characters (a
    /// real one is plain RFC 3339 and passes as it is); `None` when the reply holds no string.
    pub expires_at: Option<String>,
    /// The whitelist it reports, inert: information only, never a source of an address or of
    /// the word "verified" (03-tui.md §Password check).
    pub whitelist: Value,
}

/// The lock state and the active key, as `zwallet status` reports them.
#[derive(Clone, Debug, PartialEq)]
pub struct StatusView {
    /// wallet.json exists.
    pub configured: bool,
    /// No daemon process for the home: the process table decides, not `d.sock`.
    pub locked: bool,
    pub daemon_pids: Vec<i32>,
    /// Daemons of this user for other homes (`zwallet lock` stops them too).
    pub other_daemon_pids: Vec<i32>,
    pub serving: Option<Serving>,
    /// The globally active key's name, when there is a wallet.
    pub active: Option<String>,
    /// The active key's address on [`PRIMARY_CHAIN`] and what verifies it, when that key is in
    /// the list.
    pub address: Option<(Option<String>, Trust)>,
}

/// The lock state, read again from the process table on every call, and the active key's
/// address with its [`Trust`].
pub async fn status(home: &Path) -> Result<StatusView> {
    let file = match store::read_wallet_file(home) {
        Ok(f) => Some(f),
        Err(e) if e.code == ErrorCode::WalletNotFound => None,
        Err(e) => return Err(e),
    };
    let (daemon_pids, serving) = live_daemons(home).await?;
    let other_daemon_pids = protocol::other_daemon_pids(home)?;
    let locked = daemon_pids.is_empty();
    // `live_daemons` matched the reply's pid against the process table. Whatever answers on
    // `d.sock` writes the rest: the expiry escaped, the whitelist with every string escaped.
    let serving = serving.and_then(|d| {
        Some(Serving {
            pid: d["pid"].as_i64().and_then(|p| i32::try_from(p).ok())?,
            expires_at: d["expiresAt"].as_str().map(shown),
            whitelist: inert_whitelist(&d["whitelist"]),
        })
    });
    let mut address = None;
    if let Some(file) = &file
        && let Ok(key) = file.key(None)
    {
        let primary = chains::find(PRIMARY_CHAIN)?;
        // Derived only by a listed daemon that says it serves this session.
        address = Some(if serving.is_some() {
            let mut daemon = Daemon::new(home);
            match daemon.derive(key.index, primary).await? {
                Some(d) => {
                    daemon.check_cache(key, primary, &d)?;
                    (Some(d), daemon.trust())
                }
                None => (primary_address(key)?, Trust::Locked),
            }
        } else if locked {
            (primary_address(key)?, Trust::Locked)
        } else {
            (primary_address(key)?, Trust::Unverified(None))
        });
    }
    Ok(StatusView {
        configured: file.is_some(),
        locked,
        daemon_pids,
        other_daemon_pids,
        serving,
        active: file.map(|f| f.active),
        address,
    })
}

/// The RPC endpoint for `chain` without a `--rpc` flag: `ZWALLET_RPC_<CHAIN>`, else the chain's
/// default. Shown only through [`chains::redact_url`].
pub fn net(chain: &'static Chain) -> Net {
    crate::commands::net(chain, None)
}

/// What [`unlock`] needs, checked and pinned before the password is asked for
/// ([`prepare_unlock`]).
pub struct UnlockPlan {
    home: PathBuf,
    /// wallet.json as read before the password: only its key list's indices are used, for the
    /// index warning; the key, index and address a session reports are its daemon's.
    file: WalletFile,
    /// Pinned before the password, so a binary swapped in while the human types is never started.
    exe: DaemonExe,
    ttl: Duration,
}

/// `unlock`'s checks before the password, in its order: the socket path, wallet.json (nothing to
/// unlock without it), Linux's ptrace policy (`allow_ptrace_scope_0` accepts scope 0) and whether
/// `/proc` hides the daemons, then the daemon binary pinned.
pub fn prepare_unlock(
    home: &Path,
    ttl: Duration,
    allow_ptrace_scope_0: bool,
) -> Result<UnlockPlan> {
    protocol::socket_path(home)?;
    let file = store::read_wallet_file(home)?;
    protocol::check_ptrace_scope(allow_ptrace_scope_0)?;
    protocol::check_daemons_findable()?;
    let exe = DaemonExe::resolve()?;
    Ok(UnlockPlan {
        home: home.to_path_buf(),
        file,
        exe,
        ttl,
    })
}

/// A session [`unlock`] started.
#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    /// The readiness its daemon child reported on its own channel, `{ locked, expiresAt, active,
    /// index, address, pid, whitelist }`: the address derived from the seed the password opened.
    pub ready: Value,
    /// The whitelist the session signs under, as its daemon verified it, made inert.
    pub whitelist: Value,
    /// The index warning (as `address --verify` gives it) for the key the session reports.
    pub warning: Option<String>,
}

/// Starts a session with `password` for the plan's duration (the staged unlock,
/// [`protocol::start_daemon`]).
pub async fn unlock(plan: &UnlockPlan, password: &str) -> Result<Session> {
    let ready = protocol::start_daemon(&plan.home, &plan.exe, password, plan.ttl).await?;
    Ok(Session {
        whitelist: inert_whitelist(&ready["whitelist"]),
        warning: session_index_warning(&plan.file, &ready),
        ready,
    })
}

/// `key`'s address on `chain` without the password, and what verifies it, as `zwallet address`
/// gives it: the daemon's derivation checked against the cache, or the cache while locked. A
/// mismatch is `VALIDATION_ERROR`.
pub async fn address(home: &Path, key: &str, chain: &Chain) -> Result<(String, Trust)> {
    let file = store::read_wallet_file(home)?;
    let mut daemon = Daemon::new(home);
    let address = daemon.address(file.key(Some(key))?, chain).await?;
    Ok((address, daemon.trust()))
}

/// Makes `name` the globally active key, the one the agent uses by default. Its address on
/// [`PRIMARY_CHAIN`] is checked first ([`address`]), so a cache the daemon contradicts never
/// becomes active; returns that address and what verifies it.
pub async fn set_active(home: &Path, name: &str) -> Result<(String, Trust)> {
    let checked = address(home, name, chains::find(PRIMARY_CHAIN)?).await?;
    let (home, name) = (home.to_path_buf(), name.to_string());
    // The metadata lock may wait for a concurrent CLI writer. Keep the TUI's current-thread
    // runtime free to draw, receive input and stop signing while that lock is held.
    tokio::task::spawn_blocking(move || store::set_active(&home, &name))
        .await
        .map_err(|_| WalletError::validation("active key write did not finish"))??;
    Ok(checked)
}

/// What the password opened ([`open_with_password`]): never the seed, never the whitelist salt.
#[derive(Clone, Debug, PartialEq)]
pub struct Opened {
    /// The named keys, each with its address on [`PRIMARY_CHAIN`] derived from the seed here:
    /// verified by the password.
    pub keys: Vec<KeyRow>,
    /// The globally active key, as the opened wallet.json names it.
    pub active: String,
    /// Indices among the named keys that zwallet never assigns: the key list was edited.
    pub warning: Option<String>,
    /// The whitelist anchor sealed in wallet.json, `None` with no whitelist yet; an anchor that
    /// cannot verify a list is the error `list --verify` gives: a malformed one, or one zwallet
    /// 0.1.0 sealed (no salt).
    pub anchor: Result<Option<WhitelistAnchor>>,
    /// A whitelist salt is sealed with the anchor (an edit derives the whitelist key from it).
    pub salted: bool,
}

/// Opens wallet.json with `password` in this process, as `list --verify` does: the seed decrypted
/// and every cached address checked against it (a modified cache is `VALIDATION_ERROR`, naming
/// `zwallet address --verify`); the addresses of `keys` (every key with `None`) derived from it;
/// the sealed whitelist anchor. Reads no list file. The seed is dropped before it returns.
pub fn open_with_password(home: &Path, password: &str, keys: Option<&[&str]>) -> Result<Opened> {
    let (unsealed, file) = store::unlock(home, password, &chains::derive_addresses)?;
    opened_from(&unsealed, &file, keys)
}

fn opened_from(
    unsealed: &store::Unsealed,
    file: &WalletFile,
    keys: Option<&[&str]>,
) -> Result<Opened> {
    let primary = chains::find(PRIMARY_CHAIN)?;
    let named = match keys {
        Some(names) => names
            .iter()
            .map(|n| file.key(Some(n)))
            .collect::<Result<Vec<_>>>()?,
        None => file.keys.iter().collect(),
    };
    let mut rows = Vec::new();
    for k in &named {
        rows.push(KeyRow {
            name: k.name.clone(),
            index: k.index,
            address: Some(derive_checked(&unsealed.seed, k, primary)?),
            active: k.name == file.active,
        });
    }
    let sealed = unsealed.sealed_whitelist();
    let salted = matches!(&sealed, Ok(Some(s)) if s.salt().is_ok());
    // The anchor as the daemon reads it; for one zwallet 0.1.0 sealed, the cause with what a
    // human can do about it.
    let anchor = sealed.and_then(|sealed| {
        sealed
            .map(|s| {
                s.salt()
                    .map(|_| s.anchor)
                    .map_err(|e| WalletError::validation(core_whitelist::without_salt(&e.message)))
            })
            .transpose()
    });
    Ok(Opened {
        warning: index_warning(file, &named),
        keys: rows,
        active: file.active.clone(),
        anchor,
        salted,
    })
}

/// One address derived locally while the password is held. Never built from metadata or RPC.
#[derive(Clone, Debug)]
pub struct VerifiedAddress {
    pub key: String,
    pub chain: &'static str,
    pub family: &'static str,
    pub address: String,
}

#[derive(Clone, Debug)]
pub struct PasswordCheck {
    pub opened: Opened,
    pub addresses: Vec<VerifiedAddress>,
    pub repairs: Vec<store::CacheRepair>,
}

fn password_checked(
    unsealed: &store::Unsealed,
    file: &WalletFile,
    keys: Option<&[&str]>,
) -> Result<PasswordCheck> {
    let opened = opened_from(unsealed, file, keys)?;
    let mut addresses = Vec::new();
    for row in &opened.keys {
        let key = file.key(Some(&row.name))?;
        for chain in chains::CHAINS {
            addresses.push(VerifiedAddress {
                key: row.name.clone(),
                chain: chain.key,
                family: chain.family.name(),
                address: derive_checked(&unsealed.seed, key, chain)?,
            });
        }
    }
    Ok(PasswordCheck {
        opened,
        addresses,
        repairs: Vec::new(),
    })
}

/// Check selected keys (plus callers' chosen active key) on every chain family in one KDF run.
/// The seed and salt are dropped here and never enter a TUI message.
pub fn check_password(home: &Path, password: &str, keys: Option<&[&str]>) -> Result<PasswordCheck> {
    let (unsealed, file) = store::unlock(home, password, &chains::derive_addresses)?;
    password_checked(&unsealed, &file, keys)
}

pub struct Repaired {
    pub added: Vec<store::CacheAddition>,
    pub checked: PasswordCheck,
    /// Public metadata, used only to preserve the CLI's index-warning behavior.
    pub file: WalletFile,
}

/// The shared address-repair path. It changes only the address cache and retains the sealed
/// policy. Every returned address is derived from the password-opened seed.
pub fn repair(home: &Path, password: &str) -> Result<Repaired> {
    let (unsealed, file, repairs, added) =
        store::unlock_repairing(home, password, &chains::derive_addresses)?;
    let mut checked = password_checked(&unsealed, &file, None)?;
    checked.repairs = repairs;
    Ok(Repaired {
        checked,
        file,
        added,
    })
}

/// The whitelist the password check verified ([`check_list`]).
#[derive(Clone, Debug, PartialEq)]
pub struct CheckedList {
    /// The list the seal names: its entries, its generation and its own policy (fee caps,
    /// message signing), not the wallet's sealed policy object.
    pub list: Whitelist,
    /// The file it was read from: [`WHITELIST_FILE`], or the [`NEXT_FILE`] an interrupted edit
    /// left.
    pub source: &'static str,
    /// The bytes of both list files as read ([`whitelist::check_unchanged`] compares with it).
    pub fingerprint: Fingerprint,
}

/// The list `anchor` names, from both list files read once, verified against the sealed anchor
/// alone (format, generation, signature, digest; no key is derived), with the fingerprint of the
/// bytes read. Otherwise the reason `list --verify` gives: an anchor that cannot verify a list,
/// no whitelist yet, or a list that does not verify.
pub fn check_list(
    home: &Path,
    anchor: &Result<Option<WhitelistAnchor>>,
) -> std::result::Result<CheckedList, String> {
    let files = ListFiles::read(home);
    let anchor = match anchor {
        Err(e) => return Err(e.message.clone()),
        // A list file with no seal is never offered as the first add: that add refuses.
        Ok(None) => {
            return Err(match core_whitelist::unsealed_files_note(&files) {
                Some(note) => format!(
                    "{note}: it is not verified, and every edit refuses. {}",
                    core_whitelist::recovery()
                ),
                None => format!("wallet.json seals no whitelist key yet; {FIRST_ADD}"),
            });
        }
        Ok(Some(a)) => a,
    };
    let base = core_whitelist::pick_base_from(&files, Some(anchor), &WHITELIST_RULES);
    let source = match base.source {
        BaseSource::Current => WHITELIST_FILE,
        BaseSource::PromotedNext => NEXT_FILE,
        // With an anchor, `discarded` always says why; the same one remedy as an edit's refusal.
        BaseSource::Empty => {
            let why = base.discarded.map(|d| d.reason).unwrap_or_default();
            return Err(format!("{why}. {}", core_whitelist::recovery()));
        }
    };
    Ok(CheckedList {
        list: base.list,
        source,
        fingerprint: files.fingerprint(),
    })
}
