//! `zwallet whitelist …` and `@nickname` recipients (`.prd/04-whitelist.md` §Edits, §CLI).
//!
//! A change runs only with a typed password, and in this order:
//! 0. the arguments are checked, and the change is shown on the channel the password is read
//!    from (the terminal, or stderr with `--password-stdin`), so the human sees what the password
//!    authorizes; then the password is read and checked against wallet.json, and the change is
//!    tried on the list the edit would start from. A wrong password or a refused change stops
//!    nothing;
//! 1. every zwallet daemon of this user is stopped (the `zwallet lock` path, which returns only
//!    once none is left): nothing is signed while the list changes;
//! 2. to 4. [`whitelist::edit`] (or [`whitelist::reset`]) under `w.lock`: the verified base,
//!    the change at the next generation, signed, staged, re-sealed, renamed;
//! 5. the wallet stays locked, unless `--unlock <duration>` starts a new session for that
//!    duration with the password just typed (the staged unlock `zwallet unlock` uses). No
//!    duration or expiry is taken from a running daemon.

use super::start_session;
use crate::chains::evm::{self, registry::WHITELIST_RULES};
use crate::chains::{self, Chain, PRIMARY_CHAIN};
use crate::cli::{
    Out, Toggle, WhitelistAddArgs, WhitelistCommand, WhitelistEditArgs, WhitelistListArgs,
};
use crate::core::amount::{format_units, parse_units};
use crate::core::error::{Result, WalletError};
use crate::core::password::read_password;
use crate::core::protocol::{self, DaemonExe};
use crate::core::store::{self, WhitelistAnchor};
use crate::core::time::parse_duration;
use crate::core::whitelist::{
    self, ANY_CHAIN, Base, BaseSource, Edited, Entry, EntryKind, NEXT_FILE, WHITELIST_FILE,
    Whitelist,
};
use alloy::hex;
use alloy::primitives::U256;
use serde_json::{Value, json};
use std::path::Path;

const LIST_HINT: &str = "see `zwallet whitelist list`";
const FIRST_ADD: &str = "`zwallet whitelist add <nick> <addr> --chains <chain>` creates it";
/// What a contract entry delegates (`.prd/04-whitelist.md` §Entries).
const RESIDUAL: &str = "a listed contract may forward value or act on its arguments";
const VERIFY_NOTE: &str =
    "not verified: run `zwallet whitelist list --verify` to check the signature";

pub(super) async fn run(home: &Path, cmd: WhitelistCommand) -> Result<Out> {
    match cmd {
        WhitelistCommand::List(a) => list(home, a),
        WhitelistCommand::Add(a) => {
            let change = add_change(&a)?;
            edit(home, change, &a.edit).await
        }
        WhitelistCommand::Remove(a) => {
            whitelist::validate_nickname(&a.nickname)?;
            edit(home, Change::Remove(a.nickname), &a.edit).await
        }
        WhitelistCommand::FeeCap(a) => {
            let chain = chains::find(&a.chain)?;
            let bad = |why: String| WalletError::validation(format!("fee cap: {why}"));
            let wei = parse_units(&a.amount, chain.decimals).map_err(|e| bad(e.message))?;
            if wei.is_zero() {
                return Err(bad("must be greater than zero".into()));
            }
            edit(home, Change::FeeCap { chain, wei }, &a.edit).await
        }
        WhitelistCommand::AllowMessageSigning(a) => {
            edit(
                home,
                Change::MessageSigning(a.setting == Toggle::On),
                &a.edit,
            )
            .await
        }
        WhitelistCommand::Reset(a) => edit(home, Change::Reset, &a).await,
    }
}

/// `--to` of `transfer` and `invoke`: an address as given, or `@nickname` looked up in
/// whitelist.json as it is on disk. Unverified on purpose: it only saves typing the address; the
/// key daemon decides whether a send there is allowed.
pub(super) fn recipient(home: &Path, to: &str) -> Result<(String, Option<String>)> {
    let Some(nickname) = to.trim().strip_prefix('@') else {
        return Ok((to.to_string(), None));
    };
    let unresolved = |why: &str| {
        WalletError::validation(format!("cannot resolve '@{nickname}': {why}; {LIST_HINT}"))
    };
    whitelist::validate_nickname(nickname).map_err(|e| unresolved(&e.message))?;
    let signed = whitelist::read_list(&home.join(WHITELIST_FILE))
        .map_err(|e| unresolved(&e.message))?
        .ok_or_else(|| unresolved("there is no whitelist yet"))?;
    let entry = signed
        .list
        .by_nickname(nickname)
        .ok_or_else(|| unresolved("the whitelist has no entry by that name"))?;
    Ok((entry.address.clone(), Some(nickname.to_string())))
}

