//! Command handlers. Everything here runs in the one-shot CLI process; the only key access is
//! through the daemon socket (or, for `create`/`backup`, a password typed by the human).

use crate::chains::evm::{self, Evm, rpc as evm_rpc};
use crate::chains::tron;
use crate::chains::{
    self, CHAINS, Chain, ChainFamily, Family, Net, PRIMARY_CHAIN, PreparedTx, TransferRequest,
    redact_url,
};
use crate::cli::{
    AddressArgs, AddressesArgs, BalanceArgs, CallArgs, Cli, Command, CreateArgs, InvokeArgs,
    KeyArg, Out, Qr, SendArgs, SignMessageArgs, TransferArgs, TxArgs, UnlockArgs, VerifyArgs,
    render_human,
};
use crate::core::amount::{format_units, parse_units};
use crate::core::error::{ErrorCode, Result, WalletError};
use crate::core::password::read_password;
use crate::core::protocol::{self, Peer, PeerCheck, Request, SIGN_TIMEOUT};
use crate::core::seed::Seed;
use crate::core::store::{self, KeyEntry, WalletFile};
use crate::core::time::parse_duration;
use crate::daemon;
use crate::qr;
use crate::wallet;
use alloy::hex;
use alloy::primitives::{B256, U256};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub(crate) mod whitelist;

fn home() -> Result<PathBuf> {
    store::resolve_home(std::env::var_os("ZWALLET_HOME"), std::env::home_dir())
}

/// Closes every inherited descriptor before `zwallet daemon` opens its own (see
/// [`protocol::close_inherited_fds`]); `cli::main_entry` calls it for that command alone.
pub fn close_inherited_fds() -> Result<()> {
    protocol::close_inherited_fds()
}

pub async fn run(cli: Cli) -> Result<Out> {
    let json = cli.json;
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
        Command::Task(a) => task(&home()?, a).await,
        Command::Status(qr) => status(&home()?, qr.no_qr).await,
        Command::Backup => backup(&home()?),
        Command::Unlock(a) => unlock(&home()?, a).await,
        Command::Lock => lock(),
        Command::Whitelist(w) => whitelist::run(&home()?, w).await,
        Command::Tui {
            warm,
            chain,
            allow_ptrace_scope_0,
        } => tui(&home()?, &warm, &chain, json, allow_ptrace_scope_0).await,
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

pub(crate) fn net(chain: &'static Chain, flag: Option<&str>) -> Net {
    Net {
        chain,
        rpc_url: rpc_url(chain, flag),
    }
}

/// `zwallet tui`: the arguments are checked first, then the terminal ([`crate::tui::run`]).
/// Its error (the terminal, or a lock that failed while `q` waited for it) is printed as every
/// command's is, after the terminal is given back; a clean quit prints nothing.
async fn tui(
    home: &Path,
    warm: &str,
    chain: &str,
    json: bool,
    allow_ptrace_scope_0: bool,
) -> Result<Out> {
    let chain = chains::find(chain)?;
    let warm = parse_duration(warm)?;
    if json {
        return Err(WalletError::validation(
            "zwallet tui is interactive: --json does not apply",
        ));
    }
    crate::tui::run(
        home.to_path_buf(),
        wallet::net(chain),
        warm,
        allow_ptrace_scope_0,
    )
    .await?;
    Ok(Out::silent())
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
                 derive it, or run `zwallet address --chain {} --verify` (with the password) to \
                 add it",
                key.name, chain.key, chain.key
            ),
        )
    })?;
    reply_address(&v, chain)
}

