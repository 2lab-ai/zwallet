//! Command handlers. Everything here runs in the one-shot CLI process; the only key access is
//! through the daemon socket (or, for `create`/`backup`, a password typed by the human).

use crate::chains::evm::{self, Evm, abi, rpc as evm_rpc};
use crate::chains::{
    self, Broadcast, CHAINS, Chain, ChainFamily, Family, Net, PRIMARY_CHAIN, PreparedTx,
    TransferRequest, redact_url,
};
use crate::cli::{
    AddressArgs, AddressesArgs, BalanceArgs, CallArgs, Cli, Command, CreateArgs, InvokeArgs,
    KeyArg, Out, SendArgs, SignMessageArgs, TransferArgs, TxArgs, UnlockArgs, VerifyArgs,
    render_human,
};
use crate::core::amount::{format_units, parse_units};
use crate::core::error::{ErrorCode, Result, WalletError};
use crate::core::password::read_password;
use crate::core::protocol::{self, DaemonExe, Peer, PeerCheck, Request, SIGN_TIMEOUT};
use crate::core::seed::{DEFAULT_WORD_COUNT, Seed, VALID_WORD_COUNTS};
use crate::core::store::{self, KeyEntry, MasterOptions, WalletFile};
use crate::core::time::parse_duration;
use crate::daemon;
use crate::qr;
use alloy::hex;
use alloy::primitives::{B256, U256};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::Duration;
use zeroize::Zeroizing;

mod whitelist;

/// How long `--wait` polls for a receipt before reporting `status: "pending"`, unless
/// `--wait-timeout` says otherwise.
const DEFAULT_WAIT_TIMEOUT: Duration = Duration::from_secs(90);

fn home() -> Result<PathBuf> {
    store::resolve_home(std::env::var_os("ZWALLET_HOME"), std::env::home_dir())
}

/// Closes every inherited descriptor before `zwallet daemon` opens its own (see
/// [`protocol::close_inherited_fds`]); `cli::main_entry` calls it for that command alone.
pub fn close_inherited_fds() -> Result<()> {
    protocol::close_inherited_fds()
}

pub async fn run(cli: Cli) -> Result<Out> {
    match cli.command {
        Command::Chains => Ok(chains_list()),
        Command::Create(a) => create(&home()?, a).await,
        Command::List(v) => list(&home()?, v).await,
        Command::Change { name, qr } => change(&home()?, &name, qr.no_qr).await,
        Command::Address(a) => address(&home()?, a).await,
        Command::Addresses(a) => addresses(&home()?, a).await,
        Command::Balance(a) => balance(&home()?, a).await,
        Command::Call(a) => call(&home()?, a).await,
        Command::Invoke(a) => invoke(&home()?, a).await,
        Command::Transfer(a) => transfer(&home()?, a).await,
        Command::SignMessage(a) => sign_message(&home()?, a).await,
        Command::Tx(a) => tx(a).await,
        Command::Status(qr) => status(&home()?, qr.no_qr).await,
        Command::Backup => backup(&home()?),
        Command::Unlock(a) => unlock(&home()?, a).await,
        Command::Lock => lock(),
        Command::Whitelist(w) => whitelist::run(&home()?, w).await,
        Command::Daemon { home } => {
            daemon::run(home).await?;
            Ok(Out::silent())
        }
    }
}

fn rpc_url(chain: &Chain, flag: Option<&str>) -> String {
    let env = std::env::var(chain.rpc_env_var()).ok();
    chain.resolve_rpc(flag, env.as_deref())
}

fn net(chain: &'static Chain, flag: Option<&str>) -> Net {
    Net {
        chain,
        rpc_url: rpc_url(chain, flag),
    }
}

fn chains_list() -> Out {
    let rows: Vec<Value> = CHAINS
        .iter()
        .map(|c| {
            json!({
                "key": c.key,
                "name": c.name,
                "family": c.family.name(),
                "symbol": c.symbol,
                "chainId": c.chain_id_json(),
                "rpc": redact_url(&rpc_url(c, None)),
            })
        })
        .collect();
    let text = CHAINS
        .iter()
        .map(|c| {
            format!(
                "{:<10} {:<9} {:<5} {}",
                c.key,
                c.chain_id_json(),
                c.symbol,
                redact_url(&rpc_url(c, None))
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    Out::with_text(Value::Array(rows), text)
}

/// Is a daemon holding the key right now?
async fn unlocked(home: &Path) -> Result<bool> {
    Ok(protocol::status(home)
        .await?
        .is_some_and(|s| s["locked"] == json!(false)))
}

/// Fails fast with `WALLET_LOCKED` before doing any RPC work for a signing command.
async fn require_unlocked(home: &Path) -> Result<()> {
    if unlocked(home).await? {
        Ok(())
    } else {
        Err(WalletError::locked())
    }
}

/// A key's address on `chain`: from the cache, or derived by the daemon for chains added after
/// the key was created.
async fn key_address(home: &Path, key: &KeyEntry, chain: &Chain) -> Result<String> {
    if let Some(a) = key.addresses.get(chain.key) {
        return cached_address(key, chain.key, a);
    }
    let v = protocol::send(
        home,
        &Request::Address {
            chain: chain.key.into(),
            index: key.index,
        },
        SIGN_TIMEOUT,
    )
    .await?
    .ok_or_else(|| {
        WalletError::new(
            ErrorCode::WalletLocked,
            format!(
                "key '{}' has no cached {} address yet; unlock once (`zwallet unlock 24h`) to \
                 derive it",
                key.name, chain.key
            ),
        )
    })?;
    reply_address(&v, chain)
}

/// Derives every chain's address for key `index` through the unlocked daemon.
async fn derive_via_daemon(home: &Path, index: u32) -> Result<BTreeMap<String, String>> {
    let mut per_family: BTreeMap<&str, String> = BTreeMap::new();
    let mut out = BTreeMap::new();
    for chain in CHAINS {
        let family = chain.family.name();
        if !per_family.contains_key(family) {
            let v = protocol::send_unlocked(
                home,
                &Request::Address {
                    chain: chain.key.into(),
                    index,
                },
                SIGN_TIMEOUT,
            )
            .await?;
            per_family.insert(family, reply_address(&v, chain)?);
        }
        out.insert(chain.key.to_string(), per_family[family].clone());
    }
    Ok(out)
}

/// The address `key` shows by default: the cached one on [`PRIMARY_CHAIN`], else on any chain;
/// `None` when the cache has none. A cached value that is not an address is an error.
fn primary_address(key: &KeyEntry) -> Result<Option<String>> {
    key.addresses
        .get_key_value(PRIMARY_CHAIN)
        .or_else(|| key.addresses.iter().next())
        .map(|(chain, a)| cached_address(key, chain, a))
        .transpose()
}

/// The most characters of a value zwallet does not control (wallet.json's cache, a reply on the
/// daemon socket) that a message shows, escaped (see [`crate::core::text::shown`]).
const VALUE_SHOWN: usize = 64;

/// `value` escaped for a message (see [`VALUE_SHOWN`]).
fn shown(value: &str) -> String {
    crate::core::text::shown(value, VALUE_SHOWN)
}

/// Whether `value` has the shape of an address on `chain`: for EVM, `0x` and 40 hex digits (any
/// case). Says nothing about whose address it is.
fn address_shaped(chain: &Chain, value: &str) -> bool {
    match &chain.family {
        Family::Evm(_) => {
            value.len() == 42
                && value.starts_with("0x")
                && value[2..].bytes().all(|b| b.is_ascii_hexdigit())
        }
    }
}

/// `cached`, wallet.json's cached address of `key` on chain key `chain`, if it has an address's
/// shape; otherwise `VALIDATION_ERROR` showing the value escaped. The cache is plain metadata
/// anyone of this user can edit: a value that is no address (a terminal escape, a fake line)
/// must never be shown or compared as one.
fn cached_address(key: &KeyEntry, chain: &str, cached: &str) -> Result<String> {
    if chains::find(chain).is_ok_and(|c| address_shaped(c, cached)) {
        return Ok(cached.to_string());
    }
    Err(WalletError::validation(format!(
        "wallet.json's cached {} address for key '{}' is {}, which is not an address; the cache \
         was modified, do not send funds to it",
        shown(chain),
        key.name,
        shown(cached)
    )))
}

/// The `address` of a daemon's reply for `chain`, if it has an address's shape; otherwise
/// `VALIDATION_ERROR` showing the value escaped. Whatever answers on the socket chooses this
/// text, so a value that is no address is never shown, compared, cached or verified as one.
fn reply_address(reply: &Value, chain: &Chain) -> Result<String> {
    match reply["address"].as_str() {
        Some(a) if address_shaped(chain, a) => Ok(a.to_string()),
        Some(a) => Err(WalletError::validation(format!(
            "the process answering on the key daemon socket returned {} as a {} address, which \
             is not an address; not using it",
            shown(a),
            chain.key
        ))),
        None => Err(WalletError::validation(
            "the process answering on the key daemon socket returned no address",
        )),
    }
}

/// The key daemon's own derivation of key `index` on `chain`, or `None` while the wallet is
/// locked (no daemon, or one that just locked itself). Whoever answers is not checked: for the
/// cross-check of `list` only (see [`Trust`]).
async fn daemon_derive(home: &Path, index: u32, chain: &Chain) -> Result<Option<String>> {
    let reply = match protocol::send(
        home,
        &Request::Address {
            chain: chain.key.into(),
            index,
        },
        SIGN_TIMEOUT,
    )
    .await
    {
        Ok(Some(v)) => v,
        Ok(None) => return Ok(None),
        Err(e) if e.code == ErrorCode::WalletLocked => return Ok(None),
        Err(e) => return Err(e),
    };
    reply_address(&reply, chain).map(Some)
}

/// wallet.json's cached address for `key` on `chain` must be the one `by` derived from the seed;
/// the cache is plain metadata that anyone running as this user can edit.
fn check_cache(key: &KeyEntry, chain: &str, derived: &str, by: &str) -> Result<()> {
    match cache_mismatch(key, chain, derived)? {
        Some(cached) => Err(WalletError::validation(format!(
            "wallet.json's cached {chain} address for key '{}' ({cached}) does not match the \
             address {by} derives from the seed ({}); the cache was modified, do not send funds \
             to it",
            key.name,
            shown(derived)
        ))),
        None => Ok(()),
    }
}

/// The cached address of `key` on `chain` when it differs from `derived`, `None` when it agrees
/// or there is none. A cached value that is not an address is an error ([`cached_address`]).
fn cache_mismatch(key: &KeyEntry, chain: &str, derived: &str) -> Result<Option<String>> {
    match key.addresses.get(chain) {
        Some(cached) => {
            let cached = cached_address(key, chain, cached)?;
            Ok((!cached.eq_ignore_ascii_case(derived)).then_some(cached))
        }
        None => Ok(None),
    }
}

const DAEMON: &str = "the key daemon";

/// How far an address shown without the password can be trusted. There are three levels:
/// - `verified: true, verifiedBy: "password"`: derived from the seed after the human's password,
///   in this process or by the daemon child `unlock` just started.
/// - `verified: true, verifiedBy: "session"` ([`Trust::Session`]): while unlocked, derived by this
///   zwallet build's own key daemon for this home ([`protocol::check_peer`]), and equal to the
///   cache. That daemon decrypted this home's wallet.json with the password typed at `unlock`.
/// - `verified: false`: the cache in wallet.json, which any process of this user can edit, or a
///   derivation by something else on `d.sock` (with a `warning` naming it).
///
/// What a session cannot rule out: a process of this user that swapped in its own wallet.json
/// and unlocked it with its own password through this very binary, or injected code into it. Only
/// the address the human saved from their first unlock tells that apart: if it ever changes, stop.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Trust {
    Session,
    /// The wallet is locked: the cache.
    Locked,
    /// Unlocked, but not verified by the session: the warning, when what answered is known not
    /// to be this build's key daemon of this home.
    Unverified(Option<String>),
}

impl Trust {
    fn verified(&self) -> bool {
        *self == Self::Session
    }

    /// Sets `verified`, `verifiedBy` and `warning` on the object `out`.
    fn mark(&self, out: &mut Value) {
        out["verified"] = json!(self.verified());
        if self.verified() {
            out["verifiedBy"] = json!("session");
        }
        if let Self::Unverified(Some(w)) = self {
            out["warning"] = json!(w);
        }
    }

    /// The one line under an address shown unverified.
    fn note(&self) -> Option<&'static str> {
        match self {
            Self::Session => None,
            Self::Locked => Some(LOCKED_NOTE),
            Self::Unverified(_) => Some(UNVERIFIED_NOTE),
        }
    }

    fn warning(&self) -> Option<&str> {
        match self {
            Self::Unverified(Some(w)) => Some(w),
            _ => None,
        }
    }
}

/// The key daemon as one command asks it for addresses: each derivation is checked against the
/// cache, and each process that gave one is checked once ([`protocol::check_peer`]); then
/// [`Daemon::trust`] says what that verifies.
struct Daemon<'a> {
    home: &'a Path,
    peers: Vec<(Peer, PeerCheck)>,
    /// Some derivation came back.
    answered: bool,
    /// Some address came from the cache instead (the wallet was or became locked).
    cached: bool,
}