/// One password-backed change.
enum Change {
    Add(Entry),
    Remove(String),
    FeeCap {
        chain: &'static Chain,
        wei: U256,
    },
    MessageSigning(bool),
    /// A new key and an empty list ([`whitelist::reset`]).
    Reset,
}

impl Change {
    fn name(&self) -> &'static str {
        match self {
            Self::Add(_) => "added",
            Self::Remove(_) => "removed",
            Self::FeeCap { .. } => "fee-cap",
            Self::MessageSigning(_) => "allow-message-signing",
            Self::Reset => "reset",
        }
    }

    /// The change on `list` (a reset starts over instead, see [`whitelist::reset`]).
    fn apply(&self, list: &mut Whitelist) -> Result<()> {
        match self {
            Self::Add(entry) => list.add(entry.clone()),
            Self::Remove(nickname) => list.remove(nickname).map(drop).ok_or_else(|| {
                WalletError::validation(format!(
                    "the whitelist has no entry '{nickname}'; {LIST_HINT}"
                ))
            }),
            Self::FeeCap { chain, wei } => {
                list.policy.set_max_fee_wei(chain.key, *wei);
                Ok(())
            }
            Self::MessageSigning(on) => {
                list.policy.allow_message_signing = *on;
                Ok(())
            }
            Self::Reset => Ok(()),
        }
    }
}

/// `whitelist add`: the entry, checked as the list will check it before anything is asked or
/// stopped. `--chains` defaults to the default chain ([`PRIMARY_CHAIN`]); `"*"` only when given.
fn add_change(a: &WhitelistAddArgs) -> Result<Change> {
    whitelist::validate_nickname(&a.nickname)?;
    let address = format!("0x{}", hex::encode(evm::parse_address(&a.address)?));
    let keys: Vec<String> = if a.chains.is_empty() {
        vec![PRIMARY_CHAIN.to_string()]
    } else {
        a.chains
            .iter()
            .map(|c| match c.trim() {
                ANY_CHAIN => Ok(ANY_CHAIN.to_string()),
                key => chains::find(key).map(|chain| chain.key.to_string()),
            })
            .collect::<Result<_>>()?
    };
    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    let entry = if a.contract {
        let selectors: Vec<&str> = a.selectors.iter().map(String::as_str).collect();
        Entry::new_contract(&a.nickname, &address, &keys, &selectors)
    } else {
        Entry::new_address(&a.nickname, &address, &keys)
    };
    // Format, chains, "*", selectors, and no registry token as a contract entry.
    let mut alone = Whitelist::empty(&WHITELIST_RULES);
    alone.generation = 1;
    alone.add(entry.clone())?;
    alone.validate(&WHITELIST_RULES)?;
    Ok(Change::Add(entry))
}

/// `change` tried on the list an edit would start from, so that one the list refuses (a
/// nickname or address already listed, an unknown nickname to remove) stops nothing.
fn try_change(change: &Change, base: &Base) -> Result<()> {
    let mut trial = base.list.clone();
    let tried = change.apply(&mut trial).and_then(|()| {
        trial.generation = base.list.generation.saturating_add(1);
        trial.validate(&WHITELIST_RULES)
    });
    tried.map_err(|e| match &base.discarded {
        Some(d) => WalletError::new(
            e.code,
            format!(
                "{}. The edit would start from an empty list: {}",
                e.message, d.reason
            ),
        ),
        None => e,
    })
}

/// Tells the human to start a new session after a change that left the wallet locked.
const UNLOCK_HINT: &str = "pass --unlock <duration> to keep signing";

/// Step 5 with `--unlock`.
enum Session {
    /// No `--unlock`: the wallet is locked.
    Locked,
    /// Unlocked for `duration` with the password just typed: the daemon's readiness report.
    Unlocked { duration: String, ready: Value },
    /// Unlocking failed after the change: the wallet is locked.
    Failed(WalletError),
}

impl Session {
    /// Fills `locked` and the session fields of a change's output.
    fn report(&self, out: &mut Value) {
        match self {
            Self::Locked => {
                out["locked"] = json!(true);
                out["hint"] = json!(UNLOCK_HINT);
            }
            Self::Unlocked { duration, ready } => {
                out["locked"] = json!(false);
                out["duration"] = json!(duration);
                out["expiresAt"] = ready["expiresAt"].clone();
                out["daemonPid"] = ready["pid"].clone();
                // The list the new session signs under, as its daemon verified it.
                out["whitelist"] = ready["whitelist"].clone();
            }
            Self::Failed(e) => {
                out["locked"] = json!(true);
                out["unlockError"] = json!(e.message);
                out["hint"] = json!(UNLOCK_HINT);
            }
        }
    }
}

