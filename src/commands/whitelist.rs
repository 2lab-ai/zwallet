//! `zwallet whitelist …` and `@nickname` recipients (`.prd/04-whitelist.md` §Edits, §CLI).
//!
//! A change runs only with a typed password, and in this order:
//! 1. remember the running session's `expiresAt`, then stop every zwallet daemon of this user
//!    (the `zwallet lock` path, which returns only once none is left): nothing is signed while
//!    the list changes;
//! 2. to 4. [`whitelist::edit`] (or [`whitelist::reset`]) under `w.lock`: the verified base,
//!    the change at the next generation, signed, staged, re-sealed, renamed;
//! 5. re-warm: unlock again with the same password until the remembered `expiresAt` (not for a
//!    fresh duration) through the staged unlock, then wipe the password. Without a session, or
//!    once it has run out, the wallet stays locked.
//!
//! Before step 1 the password is checked against wallet.json and the change is tried on the
//! current list, so a typo or a refused change leaves a running session alone.

use super::{live_daemons, start_session};
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
use crate::core::time::{parse_rfc3339_utc, rfc3339_utc, unix_secs};
use crate::core::whitelist::{
    self, ANY_CHAIN, Base, BaseSource, Edited, Entry, EntryKind, NEXT_FILE, WHITELIST_FILE,
    Whitelist,
};
use alloy::hex;
use alloy::primitives::U256;
use serde_json::{Value, json};
use std::path::Path;
use std::time::{Duration, SystemTime};

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

/// What `unlock` checks before it starts a daemon, for an edit that will re-warm one.
fn rewarm_checks(opts: &WhitelistEditArgs) -> Result<()> {
    protocol::check_ptrace_scope(opts.allow_ptrace_scope_0)?;
    protocol::check_daemons_findable()
}

/// The running session's `expiresAt` in UNIX seconds, as a daemon of this home that answers as
/// unlocked reports it ([`live_daemons`]); `None` without one.
async fn session_expiry(home: &Path) -> Result<Option<u64>> {
    let (_, serving) = live_daemons(home).await?;
    Ok(serving.and_then(|s| s["expiresAt"].as_str().and_then(parse_rfc3339_utc)))
}

/// What step 5 did with the session that was running.
enum Session {
    /// None was running: the wallet stays locked.
    NotRunning,
    /// Unlocked again until the remembered expiry (plus the new daemon's start-up).
    Rewarmed { previous: u64, ready: Value },
    /// It had run out by the time of the re-warm: the wallet stays locked.
    Expired { previous: u64 },
    /// Unlocking again failed: the wallet is locked.
    Failed { previous: u64, error: WalletError },
}

impl Session {
    /// Fills `locked` and the session fields of a change's output.
    fn report(&self, out: &mut Value) {
        let previous = match self {
            Self::NotRunning => {
                out["locked"] = json!(true);
                return;
            }
            Self::Rewarmed { previous, ready } => {
                out["locked"] = json!(false);
                out["expiresAt"] = ready["expiresAt"].clone();
                out["daemonPid"] = ready["pid"].clone();
                if let Some(w) = ready.get("whitelist") {
                    out["whitelist"] = w.clone();
                }
                previous
            }
            Self::Expired { previous } => {
                out["locked"] = json!(true);
                out["rewarm"] = json!("the session had expired");
                previous
            }
            Self::Failed { previous, error } => {
                out["locked"] = json!(true);
                out["rewarm"] = json!(format!("unlocking again failed: {}", error.message));
                previous
            }
        };
        out["previousExpiresAt"] = json!(rfc3339_utc(*previous));
    }

    /// `error` (the edit failed after the daemons were stopped), saying what became of the
    /// session.
    fn annotate(&self, error: WalletError) -> WalletError {
        let (note, locked) = match self {
            Self::NotRunning => return error,
            Self::Rewarmed { ready, .. } => (
                format!(
                    "the session that was running is unlocked again until {}",
                    ready["expiresAt"].as_str().unwrap_or("?")
                ),
                false,
            ),
            Self::Expired { .. } => (
                "the session that was running had expired; the wallet is locked".to_string(),
                true,
            ),
            Self::Failed { error: e, .. } => (
                format!(
                    "unlocking the session again failed ({}); the wallet is locked, run `zwallet \
                     unlock`",
                    e.message
                ),
                true,
            ),
        };
        WalletError::new(error.code, format!("{}; {note}", error.message))
            .with_detail("locked", locked)
    }
}