impl<'a> Daemon<'a> {
    fn new(home: &'a Path) -> Self {
        Self {
            home,
            peers: Vec::new(),
            answered: false,
            cached: false,
        }
    }

    /// The daemon's derivation of key `index` on `chain`, or `None` while the wallet is locked.
    async fn derive(&mut self, index: u32, chain: &Chain) -> Result<Option<String>> {
        let request = Request::Address {
            chain: chain.key.into(),
            index,
        };
        let answer = match protocol::send_answered(self.home, &request, SIGN_TIMEOUT).await {
            Ok(Some(a)) => a,
            Ok(None) => return Ok(None),
            Err(e) if e.code == ErrorCode::WalletLocked => return Ok(None),
            Err(e) => return Err(e),
        };
        if !self.peers.iter().any(|(p, _)| *p == answer.peer) {
            let check = protocol::check_peer(self.home, &answer.peer)?;
            self.peers.push((answer.peer, check));
        }
        // An answer that is no address is never used, whoever gave it; the error names an
        // impostor when one answered.
        let address = reply_address(&answer.value, chain).map_err(|e| self.naming_impostor(e))?;
        self.answered = true;
        Ok(Some(address))
    }

    /// `e` with the warning naming an impostor on `d.sock` (message and `warning` detail), when
    /// one answered this command.
    fn naming_impostor(&self, e: WalletError) -> WalletError {
        match self.trust().warning() {
            Some(w) => {
                let message = format!(
                    "{}. That process is not this build's key daemon: {w}",
                    e.message
                );
                WalletError { message, ..e }.with_detail("warning", w)
            }
            None => e,
        }
    }

    /// `key`'s address on `chain` from `derived` (this command's derivation for the chain's
    /// family) checked against the cache, or without one the cache (the daemon's derivation for
    /// a chain added after the key).
    async fn checked(
        &mut self,
        key: &KeyEntry,
        chain: &Chain,
        derived: Option<&String>,
    ) -> Result<String> {
        if let Some(d) = derived {
            self.check_cache(key, chain, d)?;
            return Ok(d.clone());
        }
        self.cached = true;
        key_address(self.home, key, chain).await
    }

    /// [`check_cache`] of `derived` (what answered on `d.sock` derived) for `key` on `chain`. A
    /// mismatch aborts the command with `VALIDATION_ERROR`. When what answered is known not to be
    /// this build's key daemon of the home, the message says so instead of blaming the cache: that
    /// process's answer is no derivation from the seed, so neither address is to be trusted. It
    /// names the process (its warning, also the `warning` detail).
    fn check_cache(&self, key: &KeyEntry, chain: &Chain, derived: &str) -> Result<()> {
        let Some(w) = self.trust().warning().map(str::to_string) else {
            return check_cache(key, chain.key, derived, DAEMON);
        };
        match cache_mismatch(key, chain.key, derived).map_err(|e| self.naming_impostor(e))? {
            None => Ok(()),
            Some(cached) => {
                let reason = w
                    .strip_suffix(&format!("; {}", protocol::RELOCK))
                    .unwrap_or(&w);
                Err(WalletError::validation(format!(
                    "the process answering on the key daemon socket reported {} as key '{}''s {} \
                     address, which contradicts the {cached} cached in wallet.json. That process \
                     is not this build's key daemon: {reason}. Do not send funds to either \
                     address; run `zwallet lock`, then unlock again",
                    shown(derived),
                    key.name,
                    chain.key
                ))
                .with_detail("warning", w))
            }
        }
    }

    /// `key`'s address on `chain`: derived and checked against the cache, or the cache while
    /// locked. A mismatch is `VALIDATION_ERROR`.
    async fn address(&mut self, key: &KeyEntry, chain: &Chain) -> Result<String> {
        let derived = self.derive(key.index, chain).await?;
        self.checked(key, chain, derived.as_ref()).await
    }

    /// The cache's address for `key` on `chain`, used when the daemon did not derive one (the
    /// wallet is locked, or the chain was added after the key). Marks the result as cached, so
    /// [`Trust`] does not read it as session-verified.
    async fn cache_only(&mut self, key: &KeyEntry, chain: &Chain) -> Result<String> {
        self.cached = true;
        key_address(self.home, key, chain).await
    }