/// One change, linearized as the module doc says.
async fn edit(home: &Path, change: Change, opts: &WhitelistEditArgs) -> Result<Out> {
    store::read_wallet_file(home)?;
    // `unlock`'s own checks, before the prompt and before anything changes. The binary is pinned
    // here, as `unlock` pins it: one swapped in while the human types is never started.
    let unlock = match opts.unlock.as_deref() {
        None => None,
        Some(d) => {
            let ttl = parse_duration(d)?;
            protocol::socket_path(home)?;
            protocol::check_ptrace_scope(opts.allow_ptrace_scope_0)?;
            protocol::check_daemons_findable()?;
            Some((d.trim().to_string(), ttl, DaemonExe::resolve()?))
        }
    };
    // 0. What the password authorizes, where the password is read.
    let shown = summary(home, &change);
    let password = if opts.password.password_stdin {
        eprintln!("{shown}");
        read_password(true, "", false)?
    } else {
        read_password(false, &format!("{shown}\nWallet password: "), false)?
    };
    // A wrong password is refused here, before anything is stopped.
    let anchor = store::unlock(home, &password, &chains::derive_addresses)?
        .0
        .whitelist_anchor()?;
    let base = whitelist::pick_base(home, anchor.as_ref(), &WHITELIST_RULES);
    if !matches!(change, Change::Reset) {
        try_change(&change, &base)?;
    }
    // 1.
    let stopped = protocol::stop_all_daemons()?;
    // 2. to 4.
    let edited = match &change {
        Change::Reset => whitelist::reset(home, &password, &WHITELIST_RULES),
        _ => whitelist::edit(home, &password, &WHITELIST_RULES, |list| change.apply(list)),
    }
    .map_err(|e| {
        WalletError::new(
            e.code,
            format!(
                "{}; every zwallet daemon was stopped before the edit, so the wallet is locked",
                e.message
            ),
        )
        .with_detail("locked", true)
    })?;
    // 5.
    let session = match unlock {
        None => Session::Locked,
        Some((duration, ttl, exe)) => match start_session(home, &exe, &password, ttl).await {
            Ok(ready) => Session::Unlocked { duration, ready },
            Err(e) => Session::Failed(e),
        },
    };
    drop(password);
    let out = edit_output(&change, &base, &edited, &stopped, &session);
    let text = edit_text(&out);
    Ok(Out::with_text(out, text))
}

/// Step 0: the change the password is about to authorize, from the arguments and whitelist.json
/// as it is on disk. That file is unverified here (only the password opens the seal that says
/// whether it is the signed list), so the text says what happens if it is not. Every value read
/// from the file is escaped ([`shown`], [`printable`]).
fn summary(home: &Path, change: &Change) -> String {
    let current = whitelist::read_list(&home.join(WHITELIST_FILE));
    let on_disk = match &current {
        Ok(Some(s)) => Some(&s.list),
        _ => None,
    };
    let mut lines = vec!["The password authorizes this whitelist change:".to_string()];
    match change {
        Change::Add(e) => {
            lines.push(format!("  add {}: {}", e.nickname, kind_text(e.kind)));
            lines.push(format!("    address: {}", checksummed(&e.address)));
            lines.push(format!("    chains: {}", chains_text(&e.chains)));
            if let Some(sels) = &e.selectors {
                lines.push(format!("    selectors: {}", sels.join(", ")));
            }
        }
        Change::Remove(nickname) => {
            let what = match on_disk.and_then(|l| l.by_nickname(nickname)) {
                Some(e) => format!(
                    "whitelist.json lists it as {} {} on {}",
                    match e.kind {
                        EntryKind::Address => "a payee",
                        EntryKind::Contract => "a contract",
                    },
                    checksummed(&e.address),
                    chains_text(&e.chains)
                ),
                None => "whitelist.json has no entry by that name".to_string(),
            };
            lines.push(format!("  remove {nickname}: {what}"));
        }
        Change::FeeCap { chain, wei } => {
            let old = match &current {
                Ok(Some(s)) => s.list.policy.max_fee_wei.get(chain.key).cloned(),
                // A first edit starts from the default caps.
                Ok(None) => WHITELIST_RULES
                    .default_fee_caps_wei
                    .iter()
                    .find(|(k, _)| *k == chain.key)
                    .map(|(_, v)| v.to_string()),
                Err(_) => None,
            };
            let old = match old {
                Some(v) => match U256::from_str_radix(&v, 10) {
                    Ok(w) => fee_text(chain, w),
                    Err(_) => format!("{} (unreadable)", shown(&v)),
                },
                None => "none".to_string(),
            };
            lines.push(format!(
                "  fee-cap {}: {old} -> {}",
                chain.key,
                fee_text(chain, *wei)
            ));
        }
        Change::MessageSigning(on) => {
            let old = match &current {
                Ok(Some(s)) => on_off(s.list.policy.allow_message_signing),
                Ok(None) => on_off(false),
                Err(_) => "unknown",
            };
            lines.push(format!(
                "  allow-message-signing: {old} -> {} (some contracts take an EIP-191 signature \
                 as spending authority)",
                on_off(*on)
            ));
        }
        Change::Reset => {
            let dropped = on_disk.map(list_names).unwrap_or_default();
            lines.push(format!(
                "  reset: a new whitelist key and an empty list with the default fee caps and \
                 message signing off; every entry is dropped: {}",
                if dropped.is_empty() { "none" } else { &dropped }
            ));
        }
    }
    match &current {
        Ok(Some(s)) => {
            let n = s.list.entries.len();
            lines.push(format!(
                "  whitelist.json now: gen {}, {n} {} ({}), not verified yet; if it is not the \
                 sealed list, the change starts from an empty list instead",
                s.list.generation,
                if n == 1 { "entry" } else { "entries" },
                list_names(&s.list)
            ));
            lines.push(format!(
                "  after the change: gen {}",
                s.list.generation.saturating_add(1)
            ));
        }
        Ok(None) => {
            lines.push("  whitelist.json: none yet".to_string());
            lines.push(
                "  after the change: gen 1 with a new whitelist key (the sealed generation + 1 \
                 if wallet.json already seals a whitelist)"
                    .to_string(),
            );
        }
        Err(e) => {
            lines.push(format!(
                "  whitelist.json: unreadable ({}); the change starts from an empty list",
                printable(&e.message)
            ));
            lines.push("  after the change: the sealed generation + 1".to_string());
        }
    }
    lines.join("\n")
}