/// Derives every chain's address for key `index` through the unlocked daemon.
pub(crate) async fn derive_via_daemon(home: &Path, index: u32) -> Result<BTreeMap<String, String>> {
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
pub(crate) fn primary_address(key: &KeyEntry) -> Result<Option<String>> {
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
pub(crate) fn shown(value: &str) -> String {
    crate::core::text::shown(value, VALUE_SHOWN)
}

/// The session whitelist a daemon reports, inert for `--json` and the terminal. A genuine daemon
/// sends an object (or null): every string and key is ASCII-escaped and its structure and counts
/// are kept, but its `reason` (the only unbounded prose field, which can quote whitelist.json) is
/// cut to a prose budget, and `render_human` bounds the rest. Anything else (a string, an array,
/// a number from whatever answers on `d.sock`) is no whitelist: it becomes one string, its JSON
/// text made inert and cut to [`PROSE_MAX`](crate::core::text::PROSE_MAX), in both outputs.
pub(crate) fn inert_whitelist(value: &Value) -> Value {
    use crate::core::text::{PROSE_MAX, ascii_inert, inert_json};
    match value {
        Value::Null => Value::Null,
        Value::Object(_) => {
            let mut w = inert_json(value);
            if let Some(reason) = w.get("reason").and_then(Value::as_str) {
                w["reason"] = json!(ascii_inert(reason, PROSE_MAX));
            }
            w
        }
        other => json!(ascii_inert(&other.to_string(), PROSE_MAX)),
    }
}

/// What the key holder decoded from the bytes it signed, for `signerDecoded`: an object from a
/// genuine daemon, made inert with its structure kept; anything else (whatever answers on `d.sock`
/// may send a 1 MiB string) one string, made inert and cut as [`inert_whitelist`] does.
pub(crate) fn signer_decoded(view: &Value) -> Value {
    use crate::core::text::{PROSE_MAX, ascii_inert, inert_json};
    if view.is_object() {
        inert_json(view)
    } else {
        json!(ascii_inert(&view.to_string(), PROSE_MAX))
    }
}

/// Whether `value` has the shape of an address on `chain`: for EVM, `0x` and 40 hex digits (any
/// case); for Tron, 34 base58 characters whose checksum holds and whose payload starts `0x41`
/// ([`tron::address::parse`]). Says nothing about whose address it is.
fn address_shaped(chain: &Chain, value: &str) -> bool {
    match &chain.family {
        Family::Evm(_) => {
            value.len() == 42
                && value.starts_with("0x")
                && value[2..].bytes().all(|b| b.is_ascii_hexdigit())
        }
        Family::Tron(_) => tron::address::is_address(value),
    }
}

/// Whether `a` and `b` are the same address on chain key `chain`: EVM hex in any case, Tron byte
/// for byte ([`tron::address::same`]: a case variant of a base58check string is another one).
fn same_address(chain: &str, a: &str, b: &str) -> bool {
    match chains::find(chain).map(|c| &c.family) {
        Ok(Family::Tron(_)) => tron::address::same(a, b),
        _ => a.eq_ignore_ascii_case(b),
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

/// wallet.json's cached address for `key` on `chain` must be the one this process derived from
/// the seed after the password (`derived`); the cache is plain metadata that anyone running as
/// this user can edit.
fn check_cache(key: &KeyEntry, chain: &str, derived: &str) -> Result<()> {
    match cache_mismatch(key, chain, derived)? {
        Some(cached) => Err(mismatch_error(
            &key.name,
            chain,
            &cached,
            derived,
            &Deriver::Seed("this process"),
        )),
        None => Ok(()),
    }
}

/// Who produced the address a cached one is compared with, which decides what a mismatch may say.
#[derive(Debug)]
enum Deriver<'a> {
    /// Derived from the seed: by this process after the password, or by this build's own key
    /// daemon of the home. A mismatch means the cache was modified.
    Seed(&'a str),
    /// Reported by whatever answers on `d.sock`, not known to be this build's key daemon: either
    /// side may be the tampered one. With the reason when it is known to be another process.
    Socket(Option<&'a str>),
}

/// The `VALIDATION_ERROR` for a cached address (`cached`) of key `name` on `chain` that differs
/// from `other`, worded by who produced `other` ([`Deriver`]). Only a derivation from the seed
/// blames the cache; an address reported on the socket by a process not known to be this
/// build's key daemon is no derivation, so neither address is to be funded and the password
/// check decides. `other` is escaped; `cached` is address-shaped already ([`cache_mismatch`]).
fn mismatch_error(
    name: &str,
    chain: &str,
    cached: &str,
    other: &str,
    by: &Deriver<'_>,
) -> WalletError {
    let other = shown(other);
    match by {
        Deriver::Seed(who) => WalletError::validation(format!(
            "wallet.json's cached {chain} address for key '{name}' ({cached}) does not match the \
             address {who} derives from the seed ({other}); the cache was modified, do not send \
             funds to it. `zwallet address --verify` repairs the cache from the seed"
        )),
        Deriver::Socket(impostor) => {
            let mut message = format!(
                "the process answering on the key daemon socket reported {other} as the {chain} \
                 address of key '{name}', but wallet.json caches {cached}: one of them was \
                 tampered with. Do not send funds to either address; run `zwallet address \
                 --verify` (with the password it says which is right)"
            );
            if let Some(reason) = impostor {
                message.push_str(&format!(
                    ". That process is not this build's key daemon: {reason}; {}",
                    protocol::RELOCK
                ));
            }
            WalletError::validation(message)
        }
    }
}

/// The cached address of `key` on `chain` when it differs from `derived`, `None` when it agrees
/// or there is none. A cached value that is not an address is an error ([`cached_address`]).
fn cache_mismatch(key: &KeyEntry, chain: &str, derived: &str) -> Result<Option<String>> {
    match key.addresses.get(chain) {
        Some(cached) => {
            let cached = cached_address(key, chain, cached)?;
            Ok((!same_address(chain, &cached, derived)).then_some(cached))
        }
        None => Ok(None),
    }
}

/// How far an address shown without the password can be trusted. There are three levels:
/// - `verified: true, verifiedBy: "password"`: derived from the seed after the human's password,
///   in this process or by the daemon child `unlock` just started.
/// - `verified: true, verifiedBy: "session"` ([`Trust::Session`]): while unlocked, derived by this
///   zwallet build's own key daemon for this home ([`protocol::check_peer`]: on macOS, for signed
///   builds, every build zwallet ships), and equal to the cache. That daemon decrypted this home's
///   wallet.json with the password typed at `unlock`.
/// - `verified: false`: the cache in wallet.json, which any process of this user can edit, or a
///   derivation by something else on `d.sock`: with a `warning` naming it when it is known to be
///   another program; one whose code cannot be read (an unsigned program, such as an x86_64 one
///   on any Mac, or a non-dumpable one on Linux) is not named, only not verified.
///
/// What a session cannot rule out: a process of this user that swapped in its own wallet.json
/// and unlocked it with its own password through this very binary, or injected code into it. Only
/// the address the human saved from their first unlock tells that apart: if it ever changes, stop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Trust {
    Session,
    /// The wallet is locked: the cache.
    Locked,
    /// Unlocked, but not verified by the session: the warning, when what answered is known not
    /// to be this build's key daemon of this home.
    Unverified(Option<String>),
}

impl Trust {
    pub fn verified(&self) -> bool {
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

    pub fn warning(&self) -> Option<&str> {
        match self {
            Self::Unverified(Some(w)) => Some(w),
            _ => None,
        }
    }
}

/// The key daemon as one command asks it for addresses: each derivation is checked against the
/// cache, and each process that gave one is checked once ([`protocol::check_peer`]); then
/// [`Daemon::trust`] says what that verifies.
pub(crate) struct Daemon<'a> {
    home: &'a Path,
    peers: Vec<(Peer, PeerCheck)>,
    /// Some derivation came back.
    answered: bool,
    /// Some address came from the cache instead (the wallet was or became locked).
    cached: bool,
}

impl<'a> Daemon<'a> {
    pub(crate) fn new(home: &'a Path) -> Self {
        Self {
            home,
            peers: Vec::new(),
            answered: false,
            cached: false,
        }
    }

    /// The daemon's derivation of key `index` on `chain`, or `None` while the wallet is locked.
    pub(crate) async fn derive(&mut self, index: u32, chain: &Chain) -> Result<Option<String>> {
        self.fetch(index, chain).await?
    }

    /// [`Daemon::derive`] in two levels, for a caller that keeps going when one chain family
    /// cannot be read: the outer error must abort the command (an answer that is no address, a
    /// failed peer check: tampering, never a row), the inner one is that the socket could not be
    /// asked or relayed an error.
    pub(crate) async fn fetch(
        &mut self,
        index: u32,
        chain: &Chain,
    ) -> Result<Result<Option<String>>> {
        let request = Request::Address {
            chain: chain.key.into(),
            index,
        };
        let answer = match protocol::send_answered(self.home, &request, SIGN_TIMEOUT).await {
            Ok(Some(a)) => a,
            Ok(None) => return Ok(Ok(None)),
            Err(e) if e.code == ErrorCode::WalletLocked => return Ok(Ok(None)),
            Err(e) => return Ok(Err(e)),
        };
        if !self.peers.iter().any(|(p, _)| *p == answer.peer) {
            let check = protocol::check_peer(self.home, &answer.peer)?;
            self.peers.push((answer.peer, check));
        }
        // An answer that is no address is never used, whoever gave it; the error names an
        // impostor when one answered.
        let address = reply_address(&answer.value, chain).map_err(|e| self.naming_impostor(e))?;
        self.answered = true;
        Ok(Ok(Some(address)))
    }

    /// Why a process that answered this command is not this build's key daemon, if one is known
    /// not to be (a [`PeerCheck::Other`] peer).
    fn impostor(&self) -> Option<&str> {
        self.peers.iter().find_map(|(_, c)| match c {
            PeerCheck::Other(reason) => Some(reason.as_str()),
            _ => None,
        })
    }

    /// Whether every process that answered this command is this build's key daemon of the home
    /// (and one did).
    fn all_this_build(&self) -> bool {
        !self.peers.is_empty() && self.peers.iter().all(|(_, c)| *c == PeerCheck::ThisBuild)
    }

    /// `e` naming an impostor on `d.sock` (in the message, and as the `warning` detail), when one
    /// answered this command.
    fn naming_impostor(&self, e: WalletError) -> WalletError {
        match (self.impostor(), self.trust().warning()) {
            (Some(reason), Some(w)) => {
                let message = format!(
                    "{}. That process is not this build's key daemon: {reason}; {}",
                    e.message,
                    protocol::RELOCK
                );
                let w = w.to_string();
                WalletError { message, ..e }.with_detail("warning", w)
            }
            _ => e,
        }
    }

    /// `key`'s address on `chain` from `derived` (this command's derivation for the chain's
    /// family) checked against the cache, or without one the cache (the daemon's derivation for
    /// a chain added after the key).
    pub(crate) async fn checked(
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

    /// `derived` (what answered on `d.sock` gave) checked against `key`'s cached address on
    /// `chain`. A mismatch aborts the command with `VALIDATION_ERROR` ([`mismatch_error`]): it
    /// blames the cache only when every process that answered is this build's key daemon of the
    /// home (its answer is the seed's derivation); otherwise either side may be the tampered one,
    /// and a process known to be another one is named (also as the `warning` detail).
    pub(crate) fn check_cache(&self, key: &KeyEntry, chain: &Chain, derived: &str) -> Result<()> {
        let Some(cached) =
            cache_mismatch(key, chain.key, derived).map_err(|e| self.naming_impostor(e))?
        else {
            return Ok(());
        };
        let by = if self.all_this_build() {
            Deriver::Seed("the key daemon")
        } else {
            Deriver::Socket(self.impostor())
        };
        let e = mismatch_error(&key.name, chain.key, &cached, derived, &by);
        Err(match self.trust().warning() {
            Some(w) => e.with_detail("warning", w),
            None => e,
        })
    }

    /// `key`'s address on `chain`: derived and checked against the cache, or the cache while
    /// locked. A mismatch is `VALIDATION_ERROR`.
    pub(crate) async fn address(&mut self, key: &KeyEntry, chain: &Chain) -> Result<String> {
        let derived = self.derive(key.index, chain).await?;
        self.checked(key, chain, derived.as_ref()).await
    }

    /// The cache's address for `key` on `chain`, used when the daemon did not derive one (the
    /// wallet is locked, or the chain was added after the key). Marks the result as cached, so
    /// [`Trust`] does not read it as session-verified.
    pub(crate) async fn cache_only(&mut self, key: &KeyEntry, chain: &Chain) -> Result<String> {
        self.cached = true;
        key_address(self.home, key, chain).await
    }

    /// What the addresses this command took verify: [`Trust::Session`] only when every one was
    /// derived, and every process that answered is this build's key daemon of this home.
    pub(crate) fn trust(&self) -> Trust {
        let other = self.peers.iter().find_map(|(_, c)| c.warning());
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

/// `list`'s note while this build's session verifies the active key: the other keys' addresses
/// are not verified, and the password checks them.
const OTHER_KEYS_NOTE: &str =
    "not verified: the other keys' addresses; `zwallet list --verify` checks them";

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

/// The line under `scan to send to <address>` for an EVM address: the code carries no chain.
/// Two lines, so both fit an 80-column terminal.
const EVERY_EVM_NETWORK: &str = "(the same address on every EVM network in `zwallet chains`)";

/// The same line for a Tron address, which no EVM network takes (`.prd/05-tron.md` §CLI
/// surface).
const TRON_ONLY: &str = "(Tron only: TRX and TRC20 tokens; not an EVM address)";

/// `out` with `address` (on `chain`) as a QR code after its human text and any code before it
/// ([`Out::qr`]), for a phone wallet to scan, under the lines `scan to send to <address>` and
/// [`EVERY_EVM_NETWORK`] or [`TRON_ONLY`]. Only for an address reported `verified: true` (by the
/// password or by the session), only with stdout on a terminal, and not with `--no-qr`: agents
/// read zwallet through pipes, and `--json` prints no human text.
fn with_qr(mut out: Out, verified: bool, chain: &Chain, address: Option<&str>, no_qr: bool) -> Out {
    let Some(address) = address.filter(|_| verified && qr_shown(no_qr)) else {
        return out;
    };
    // No colours when NO_COLOR is set and not empty (no-color.org).
    let color = std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty());
    let Some(code) = qr::render(address, color) else {
        return out;
    };
    let networks = match chain.family {
        Family::Evm(_) => EVERY_EVM_NETWORK,
        Family::Tron(_) => TRON_ONLY,
    };
    out.qr.push(Qr {
        caption: format!("scan to send to {address}\n{networks}"),
        code,
    });
    out
}

/// The chain whose address the key shows by default ([`PRIMARY_CHAIN`]).
fn primary_chain() -> Result<&'static Chain> {
    chains::find(PRIMARY_CHAIN)
}

/// `CAPABILITY_UNSUPPORTED` for what `chain`'s family does not do (in this version).
fn unsupported(chain: &Chain, what: &str) -> WalletError {
    WalletError::new(
        ErrorCode::CapabilityUnsupported,
        format!(
            "{} ({} family) does not support {what}",
            chain.key,
            chain.family.name()
        ),
    )
}

/// Whether human output may end with a QR code: stdout on a terminal, and no `--no-qr`.
fn qr_shown(no_qr: bool) -> bool {
    !no_qr && std::io::stdout().is_terminal()
}

/// `out` with `line` at the end of its human text (a QR code stays after it).
fn with_line(out: Out, line: &str) -> Out {
    let text = out.text.unwrap_or_else(|| render_human(&out.value));
    Out {
        text: Some(format!("{text}\n{line}")),
        ..out
    }
}

/// What the `create` that makes the master seed shows where the QR code would be.
const QR_AFTER_BACKUP: &str =
    "QR after backup: run `zwallet backup`, then `zwallet address --verify` (or unlock) to show it";

/// `key`'s address on `chain` derived from `seed` here, checked against the cache.
pub(crate) fn derive_checked(seed: &Seed, key: &KeyEntry, chain: &Chain) -> Result<String> {
    let derived = chain.family.keys().derive_address(seed, key.index)?;
    check_cache(key, chain.key, &derived)?;
    Ok(derived)
}

async fn create(home: &Path, a: CreateArgs) -> Result<Out> {
    let ttl = a.unlock.as_deref().map(parse_duration).transpose()?;
    let plan = wallet::prepare_create(home, &a.name, a.words, ttl, a.allow_ptrace_scope_0)?;
    let human = a.password.password_stdin || std::io::stdin().is_terminal();
    let created = if !plan.creates_master() && ttl.is_none() && !human && unlocked(home).await? {
        wallet::create_via_daemon(plan).await?
    } else {
        let first = plan.creates_master();
        let prompt = if first {
            "New wallet password: "
        } else {
            "Wallet password: "
        };
        let password = read_password(a.password.password_stdin, prompt, first)?;
        wallet::create(plan, password).await?
    };
    let address = created.address();
    let mut out = json!({
        "name": created.name,
        "index": created.index,
        "address": address,
        "active": created.active,
        "master": if created.master_created { "created" } else { "existing" },
        "home": home.display().to_string(),
        "chains": CHAINS.iter().map(|c| c.key).collect::<Vec<_>>(),
        "verified": created.verified,
    });
    if created.verified {
        out["verifiedBy"] = json!(BY_PASSWORD);
    }
    if let Some(session) = &created.session {
        out["locked"] = json!(false);
        out["expiresAt"] = session.ready["expiresAt"].clone();
        out["daemonPid"] = session.ready["pid"].clone();
        out["whitelist"] = session.whitelist.clone();
    }
    if let Some(reminder) = created.backup_reminder
        && std::io::stderr().is_terminal()
    {
        crate::core::terminal::eprint_line(reminder);
    }
    let out = with_note(out, (!created.verified).then_some(UNVERIFIED_NOTE));
    if created.master_created {
        return Ok(if qr_shown(a.qr.no_qr) {
            with_line(out, QR_AFTER_BACKUP)
        } else {
            out
        });
    }
    Ok(with_qr(
        out,
        created.verified,
        primary_chain()?,
        address,
        a.qr.no_qr,
    ))
}

async fn list(home: &Path, v: VerifyArgs) -> Result<Out> {
    let (active, rows, warning, verified, note) = if v.verify {
        store::read_wallet_file(home)?;
        let pw = read_password(v.password_stdin, "Wallet password: ", false)?;
        // Addresses derived from this seed are the ones reported `verified: true`: no other
        // process or file can make them.
        let opened = wallet::open_with_password(home, &pw, None)?;
        drop(pw);
        (opened.active, opened.keys, opened.warning, true, None)
    } else {
        let keys = wallet::keys(home).await?;
        // A session of this build verifies the active key (`status` and `address` say so):
        // nothing asks to verify it, and the other keys stay flagged.
        let note = match keys.trust {
            Trust::Session if keys.rows.iter().all(|k| k.active) => None,
            Trust::Session => Some(OTHER_KEYS_NOTE),
            _ => Some(UNVERIFIED_NOTE),
        };
        let warning = keys.trust.warning().map(str::to_string);
        (keys.active, keys.rows, warning, false, note)
    };
    let keys: Vec<Value> = rows
        .iter()
        .map(|k| {
            json!({
                "name": k.name,
                "index": k.index,
                "address": k.address,
                "active": k.active,
            })
        })
        .collect();
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
    let mut out = json!({ "active": active, "keys": keys, "verified": verified });
    if verified {
        out["verifiedBy"] = json!(BY_PASSWORD);
    }
    // The warning line before the note, as in `address`.
    if let Some(w) = warning {
        text.push_str(&format!("\nwarning: {w}"));
        out["warning"] = json!(w);
    }
    if let Some(note) = note {
        text.push('\n');
        text.push_str(note);
    }
    Ok(Out::with_text(out, text))
}

/// For `--verify`: derivation indices in wallet.json's key list that zwallet never writes, so
/// the list was edited: one of `shown` at or past `nextIndex`, or an index below the highest of
/// `shown` that no key uses. The address is still this seed's, but where it sits matters for a
/// wallet restored from the recovery phrase, which lists accounts in index order.
pub(crate) fn index_warning(file: &WalletFile, shown: &[&KeyEntry]) -> Option<String> {
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
    let (address, trust) = wallet::set_active(home, name).await?;
    let mut out = json!({ "active": name, "address": address });
    trust.mark(&mut out);
    Ok(with_qr(
        with_note(out, trust.note()),
        trust.verified(),
        primary_chain()?,
        Some(&address),
        no_qr,
    ))
}

async fn address(home: &Path, a: AddressArgs) -> Result<Out> {
    let chain = chains::find(&a.chain)?;
    if a.verify.verify {
        // With the password the seed is authoritative: cached addresses that differ from it are
        // repaired, loudly, instead of refused.
        store::read_wallet_file(home)?;
        let pw = read_password(a.verify.password_stdin, "Wallet password: ", false)?;
        let repaired = wallet::repair(home, &pw)?;
        drop(pw);
        let file = repaired.file;
        let repairs = repaired.checked.repairs;
        let added = repaired.added;
        let key = file.key(a.key.key.as_deref())?;
        let address = repaired
            .checked
            .addresses
            .iter()
            .find(|row| row.key == key.name && row.chain == chain.key)
            .map(|row| row.address.clone())
            .ok_or_else(|| WalletError::validation("verified address missing"))?;
        let mut out = json!({
            "chain": chain.key,
            "key": key.name,
            "index": key.index,
            "address": address,
            "verified": true,
            "verifiedBy": BY_PASSWORD,
        });
        // Chains this build derives that the cache lacked (added to zwallet after the key was
        // made): filled in from the seed, which is no sign of tampering.
        if !added.is_empty() {
            out["added"] = json!(
                added
                    .iter()
                    .map(|a| json!({ "key": a.key, "chain": a.chain, "now": a.now }))
                    .collect::<Vec<_>>()
            );
        }
        let mut warnings: Vec<String> = Vec::new();
        if !repairs.is_empty() {
            out["repaired"] = json!(
                repairs
                    .iter()
                    .map(|r| json!({ "key": r.key, "chain": r.chain, "was": shown(&r.was), "now": r.now }))
                    .collect::<Vec<_>>()
            );
            warnings.push(format!(
                "wallet.json's address cache did not match the seed: {} repaired from it (see \
                 `repaired`). Something modified wallet.json; check what can write to it before \
                 funding",
                if repairs.len() == 1 {
                    "1 entry was".to_string()
                } else {
                    format!("{} entries were", repairs.len())
                }
            ));
        }
        // A repaired file was edited: its whole key list is checked for indices zwallet never
        // assigns, not only the key shown.
        let checked: Vec<&KeyEntry> = if repairs.is_empty() {
            vec![key]
        } else {
            file.keys.iter().collect()
        };
        warnings.extend(index_warning(&file, &checked));
        if !warnings.is_empty() {
            out["warning"] = json!(warnings.join(". "));
        }
        // Human text: the fields, then one line per repaired entry; the old value is marked so
        // nobody funds it.
        let mut shown_out = out.clone();
        if let Some(o) = shown_out.as_object_mut() {
            o.remove("repaired");
            o.remove("added");
        }
        let mut text = render_human(&shown_out);
        for r in &repairs {
            text.push_str(&format!(
                "\nrepaired: key '{}' {}: was {} (do not fund), now {}",
                r.key,
                r.chain,
                shown(&r.was),
                r.now
            ));
        }
        for a in &added {
            text.push_str(&format!("\nadded: key '{}' {}: {}", a.key, a.chain, a.now));
        }
        return Ok(with_qr(
            Out::with_text(out, text),
            true,
            chain,
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
        chain,
        Some(&address),
        a.qr.no_qr,
    ))
}

async fn addresses(home: &Path, a: AddressesArgs) -> Result<Out> {
    let file = store::read_wallet_file(home)?;
    let key = file.key(a.key.key.as_deref())?;
    let addresses = wallet::addresses(home, &key.name).await?;
    let mut codes: Vec<(&'static Chain, String)> = Vec::new();
    let rows:Vec<Value>=addresses.rows.iter().map(|row| {
        if let Some(e)=&row.error {return json!({"chainId":row.chain.key,"error":e.message,"errorCode":e.code.as_str()});}
        if !codes.iter().any(|(chain,_)|chain.family.name()==row.chain.family.name()) {codes.push((row.chain,row.address.clone()));}
        json!({"chainId":row.chain.key,"address":row.address})
    }).collect();
    let trust = addresses.trust;
    let mut text = rows
        .iter()
        .map(|r| {
            let shown = match r["address"].as_str() {
                Some(a) => a.to_string(),
                None => format!(
                    "error: {} [{}]",
                    brief(r["error"].as_str().unwrap_or("")),
                    r["errorCode"].as_str().unwrap_or("")
                ),
            };
            format!("{:<10} {shown}", r["chainId"].as_str().unwrap_or(""))
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
    // One code per family: the address is the same on every EVM chain, Tron's is its own.
    let mut out = Out::with_text(out, text);
    for (chain, address) in &codes {
        out = with_qr(out, trust.verified(), chain, Some(address), a.qr.no_qr);
    }
    Ok(out)
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
    let net = net(chain, a.rpc.as_deref());
    let (owner, trust, read) =
        wallet::read::balance_detail(home, &key.name, &net, a.token.as_deref()).await?;
    let bal = read.balance;
    let warning = read.warning;
    let mut out = json!({
        "chain": chain.key,
        "key": key.name,
        "address": owner,
        "symbol": bal.symbol,
        "decimals": bal.decimals,
        "balance": format_units(bal.raw, bal.decimals),
        "raw": bal.raw.to_string(),
    });
    if let Some(active) = read.activated {
        out["activated"] = json!(active);
    }
    if let Some(token) = read.token {
        out["token"] = json!(token);
    }
    if let Some(source) = read.symbol_source {
        out["symbolSource"] = json!(source);
    }
    trust.mark(&mut out);
    // The account's own warning (a Tron owner permission held by other keys) after any about
    // what answered on the socket.
    if let Some(w) = warning {
        let joined = match out["warning"].as_str() {
            Some(first) => format!("{first}. {w}"),
            None => w,
        };
        out["warning"] = json!(joined);
    }
    Ok(with_qr(
        Out::json(out),
        trust.verified(),
        chain,
        Some(&owner),
        a.qr.no_qr,
    ))
}

/// `balance` without `--chain`: the native coin and every registry token on every chain,
/// mainnets first. The networks are read at once, each within [`wallet::read::NETWORK_TIMEOUT`], through
/// `ZWALLET_RPC_<CHAIN>` or the default RPC. A network whose endpoint, chain id or native balance
/// fails is a row with `error` and `errorCode` instead of `native` and `tokens`; a token whose
/// read fails is a token row with them instead of `balance` and `raw`. Only when no network
/// answers is it an error (their shared code, else `RPC_ERROR`; each network's error cut short
/// as its line shows it, and whole in the `networks` detail). While unlocked the address is the
/// daemon's derivation (once per chain family), checked against the cache before any RPC.
async fn every_balance(home: &Path, k: &KeyArg, no_qr: bool) -> Result<Out> {
    let file = store::read_wallet_file(home)?;
    let key = file.key(k.key.as_deref())?;
    let all = wallet::every_balance(home, &key.name).await?;
    let address = all.address;
    let trust = all.trust;
    let order: Vec<_> = all.networks.iter().map(|row| row.chain).collect();
    let mut rows = Vec::with_capacity(order.len());
    let mut lines = vec![format!("key: {}", key.name), format!("address: {address}")];
    let mut failed: Vec<(&str, WalletError)> = Vec::new();
    let mut held = 0;
    let mut failed_tokens = 0;
    for network in all.networks {
        let chain = network.chain;
        let mut row = json!({ "chain": chain.key, "chainId": chain.chain_id_json() });
        let mut own = String::new();
        if let Some(owner) = network.address
            && owner != address
        {
            own = format!(" ({owner})");
            row["address"] = json!(owner);
        }
        let result = network.balances;
        match result {
            Ok(wallet::read::NetworkRead {
                native,
                tokens,
                activated,
                warning,
            }) => {
                lines.push(format!("{:<10} {}{own}", chain.key, amount_text(&native)));
                if let Some(a) = activated {
                    row["activated"] = json!(a);
                }
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
                if let Some(w) = warning {
                    lines.push(format!("{:<10} warning: {w}", ""));
                    row["warning"] = json!(w);
                }
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
        // Each network's error cut short as its line shows it ([`brief`]): eight endpoints'
        // texts of up to 512 each would be one line of about 4,000 characters from outside.
        // `--json` keeps each whole, as a network's row does when another one answers, in
        // `networks`, a detail a human is not shown.
        let why: Vec<String> = failed
            .iter()
            .map(|(chain, e)| format!("{chain}: {}", brief(&e.message)))
            .collect();
        let networks: Vec<Value> = failed
            .iter()
            .map(|(chain, e)| {
                json!({ "chain": chain, "error": e.message, "errorCode": e.code.as_str() })
            })
            .collect();
        return Err(
            WalletError::new(code, format!("no network answered: {}", why.join("; ")))
                .with_detail("networks", networks),
        );
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
    // The code after the networks: the top-level address's (an EVM one; `balance --chain tron`
    // shows Tron's).
    Ok(with_qr(
        Out::with_text(out, lines.join("\n")),
        trust.verified(),
        primary_chain()?,
        Some(&address),
        no_qr,
    ))
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

/// One inert, bounded network error line, shared with the TUI.
fn brief(message: &str) -> String {
    wallet::brief(message)
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
    let mut input = wallet::CallInput {
        from: None,
        to: a.to,
        sig: a.sig,
        args: a.args,
    };
    input.validate()?;
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
    input.from = from.map(|f| f.to_checksum(None));
    let result = wallet::call(&net, &input).await?;
    let text = result.cli_text();
    Ok(Out::with_text(result.fields, text))
}

async fn invoke(home: &Path, a: InvokeArgs) -> Result<Out> {
    let chain = chains::find(&a.chain)?;
    chains::require_contract_calls(chain)?;
    let (to, nickname) = whitelist::recipient(home, &a.to)?;
    let to = evm::parse_address(&to)?;
    let (input, sig) = match (&a.sig, &a.data) {
        (Some(sig), None) => wallet::InvokeData::Signature {
            sig: sig.clone(),
            args: a.args.clone(),
        }
        .encode()?,
        (None, Some(data)) => {
            if !a.args.is_empty() {
                return Err(WalletError::validation(
                    "positional arguments need --sig; --data is the complete calldata",
                ));
            }
            wallet::InvokeData::Raw(data.clone()).encode()?
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
    send.max_fee_gwei
        .as_deref()
        .map(wallet::parse_fee_ceiling)
        .transpose()
}

async fn task(home: &Path, args: crate::cli::TaskCommand) -> Result<Out> {
    use crate::cli::TaskCommand;
    use crate::wallet::tasks;
    let value = match args {
        TaskCommand::Create {
            id,
            chain,
            key,
            allocation,
            rpc,
        } => {
            let chain = chains::find(&chain)?;
            let file = store::read_wallet_file(home)?;
            let key = file.key(key.key.as_deref())?.name.clone();
            let input = tasks::TaskCreate {
                id,
                key,
                allocations: tasks::parse_allocations(chain, &allocation)?,
            };
            serde_json::to_value(tasks::create(home, &net(chain, rpc.as_deref()), &input).await?)
        }
        TaskCommand::List => serde_json::to_value(tasks::list(home)?),
        TaskCommand::Report {
            id,
            offline,
            rpc,
            reveal_addresses,
        } => {
            let info = tasks::info(home, &id)?;
            let chain = chains::find(&info.chain)?;
            serde_json::to_value(
                tasks::report(
                    home,
                    &id,
                    &net(chain, rpc.as_deref()),
                    offline,
                    reveal_addresses,
                )
                .await?,
            )
        }
    }
    .map_err(|_| WalletError::validation("cannot encode the task report"))?;
    let text = serde_json::to_string_pretty(&value)
        .map_err(|_| WalletError::validation("cannot encode the task report"))?;
    Ok(Out::with_text(value, text))
}

async fn transfer(home: &Path, a: TransferArgs) -> Result<Out> {
    let chain = chains::find(&a.chain)?;
    let net = net(chain, a.rpc.as_deref());
    match &chain.family {
        Family::Evm(_) => transfer_with(&Evm, home, &net, a).await,
        Family::Tron(_) => {
            if a.send.nonce.is_some()
                || a.send.bump.is_some()
                || a.send.replace.is_some()
                || a.send.allow_pending
                || a.send.max_fee_gwei.is_some()
            {
                return Err(WalletError::validation(
                    "Tron has no nonce; --nonce, --replace, --bump, --allow-pending and --max-fee-gwei do not apply",
                ));
            }
            tron::send::validate_token(a.token.as_deref())?;
            transfer_with(&tron::Tron, home, &net, a).await
        }
    }
}

async fn transfer_with<F: ChainFamily>(
    fam: &F,
    home: &Path,
    net: &Net,
    a: TransferArgs,
) -> Result<Out> {
    let chain = net.chain;
    let (to, nickname) = whitelist::recipient_for_chain(home, &a.to, Some(chain))?;
    let to = fam.normalize_address(&to)?;
    if let Some(t) = &a.token {
        if matches!(chain.family, Family::Evm(_)) {
            fam.normalize_address(t)?;
        }
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

/// CLI adapter for the common signing, journaling and broadcast tail.
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
    lock: Option<chains::SendLock>,
    kind: &str,
) -> Result<Out> {
    if let Some(task) = &send.task {
        wallet::tasks::validate_binding(home, task, net.chain, from)?;
        out.insert("taskId".into(), json!(task));
    }
    if let Some(replaces) = &send.replace {
        out.insert("replaces".into(), json!(replaces));
    }
    let options = wallet::send::SendOptions {
        dry_run: send.dry_run,
        nonce: send.nonce,
        allow_duplicate: send.allow_duplicate,
        wait: send.wait,
        wait_timeout: send.wait_timeout,
        duplicate_window_secs: None,
    };
    let result = wallet::send::send_prepared(
        fam,
        net,
        home,
        index,
        from,
        prepared,
        &options,
        out,
        lock,
        kind,
        Some(std::sync::Arc::new(crate::core::terminal::eprint_line)),
    )
    .await?;
    Ok(send_out(result.fields, result.duplicates_text.as_deref()))
}

/// A send's result: `--json` as it is; the human text the same, but with `duplicateSends` as
/// `duplicates` (the journal's matching lines through [`chains::SendJournal::entries_text`]) when
/// there are any, so journal text reaches the terminal only in the shape zwallet writes.
fn send_out(out: Map<String, Value>, duplicates: Option<&str>) -> Out {
    let value = Value::Object(out);
    match duplicates {
        None => Out::json(value),
        Some(text) => {
            let mut human = value.clone();
            human["duplicateSends"] = json!(text);
            Out::with_text(value, render_human(&human))
        }
    }
}

/// `tx`: a transaction looked up by hash on the chain's node. Needs no wallet and no key, so it
/// works while locked; every message that says to look a send up names this command.
async fn tx(a: TxArgs) -> Result<Out> {
    let chain = chains::find(&a.chain)?;
    let net = net(chain, a.rpc.as_deref());
    let found = match &chain.family {
        Family::Evm(_) => {
            if a.wait {
                return Err(WalletError::validation(
                    "tx --wait is supported on Tron; EVM sends use transfer --wait",
                ));
            }
            Evm.lookup_tx(&net, &a.tx_hash).await?
        }
        Family::Tron(_) => {
            let hash = tron::lookup::parse_hash(&a.tx_hash)?;
            let expiration = home().ok().and_then(|home| {
                store::read_wallet_file(&home).ok().and_then(|file| {
                    file.keys
                        .iter()
                        .filter_map(|key| key.addresses.get("tron"))
                        .find_map(|address| {
                            chains::SendJournal::new(&home, chain, address).expiration(&hash)
                        })
                })
            });
            if a.wait {
                tron::lookup::wait(
                    &net,
                    &hash,
                    expiration,
                    a.wait_timeout.unwrap_or(wallet::send::DEFAULT_WAIT_TIMEOUT),
                )
                .await?
            } else {
                tron::lookup::lookup_tx(&net, &hash, expiration).await?
            }
        }
    };
    Ok(Out::json(Value::Object(found)))
}

async fn sign_message(home: &Path, a: SignMessageArgs) -> Result<Out> {
    let chain = chains::find(&a.chain)?;
    // Before the wallet is read: no Tron message is ever signed (`.prd/05-tron.md` §Non-goals).
    if let Family::Tron(_) = chain.family {
        return Err(unsupported(chain, "message signing"));
    }
    let file = store::read_wallet_file(home)?;
    let key = file.key(a.key.key.as_deref())?;
    let expected = key_address(home, key, chain).await?;
    Ok(Out::json(
        wallet::tools::sign_message_bytes(home, chain, key.index, &key.name, &expected, &a.message)
            .await?,
    ))
}

async fn status(home: &Path, no_qr: bool) -> Result<Out> {
    let status = wallet::status(home).await?;
    let mut out = json!({
        "wallet": if status.configured { "configured" } else { "not configured" },
        "locked": status.locked,
        "home": home.display().to_string(),
    });
    if !status.locked {
        out["daemonPids"] = json!(status.daemon_pids);
    }
    out["otherDaemonPids"] = json!(status.other_daemon_pids);
    if let Some(d) = &status.serving {
        out["daemonPid"] = json!(d.pid);
        out["expiresAt"] = json!(d.expires_at);
        // The whitelist as the serving daemon verifies it right now.
        out["whitelist"] = d.whitelist.clone();
    }
    if let Some(active) = &status.active {
        out["active"] = json!(active);
    }
    // The active key's address, and what verifies it.
    let shown = status.address;
    if let Some((address, trust)) = &shown {
        out["address"] = json!(address);
        trust.mark(&mut out);
    }
    let others = status.other_daemon_pids;
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
        Some((address, trust)) => with_qr(
            text,
            trust.verified(),
            primary_chain()?,
            address.as_deref(),
            no_qr,
        ),
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
pub(crate) async fn live_daemons(home: &Path) -> Result<(Vec<i32>, Option<Value>)> {
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
    // Fails before prompting when there is nothing to unlock.
    let plan = wallet::prepare_unlock(home, ttl, a.allow_ptrace_scope_0)?;
    let pw = read_password(a.password.password_stdin, "Wallet password: ", false)?;
    let session = wallet::unlock(&plan, &pw).await?;
    drop(pw);
    let ready = &session.ready;
    let mut out = json!({
        "locked": false,
        "duration": a.duration.trim(),
        "expiresAt": ready["expiresAt"],
        "active": ready["active"],
        "index": ready["index"],
        "address": ready["address"],
        "daemonPid": ready["pid"],
        // Derived by this process's own daemon child from the seed this password opened.
        "verified": true,
        "verifiedBy": BY_PASSWORD,
        // The whitelist this session signs under, as its daemon verified it: the generation and
        // the entries, so a rolled-back list is visible.
        "whitelist": session.whitelist,
    });
    if let Some(w) = session.warning {
        out["warning"] = json!(w);
    }
    Ok(with_qr(
        Out::json(out),
        true,
        primary_chain()?,
        ready["address"].as_str(),
        a.qr.no_qr,
    ))
}

/// [`index_warning`] (as `address --verify` gives it) for the key a new session reports: its
/// name and index as the daemon derived them, against `file`'s key list and `nextIndex`.
pub(crate) fn session_index_warning(file: &WalletFile, session: &Value) -> Option<String> {
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
    fn a_mismatch_blames_the_cache_only_for_a_derivation_from_the_seed() {
        let (cached, other) = (
            "0x1111111111111111111111111111111111111111",
            "0x2222222222222222222222222222222222222222",
        );
        // This process (password) or this build's key daemon: the cache was modified.
        let seed = mismatch_error(
            "main",
            "ethereum",
            cached,
            other,
            &Deriver::Seed("this process"),
        );
        assert!(
            seed.message.contains("derives from the seed"),
            "{}",
            seed.message
        );
        assert!(
            seed.message.contains("the cache was modified"),
            "{}",
            seed.message
        );
        // Whatever answers on the socket, not known to be this build's (Linux; a failed kernel
        // query): neutral, both addresses suspect, the password check decides; no relock advice.
        let unknown = mismatch_error("main", "ethereum", cached, other, &Deriver::Socket(None));
        let m = &unknown.message;
        assert_eq!(unknown.code, ErrorCode::ValidationError);
        assert!(
            !m.contains("derives from the seed") && !m.contains("cache was modified"),
            "{m}"
        );
        assert!(
            m.contains(&format!(
                "reported {other} as the ethereum address of key 'main', but wallet.json \
                 caches {cached}: one of them was tampered with"
            )) && m.contains("Do not send funds to either address")
                && m.contains("`zwallet address --verify`")
                && !m.contains("unlock again"),
            "{m}"
        );
        // Known to be another process: named, with the relock advice once.
        let other_proc = mismatch_error(
            "main",
            "ethereum",
            cached,
            other,
            &Deriver::Socket(Some(
                "the process answering on the key daemon socket (pid 7) is X",
            )),
        );
        let m = &other_proc.message;
        assert!(
            m.contains("(pid 7) is X; run `zwallet lock` and unlock again"),
            "{m}"
        );
        assert_eq!(m.matches("and unlock again").count(), 1, "{m}");
        // An answer that is no address is shown escaped.
        let evil = mismatch_error(
            "main",
            "ethereum",
            cached,
            "0x\u{1b}[2J\n",
            &Deriver::Socket(None),
        );
        assert!(!evil.message.contains('\u{1b}') && !evil.message.contains('\n'));
    }

    #[test]
    fn relock_failure_is_reported_not_swallowed() {
        let ok = wallet::relocked("mismatch".into(), Ok(vec![42]));
        assert!(
            ok.message.ends_with("so the wallet was locked again"),
            "{}",
            ok.message
        );
        let failed = wallet::relocked(
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
    fn the_cache_compares_tron_addresses_byte_for_byte_and_evm_ones_in_any_case() {
        const TWER: &str = "TWer2Ygk5TEheHp3TPuYeqxmB6SsGZmaL6";
        const EVM: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
        let mut k = key("main", 0);
        k.addresses.insert("tron".into(), TWER.into());
        k.addresses
            .insert("ethereum".into(), EVM.to_ascii_lowercase());
        assert_eq!(cache_mismatch(&k, "tron", TWER).unwrap(), None);
        // The comparison is `tron::address::same`: a case variant of the derived address is
        // another address (it never gets this far: the shape check refuses it first).
        assert!(!crate::chains::tron::address::same(
            TWER,
            &TWER.to_ascii_lowercase()
        ));
        assert_eq!(
            cache_mismatch(&k, "tron", "TPjjvMwjPoDC32V2dGDYTkLH4E5LAtBZ6C").unwrap(),
            Some(TWER.to_string())
        );
        // EVM hex is the same address in either case.
        assert_eq!(cache_mismatch(&k, "ethereum", EVM).unwrap(), None);
        // A Tron address is never the shape of an EVM one, nor the other way round.
        let tron = chains::find("tron").unwrap();
        let eth = chains::find("ethereum").unwrap();
        assert!(address_shaped(tron, TWER) && !address_shaped(eth, TWER));
        assert!(address_shaped(eth, EVM) && !address_shaped(tron, EVM));
        assert!(!address_shaped(tron, "tWer2Ygk5TEheHp3TPuYeqxmB6SsGZmaL6"));
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