    /// What the addresses this command took verify: [`Trust::Session`] only when every one was
    /// derived, and every process that answered is this build's key daemon of this home.
    fn trust(&self) -> Trust {
        let other = self.peers.iter().find_map(|(_, c)| match c {
            PeerCheck::Other(w) => Some(w.clone()),
            _ => None,
        });
        if other.is_some() {
            Trust::Unverified(other)
        } else if !self.answered {
            Trust::Locked
        } else if self.cached || self.peers.iter().any(|(_, c)| *c != PeerCheck::ThisBuild) {
            Trust::Unverified(None)
        } else {
            Trust::Session
        }
    }
}

/// `verifiedBy` of an address derived from the seed after the human's password ([`Trust`]).
const BY_PASSWORD: &str = "password";

/// The note on an address shown unverified while the wallet is unlocked (or with no session to
/// verify it, as `list` and an agent's `create`).
const UNVERIFIED_NOTE: &str = "not verified: run `zwallet address --verify` before funding";

/// The note on an address shown while the wallet is locked.
const LOCKED_NOTE: &str = "not verified: the wallet is locked; `zwallet unlock <dur>` or `zwallet \
                           address --verify` checks it";

/// Human text for a result object, with `note` under it.
fn with_note(value: Value, note: Option<&str>) -> Out {
    let mut text = render_human(&value);
    if let Some(note) = note {
        text.push('\n');
        text.push_str(note);
    }
    Out::with_text(value, text)
}

/// The line under `scan to send to <address>`: the code carries no chain. Two lines, so both fit
/// an 80-column terminal.
const EVERY_EVM_NETWORK: &str = "(the same address on every EVM network in `zwallet chains`)";

/// `out` with `address` as a QR code at the end of its human text, for a phone wallet to scan,
/// under the lines `scan to send to <address>` and [`EVERY_EVM_NETWORK`]. Only for an address
/// reported `verified: true` (by the password or by the session), only with stdout on a
/// terminal, and not with `--no-qr`: agents read zwallet through pipes, and `--json` prints no
/// human text.
fn with_qr(out: Out, verified: bool, address: Option<&str>, no_qr: bool) -> Out {
    let Some(address) = address.filter(|_| verified && qr_shown(no_qr)) else {
        return out;
    };
    // No colours when NO_COLOR is set and not empty (no-color.org).
    let color = std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty());
    match qr::render(address, color) {
        Some(code) => with_line(
            out,
            &format!("scan to send to {address}\n{EVERY_EVM_NETWORK}\n{code}"),
        ),
        None => out,
    }
}

/// Whether human output may end with a QR code: stdout on a terminal, and no `--no-qr`.
fn qr_shown(no_qr: bool) -> bool {
    !no_qr && std::io::stdout().is_terminal()
}

/// `out` with `line` at the end of its human text.
fn with_line(out: Out, line: &str) -> Out {
    let text = out.text.unwrap_or_else(|| render_human(&out.value));
    Out::with_text(out.value, format!("{text}\n{line}"))
}

/// What the `create` that makes the master seed shows where the QR code would be.
const QR_AFTER_BACKUP: &str =
    "QR after backup: run `zwallet backup`, then `zwallet address --verify` (or unlock) to show it";

/// Decrypts wallet.json in this process with the human's password and checks every key's cached
/// addresses against the seed (a mismatch is `VALIDATION_ERROR`). Addresses derived from this
/// seed are the ones reported `verified: true`: no other process or file can make them.
fn open_with_password(home: &Path, password_stdin: bool) -> Result<(Seed, WalletFile)> {
    store::read_wallet_file(home)?;
    let pw = read_password(password_stdin, "Wallet password: ", false)?;
    let (unsealed, file) = store::unlock(home, &pw, &chains::derive_addresses)?;
    Ok((unsealed.seed, file))
}

/// `key`'s address on `chain` derived from `seed` here, checked against the cache.
fn derive_checked(seed: &Seed, key: &KeyEntry, chain: &Chain) -> Result<String> {
    let derived = chain.family.keys().derive_address(seed, key.index)?;
    check_cache(key, chain.key, &derived, "this process")?;
    Ok(derived)
}

async fn start_session(
    home: &Path,
    exe: &DaemonExe,
    password: &str,
    ttl: Duration,
) -> Result<Value> {
    protocol::start_daemon(home, exe, password, ttl).await
}

async fn create(home: &Path, a: CreateArgs) -> Result<Out> {
    store::validate_key_name(&a.name)?;
    let ttl = a.unlock.as_deref().map(parse_duration).transpose()?;
    if ttl.is_some() {
        protocol::socket_path(home)?;
        // Before the prompt and before anything is written: refused here, nothing was created.
        protocol::check_ptrace_scope(a.allow_ptrace_scope_0)?;
        protocol::check_daemons_findable()?;
    }
    // Pinned before any password prompt (see `DaemonExe`).
    let exe = ttl.map(|_| DaemonExe::resolve()).transpose()?;
    let (file, addresses, master, password, verified) = match store::read_wallet_file(home) {
        Err(e) if e.code == ErrorCode::WalletNotFound => {
            let words = a.words.unwrap_or(DEFAULT_WORD_COUNT);
            if !VALID_WORD_COUNTS.contains(&words) {
                return Err(WalletError::validation(format!(
                    "--words must be one of {VALID_WORD_COUNTS:?}"
                )));
            }
            let pw = read_password(a.password.password_stdin, "New wallet password: ", true)?;
            store::validate_new_password(&pw)?;
            let opts = MasterOptions {
                words,
                ..MasterOptions::default()
            };
            let file = store::create_master(home, &pw, &a.name, opts, &chains::derive_addresses)?;
            // Derived from the seed create_master generated in memory (and verified on disk
            // before publishing), not read back from the cache.
            let addresses = file.key(Some(&a.name))?.addresses.clone();
            (file, addresses, "created", Some(pw), true)
        }
        Err(e) => return Err(e),
        Ok(_) => {
            if a.words.is_some() {
                return Err(WalletError::validation(
                    "--words only applies when creating the master seed",
                ));
            }
            let (file, addresses, pw) = add_subkey(home, &a, ttl.is_none()).await?;
            // Derived here from the seed the password opened, or else by whatever daemon
            // answered.
            let verified = pw.is_some();
            (file, addresses, "existing", pw, verified)
        }
    };
    let key = file.key(Some(&a.name))?;
    let address = addresses
        .get(PRIMARY_CHAIN)
        .or_else(|| addresses.values().next())
        .cloned();
    let mut out = json!({
        "name": key.name,
        "index": key.index,
        "address": address,
        "active": file.active,
        "master": master,
        "home": home.display().to_string(),
        "chains": CHAINS.iter().map(|c| c.key).collect::<Vec<_>>(),
        "verified": verified,
    });
    if verified {
        out["verifiedBy"] = json!(BY_PASSWORD);
    }
    if let (Some(ttl), Some(exe), Some(pw)) = (ttl, &exe, password.as_deref()) {
        let session = start_session(home, exe, pw, ttl).await?;
        // The daemon derived the active key's address from the seed on its own; it must be the
        // key this call just created.
        let same = session["active"] == json!(a.name)
            && session["address"]
                .as_str()
                .zip(address.as_deref())
                .is_some_and(|(d, c)| d.eq_ignore_ascii_case(c));
        if !same {
            let mismatch = format!(
                "after create the key daemon reports active key {} at {}, not '{}' at {}; \
                 wallet.json changed meanwhile",
                session["active"],
                session["address"],
                a.name,
                address.as_deref().unwrap_or("-")
            );
            return Err(relocked(mismatch, protocol::stop_all_daemons()));
        }
        out["locked"] = json!(false);
        out["expiresAt"] = session["expiresAt"].clone();
        out["daemonPid"] = session["pid"].clone();
        // The whitelist this session signs under, as `unlock` shows it.
        out["whitelist"] = session["whitelist"].clone();
    }
    if master == "created" && std::io::stderr().is_terminal() {
        eprintln!(
            "Write down your recovery phrase now: run `zwallet backup` before funding this wallet."
        );
    }
    // Unverified only when derived through the daemon, so while unlocked.
    let out = with_note(out, (!verified).then_some(UNVERIFIED_NOTE));
    if master == "created" {
        // The recovery phrase is not backed up yet, and funds sent before it is are at risk:
        // no code to scan here, only how to get one.
        return Ok(if qr_shown(a.qr.no_qr) {
            with_line(out, QR_AFTER_BACKUP)
        } else {
            out
        });
    }
    Ok(with_qr(out, verified, address.as_deref(), a.qr.no_qr))
}

/// The error for a `create --unlock` whose new session shows another key: says whether locking
/// again worked, never claims it when it did not.
fn relocked(mismatch: String, relock: Result<Vec<i32>>) -> WalletError {
    match relock {
        Ok(_) => WalletError::validation(format!("{mismatch}, so the wallet was locked again")),
        Err(e) => WalletError::validation(format!(
            "{mismatch}, and locking the wallet again failed: {}. The wallet may still be \
             unlocked: run `zwallet lock`",
            e.message
        )),
    }
}