fn kind_text(kind: EntryKind) -> String {
    match kind {
        EntryKind::Address => {
            "a payee (native coin and registry tokens may be sent to it)".to_string()
        }
        EntryKind::Contract => format!("a contract (only calls with its selectors; {RESIDUAL})"),
    }
}

/// An address for a human: EIP-55, or escaped as found when it is not one.
fn checksummed(address: &str) -> String {
    evm::parse_address(address)
        .map(|a| a.to_checksum(None))
        .unwrap_or_else(|_| shown(address))
}

/// `["*"]` spelled out; chain keys otherwise.
fn chains_text(chains: &[String]) -> String {
    if chains.len() == 1 && chains[0] == ANY_CHAIN {
        return "every chain (*)".to_string();
    }
    chains
        .iter()
        .map(|c| shown(c))
        .collect::<Vec<_>>()
        .join(", ")
}

fn fee_text(chain: &Chain, wei: U256) -> String {
    format!(
        "{} {} ({wei} wei)",
        format_units(wei, chain.decimals),
        chain.symbol
    )
}

fn on_off(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

/// A list's nicknames, each [`shown`], comma-separated.
fn list_names(list: &Whitelist) -> String {
    list.entries
        .iter()
        .map(|e| shown(&e.nickname))
        .collect::<Vec<_>>()
        .join(", ")
}

fn base_name(source: BaseSource) -> &'static str {
    match source {
        BaseSource::Current => "current",
        BaseSource::PromotedNext => "promoted-next",
        BaseSource::Empty => "empty",
    }
}

fn nicknames(list: &Whitelist) -> Vec<&str> {
    list.entries.iter().map(|e| e.nickname.as_str()).collect()
}

/// `whitelist add|remove|fee-cap|allow-message-signing|reset` output: the list now in
/// whitelist.json (verified against the new seal), the change, and the session.
fn edit_output(
    change: &Change,
    base: &Base,
    edited: &Edited,
    stopped: &[i32],
    session: &Session,
) -> Value {
    let list = &edited.list;
    let mut out = json!({
        "change": change.name(),
        "gen": list.generation,
        "previousGen": edited.previous_generation,
        "base": base_name(edited.base),
        "keyCreated": edited.key_created,
        "entries": nicknames(list),
        "verified": true,
        "stoppedDaemons": stopped,
    });
    match change {
        Change::Add(entry) => {
            let signed = list.by_nickname(&entry.nickname).unwrap_or(entry);
            out["entry"] = json!(signed);
            if signed.kind == EntryKind::Contract {
                out["warning"] = json!(RESIDUAL);
            }
        }
        Change::Remove(nickname) => {
            out["entry"] = json!(base.list.by_nickname(nickname));
        }
        Change::FeeCap { chain, wei } => {
            out["feeCap"] = json!({
                "chain": chain.key,
                "maxFeeWei": wei.to_string(),
                "maxFee": format!("{} {}", format_units(*wei, chain.decimals), chain.symbol),
            });
        }
        Change::MessageSigning(on) => out["allowMessageSigning"] = json!(on),
        Change::Reset => out["dropped"] = json!(nicknames(&base.list)),
    }
    let discarded = edited.discarded.as_ref().or(match change {
        Change::Reset => base.discarded.as_ref(),
        _ => None,
    });
    if let Some(d) = discarded {
        out["discarded"] = json!({ "reason": d.reason, "entries": d.entries });
    }
    session.report(&mut out);
    out
}