/// Step 5: unlock again with `password` until `remembered`, if that is still ahead.
async fn rewarm(home: &Path, exe: &DaemonExe, password: &str, remembered: Option<u64>) -> Session {
    let Some(previous) = remembered else {
        return Session::NotRunning;
    };
    let now = unix_secs(SystemTime::now());
    if previous <= now {
        return Session::Expired { previous };
    }
    match start_session(home, exe, password, Duration::from_secs(previous - now)).await {
        Ok(ready) => Session::Rewarmed { previous, ready },
        Err(error) => Session::Failed { previous, error },
    }
}

/// One change, linearized as the module doc says.
async fn edit(home: &Path, change: Change, opts: &WhitelistEditArgs) -> Result<Out> {
    store::read_wallet_file(home)?;
    // Pinned before the prompt, as `unlock` pins it: a binary swapped in while the human types
    // is never the one re-warmed.
    let exe = DaemonExe::resolve()?;
    let mut checked = false;
    if session_expiry(home).await?.is_some() {
        rewarm_checks(opts)?;
        checked = true;
    }
    let password = read_password(opts.password.password_stdin, "Wallet password: ", false)?;
    // A wrong password is refused here, before anything is stopped.
    let anchor = store::unlock(home, &password, &chains::derive_addresses)?
        .0
        .whitelist_anchor()?;
    let base = whitelist::pick_base(home, anchor.as_ref(), &WHITELIST_RULES);
    if !matches!(change, Change::Reset) {
        try_change(&change, &base)?;
    }

    // 1. Remember the session, then stop every daemon.
    let remembered = session_expiry(home).await?;
    if remembered.is_some() && !checked {
        rewarm_checks(opts)?;
    }
    let stopped = protocol::stop_all_daemons()?;
    // 2. to 4.
    let edited = match &change {
        Change::Reset => whitelist::reset(home, &password, &WHITELIST_RULES),
        _ => whitelist::edit(home, &password, &WHITELIST_RULES, |list| change.apply(list)),
    };
    // 5. Whether or not the edit went through: the session was this password's.
    let session = rewarm(home, &exe, &password, remembered).await;
    drop(password);
    let edited = edited.map_err(|e| session.annotate(e))?;
    let out = edit_output(&change, &base, &edited, &stopped, &session);
    let text = edit_text(&out);
    Ok(Out::with_text(out, text))
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
    lines.push(if out["locked"] == json!(false) {
        format!(
            "unlocked again until {} (daemon {})",
            s("expiresAt"),
            s("daemonPid")
        )
    } else if let Some(why) = out["rewarm"].as_str() {
        format!("locked: {why}")
    } else {
        "locked".into()
    });
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
    fn the_session_note_never_claims_an_unlock_that_failed() {
        let failed = Session::Failed {
            previous: 0,
            error: WalletError::validation("daemon refused"),
        };
        let e = failed.annotate(WalletError::validation("disk full"));
        assert!(
            e.message
                .starts_with("disk full; unlocking the session again failed")
                && e.message.contains("zwallet unlock"),
            "{e}"
        );
        assert_eq!(e.details["locked"], true);
        let ok = Session::Rewarmed {
            previous: 0,
            ready: json!({ "expiresAt": "2030-01-01T00:00:00Z", "pid": 9 }),
        };
        let e = ok.annotate(WalletError::validation("disk full"));
        assert!(e.message.contains("unlocked again until 2030"), "{e}");
        assert_eq!(e.details["locked"], false);
        assert_eq!(
            Session::NotRunning
                .annotate(WalletError::validation("x"))
                .message,
            "x"
        );
        let mut out = json!({});
        Session::Expired { previous: 0 }.report(&mut out);
        assert_eq!(out["locked"], true);
        assert_eq!(out["previousExpiresAt"], "1970-01-01T00:00:00Z");
    }
}
