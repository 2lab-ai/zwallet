//! The operations the CLI and the TUI share (`.prd/v0-2-tui/flow.md`): plain values in, plain
//! values out. Nothing here reads `/dev/tty` or prints; the CLI command that used the code calls
//! the function here and prints what it returns, and the TUI draws it.

use crate::chains::{self, Chain, Net, PRIMARY_CHAIN};
pub use crate::commands::Trust;
use crate::commands::{Daemon, inert_whitelist, live_daemons, primary_address, shown};
use crate::core::error::{ErrorCode, Result};
use crate::core::{protocol, store};
use serde_json::Value;
use std::path::Path;

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
    /// Whatever answered on `d.sock` and is known not to be this build's key daemon, also when
    /// its answers matched the cache ([`protocol::PeerCheck::warning`]).
    pub warning: Option<String>,
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
    let warning = daemon.trust().warning().map(str::to_string);
    Ok(Keys {
        active: file.active,
        rows,
        warning,
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