fn edit_text(out: &Value) -> String {
    let s = |k: &str| match &out[k] {
        Value::String(s) => s.clone(),
        Value::Null => "-".into(),
        v => v.to_string(),
    };
    let what = match out["change"].as_str().unwrap_or("") {
        "added" | "removed" => {
            let e = &out["entry"];
            let mut what = format!(
                "{} {} ({} {} on {})",
                s("change"),
                shown(e["nickname"].as_str().unwrap_or("?")),
                shown(e["kind"].as_str().unwrap_or("?")),
                shown(e["address"].as_str().unwrap_or("?")),
                join(&e["chains"])
            );
            if e["selectors"].is_array() {
                what.push_str(&format!(", selectors {}", join(&e["selectors"])));
            }
            what
        }
        "fee-cap" => format!(
            "fee cap on {} set to {}",
            out["feeCap"]["chain"].as_str().unwrap_or("?"),
            out["feeCap"]["maxFee"].as_str().unwrap_or("?")
        ),
        "allow-message-signing" => format!(
            "message signing {}",
            if out["allowMessageSigning"] == json!(true) {
                "allowed"
            } else {
                "refused"
            }
        ),
        _ => format!("reset: a new key and an empty list (dropped {})", {
            let d = join(&out["dropped"]);
            if d.is_empty() { "nothing".into() } else { d }
        }),
    };
    let mut lines = vec![
        format!(
            "whitelist gen {} ({}): {what}",
            s("gen"),
            match out["previousGen"].as_u64() {
                Some(g) => format!("was {g}"),
                None => "the first".into(),
            }
        ),
        format!("entries: {}", join(&out["entries"])),
    ];
    if let Some(w) = out["warning"].as_str() {
        lines.push(format!("warning: {w}"));
    }
    if out["discarded"].is_object() {
        lines.push(format!(
            "discarded (not signed, the edit started from an empty list): {}; its unverified \
             entries: {}",
            printable(out["discarded"]["reason"].as_str().unwrap_or("?")),
            ascii_json(&out["discarded"]["entries"])
        ));
    }
    if out["keyCreated"] == json!(true) {
        lines.push(format!("created a new {}", whitelist::KEY_FILE));
    }
    if out["stoppedDaemons"]
        .as_array()
        .is_some_and(|p| !p.is_empty())
    {
        lines.push(format!("stopped daemons: {}", join(&out["stoppedDaemons"])));
    }
    if out["locked"] == json!(false) {
        lines.push(format!(
            "unlocked for {} until {} (daemon {}); it signs under whitelist gen {}",
            s("duration"),
            s("expiresAt"),
            s("daemonPid"),
            ascii_json(&out["whitelist"]["gen"])
        ));
    } else {
        if let Some(e) = out["unlockError"].as_str() {
            lines.push(format!("unlocking failed: {}", printable(e)));
        }
        lines.push(format!("locked: {UNLOCK_HINT}"));
    }
    lines.join("\n")
}