/// Adds key `a.name` to an existing master. With a terminal or `--password-stdin` (a human), or
/// without `may_use_daemon`, it is derived here from the seed decrypted with a one-off password,
/// and the password is returned. Otherwise (an agent, while unlocked) it is derived through
/// whatever key daemon answers, and nothing proves which seed that is. The password and scrypt
/// run before the writer lock is taken, so a slow human never blocks other writers; the index is
/// planned, derived and published under the lock, so concurrent creates get distinct indices.
async fn add_subkey(
    home: &Path,
    a: &CreateArgs,
    may_use_daemon: bool,
) -> Result<(
    WalletFile,
    BTreeMap<String, String>,
    Option<Zeroizing<String>>,
)> {
    let human = a.password.password_stdin || std::io::stdin().is_terminal();
    let local = if may_use_daemon && !human && unlocked(home).await? {
        None
    } else {
        let pw = read_password(a.password.password_stdin, "Wallet password: ", false)?;
        let (unsealed, file) = store::unlock(home, &pw, &chains::derive_addresses)?;
        Some((unsealed.seed, file.crypto.ciphertext, pw))
    };
    let lock = store::MetaLock::acquire(home)?;
    let file = lock.read()?;
    if let Some((_, sealed, _)) = &local
        && file.crypto.ciphertext != *sealed
    {
        return Err(WalletError::validation(
            "wallet.json's master changed while the password was checked; retry",
        ));
    }
    let index = store::plan_new_key(&file, &a.name)?;
    let addresses = match &local {
        None => derive_via_daemon(home, index).await?,
        Some((seed, ..)) => chains::derive_addresses(seed, index)?,
    };
    let file = lock.add_key(&a.name, index, addresses.clone())?;
    Ok((file, addresses, local.map(|(.., pw)| pw)))
}