/// A JSON array's items, comma-separated, each [`shown`] (or as JSON if not a string).
fn join(v: &Value) -> String {
    v.as_array()
        .map(|items| {
            items
                .iter()
                .map(|i| match i {
                    Value::String(s) => shown(s),
                    other => ascii_json(other),
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

/// The most characters of one value from a file that human output shows.
const SHOWN_CHARS: usize = 64;

/// A value from a file (a hand-edited whitelist.json is anything) for a terminal: as is when it
/// is at most [`SHOWN_CHARS`] of printable ASCII, else quoted with escapes (`{:?}`, which escapes
/// control and format characters such as ESC and U+202E) and cut there.
fn shown(value: &str) -> String {
    if !value.is_empty()
        && value.len() <= SHOWN_CHARS
        && value.bytes().all(|b| b.is_ascii_graphic())
    {
        return value.to_string();
    }
    match value.char_indices().nth(SHOWN_CHARS) {
        Some((cut, _)) => format!("{:?}…", &value[..cut]),
        None => format!("{value:?}"),
    }
}

/// Free text that may quote a file (a reason) for a terminal: every character that is not
/// printable as it is (control and format characters) escaped as `{:?}` would, quotes and
/// backslashes left as they are.
fn printable(text: &str) -> String {
    text.chars()
        .flat_map(|c| {
            let escaped: Vec<char> = if matches!(c, '"' | '\\' | '\'') {
                vec![c]
            } else {
                c.escape_debug().collect()
            };
            escaped
        })
        .collect()
}

/// `value` as compact JSON in printable ASCII: every other character as a `\u` escape, which
/// is still the same JSON.
fn ascii_json(value: &Value) -> String {
    let mut out = String::new();
    for c in value.to_string().chars() {
        if c == ' ' || c.is_ascii_graphic() {
            out.push(c);
        } else {
            let mut units = [0u16; 2];
            for unit in c.encode_utf16(&mut units) {
                out.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    out
}

/// `whitelist list`: whitelist.json as it is, `verified: false`; with `--verify`, the list the
/// seal names (whitelist.json, or a staged whitelist.next.json an interrupted edit left behind)
/// with `verified: true`, or why there is none.
fn list(home: &Path, a: WhitelistListArgs) -> Result<Out> {
    store::read_wallet_file(home)?;
    let out = if a.verify {
        let password = read_password(a.password_stdin, "Wallet password: ", false)?;
        let anchor = store::unlock(home, &password, &chains::derive_addresses)?
            .0
            .whitelist_anchor();
        drop(password);
        verified_view(home, anchor)
    } else {
        unverified_view(home, None)
    };
    let text = list_text(&out, a.verify);
    Ok(Out::with_text(out, text))
}

fn verified_view(home: &Path, anchor: Result<Option<WhitelistAnchor>>) -> Value {
    let anchor = match anchor {
        Err(e) => return unverified_view(home, Some(e.message)),
        Ok(None) => {
            return unverified_view(
                home,
                Some(format!(
                    "wallet.json seals no whitelist key yet; {FIRST_ADD}"
                )),
            );
        }
        Ok(Some(a)) => a,
    };
    let base = whitelist::pick_base(home, Some(&anchor), &WHITELIST_RULES);
    let mut out = match base.source {
        BaseSource::Current => list_view(&base.list, true, WHITELIST_FILE),
        BaseSource::PromotedNext => list_view(&base.list, true, NEXT_FILE),
        BaseSource::Empty => unverified_view(home, base.discarded.map(|d| d.reason)),
    };
    out["sealedGen"] = json!(anchor.generation);
    out
}

fn list_view(list: &Whitelist, verified: bool, source: &str) -> Value {
    json!({
        "gen": list.generation,
        "entries": list.entries,
        "policy": list.policy,
        "verified": verified,
        "source": source,
    })
}

/// whitelist.json as it is on disk, with `reason` (why it is not verified) when there is one.
fn unverified_view(home: &Path, reason: Option<String>) -> Value {
    let path = home.join(WHITELIST_FILE);
    let mut out = match whitelist::read_list(&path) {
        Ok(Some(signed)) => list_view(&signed.list, false, WHITELIST_FILE),
        Ok(None) => json!({
            "gen": null,
            "entries": [],
            "policy": null,
            "verified": false,
            "reason": format!("there is no whitelist yet; {FIRST_ADD}"),
        }),
        Err(e) => {
            // Unreadable as a list: whatever entries it shows, for the human to look at.
            let raw = store::read_wallet_bytes(&path)
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .unwrap_or(Value::Null);
            json!({
                "gen": raw.get("gen"),
                "entries": raw.get("entries"),
                "policy": null,
                "verified": false,
                "source": WHITELIST_FILE,
                "reason": e.message,
            })
        }
    };
    if let Some(r) = reason {
        out["reason"] = json!(r);
    }
    out
}

fn list_text(out: &Value, verify: bool) -> String {
    let mut lines = vec![format!(
        "whitelist gen {}, {}{}",
        match &out["gen"] {
            Value::Null => "-".to_string(),
            g => ascii_json(g),
        },
        if out["verified"] == json!(true) {
            "verified"
        } else {
            "not verified"
        },
        match (out["source"].as_str(), out["sealedGen"].as_u64()) {
            (Some(src), Some(g)) => format!(" ({src}; sealed gen {g})"),
            (Some(src), None) => format!(" ({src})"),
            (None, Some(g)) => format!(" (sealed gen {g})"),
            (None, None) => String::new(),
        }
    )];
    for e in out["entries"].as_array().into_iter().flatten() {
        if !e.is_object() {
            lines.push(format!("  {}", ascii_json(e)));
            continue;
        }
        let field = |k: &str| match &e[k] {
            Value::String(s) => shown(s),
            other => ascii_json(other),
        };
        let mut line = format!(
            "  {:<16} {:<8} {} {}",
            field("nickname"),
            field("kind"),
            field("address"),
            join(&e["chains"])
        );
        if e["selectors"].is_array() {
            line.push_str(&format!("  selectors {}", join(&e["selectors"])));
        }
        lines.push(line);
    }
    if let Some(caps) = out["policy"]["maxFeeWei"].as_object() {
        let caps: Vec<String> = caps
            .iter()
            .map(|(key, wei)| {
                let wei = wei.as_str().unwrap_or("?");
                match (chains::find(key), U256::from_str_radix(wei, 10)) {
                    (Ok(c), Ok(w)) => {
                        format!("{} {} {}", c.key, format_units(w, c.decimals), c.symbol)
                    }
                    _ => format!("{} {} wei", shown(key), shown(wei)),
                }
            })
            .collect();
        lines.push(format!("fee caps: {}", caps.join(", ")));
        lines.push(format!(
            "message signing: {}",
            if out["policy"]["allowMessageSigning"] == json!(true) {
                "on"
            } else {
                "off"
            }
        ));
    }
    if let Some(r) = out["reason"].as_str() {
        lines.push(format!("reason: {}", printable(r)));
    }
    if !verify {
        lines.push(VERIFY_NOTE.to_string());
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::whitelist::WhitelistKey;
    use std::os::unix::fs::OpenOptionsExt;

    fn home_with(entries: &[Entry]) -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let mut list = Whitelist::empty(&WHITELIST_RULES);
        list.generation = 1;
        for e in entries {
            list.add(e.clone()).unwrap();
        }
        let signed = WhitelistKey::generate().unwrap().sign(&list);
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(tmp.path().join(WHITELIST_FILE))
            .unwrap();
        std::io::Write::write_all(&mut f, &signed.to_file_bytes().unwrap()).unwrap();
        tmp
    }

    #[test]
    fn at_nickname_resolves_to_the_entry_address_and_addresses_pass_through() {
        let alice = "0x5aaeb6053f3e94c9b9a09f33669435e7ef1beaed";
        let router = "0x2222222222222222222222222222222222222222";
        let tmp = home_with(&[
            Entry::new_address("alice", alice, &["base"]),
            Entry::new_contract("router", router, &["base"], &["0xd0e30db0"]),
        ]);
        let home = tmp.path();
        assert_eq!(
            recipient(home, "@alice").unwrap(),
            (alice.to_string(), Some("alice".to_string()))
        );
        assert_eq!(
            recipient(home, " @router").unwrap(),
            (router.to_string(), Some("router".to_string()))
        );
        // Anything without '@' is left for the address parser.
        assert_eq!(
            recipient(home, "0xAbC").unwrap(),
            ("0xAbC".to_string(), None)
        );
        for to in ["@bob", "@Alice", "@", "@a b"] {
            let e = recipient(home, to).unwrap_err();
            assert_eq!(e.code, crate::core::error::ErrorCode::ValidationError);
            assert!(e.message.contains("zwallet whitelist list"), "{to}: {e}");
        }
        let empty = tempfile::tempdir().unwrap();
        let e = recipient(empty.path(), "@alice").unwrap_err();
        assert!(e.message.contains("no whitelist yet"), "{e}");
    }

    #[test]
    fn a_change_the_current_list_refuses_says_where_the_edit_would_start() {
        let mut list = Whitelist::empty(&WHITELIST_RULES);
        list.generation = 3;
        list.add(Entry::new_address(
            "alice",
            "0x1111111111111111111111111111111111111111",
            &["base"],
        ))
        .unwrap();
        let base = Base {
            list,
            source: BaseSource::Current,
            discarded: None,
        };
        try_change(&Change::Remove("alice".into()), &base).unwrap();
        let e = try_change(&Change::Remove("bob".into()), &base).unwrap_err();
        assert!(e.message.contains("no entry 'bob'"), "{e}");
        let dup = Entry::new_address(
            "alice",
            "0x2222222222222222222222222222222222222222",
            &["base"],
        );
        try_change(&Change::Add(dup), &base).unwrap_err();
        let unverified = Base {
            list: Whitelist::empty(&WHITELIST_RULES),
            source: BaseSource::Empty,
            discarded: Some(whitelist::Discarded {
                reason: "whitelist.json: signature does not verify".into(),
                entries: Value::Null,
            }),
        };
        let e = try_change(&Change::Remove("alice".into()), &unverified).unwrap_err();
        assert!(
            e.message.contains("start from an empty list")
                && e.message.contains("signature does not verify"),
            "{e}"
        );
    }

    #[test]
    fn file_values_reach_human_output_escaped() {
        assert_eq!(shown("alice"), "alice");
        assert_eq!(shown("mallory\u{1b}[2J"), "\"mallory\\u{1b}[2J\"");
        assert_eq!(shown("a\u{202e}b"), "\"a\\u{202e}b\"");
        assert_eq!(shown(""), "\"\"");
        let long = "x".repeat(SHOWN_CHARS + 1);
        assert_eq!(shown(&long), format!("{:?}…", &long[..SHOWN_CHARS]));
        assert_eq!(
            printable("entry \"m\u{1b}\": bad \u{202e}"),
            "entry \"m\\u{1b}\": bad \\u{202e}"
        );
        let raw = json!([{ "nickname": "m\u{1b}\u{202e}é" }]);
        let text = ascii_json(&raw);
        assert!(
            text.is_ascii() && !text.chars().any(|c| c.is_ascii_control()),
            "{text}"
        );
        assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), raw);
        // An edit that discarded such a list says so in printable ASCII only.
        let out = json!({
            "change": "added", "gen": 2, "previousGen": 1, "entries": ["bob"],
            "entry": { "nickname": "bob", "kind": "address", "address": "0xb0b", "chains": ["base"] },
            "discarded": { "reason": "bad \u{202e}", "entries": raw },
            "keyCreated": false, "stoppedDaemons": [], "locked": true,
        });
        let text = edit_text(&out);
        assert!(
            !text.contains('\u{1b}') && !text.contains('\u{202e}'),
            "{text:?}"
        );
        assert!(text.contains("discarded"), "{text}");
    }

    #[test]
    fn a_change_reports_locked_unless_a_new_session_started() {
        let mut out = json!({});
        Session::Locked.report(&mut out);
        assert_eq!(out, json!({ "locked": true, "hint": UNLOCK_HINT }));
        let mut out = json!({});
        Session::Failed(WalletError::validation("daemon refused")).report(&mut out);
        assert_eq!(out["locked"], true);
        assert_eq!(out["unlockError"], "daemon refused");
        assert!(edit_text(&json!({ "change": "reset", "entries": [], "locked": true, "unlockError": "x\u{1b}" })).contains("locked: pass --unlock"));
        let ready = json!({
            "expiresAt": "2030-01-01T00:00:00Z",
            "pid": 9,
            "whitelist": { "gen": 4, "verified": true, "entries": ["alice"] },
        });
        let mut out = json!({});
        Session::Unlocked {
            duration: "24h".into(),
            ready: ready.clone(),
        }
        .report(&mut out);
        assert_eq!(
            out,
            json!({
                "locked": false,
                "duration": "24h",
                "expiresAt": "2030-01-01T00:00:00Z",
                "daemonPid": 9,
                "whitelist": ready["whitelist"],
            })
        );
    }

    #[test]
    fn the_summary_says_what_the_password_authorizes() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let base = chains::find("base").unwrap();
        let alice = Entry::new_address(
            "alice",
            "0x5aaeb6053f3e94c9b9a09f33669435e7ef1beaed",
            &["*"],
        );
        let text = summary(home, &Change::Add(alice));
        for want in [
            "The password authorizes this whitelist change:",
            "add alice: a payee",
            "address: 0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed",
            "chains: every chain (*)",
            "whitelist.json: none yet",
            "after the change: gen 1",
        ] {
            assert!(text.contains(want), "{want:?} not in {text}");
        }
        let cap = Change::FeeCap {
            chain: base,
            wei: U256::from(1_000_000_000_000_000u64),
        };
        let text = summary(home, &cap);
        assert!(
            text.contains("fee-cap base: 0.0005 ETH (500000000000000 wei) -> 0.001 ETH"),
            "{text}"
        );
        let tmp = home_with(&[Entry::new_address(
            "bob",
            "0x3333333333333333333333333333333333333333",
            &["base"],
        )]);
        let text = summary(tmp.path(), &Change::MessageSigning(true));
        assert!(text.contains("allow-message-signing: off -> on"), "{text}");
        assert!(
            text.contains("now: gen 1, 1 entry (bob), not verified yet"),
            "{text}"
        );
        assert!(text.contains("after the change: gen 2"), "{text}");
        let text = summary(tmp.path(), &Change::Reset);
        assert!(text.contains("every entry is dropped: bob"), "{text}");
        let text = summary(tmp.path(), &Change::Remove("bob".into()));
        assert!(
            text.contains("remove bob: whitelist.json lists it as a payee 0x3333333333333333333333333333333333333333 on base"),
            "{text}"
        );
        // An unreadable file: its reason, escaped.
        std::fs::write(tmp.path().join(WHITELIST_FILE), "{\"x\u{1b}\": 1}").unwrap();
        let text = summary(tmp.path(), &Change::Reset);
        assert!(text.contains("whitelist.json: unreadable"), "{text}");
        assert!(!text.contains('\u{1b}'), "{text:?}");
    }
}