async fn list(home: &Path, v: VerifyArgs) -> Result<Out> {
    let primary = chains::find(PRIMARY_CHAIN)?;
    let (file, opened) = if v.verify {
        let (seed, file) = open_with_password(home, v.password_stdin)?;
        (file, Some(seed))
    } else {
        (store::read_wallet_file(home)?, None)
    };
    let mut keys = Vec::new();
    for k in &file.keys {
        let address = match &opened {
            Some(seed) => Some(derive_checked(seed, k, primary)?),
            None => match daemon_derive(home, k.index, primary).await? {
                Some(d) => {
                    check_cache(k, primary.key, &d, DAEMON)?;
                    Some(d)
                }
                None => primary_address(k)?,
            },
        };
        keys.push(json!({
            "name": k.name,
            "index": k.index,
            "address": address,
            "active": k.name == file.active,
        }));
    }
    let verified = opened.is_some();
    let warning = if verified {
        index_warning(&file, &file.keys.iter().collect::<Vec<_>>())
    } else {
        None
    };
    let mut text = keys
        .iter()
        .map(|k| {
            format!(
                "{} {:<16} {:>3}  {}",
                if k["active"] == json!(true) { "*" } else { " " },
                k["name"].as_str().unwrap_or(""),
                k["index"],
                k["address"].as_str().unwrap_or("-")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    if !verified {
        text.push('\n');
        text.push_str(UNVERIFIED_NOTE);
    }
    let mut out = json!({ "active": file.active, "keys": keys, "verified": verified });
    if verified {
        out["verifiedBy"] = json!(BY_PASSWORD);
    }
    if let Some(w) = warning {
        text.push_str(&format!("\nwarning: {w}"));
        out["warning"] = json!(w);
    }
    Ok(Out::with_text(out, text))
}

/// For `--verify`: derivation indices in wallet.json's key list that zwallet never writes, so
/// the list was edited: one of `shown` at or past `nextIndex`, or an index below the highest of
/// `shown` that no key uses. The address is still this seed's, but where it sits matters for a
/// wallet restored from the recovery phrase, which lists accounts in index order.
fn index_warning(file: &WalletFile, shown: &[&KeyEntry]) -> Option<String> {
    let mut notes: Vec<String> = shown
        .iter()
        .filter(|k| k.index >= file.next_index)
        .map(|k| {
            format!(
                "key '{}' has index {}, at or past nextIndex {}",
                k.name, k.index, file.next_index
            )
        })
        .collect();
    let used: BTreeSet<u32> = file.keys.iter().map(|k| k.index).collect();
    if let Some(top) = shown.iter().map(|k| k.index).max() {
        let below = used.range(..top).count() as u64;
        if u64::from(top) > below {
            // The lowest unused index: the first place the sorted indices skip one.
            let first = used
                .iter()
                .zip(0u32..)
                .find(|(used, expected)| *used != expected)
                .map_or(used.len() as u32, |(_, expected)| expected);
            notes.push(format!(
                "{} index(es) below {top} belong to no key (the first is {first})",
                u64::from(top) - below
            ));
        }
    }
    (!notes.is_empty()).then(|| {
        format!(
            "{}. zwallet never assigns indices like this, so wallet.json's key list was edited. \
             The address is still derived from this seed, but a wallet restored from the \
             recovery phrase lists accounts in index order and may not show it",
            notes.join("; ")
        )
    })
}

async fn change(home: &Path, name: &str, no_qr: bool) -> Result<Out> {
    let primary = chains::find(PRIMARY_CHAIN)?;
    // Check the target before switching to it, so a cache the daemon contradicts never becomes
    // active.
    let file = store::read_wallet_file(home)?;
    let mut daemon = Daemon::new(home);
    let address = daemon.address(file.key(Some(name))?, primary).await?;
    let file = store::set_active(home, name)?;
    let trust = daemon.trust();
    let mut out = json!({ "active": file.active, "address": address });
    trust.mark(&mut out);
    Ok(with_qr(
        with_note(out, trust.note()),
        trust.verified(),
        Some(&address),
        no_qr,
    ))
}

async fn address(home: &Path, a: AddressArgs) -> Result<Out> {
    let chain = chains::find(&a.chain)?;
    if a.verify.verify {
        let (seed, file) = open_with_password(home, a.verify.password_stdin)?;
        let key = file.key(a.key.key.as_deref())?;
        let address = derive_checked(&seed, key, chain)?;
        let mut out = json!({
            "chain": chain.key,
            "key": key.name,
            "index": key.index,
            "address": address,
            "verified": true,
            "verifiedBy": BY_PASSWORD,
        });
        if let Some(w) = index_warning(&file, &[key]) {
            out["warning"] = json!(w);
        }
        return Ok(with_qr(
            with_note(out, None),
            true,
            Some(&address),
            a.qr.no_qr,
        ));
    }
    let file = store::read_wallet_file(home)?;
    let key = file.key(a.key.key.as_deref())?;
    let mut daemon = Daemon::new(home);
    let address = daemon.address(key, chain).await?;
    let trust = daemon.trust();
    let mut out = json!({ "chain": chain.key, "key": key.name, "address": address });
    trust.mark(&mut out);
    Ok(with_qr(
        with_note(out, trust.note()),
        trust.verified(),
        Some(&address),
        a.qr.no_qr,
    ))
}

async fn addresses(home: &Path, a: AddressesArgs) -> Result<Out> {
    let file = store::read_wallet_file(home)?;
    let key = file.key(a.key.key.as_deref())?;
    // One daemon derivation per family (EVM chains share an address), checked per chain.
    let mut daemon = Daemon::new(home);
    let mut derived: BTreeMap<&str, Option<String>> = BTreeMap::new();
    let mut rows = Vec::new();
    let mut primary = None;
    for chain in CHAINS {
        let family = chain.family.name();
        if !derived.contains_key(family) {
            derived.insert(family, daemon.derive(key.index, chain).await?);
        }
        let address = daemon.checked(key, chain, derived[family].as_ref()).await?;
        if chain.key == PRIMARY_CHAIN {
            primary = Some(address.clone());
        }
        rows.push(json!({ "chainId": chain.key, "address": address }));
    }
    let trust = daemon.trust();
    let mut text = rows
        .iter()
        .map(|r| {
            format!(
                "{:<10} {}",
                r["chainId"].as_str().unwrap_or(""),
                r["address"].as_str().unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    for line in [
        trust.warning().map(|w| format!("warning: {w}")),
        trust.note().map(str::to_string),
    ]
    .into_iter()
    .flatten()
    {
        text.push('\n');
        text.push_str(&line);
    }
    let mut out = json!({ "key": key.name, "addresses": rows });
    trust.mark(&mut out);
    // One code: the address is the same on every EVM chain.
    Ok(with_qr(
        Out::with_text(out, text),
        trust.verified(),
        primary.as_deref(),
        a.qr.no_qr,
    ))
}

async fn balance(home: &Path, a: BalanceArgs) -> Result<Out> {
    let chain = match (&a.chain, &a.token, &a.rpc) {
        (Some(chain), _, _) => chains::find(chain)?,
        (None, None, None) => return every_balance(home, &a.key, a.qr.no_qr).await,
        // clap refuses these already (`requires`).
        _ => {
            return Err(WalletError::validation(
                "--token and --rpc need --chain: a token contract and an RPC URL belong to one \
                 chain",
            ));
        }
    };
    let file = store::read_wallet_file(home)?;
    let key = file.key(a.key.key.as_deref())?;
    // While unlocked, the daemon's derivation, checked against the cache before any RPC.
    let mut daemon = Daemon::new(home);
    let owner = daemon.address(key, chain).await?;
    let trust = daemon.trust();
    let net = net(chain, a.rpc.as_deref());
    let bal = match &chain.family {
        Family::Evm(_) => match &a.token {
            Some(token) => Evm.token_balance(&net, &owner, token).await?,
            None => Evm.native_balance(&net, &owner).await?,
        },
    };
    let mut out = json!({
        "chain": chain.key,
        "key": key.name,
        "address": owner,
        "symbol": bal.symbol,
        "decimals": bal.decimals,
        "balance": format_units(bal.raw, bal.decimals),
        "raw": bal.raw.to_string(),
    });
    trust.mark(&mut out);
    if let Some(token) = &a.token {
        out["token"] = json!(Evm.normalize_address(token)?);
    }
    Ok(with_qr(
        Out::json(out),
        trust.verified(),
        Some(&owner),
        a.qr.no_qr,
    ))
}

/// How long `balance` without `--chain` waits for one network: its chain id check and every
/// balance read on it, together.
const NETWORK_TIMEOUT: Duration = Duration::from_secs(8);

/// The most characters of an error a network's line shows in `balance` without `--chain`
/// (`--json` has the whole message).
const BRIEF_ERROR_CHARS: usize = 72;

/// One network's balances for `balance` without `--chain`: the native coin and every registry
/// token there.
type NetworkBalances = Result<(chains::Balance, Vec<chains::TokenBalance>)>;

/// `balance` without `--chain`: the native coin and every registry token on every chain,
/// mainnets first. The networks are read at once, each within [`NETWORK_TIMEOUT`], through
/// `ZWALLET_RPC_<CHAIN>` or the default RPC. A network whose endpoint, chain id or native balance
/// fails is a row with `error` and `errorCode` instead of `native` and `tokens`; a token whose
/// read fails is a token row with them instead of `balance` and `raw`. Only when no network
/// answers is it an error (their shared code, else `RPC_ERROR`). While unlocked the address is
/// the daemon's derivation (once per chain family), checked against the cache before any RPC.
async fn every_balance(home: &Path, k: &KeyArg, no_qr: bool) -> Result<Out> {
    let file = store::read_wallet_file(home)?;
    let key = file.key(k.key.as_deref())?;
    let mut order: Vec<&'static Chain> = CHAINS.iter().collect();
    order.sort_by_key(|c| c.is_testnet());
    // The address on each chain first, as `balance --chain` finds it: the daemon's derivation,
    // or while locked the cache (the daemon's for a chain added after the key). A cache the
    // daemon contradicts aborts the whole command here with `VALIDATION_ERROR` (a tamper signal,
    // via `check_cache`'s `?`), before any RPC; a daemon that cannot be reached for a chain
    // family becomes that family's chains' error rows (fail closed per chain, with `read_network`
    // below). The two are told apart by their source, not by matching the error's text.
    let mut daemon = Daemon::new(home);
    let mut derived: BTreeMap<&str, Result<Option<String>>> = BTreeMap::new();
    let mut owners = Vec::with_capacity(order.len());
    for chain in &order {
        let family = chain.family.name();
        if !derived.contains_key(family) {
            derived.insert(family, daemon.derive(key.index, chain).await);
        }
        owners.push(match &derived[family] {
            // Derived: the cache must agree, or the command fails now (naming an impostor on
            // `d.sock`, if one answered).
            Ok(Some(d)) => {
                daemon.check_cache(key, chain, d)?;
                Ok(d.clone())
            }
            // Locked (or a chain added after the key): the cache, a per-chain failure if absent.
            Ok(None) => daemon.cache_only(key, chain).await,
            // The daemon could not be reached for this family: a per-chain error row.
            Err(e) => Err(e.clone()),
        });
    }
    let trust = daemon.trust();
    // The same on every EVM chain; a row names its own only where it differs. (When no chain
    // has one, every row fails and this is never shown.)
    let address = owners
        .iter()
        .find_map(|o| o.as_ref().ok())
        .cloned()
        .unwrap_or_default();
    let reads: Vec<_> = order
        .iter()
        .zip(owners)
        .map(|(chain, owner)| {
            owner.map(|owner| {
                let read = tokio::spawn(read_network(net(chain, None), owner.clone()));
                (owner, read)
            })
        })
        .collect();
    let mut rows = Vec::with_capacity(order.len());
    let mut lines = vec![format!("key: {}", key.name), format!("address: {address}")];
    let mut failed: Vec<(&str, WalletError)> = Vec::new();
    let mut held = 0;
    let mut failed_tokens = 0;
    for (chain, read) in order.iter().zip(reads) {
        let mut row = json!({ "chain": chain.key, "chainId": chain.chain_id_json() });
        let mut own = String::new();
        let result = match read {
            Ok((owner, read)) => {
                if owner != address {
                    own = format!(" ({owner})");
                    row["address"] = json!(owner);
                }
                read.await.unwrap_or_else(|_| {
                    Err(WalletError::rpc("the balance read stopped unexpectedly"))
                })
            }
            Err(e) => Err(e),
        };
        match result {
            Ok((native, tokens)) => {
                lines.push(format!("{:<10} {}{own}", chain.key, amount_text(&native)));
                let mut any = !native.raw.is_zero();
                let mut token_rows = Vec::with_capacity(tokens.len());
                for t in tokens {
                    let mut v = json!({
                        "symbol": t.symbol,
                        "address": t.address,
                        "decimals": t.decimals,
                    });
                    match t.raw {
                        Ok(raw) => {
                            let b = chains::Balance {
                                symbol: t.symbol,
                                decimals: t.decimals,
                                raw,
                            };
                            if !raw.is_zero() {
                                any = true;
                                lines.push(format!("{:<10} {}", "", amount_text(&b)));
                            }
                            v["balance"] = json!(format_units(raw, b.decimals));
                            v["raw"] = json!(raw.to_string());
                        }
                        Err(e) => {
                            failed_tokens += 1;
                            lines.push(format!("{:<10} {} {}", "", t.symbol, error_text(&e)));
                            v["error"] = json!(e.message);
                            v["errorCode"] = json!(e.code.as_str());
                        }
                    }
                    token_rows.push(v);
                }
                if any {
                    held += 1;
                }
                row["native"] = amount_json(&native);
                row["tokens"] = Value::Array(token_rows);
            }
            Err(e) => {
                lines.push(format!("{:<10} {}", chain.key, error_text(&e)));
                row["error"] = json!(e.message);
                row["errorCode"] = json!(e.code.as_str());
                failed.push((chain.key, e));
            }
        }
        rows.push(row);
    }
    if failed.len() == order.len() {
        let code = failed[0].1.code;
        let code = if failed.iter().all(|(_, e)| e.code == code) {
            code
        } else {
            ErrorCode::RpcError
        };
        let why: Vec<String> = failed
            .iter()
            .map(|(chain, e)| format!("{chain}: {}", e.message))
            .collect();
        return Err(WalletError::new(
            code,
            format!("no network answered: {}", why.join("; ")),
        ));
    }
    let mut total = format!("{held} of {} networks have a balance", order.len());
    if !failed.is_empty() {
        total.push_str(&format!("; {} did not answer", failed.len()));
    }
    if failed_tokens > 0 {
        let reads = if failed_tokens == 1 { "read" } else { "reads" };
        total.push_str(&format!("; {failed_tokens} token {reads} failed"));
    }
    lines.push(total);
    if let Some(w) = trust.warning() {
        lines.push(format!("warning: {w}"));
    }
    let mut out = json!({ "key": key.name, "address": address, "chains": rows });
    trust.mark(&mut out);
    // The code after the networks.
    Ok(with_qr(
        Out::with_text(out, lines.join("\n")),
        trust.verified(),
        Some(&address),
        no_qr,
    ))
}

/// One network's [`NetworkBalances`], read within [`NETWORK_TIMEOUT`].
async fn read_network(net: Net, owner: String) -> NetworkBalances {
    match &net.chain.family {
        Family::Evm(_) => Evm.wallet_balances(&net, &owner, NETWORK_TIMEOUT).await,
    }
}

/// `{ symbol, decimals, balance, raw }`.
fn amount_json(b: &chains::Balance) -> Value {
    json!({
        "symbol": b.symbol,
        "decimals": b.decimals,
        "balance": format_units(b.raw, b.decimals),
        "raw": b.raw.to_string(),
    })
}

/// `1.5 ETH`.
fn amount_text(b: &chains::Balance) -> String {
    format!("{} {}", format_units(b.raw, b.decimals), b.symbol)
}

/// `error: <brief message> [<CODE>]`, as a network's or token's line shows a failed read.
fn error_text(e: &WalletError) -> String {
    format!("error: {} [{}]", brief(&e.message), e.code)
}

/// The first line of an error message, cut to [`BRIEF_ERROR_CHARS`] characters.
fn brief(message: &str) -> String {
    let line = message.lines().next().unwrap_or_default();
    if line.chars().count() <= BRIEF_ERROR_CHARS {
        return line.to_string();
    }
    let cut: String = line.chars().take(BRIEF_ERROR_CHARS - 1).collect();
    format!("{cut}…")
}

/// Returns (from, index) of the selected key on `chain`.
async fn signing_key(home: &Path, key: &KeyArg, chain: &Chain) -> Result<(String, u32, String)> {
    let file = store::read_wallet_file(home)?;
    let entry = file.key(key.key.as_deref())?;
    let from = key_address(home, entry, chain).await?;
    Ok((from, entry.index, entry.name.clone()))
}

async fn call(home: &Path, a: CallArgs) -> Result<Out> {
    let chain = chains::find(&a.chain)?;
    chains::require_contract_calls(chain)?;
    let to = evm::parse_address(&a.to)?;
    let func = abi::parse_function(&a.sig)?;
    let data = abi::encode_call(&func, &a.args)?;
    // eth_call from the wallet's address when there is one; reads work without a wallet too.
    let from = match store::read_wallet_file(home) {
        Ok(file) => {
            let key = file.key(a.key.key.as_deref())?;
            Some(evm::parse_address(&key_address(home, key, chain).await?)?)
        }
        Err(e) if e.code == ErrorCode::WalletNotFound && a.key.key.is_none() => None,
        Err(e) => return Err(e),
    };
    let net = net(chain, a.rpc.as_deref());
    let p = evm_rpc::connect(&net).await?;
    let raw = evm_rpc::eth_call(&p, from, to, data).await?;
    let result = if func.outputs.is_empty() {
        Value::Null
    } else {
        Value::Array(abi::decode_output(&func, &raw)?)
    };
    let sig = if func.outputs.is_empty() {
        func.signature()
    } else {
        func.signature_with_outputs()
    };
    Ok(Out::json(json!({
        "chain": chain.key,
        "from": from.map(|f| f.to_checksum(None)),
        "to": to.to_checksum(None),
        "sig": sig,
        "args": a.args,
        "result": result,
        "raw": format!("0x{}", hex::encode(&raw)),
    })))
}

async fn invoke(home: &Path, a: InvokeArgs) -> Result<Out> {
    let chain = chains::find(&a.chain)?;
    chains::require_contract_calls(chain)?;
    let (to, nickname) = whitelist::recipient(home, &a.to)?;
    let to = evm::parse_address(&to)?;
    let (input, sig) = match (&a.sig, &a.data) {
        (Some(sig), None) => {
            let func = abi::parse_function(sig)?;
            (abi::encode_call(&func, &a.args)?, Some(func.signature()))
        }
        (None, Some(data)) => {
            if !a.args.is_empty() {
                return Err(WalletError::validation(
                    "positional arguments need --sig; --data is the complete calldata",
                ));
            }
            (abi::parse_calldata(data)?, None)
        }
        _ => {
            return Err(WalletError::validation(
                "give either --sig with arguments or --data",
            ));
        }
    };
    let value = match &a.value {
        Some(v) => parse_units(v, chain.decimals)?,
        None => U256::ZERO,
    };
    let max_fee_per_gas = max_fee_wei(&a.send)?;
    let (bump, replace) = replacement(&a.send)?;
    let (from, index, key_name) = signing_key(home, &a.key, chain).await?;
    require_unlocked(home).await?;
    let lock = send_lock(home, chain, &from, &a.send)?;
    let net = net(chain, a.rpc.as_deref());
    let p = evm_rpc::connect(&net).await?;
    let intent = evm_rpc::Intent {
        from: evm::parse_address(&from)?,
        to,
        value,
        input: input.clone(),
        token: None,
        fallback_gas: None,
        dry_run: a.send.dry_run,
        nonce: a.send.nonce,
        bump,
        replace,
        max_fee_per_gas,
        allow_pending: a.send.allow_pending,
        journal: Some(chains::SendJournal::new(home, chain, &from)),
    };
    let prepared = evm_rpc::prepare(&p, &net, intent).await?;
    let mut base = Map::new();
    base.insert("chain".into(), json!(chain.key));
    base.insert("key".into(), json!(key_name));
    base.insert("from".into(), json!(from));
    base.insert("to".into(), json!(to.to_checksum(None)));
    if let Some(n) = nickname {
        base.insert("toNickname".into(), json!(n));
    }
    base.insert("sig".into(), json!(sig));
    base.insert("args".into(), json!(a.args));
    base.insert("data".into(), json!(format!("0x{}", hex::encode(&input))));
    base.insert("value".into(), json!(format_units(value, chain.decimals)));
    base.insert("valueRaw".into(), json!(value.to_string()));
    sign_and_send(
        &Evm, &net, home, index, &from, prepared, &a.send, base, lock, "invoke",
    )
    .await
}

/// The account's send lock, from the nonce read until the command returns (through `--wait`).
/// A dry-run consumes no nonce, so it takes none.
fn send_lock(
    home: &Path,
    chain: &Chain,
    from: &str,
    send: &SendArgs,
) -> Result<Option<chains::SendLock>> {
    (!send.dry_run)
        .then(|| chains::SendLock::acquire(home, chain, from))
        .transpose()
}

/// `--bump` (ten-thousandths) and `--replace` (a tx hash), validated before any RPC.
fn replacement(send: &SendArgs) -> Result<(Option<u128>, Option<B256>)> {
    let bump = send.bump.as_deref().map(chains::parse_bump).transpose()?;
    let replace = send
        .replace
        .as_deref()
        .map(evm::parse_tx_hash)
        .transpose()?;
    Ok((bump, replace))
}

/// `--max-fee-gwei` in wei (gwei = 10^9 wei), validated before any RPC.
fn max_fee_wei(send: &SendArgs) -> Result<Option<u128>> {
    let Some(gwei) = send.max_fee_gwei.as_deref() else {
        return Ok(None);
    };
    let bad = |why: String| WalletError::validation(format!("--max-fee-gwei: {why}"));
    let wei = parse_units(gwei, 9).map_err(|e| bad(e.message))?;
    if wei.is_zero() {
        return Err(bad("must be greater than zero".into()));
    }
    u128::try_from(wei)
        .map(Some)
        .map_err(|_| bad(format!("'{gwei}' is too large")))
}

async fn transfer(home: &Path, a: TransferArgs) -> Result<Out> {
    let chain = chains::find(&a.chain)?;
    let net = net(chain, a.rpc.as_deref());
    match &chain.family {
        Family::Evm(_) => transfer_with(&Evm, home, &net, a).await,
    }
}

async fn transfer_with<F: ChainFamily>(
    fam: &F,
    home: &Path,
    net: &Net,
    a: TransferArgs,
) -> Result<Out> {
    let chain = net.chain;
    let (to, nickname) = whitelist::recipient(home, &a.to)?;
    let to = fam.normalize_address(&to)?;
    if let Some(t) = &a.token {
        fam.normalize_address(t)?;
    } else {
        parse_units(&a.amount, chain.decimals)?;
    }
    let max_fee_per_gas = max_fee_wei(&a.send)?;
    let (bump, replace) = replacement(&a.send)?;
    let (from, index, key_name) = signing_key(home, &a.key, chain).await?;
    require_unlocked(home).await?;
    let lock = send_lock(home, chain, &from, &a.send)?;
    let req = TransferRequest {
        from: from.clone(),
        to: to.clone(),
        amount: a.amount.clone(),
        token: a.token.clone(),
        dry_run: a.send.dry_run,
        nonce: a.send.nonce,
        bump,
        replace: replace.map(|h| format!("{h:#x}")),
        max_fee_per_gas,
        force: a.force,
        allow_pending: a.send.allow_pending,
        journal: Some(chains::SendJournal::new(home, chain, &from)),
    };
    let prepared = fam.prepare_transfer(net, &req).await?;
    let mut base = Map::new();
    base.insert("chain".into(), json!(chain.key));
    base.insert("key".into(), json!(key_name));
    base.insert("from".into(), json!(from));
    base.insert("to".into(), json!(to));
    if let Some(n) = nickname {
        base.insert("toNickname".into(), json!(n));
    }
    let kind = if a.token.is_some() {
        "token-transfer"
    } else {
        "transfer"
    };
    sign_and_send(
        fam, net, home, index, &from, prepared, &a.send, base, lock, kind,
    )
    .await
}

/// What `out` (the command's fields merged with `prepare`'s) says the send does: the recipient
/// (the contract for an invoke), the amount with its symbol (`amount` and the asset's `symbol`
/// for a transfer, the native `value` for an invoke), the same amount in base units
/// (`amountRaw` for a transfer, `valueRaw` for an invoke), and the token contract of a token
/// transfer. The duplicate check keys on the base units, never on the symbol the node reported.
fn send_summary(out: &Map<String, Value>, chain: &Chain, kind: &str) -> chains::SendSummary {
    let text = |k: &str| out.get(k).and_then(Value::as_str);
    chains::SendSummary {
        kind: kind.to_string(),
        to: text("to").unwrap_or("?").to_string(),
        value: format!(
            "{} {}",
            text("amount").or_else(|| text("value")).unwrap_or("?"),
            text("symbol").unwrap_or(chain.symbol)
        ),
        amount_raw: text("amountRaw")
            .or_else(|| text("valueRaw"))
            .map(str::to_string),
        token: text("token").map(str::to_string),
    }
}

/// Shared tail of `transfer` and `invoke`: refuse a likely repeat, sign through the daemon,
/// check the raw bytes really come from `from`, then either report (dry-run) or broadcast (and
/// optionally wait). Right before the broadcast the send is written to the account's journal and
/// announced on stderr (`kind` names the command). `_lock` is held until this returns, through
/// any `--wait`, so no other send from the key starts while this one's outcome is still being
/// read.
///
/// A send without `--nonce` whose journal has the same send from the last 10 minutes is refused
/// before signing (unless `--allow-duplicate`): a run cut off after the broadcast and run again
/// once the first is mined passes the pending check and would pay twice. A dry-run reports the
/// matches (`duplicateSends`) instead. A `--nonce` send is an explicit replacement or retry at a
/// fixed nonce, so it cannot add a second payment and skips the check.
///
/// A dry-run first runs the daemon's whitelist check on whitelist.json as it is on disk, when
/// there is one; if that refuses, it reports `whitelist: {allowed: false, …}` and asks the daemon
/// for nothing. Without a whitelist.json the daemon decides, as for a live send, which is decided
/// by the daemon alone.
#[allow(clippy::too_many_arguments)]
async fn sign_and_send<F: ChainFamily>(
    fam: &F,
    net: &Net,
    home: &Path,
    index: u32,
    from: &str,
    prepared: PreparedTx,
    send: &SendArgs,
    mut out: Map<String, Value>,
    _lock: Option<chains::SendLock>,
    kind: &str,
) -> Result<Out> {
    let PreparedTx { unsigned, fields } = prepared;
    for (k, v) in fields {
        out.entry(k).or_insert(v);
    }
    let journal = chains::SendJournal::new(home, net.chain, from);
    let summary = send_summary(&out, net.chain, kind);
    if send.nonce.is_none() {
        let now = crate::core::time::unix_secs(std::time::SystemTime::now());
        let duplicates = journal.recent_duplicates(&summary, now)?;
        if !duplicates.is_empty() && !send.dry_run && !send.allow_duplicate {
            return Err(journal.duplicate_error(duplicates));
        }
        out.insert("duplicateSends".into(), Value::Array(duplicates));
    }
    // A dry-run the whitelist on disk refuses is explained without asking the daemon to sign.
    let checked = if send.dry_run {
        whitelist::dry_run_check(home, net.chain, from, &unsigned)?
    } else {
        whitelist::DryRunCheck::NotChecked
    };
    if let whitelist::DryRunCheck::Denied(denied) = checked {
        out.insert("dryRun".into(), json!(true));
        out.insert("whitelist".into(), denied);
        return Ok(Out::json(Value::Object(out)));
    }
    let signed_value = protocol::send_unlocked(
        home,
        &Request::SignTx {
            chain: net.chain.key.into(),
            index,
            tx: unsigned,
        },
        SIGN_TIMEOUT,
    )
    .await?;
    let signed = fam.inspect_signed(net.chain, &signed_value)?;
    if !signed.signer.eq_ignore_ascii_case(from) {
        return Err(WalletError::validation(format!(
            "the key daemon signed as {}, expected {from}; not broadcasting",
            signed.signer
        )));
    }
    // The nonce the signed bytes actually consume.
    if let Some(n) = signed.nonce {
        out.insert("nonce".into(), json!(n));
    }
    out.insert("dryRun".into(), json!(send.dry_run));
    // What the key holder decoded from the bytes it signed (not what this process intended).
    if let Some(view) = signed_value.get("decoded") {
        out.insert("signerDecoded".into(), view.clone());
    }
    // The hash is computed from the signed bytes, never taken from the node.
    out.insert("txHash".into(), json!(signed.tx_hash));
    if send.dry_run {
        // Checked here against the list on disk, then by the daemon against the sealed one.
        if matches!(checked, whitelist::DryRunCheck::Allowed) {
            out.insert("whitelist".into(), json!({ "allowed": true }));
        }
        // No rawTx: a dry-run must not hand out a transaction that can be broadcast without
        // zwallet.
        out.insert("recoveredFrom".into(), json!(signed.signer));
        return Ok(Out::json(Value::Object(out)));
    }
    let record = chains::SendRecord::new(journal, &signed, &summary);
    let outcome = fam
        .broadcast(net, &signed, send.nonce.is_some(), &record)
        .await?;
    out.insert(
        "explorerUrl".into(),
        json!(net.chain.explorer_tx_url(&signed.tx_hash)),
    );
    out.insert("journal".into(), json!(record.journal.display()));
    if let Broadcast::KnownDespite {
        warning,
        rpc_tx_hash,
    } = outcome
    {
        out.insert(
            "broadcastWarning".into(),
            json!(format!(
                "{warning}; the node reports the transaction by its own hash, so it was sent"
            )),
        );
        if let Some(h) = rpc_tx_hash {
            out.insert("rpcTxHash".into(), json!(h));
        }
    }
    if send.wait {
        let timeout = send.wait_timeout.unwrap_or(DEFAULT_WAIT_TIMEOUT);
        let receipt = fam.wait_for_receipt(net, &signed.tx_hash, timeout).await;
        chains::record_receipt(&mut out, net.chain, receipt);
    }
    Ok(Out::json(Value::Object(out)))
}

/// `tx`: a transaction looked up by hash on the chain's node. Needs no wallet and no key, so it
/// works while locked; every message that says to look a send up names this command.
async fn tx(a: TxArgs) -> Result<Out> {
    let chain = chains::find(&a.chain)?;
    let net = net(chain, a.rpc.as_deref());
    let found = match &chain.family {
        Family::Evm(_) => Evm.lookup_tx(&net, &a.tx_hash).await?,
    };
    Ok(Out::json(Value::Object(found)))
}

async fn sign_message(home: &Path, a: SignMessageArgs) -> Result<Out> {
    let chain = chains::find(&a.chain)?;
    let file = store::read_wallet_file(home)?;
    let key = file.key(a.key.key.as_deref())?;
    let expected = key_address(home, key, chain).await?;
    let signed = protocol::send_unlocked(
        home,
        &Request::SignMessage {
            chain: chain.key.into(),
            index: key.index,
            message_hex: hex::encode(a.message.as_bytes()),
        },
        SIGN_TIMEOUT,
    )
    .await?;
    match &chain.family {
        Family::Evm(_) => Evm.verify_message(a.message.as_bytes(), &signed, &expected)?,
    }
    Ok(Out::json(json!({
        "chain": chain.key,
        "key": key.name,
        "address": expected,
        "message": a.message,
        "signature": signed["signature"],
        "digest": signed["digest"],
    })))
}

async fn status(home: &Path, no_qr: bool) -> Result<Out> {
    let file = match store::read_wallet_file(home) {
        Ok(f) => Some(f),
        Err(e) if e.code == ErrorCode::WalletNotFound => None,
        Err(e) => return Err(e),
    };
    let (pids, serving) = live_daemons(home).await?;
    let others = protocol::other_daemon_pids(home)?;
    let locked = pids.is_empty();
    let mut out = json!({
        "wallet": if file.is_some() { "configured" } else { "not configured" },
        "locked": locked,
        "home": home.display().to_string(),
    });
    if !locked {
        out["daemonPids"] = json!(pids);
    }
    out["otherDaemonPids"] = json!(others);
    if let Some(d) = &serving {
        out["daemonPid"] = d["pid"].clone();
        out["expiresAt"] = d["expiresAt"].clone();
        // The whitelist as the serving daemon verifies it right now.
        out["whitelist"] = d["whitelist"].clone();
    }
    // The active key's address, and what verifies it.
    let mut shown = None;
    if let Some(file) = &file {
        out["active"] = json!(file.active);
        if let Ok(key) = file.key(None) {
            let primary = chains::find(PRIMARY_CHAIN)?;
            // Derived only by a listed daemon that says it serves this session.
            let (address, trust) = if serving.is_some() {
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
            };
            out["address"] = json!(address);
            trust.mark(&mut out);
            shown = Some((address, trust));
        }
    }
    let note = shown.as_ref().and_then(|(_, trust)| trust.note());
    let mut text = with_note(out, note);
    if !others.is_empty()
        && let Some(text) = text.text.as_mut()
    {
        text.push_str(&format!(
            "\nnote: {} zwallet daemon(s) of this user serve other homes (otherDaemonPids); \
             `zwallet lock` stops them too",
            others.len()
        ));
    }
    Ok(match shown {
        Some((address, trust)) => with_qr(text, trust.verified(), address.as_deref(), no_qr),
        None => text,
    })
}

/// How long `status` waits for a daemon that does not confirm its session (it may be exiting) to
/// leave the process table.
const EXIT_GRACE: Duration = Duration::from_secs(1);

/// This home's daemons, by the process table (any daemon process for the home means unlocked,
/// whatever answers on `d.sock`), plus `d.sock`'s status reply when it comes from one of them
/// and says unlocked. Any other reply (none, "locked", or one from no listed process) may come
/// from a daemon on its way out, which noticed expiry or a changed wallet.json and has removed
/// its socket or just said so: the process table is read again until it is empty, for
/// [`EXIT_GRACE`] at most. A squatter's reply never hides a daemon that stays.
async fn live_daemons(home: &Path) -> Result<(Vec<i32>, Option<Value>)> {
    let pids = protocol::daemon_pids(home, None)?;
    if pids.is_empty() {
        return Ok((pids, None));
    }
    let reply = protocol::status(home).await?;
    let from_listed = |v: &Value| {
        v["pid"]
            .as_i64()
            .and_then(|p| i32::try_from(p).ok())
            .is_some_and(|p| pids.contains(&p))
    };
    if let Some(v) = reply
        && v["locked"] == json!(false)
        && from_listed(&v)
    {
        return Ok((pids, Some(v)));
    }
    let start = std::time::Instant::now();
    loop {
        let now = protocol::daemon_pids(home, None)?;
        if now.is_empty() || start.elapsed() >= EXIT_GRACE {
            return Ok((now, None));
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn unlock(home: &Path, a: UnlockArgs) -> Result<Out> {
    let ttl = parse_duration(&a.duration)?;
    protocol::socket_path(home)?;
    // Fail before prompting when there is nothing to unlock. Of this read only the key list's
    // indices are used, for the index warning; the key, index and address shown are the ones the
    // daemon derived from the decrypted seed.
    let file = store::read_wallet_file(home)?;
    protocol::check_ptrace_scope(a.allow_ptrace_scope_0)?;
    protocol::check_daemons_findable()?;
    // Pinned before the prompt, so a binary swapped in while the human types is never started.
    let exe = DaemonExe::resolve()?;
    let pw = read_password(a.password.password_stdin, "Wallet password: ", false)?;
    let session = start_session(home, &exe, &pw, ttl).await?;
    drop(pw);
    let mut out = json!({
        "locked": false,
        "duration": a.duration.trim(),
        "expiresAt": session["expiresAt"],
        "active": session["active"],
        "index": session["index"],
        "address": session["address"],
        "daemonPid": session["pid"],
        // Derived by this process's own daemon child from the seed this password opened.
        "verified": true,
        "verifiedBy": BY_PASSWORD,
        // The whitelist this session signs under, as its daemon verified it: the generation and
        // the entries, so a rolled-back list is visible.
        "whitelist": session["whitelist"],
    });
    if let Some(w) = session_index_warning(&file, &session) {
        out["warning"] = json!(w);
    }
    Ok(with_qr(
        Out::json(out),
        true,
        session["address"].as_str(),
        a.qr.no_qr,
    ))
}

/// [`index_warning`] (as `address --verify` gives it) for the key a new session reports: its
/// name and index as the daemon derived them, against `file`'s key list and `nextIndex`.
fn session_index_warning(file: &WalletFile, session: &Value) -> Option<String> {
    let index = session["index"]
        .as_u64()
        .and_then(|i| u32::try_from(i).ok())?;
    let shown = KeyEntry {
        name: session["active"].as_str().unwrap_or_default().to_string(),
        index,
        created_at: String::new(),
        addresses: BTreeMap::new(),
    };
    index_warning(file, &[&shown])
}

/// The panic button: stops every zwallet daemon of this user by the process table, not only one
/// home's ([`protocol::stop_all_daemons`]). It reads no home, not even `ZWALLET_HOME` or `HOME`,
/// so nothing at a home path (a stalled mount, a listener that never answers) can hold it up.
fn lock() -> Result<Out> {
    let stopped = protocol::stop_all_daemons()?;
    Ok(Out::json(json!({
        "locked": true,
        "wasUnlocked": !stopped.is_empty(),
        "stopped": stopped,
    })))
}

fn backup(home: &Path) -> Result<Out> {
    if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
        return Err(WalletError::validation(
            "backup prints the recovery phrase, so it only runs with stdin and stdout on an \
             interactive terminal (a human at the keyboard)",
        ));
    }
    store::read_wallet_file(home)?;
    let pw = read_password(false, "Wallet password: ", false)?;
    let seed = store::unlock(home, &pw, &chains::derive_addresses)?.0.seed;
    let words: Vec<String> = seed
        .phrase()
        .split(' ')
        .enumerate()
        .map(|(i, w)| format!("{:>2}. {w}", i + 1))
        .collect();
    let text = format!(
        "Recovery phrase ({} words, BIP39 English, no passphrase). Write it down offline:\n{}",
        words.len(),
        words.join("\n")
    );
    Ok(Out::with_text(
        json!({ "mnemonic": seed.phrase(), "words": seed.word_count() }),
        text,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(name: &str, index: u32) -> KeyEntry {
        KeyEntry {
            name: name.into(),
            index,
            created_at: String::new(),
            addresses: BTreeMap::new(),
        }
    }

    fn file(next_index: u32, keys: Vec<KeyEntry>) -> WalletFile {
        serde_json::from_value(json!({
            "version": 1,
            "createdAt": "",
            "active": keys[0].name,
            "nextIndex": next_index,
            "keys": keys,
            "crypto": {
                "cipher": "", "kdf": "", "nonce": "", "ciphertext": "",
                "kdfparams": { "logN": 15, "r": 8, "p": 1, "dklen": 32, "salt": "" },
            },
        }))
        .unwrap()
    }

    #[test]
    fn relock_failure_is_reported_not_swallowed() {
        let ok = relocked("mismatch".into(), Ok(vec![42]));
        assert!(
            ok.message.ends_with("so the wallet was locked again"),
            "{}",
            ok.message
        );
        let failed = relocked(
            "mismatch".into(),
            Err(WalletError::validation(
                "could not stop the key daemon process(es) 42",
            )),
        );
        assert_eq!(failed.code, ErrorCode::ValidationError);
        assert!(
            !failed.message.contains("was locked again"),
            "{}",
            failed.message
        );
        assert!(
            failed.message.contains("locking the wallet again failed")
                && failed.message.contains("process(es) 42")
                && failed.message.contains("zwallet lock"),
            "{}",
            failed.message
        );
    }

    #[test]
    fn index_warning_flags_indices_past_next_index_and_gaps() {
        let plain = file(2, vec![key("a", 0), key("b", 1)]);
        let all: Vec<&KeyEntry> = plain.keys.iter().collect();
        assert_eq!(index_warning(&plain, &all), None);
        assert_eq!(index_warning(&plain, &[&plain.keys[1]]), None);

        let past = file(1, vec![key("a", 0), key("b", 1)]);
        let w = index_warning(&past, &[&past.keys[1]]).unwrap();
        assert!(
            w.contains("key 'b' has index 1, at or past nextIndex 1"),
            "{w}"
        );
        assert!(!w.contains("belong to no key"), "{w}");
        // Key 'a' alone is fine.
        assert_eq!(index_warning(&past, &[&past.keys[0]]), None);

        let gap = file(9, vec![key("a", 0), key("b", 3), key("c", 4)]);
        let w = index_warning(&gap, &[&gap.keys[1]]).unwrap();
        assert!(
            w.contains("2 index(es) below 3 belong to no key (the first is 1)"),
            "{w}"
        );
        // Below index 0 nothing can be missing.
        assert_eq!(index_warning(&gap, &[&gap.keys[0]]), None);

        // A huge index is counted, not walked.
        let far = file(1, vec![key("a", (1 << 31) - 1)]);
        let w = index_warning(&far, &[&far.keys[0]]).unwrap();
        assert!(w.contains("the first is 0"), "{w}");
        assert!(w.contains("at or past nextIndex 1"), "{w}");
    }
}
