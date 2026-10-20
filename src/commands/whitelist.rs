//! `zwallet whitelist …`, `@nickname` recipients, and the dry-run's whitelist check
//! (`.prd/04-whitelist.md` §Edits, §CLI).
//!
//! A change runs only with a typed password, and in this order:
//! 0. the arguments are checked; `whitelist.json` and `whitelist.next.json` are read once and
//!    the change is shown, with those files and what happens to signing, on the channel the
//!    password is read from (the terminal, or stderr with `--password-stdin`), so the human sees
//!    what the password authorizes; then the password is read and checked against wallet.json,
//!    and the change is tried on the list it builds on, from the same bytes. A wrong password, a
//!    list shown that does not verify (only `reset` starts over), or a refused change stops
//!    nothing;
//! 1. every zwallet daemon of this user is stopped (the `zwallet lock` path, which returns only
//!    once none is left): nothing is signed while the list changes;
//! 2. to 4. [`whitelist::edit_as_shown`] (or [`whitelist::reset_as_shown`]) under `w.lock`:
//!    the files read again once and refused unless they are the bytes shown, the verified base,
//!    the change at the next generation, signed, staged, re-sealed, published from memory;
//! 5. the wallet stays locked, unless `--unlock <duration>` starts a new session for that
//!    duration with the password just typed (the staged unlock `zwallet unlock` uses). No
//!    duration or expiry is taken from a running daemon.

use super::start_session;
use crate::chains::evm::policy::{self, ListView};
use crate::chains::evm::{self, registry::WHITELIST_RULES, tx};
use crate::chains::{self, Chain, Family, PRIMARY_CHAIN};
use crate::cli::{
    Out, Toggle, WhitelistAddArgs, WhitelistCommand, WhitelistEditArgs, WhitelistListArgs,
};
use crate::core::amount::{format_units, parse_units};
use crate::core::error::{Result, WalletError};
use crate::core::password::read_password;
use crate::core::protocol::{self, DaemonExe};
use crate::core::store::{self, WhitelistAnchor};
use crate::core::time::{parse_duration, rfc3339_utc, unix_secs};
use crate::core::whitelist::{
    self, ANY_CHAIN, Base, BaseSource, Edited, Entry, EntryKind, ListFiles, NEXT_FILE, SignedList,
    WHITELIST_FILE, Whitelist,
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
    // 0. What the password authorizes, where the password is read. The list files are read once
    // here; the edit is bound to these bytes.
    let files = ListFiles::read(home);
    let fingerprint = files.fingerprint();
    let told = summary(
        &files,
        &change,
        unlock.as_ref().map(|(d, ttl, _)| (d.as_str(), *ttl)),
    );
    let password = if opts.password.password_stdin {
        eprintln!("{told}");
        read_password(true, "", false)?
    } else {
        read_password(false, &format!("{told}\nWallet password: "), false)?
    };
    // A wrong password, a list shown that does not verify, or a change the list refuses is
    // refused here, before anything is stopped.
    let anchor = store::unlock(home, &password, &chains::derive_addresses)?
        .0
        .whitelist_anchor()?;
    let base = match change {
        Change::Reset => whitelist::pick_base_from(&files, anchor.as_ref(), &WHITELIST_RULES),
        _ => {
            let base = whitelist::base_for_edit(&files, anchor.as_ref(), &WHITELIST_RULES)?;
            try_change(&change, &base)?;
            base
        }
    };
    // 1. The files once more, right before anything is stopped: a swap since the summary stops
    // nothing (the edit checks again under `w.lock`).
    whitelist::check_unchanged(home, &fingerprint)?;
    let stopped = protocol::stop_all_daemons()?;
    // 2. to 4.
    let edited = match &change {
        Change::Reset => whitelist::reset_as_shown(home, &password, &WHITELIST_RULES, &fingerprint),
        _ => whitelist::edit_as_shown(home, &password, &WHITELIST_RULES, &fingerprint, |list| {
            change.apply(list)
        }),
    }
    .map_err(|e| {
        WalletError::new(
            e.code,
            format!(
                "{}; the daemons were already stopped, so the wallet is locked",
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

/// One list file as the summary shows it.
enum FileView<'a> {
    Absent,
    Unreadable(String),
    List(&'a Whitelist),
}

impl<'a> FileView<'a> {
    fn of(parsed: &'a Result<Option<SignedList>>) -> Self {
        match parsed {
            Ok(None) => Self::Absent,
            Ok(Some(s)) => Self::List(&s.list),
            Err(e) => Self::Unreadable(e.message.clone()),
        }
    }

    fn list(&self) -> Option<&'a Whitelist> {
        match self {
            Self::List(l) => Some(l),
            _ => None,
        }
    }
}

/// What a value is in each list file, for the line of a change: `value` of whitelist.json alone,
/// or `<file> says <value>` for both when there is an unfinished edit in whitelist.next.json.
/// `first` is the value a first list starts from (no file at all).
fn old_values(
    current: &FileView,
    next: &FileView,
    first: &str,
    value: impl Fn(&Whitelist) -> String,
) -> String {
    let says = |view: &FileView, name: &str| match view {
        FileView::List(l) => format!("{name} says {}", value(l)),
        FileView::Absent => format!("{name} says nothing (no file)"),
        FileView::Unreadable(_) => format!("{name} cannot be read"),
    };
    match (current, next) {
        (FileView::List(l), FileView::Absent) => value(l),
        (FileView::Absent, FileView::Absent) => first.to_string(),
        (FileView::Unreadable(_), FileView::Absent) => {
            "unknown (whitelist.json cannot be read)".into()
        }
        _ => format!(
            "{}; {}",
            says(current, WHITELIST_FILE),
            says(next, NEXT_FILE)
        ),
    }
}

/// Step 0: the change the password is about to authorize, from the arguments and the list files
/// as read once (`files`, the bytes the edit is then bound to), and what happens to signing.
/// The files are unverified here (only the password opens the seal that says which one, if any,
/// is the signed list), so the text states every outcome that can still happen: the change lands
/// on whitelist.json, or on an unfinished edit in whitelist.next.json, or (with no file at all)
/// starts the first list, or nothing changes. The last line repeats the change and what happens
/// to signing, right above the prompt. Every value read from a file is escaped ([`shown`],
/// [`printable`]) and name lists are cut ([`list_names`]).
fn summary(files: &ListFiles, change: &Change, unlock: Option<(&str, Duration)>) -> String {
    let (current_read, next_read) = (files.current.parsed(), files.next.parsed());
    let (current, next) = (FileView::of(&current_read), FileView::of(&next_read));
    let fresh = Whitelist::empty(&WHITELIST_RULES);
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
            let entry = |view: &FileView, name: &str| match view {
                FileView::List(l) => entry_text(l, nickname, name),
                FileView::Absent => format!("{name} does not exist"),
                FileView::Unreadable(_) => format!("{name} cannot be read"),
            };
            let what = match &next {
                FileView::Absent => entry(&current, WHITELIST_FILE),
                _ => format!(
                    "{}; {}",
                    entry(&current, WHITELIST_FILE),
                    entry(&next, NEXT_FILE)
                ),
            };
            lines.push(format!("  remove {nickname}: {what}"));
        }
        Change::FeeCap { chain, wei } => {
            let old = old_values(&current, &next, &cap_text(&fresh, chain), |l| {
                cap_text(l, chain)
            });
            lines.push(format!(
                "  fee-cap {}: {old} -> {}",
                chain.key,
                fee_text(chain, *wei)
            ));
        }
        Change::MessageSigning(on) => {
            let old = old_values(&current, &next, on_off(false), |l| {
                on_off(l.policy.allow_message_signing).to_string()
            });
            lines.push(format!(
                "  allow-message-signing: {old} -> {} (some contracts take an EIP-191 signature \
                 as spending authority)",
                on_off(*on)
            ));
        }
        Change::Reset => reset_lines(&mut lines, &current, &next, &fresh, unlock.is_some()),
    }
    match &current {
        FileView::List(l) => lines.push(format!(
            "  whitelist.json now: {}, not verified yet",
            gen_entries(l)
        )),
        FileView::Absent => lines.push("  whitelist.json: none yet".to_string()),
        FileView::Unreadable(why) => lines.push(unreadable_line(WHITELIST_FILE, why)),
    }
    match &next {
        FileView::List(l) => lines.push(format!(
            "  whitelist.next.json: {}, an unfinished edit, not verified yet",
            gen_entries(l)
        )),
        FileView::Absent => {}
        FileView::Unreadable(why) => lines.push(unreadable_line(NEXT_FILE, why)),
    }
    outcome_lines(&mut lines, &current, &next, matches!(change, Change::Reset));
    lines.push(
        "  every zwallet daemon of this user is stopped first, as `zwallet lock` does".to_string(),
    );
    let signing = match unlock {
        None => "after the change: the wallet stays locked".to_string(),
        Some((duration, ttl)) => {
            let until = unix_secs(SystemTime::now()).saturating_add(ttl.as_secs());
            let utc = rfc3339_utc(until);
            format!(
                "after the change: a new signing session for {duration} (until about {})",
                match local_time(until) {
                    Some(local) => format!("{local}, {utc}"),
                    None => utc,
                }
            )
        }
    };
    lines.push(format!("  {signing}"));
    lines.push(format!(
        "The password authorizes: {}; {signing}",
        headline(change)
    ));
    lines.join("\n")
}

/// A list file that exists but cannot be read or parsed: never "none", since a reset drops
/// whatever it holds.
fn unreadable_line(name: &str, why: &str) -> String {
    format!(
        "  {name} is there but cannot be read ({}): its entries are unknown, and a reset drops \
         them",
        printable(why)
    )
}

/// The reset's own lines: what it drops, every fee cap it sets back (or leaves unset, for a
/// chain with no built-in default), and what to try first.
fn reset_lines(
    lines: &mut Vec<String>,
    current: &FileView,
    next: &FileView,
    fresh: &Whitelist,
    unlocking: bool,
) {
    let names = |view: &FileView| view.list().map(list_names).filter(|n| !n.is_empty());
    let unknown =
        |name: &str| format!("{name} is there but cannot be read, so its entries are unknown");
    let part = |view: &FileView, name: &str| match view {
        FileView::Unreadable(_) => unknown(name),
        _ => format!("{name}: {}", names(view).unwrap_or_else(|| "none".into())),
    };
    let dropped = match (current, next) {
        (FileView::Unreadable(_), FileView::Absent) => unknown(WHITELIST_FILE),
        (_, FileView::Absent) => names(current).unwrap_or_else(|| "none".into()),
        _ => format!(
            "{}; {}",
            part(current, WHITELIST_FILE),
            part(next, NEXT_FILE)
        ),
    };
    lines.push(format!(
        "  reset: a new whitelist key and an empty list; every entry is dropped: {dropped}"
    ));
    lines.push("  fee caps after the reset, the built-in defaults:".to_string());
    let shown: Vec<(&str, &Whitelist)> = [(WHITELIST_FILE, current), (NEXT_FILE, next)]
        .into_iter()
        .filter_map(|(name, view)| view.list().map(|l| (name, l)))
        .collect();
    let mut keys: Vec<&String> = fresh.policy.max_fee_wei.keys().collect();
    for (_, l) in &shown {
        keys.extend(l.policy.max_fee_wei.keys());
    }
    keys.sort();
    keys.dedup();
    for key in keys {
        let Ok(chain) = chains::find(key) else {
            continue;
        };
        let default = fresh
            .policy
            .max_fee_wei
            .contains_key(key)
            .then(|| cap_text(fresh, chain));
        let differs: Vec<String> = shown
            .iter()
            .filter(|(_, l)| l.policy.max_fee_wei.get(key) != fresh.policy.max_fee_wei.get(key))
            .map(|(name, l)| format!("{name} says {}", cap_text(l, chain)))
            .collect();
        lines.push(match (default, differs.is_empty()) {
            (Some(default), true) => format!("    {key}: {default}"),
            (Some(default), false) => format!(
                "    {key}: back to the default {default} ({}, not verified)",
                differs.join("; ")
            ),
            (None, _) => format!(
                "    {key}: no default: every send there is refused until `fee-cap` sets one \
                 ({}, not verified)",
                differs.join("; ")
            ),
        });
    }
    let signing_on: Vec<&str> = shown
        .iter()
        .filter(|(_, l)| l.policy.allow_message_signing)
        .map(|(name, _)| *name)
        .collect();
    lines.push(if signing_on.is_empty() {
        "  message signing after the reset: off".to_string()
    } else {
        format!(
            "  message signing after the reset: off ({} says on, not verified)",
            signing_on.join(" and ")
        )
    });
    lines.push(if unlocking {
        "  the reset allows nothing else (no payee, no contract), and with --unlock the new \
         session signs under the default caps at once: to set yours first, run the reset \
         without --unlock, then `zwallet whitelist fee-cap`"
            .to_string()
    } else {
        "  the reset allows nothing else (no payee, no contract); set `zwallet whitelist \
         fee-cap` again before `--unlock`"
            .to_string()
    });
    // A rolled-back wallet.json accepts the copy its seal names, so the copy is only for a
    // generation the human knows is the highest.
    lines.push(
        "  if whitelist.json was only damaged or replaced, put back a genuine copy of it at the \
         sealed generation instead, but only if that is the highest generation you remember \
         seeing (unlock, status and `zwallet whitelist list` print it): it verifies again \
         without a reset, because the seal pins its digest; if you remember a higher one, \
         wallet.json was rolled back: restore the current wallet.json instead"
            .to_string(),
    );
}

/// Every outcome that can still happen, decided once the password opens wallet.json.
fn outcome_lines(lines: &mut Vec<String>, current: &FileView, next: &FileView, reset: bool) {
    let gen_after = |view: &FileView| view.list().map(|l| l.generation.saturating_add(1));
    if reset {
        lines.push(
            "  the reset makes a new key and an empty list, decided once the password opens \
             wallet.json:"
                .to_string(),
        );
        if let Some(g) = gen_after(current) {
            lines.push(format!(
                "    gen {g} if whitelist.json is the list wallet.json seals"
            ));
        }
        if let Some(g) = gen_after(next) {
            lines.push(format!(
                "    gen {g} if whitelist.next.json is the list wallet.json seals (an unfinished \
                 edit)"
            ));
        }
        lines.push(
            "    if neither file is the sealed list (it was changed or deleted): gen M+1, where M \
             is the sealed generation, shown once the password opens wallet.json"
                .to_string(),
        );
        lines.push("    gen 1 if wallet.json seals no whitelist yet".to_string());
        return;
    }
    lines.push("  what happens, decided once the password opens wallet.json:".to_string());
    if let Some(g) = gen_after(current) {
        lines.push(format!(
            "    if whitelist.json is the list wallet.json seals: the change lands on it, making \
             gen {g}"
        ));
    }
    if let Some(g) = gen_after(next) {
        lines.push(format!(
            "    if whitelist.next.json is the list wallet.json seals (an unfinished edit): the \
             change lands on it, making gen {g}"
        ));
    }
    if matches!((current, next), (FileView::Absent, FileView::Absent)) {
        lines.push(
            "    if wallet.json seals no whitelist yet: a new list from the built-in defaults \
             with a new key, gen 1"
                .to_string(),
        );
    }
    lines.push("    otherwise: nothing changes".to_string());
}

/// The change in one line, for the line right above the prompt.
fn headline(change: &Change) -> String {
    match change {
        Change::Add(e) => {
            let mut line = format!(
                "add {}, {} {} on {}",
                e.nickname,
                match e.kind {
                    EntryKind::Address => "a payee",
                    EntryKind::Contract => "a contract",
                },
                checksummed(&e.address),
                chains_text(&e.chains)
            );
            if let Some(sels) = &e.selectors {
                line.push_str(&format!(" with selectors {}", sels.join(", ")));
            }
            line
        }
        Change::Remove(nickname) => format!("remove {nickname}"),
        Change::FeeCap { chain, wei } => format!("fee-cap {} {}", chain.key, fee_text(chain, *wei)),
        Change::MessageSigning(on) => format!("allow-message-signing {}", on_off(*on)),
        Change::Reset => {
            "reset: a new key, an empty list, every fee cap back to the default".to_string()
        }
    }
}

/// `gen G, n entries (names)` of a list read from a file.
fn gen_entries(list: &Whitelist) -> String {
    let n = list.entries.len();
    format!(
        "gen {}, {n} {} ({})",
        list.generation,
        if n == 1 { "entry" } else { "entries" },
        list_names(list)
    )
}

/// What `file` says about the entry `nickname`.
fn entry_text(list: &Whitelist, nickname: &str, file: &str) -> String {
    match list.by_nickname(nickname) {
        Some(e) => format!(
            "{file} lists it as {} {} on {}",
            match e.kind {
                EntryKind::Address => "a payee",
                EntryKind::Contract => "a contract",
            },
            checksummed(&e.address),
            chains_text(&e.chains)
        ),
        None => format!("{file} has no entry by that name"),
    }
}

/// A list's fee cap on `chain`, in its coin and in wei.
fn cap_text(list: &Whitelist, chain: &Chain) -> String {
    match list.policy.max_fee_wei.get(chain.key) {
        Some(v) => match U256::from_str_radix(v, 10) {
            Ok(w) => fee_text(chain, w),
            Err(_) => format!("{} (unreadable)", shown(v)),
        },
        None => "none".to_string(),
    }
}

/// `secs` as local time with its UTC offset (`2026-10-21 01:40:12 +0900`), from the system's
/// time zone; `None` if it cannot be converted.
fn local_time(secs: u64) -> Option<String> {
    let t = libc::time_t::try_from(secs).ok()?;
    // SAFETY: `tm` is plain data owned here; localtime_r only reads `t` and writes `tm`.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
        return None;
    }
    let offset = tm.tm_gmtoff / 60;
    Some(format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} {}{:02}{:02}",
        i64::from(tm.tm_year) + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec,
        if offset < 0 { '-' } else { '+' },
        offset.abs() / 60,
        offset.abs() % 60
    ))
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

/// The most nicknames a summary names from one list; the rest are counted.
const NAMES_SHOWN: usize = 20;

/// A list's nicknames, each [`shown`], comma-separated, at most [`NAMES_SHOWN`] of them and then
/// `and M more`, so a long list cannot push the change off the screen.
fn list_names(list: &Whitelist) -> String {
    let mut names: Vec<String> = list
        .entries
        .iter()
        .take(NAMES_SHOWN)
        .map(|e| shown(&e.nickname))
        .collect();
    if list.entries.len() > NAMES_SHOWN {
        names.push(format!("and {} more", list.entries.len() - NAMES_SHOWN));
    }
    names.join(", ")
}

/// What the dry-run's whitelist check found ([`dry_run_check`]).
pub(super) enum DryRunCheck {
    /// There is no whitelist.json, so nothing was checked here: the daemon decides, against the
    /// anchor it holds (this process cannot tell a wallet with no whitelist yet from one whose
    /// list was deleted).
    NotChecked,
    /// The list on disk allows it; the daemon still checks the list the seal names.
    Allowed,
    /// The list on disk, or its absence of trust, refuses it: `{allowed: false, reason, code,
    /// command?, counterparty, chain, …}`. The daemon is not asked to sign.
    Denied(Value),
}

/// For `--dry-run`: the daemon's own check ([`policy::check_tx`]) of the unsigned transaction
/// against whitelist.json as it is on disk, unverified: the list if it parses, or unusable (with
/// the reason) if it cannot be read. Without a whitelist.json nothing is checked here. Only the
/// daemon's check, against the list the seal names, decides a send.
pub(super) fn dry_run_check(
    home: &Path,
    chain: &Chain,
    from: &str,
    unsigned: &Value,
) -> Result<DryRunCheck> {
    match &chain.family {
        Family::Evm(_) => {}
    }
    let read = whitelist::read_list(&home.join(WHITELIST_FILE));
    let view = match &read {
        Ok(None) => return Ok(DryRunCheck::NotChecked),
        Ok(Some(signed)) => ListView::Usable(&signed.list),
        Err(e) => ListView::Unusable(e.message.clone()),
    };
    let payload = unsigned
        .as_str()
        .and_then(|s| s.strip_prefix("0x"))
        .and_then(|h| hex::decode(h).ok())
        .ok_or_else(|| WalletError::validation("the unsigned transaction is not 0x hex"))?;
    let decoded = tx::decode_unsigned(&payload)?;
    let signer = evm::parse_address(from)?;
    let Err(e) = policy::check_tx(&view, chain, &signer, &decoded) else {
        return Ok(DryRunCheck::Allowed);
    };
    let mut denied = e.details.clone();
    denied.insert("allowed".into(), json!(false));
    denied.insert("reason".into(), json!(e.message));
    denied.insert("code".into(), json!(e.code.as_str()));
    Ok(DryRunCheck::Denied(Value::Object(denied)))
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
    // What a reset dropped without it having verified, or a whitelist.json found before there
    // was any whitelist; an edit with an anchor never builds on an unverified list.
    if let Some(d) = &edited.discarded {
        out["discarded"] = json!({ "reason": d.reason, "entries": d.entries });
    }
    // What the edit could not tidy up after the change landed, after a contract's residual.
    if let Some(w) = &edited.warning {
        out["warning"] = json!(match out["warning"].as_str() {
            Some(residual) => format!("{residual}; {w}"),
            None => w.clone(),
        });
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
        _ => format!("reset: a new key and an empty list; dropped: {}", {
            let d = join(&out["dropped"]);
            let discarded = &out["discarded"];
            match (
                d.is_empty(),
                discarded.is_object(),
                discarded["entries"].is_null(),
            ) {
                (false, _, _) => d,
                // No entries: whitelist.json is missing or cannot be read (the reason says
                // which), so what it held is unknown, never "none".
                (true, true, true) => "no verified entry; whitelist.json's entries unknown (it is \
                                       missing or cannot be read)"
                    .into(),
                (true, true, false) => {
                    "no verified entry; whitelist.json's unverified entries are below".into()
                }
                (true, false, _) => "nothing".into(),
            }
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
        let entries = &out["discarded"]["entries"];
        lines.push(format!(
            "discarded (not signed, the edit started from an empty list): {}{}",
            printable(out["discarded"]["reason"].as_str().unwrap_or("?")),
            if entries.is_null() {
                String::new()
            } else {
                format!("; its unverified entries: {}", ascii_json(entries))
            }
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
        // A list file with no seal is never offered as the first add: that add refuses.
        Ok(None) => {
            let reason = match whitelist::unsealed_files_note(&ListFiles::read(home)) {
                Some(note) => format!(
                    "{note}: it is not verified, and every edit refuses; {} {}",
                    whitelist::UNSEALED_REMEDY,
                    whitelist::RESET_CAVEAT
                ),
                None => format!("wallet.json seals no whitelist key yet; {FIRST_ADD}"),
            };
            return unverified_view(home, Some(reason));
        }
        Ok(Some(a)) => a,
    };
    let files = ListFiles::read(home);
    let base = whitelist::pick_base_from(&files, Some(&anchor), &WHITELIST_RULES);
    // The genuine-copy remedy, as conditional as the edit's (none above the seal: the reason
    // says what to do).
    let remedy = |reason: String| match whitelist::unverified_remedy(&files, &anchor) {
        Some(r) => {
            format!("{reason}. If whitelist.json was only damaged, replaced or deleted, {r}")
        }
        None => reason,
    };
    let mut out = match base.source {
        BaseSource::Current => list_view(&base.list, true, WHITELIST_FILE),
        BaseSource::PromotedNext => list_view(&base.list, true, NEXT_FILE),
        BaseSource::Empty => unverified_view(home, base.discarded.map(|d| remedy(d.reason))),
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
        // Without whitelist.json, a staged whitelist.next.json is an unfinished edit (or a
        // planted file): `--verify` says which, and the first add would refuse.
        Ok(None) => json!({
            "gen": null,
            "entries": [],
            "policy": null,
            "verified": false,
            "reason": if home.join(NEXT_FILE).symlink_metadata().is_ok() {
                format!(
                    "there is no {WHITELIST_FILE}, but {NEXT_FILE} exists (an unfinished edit, or \
                     a planted file): `zwallet whitelist list --verify` checks it against \
                     wallet.json"
                )
            } else {
                format!("there is no whitelist yet; {FIRST_ADD}")
            },
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
    fn r5_3_what_an_edit_could_not_tidy_up_is_a_warning_on_a_change_that_landed() {
        let mut list = Whitelist::empty(&WHITELIST_RULES);
        list.generation = 2;
        list.add(Entry::new_contract(
            "router",
            "0x2222222222222222222222222222222222222222",
            &["base"],
            &["0x12345678"],
        ))
        .unwrap();
        let left = "the change landed, but whitelist.next.json was left in place (busy)";
        let edited = Edited {
            list: list.clone(),
            previous_generation: Some(1),
            base: BaseSource::Current,
            discarded: None,
            key_created: false,
            warning: Some(left.into()),
        };
        let base = Base {
            list: list.clone(),
            source: BaseSource::Current,
            discarded: None,
        };
        let entry = list.by_nickname("router").unwrap().clone();
        let out = edit_output(&Change::Add(entry), &base, &edited, &[], &Session::Locked);
        // After a contract's residual, and the change is still reported as made.
        assert_eq!(out["warning"], format!("{RESIDUAL}; {left}"), "{out}");
        assert_eq!((&out["gen"], &out["verified"]), (&json!(2), &json!(true)));
        assert!(edit_text(&out).contains(&format!("warning: {RESIDUAL}; {left}")));
        let out = edit_output(
            &Change::MessageSigning(true),
            &base,
            &edited,
            &[],
            &Session::Locked,
        );
        assert_eq!(out["warning"], left, "{out}");
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
        let at = |home: &Path, change: &Change| summary(&ListFiles::read(home), change, None);
        let base = chains::find("base").unwrap();
        let alice = Entry::new_address(
            "alice",
            "0x5aaeb6053f3e94c9b9a09f33669435e7ef1beaed",
            &["*"],
        );
        let text = at(home, &Change::Add(alice));
        for want in [
            "The password authorizes this whitelist change:",
            "add alice: a payee",
            "address: 0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed",
            "chains: every chain (*)",
            "whitelist.json: none yet",
            "if wallet.json seals no whitelist yet: a new list from the built-in defaults with a \
             new key, gen 1",
            "otherwise: nothing changes",
            "every zwallet daemon of this user is stopped first",
            "after the change: the wallet stays locked",
        ] {
            assert!(text.contains(want), "{want:?} not in {text}");
        }
        // The last line, right above the prompt, repeats the change and what happens to signing.
        assert_eq!(
            text.lines().last().unwrap(),
            "The password authorizes: add alice, a payee 0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed \
             on every chain (*); after the change: the wallet stays locked"
        );
        let cap = Change::FeeCap {
            chain: base,
            wei: U256::from(1_000_000_000_000_000u64),
        };
        let text = at(home, &cap);
        assert!(
            text.contains("fee-cap base: 0.0005 ETH (500000000000000 wei) -> 0.001 ETH"),
            "{text}"
        );
        // With --unlock: the session and when it ends.
        let text = summary(
            &ListFiles::read(home),
            &cap,
            Some(("24h", Duration::from_secs(86_400))),
        );
        let until = rfc3339_utc(unix_secs(SystemTime::now()) + 86_400);
        assert!(
            text.contains("after the change: a new signing session for 24h (until about "),
            "{text}"
        );
        assert!(text.contains(&until[..13]), "{until} not in {text}");
        assert!(!text.contains("stays locked"), "{text}");
        assert!(
            text.lines()
                .last()
                .unwrap()
                .starts_with("The password authorizes: fee-cap base 0.001 ETH (1000000000000000 wei); after the change: a new signing session for 24h"),
            "{text}"
        );

        let tmp = home_with(&[Entry::new_address(
            "bob",
            "0x3333333333333333333333333333333333333333",
            &["base"],
        )]);
        let text = at(tmp.path(), &Change::MessageSigning(true));
        assert!(text.contains("allow-message-signing: off -> on"), "{text}");
        for want in [
            "whitelist.json now: gen 1, 1 entry (bob), not verified yet",
            "if whitelist.json is the list wallet.json seals: the change lands on it, making gen 2",
            "otherwise: nothing changes",
        ] {
            assert!(text.contains(want), "{want:?} not in {text}");
        }
        assert!(!text.contains("whitelist.next.json"), "{text}");
        let text = at(tmp.path(), &Change::Remove("bob".into()));
        assert!(
            text.contains("remove bob: whitelist.json lists it as a payee 0x3333333333333333333333333333333333333333 on base"),
            "{text}"
        );
        // An unreadable file: its reason, escaped; only "nothing changes" can happen.
        let bad = tempfile::tempdir().unwrap();
        std::fs::write(bad.path().join(WHITELIST_FILE), "{\"x\u{1b}\": 1}").unwrap();
        let text = at(bad.path(), &Change::MessageSigning(true));
        assert!(
            text.contains("whitelist.json is there but cannot be read"),
            "{text}"
        );
        assert!(!text.contains('\u{1b}'), "{text:?}");
        assert!(text.contains("otherwise: nothing changes"), "{text}");
        assert!(!text.contains("lands on"), "{text}");
    }

    #[test]
    fn b1_with_an_unfinished_edit_the_summary_shows_both_files_and_every_outcome() {
        let tmp = home_with(&[Entry::new_address(
            "bob",
            "0x3333333333333333333333333333333333333333",
            &["base"],
        )]);
        let mut staged = Whitelist::empty(&WHITELIST_RULES);
        staged.generation = 4;
        staged
            .add(Entry::new_address(
                "bob",
                "0x4444444444444444444444444444444444444444",
                &["*"],
            ))
            .unwrap();
        staged.policy.allow_message_signing = true;
        staged
            .policy
            .set_max_fee_wei("base", U256::from(2_000_000_000_000_000u64));
        let signed = WhitelistKey::generate().unwrap().sign(&staged);
        std::fs::write(tmp.path().join(NEXT_FILE), signed.to_file_bytes().unwrap()).unwrap();
        let at = |change: &Change| summary(&ListFiles::read(tmp.path()), change, None);
        let base = chains::find("base").unwrap();

        let text = at(&Change::FeeCap {
            chain: base,
            wei: U256::from(1_000_000_000_000_000u64),
        });
        for want in [
            "fee-cap base: whitelist.json says 0.0005 ETH (500000000000000 wei); \
             whitelist.next.json says 0.002 ETH (2000000000000000 wei) -> 0.001 ETH \
             (1000000000000000 wei)",
            "whitelist.next.json: gen 4, 1 entry (bob), an unfinished edit, not verified yet",
            "if whitelist.json is the list wallet.json seals: the change lands on it, making gen 2",
            "if whitelist.next.json is the list wallet.json seals (an unfinished edit): the \
             change lands on it, making gen 5",
            "otherwise: nothing changes",
        ] {
            assert!(text.contains(want), "{want:?} not in {text}");
        }
        let text = at(&Change::MessageSigning(false));
        assert!(
            text.contains(
                "allow-message-signing: whitelist.json says off; whitelist.next.json says on -> \
                 off"
            ),
            "{text}"
        );
        let text = at(&Change::Remove("bob".into()));
        assert!(
            text.contains(
                "remove bob: whitelist.json lists it as a payee \
                 0x3333333333333333333333333333333333333333 on base; whitelist.next.json lists \
                 it as a payee 0x4444444444444444444444444444444444444444 on every chain (*)"
            ),
            "{text}"
        );
        // whitelist.json gone: only the staged list or nothing.
        std::fs::remove_file(tmp.path().join(WHITELIST_FILE)).unwrap();
        let text = at(&Change::MessageSigning(false));
        assert!(
            text.contains("whitelist.json says nothing (no file)"),
            "{text}"
        );
        assert!(!text.contains("if whitelist.json is the list"), "{text}");
        assert!(text.contains("making gen 5"), "{text}");
        assert!(!text.contains("seals no whitelist yet"), "{text}");
    }

    #[test]
    fn r3_3_the_reset_summary_shows_every_cap_it_sets_back() {
        let tmp = home_with(&[Entry::new_address(
            "bob",
            "0x3333333333333333333333333333333333333333",
            &["base"],
        )]);
        // The list on disk says base 1 wei and message signing on (it is re-signed here, so
        // only its values matter).
        let mut l = whitelist::read_list(&tmp.path().join(WHITELIST_FILE))
            .unwrap()
            .unwrap()
            .list;
        l.policy.set_max_fee_wei("base", U256::from(1u64));
        l.policy.allow_message_signing = true;
        let signed = WhitelistKey::generate().unwrap().sign(&l);
        std::fs::write(
            tmp.path().join(WHITELIST_FILE),
            signed.to_file_bytes().unwrap(),
        )
        .unwrap();
        let text = summary(&ListFiles::read(tmp.path()), &Change::Reset, None);
        for want in [
            "reset: a new whitelist key and an empty list; every entry is dropped: bob",
            "fee caps after the reset, the built-in defaults:",
            "    base: back to the default 0.0005 ETH (500000000000000 wei) (whitelist.json says \
             0.000000000000000001 ETH (1 wei), not verified)",
            "    ethereum: 0.003 ETH (3000000000000000 wei)",
            "message signing after the reset: off (whitelist.json says on, not verified)",
            "the reset allows nothing else (no payee, no contract); set `zwallet whitelist \
             fee-cap` again before `--unlock`",
            "if whitelist.json was only damaged or replaced, put back a genuine copy of it at the \
             sealed generation instead, but only if that is the highest generation you remember \
             seeing (unlock, status and `zwallet whitelist list` print it): it verifies again \
             without a reset, because the seal pins its digest; if you remember a higher one, \
             wallet.json was rolled back: restore the current wallet.json instead",
            "the reset makes a new key and an empty list, decided once the password opens \
             wallet.json:",
            "    gen 2 if whitelist.json is the list wallet.json seals",
            "    if neither file is the sealed list (it was changed or deleted): gen M+1, where M \
             is the sealed generation, shown once the password opens wallet.json",
            "    gen 1 if wallet.json seals no whitelist yet",
        ] {
            assert!(text.contains(want), "{want:?} not in {text}");
        }
        assert!(
            text.lines()
                .last()
                .unwrap()
                .starts_with("The password authorizes: reset: a new key, an empty list, every fee cap back to the default;"),
            "{text}"
        );
        let text = summary(
            &ListFiles::read(tmp.path()),
            &Change::Reset,
            Some(("1h", Duration::from_secs(3600))),
        );
        assert!(
            text.contains("with --unlock the new session signs under the default caps at once"),
            "{text}"
        );
        assert!(!text.contains("again before `--unlock`"), "{text}");
    }

    #[test]
    fn r4_3_the_reset_summary_states_every_generation_it_can_make() {
        let tmp = home_with(&[Entry::new_address(
            "bob",
            "0x3333333333333333333333333333333333333333",
            &["base"],
        )]);
        let mut staged = Whitelist::empty(&WHITELIST_RULES);
        staged.generation = 6;
        let signed = WhitelistKey::generate().unwrap().sign(&staged);
        std::fs::write(tmp.path().join(NEXT_FILE), signed.to_file_bytes().unwrap()).unwrap();
        let text = summary(&ListFiles::read(tmp.path()), &Change::Reset, None);
        for want in [
            "    gen 2 if whitelist.json is the list wallet.json seals",
            "    gen 7 if whitelist.next.json is the list wallet.json seals (an unfinished edit)",
            "    if neither file is the sealed list (it was changed or deleted): gen M+1, where M \
             is the sealed generation, shown once the password opens wallet.json",
            "    gen 1 if wallet.json seals no whitelist yet",
        ] {
            assert!(text.contains(want), "{want:?} not in {text}");
        }
        assert!(!text.contains("nothing changes"), "{text}");
        // No file at all: only the last two.
        let empty = tempfile::tempdir().unwrap();
        let text = summary(&ListFiles::read(empty.path()), &Change::Reset, None);
        assert!(!text.contains("if whitelist.json is"), "{text}");
        assert!(
            text.contains("    gen 1 if wallet.json seals no whitelist yet"),
            "{text}"
        );
    }

    #[test]
    fn r4_4_a_file_that_cannot_be_read_is_never_shown_as_none() {
        let unknown = "is there but cannot be read";
        for how in ["garbage", "symlink"] {
            let tmp = tempfile::tempdir().unwrap();
            let path = tmp.path().join(WHITELIST_FILE);
            match how {
                "garbage" => std::fs::write(&path, "{ not a list").unwrap(),
                _ => {
                    let real = home_with(&[Entry::new_address(
                        "bob",
                        "0x3333333333333333333333333333333333333333",
                        &["base"],
                    )]);
                    let target = tmp.path().join("elsewhere.json");
                    std::fs::copy(real.path().join(WHITELIST_FILE), &target).unwrap();
                    std::os::unix::fs::symlink(&target, &path).unwrap();
                }
            }
            let files = ListFiles::read(tmp.path());
            let text = summary(&files, &Change::Reset, None);
            assert!(
                text.contains(&format!("every entry is dropped: whitelist.json {unknown}")),
                "{how}: {text}"
            );
            assert!(
                text.contains("its entries are unknown, and a reset drops them"),
                "{how}: {text}"
            );
            assert!(!text.contains("dropped: none"), "{how}: {text}");
            let text = summary(&files, &Change::MessageSigning(true), None);
            assert!(
                text.contains(&format!("whitelist.json {unknown}")),
                "{how}: {text}"
            );
            assert!(text.contains("its entries are unknown"), "{how}: {text}");
            assert!(!text.contains("none yet"), "{how}: {text}");
        }
        // After the reset too: what it dropped from a file it could not read is unknown, not
        // nothing.
        let out = json!({
            "change": "reset",
            "gen": 3,
            "previousGen": 2,
            "entries": [],
            "dropped": [],
            "discarded": { "reason": "whitelist.json: not a whitelist file", "entries": null },
        });
        let text = edit_text(&out);
        assert!(!text.contains("dropped: nothing"), "{text}");
        // Both on the line that says what was dropped.
        assert!(
            text.lines().next().unwrap().ends_with(
                "dropped: no verified entry; whitelist.json's entries unknown (it is missing or \
                 cannot be read)"
            ),
            "{text}"
        );
        // The same for an unfinished edit that cannot be read.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(NEXT_FILE), "{ not a list").unwrap();
        let text = summary(&ListFiles::read(tmp.path()), &Change::Reset, None);
        assert!(
            text.contains(&format!("whitelist.next.json {unknown}")),
            "{text}"
        );
        assert!(!text.contains("dropped: none"), "{text}");
    }

    #[test]
    fn n_f_a_chain_with_a_cap_but_no_default_is_named_in_the_reset_summary() {
        // Every chain has a built-in default today; a chain added without one must not drop out
        // of the summary. `fresh` here is the defaults without sepolia.
        let mut fresh = Whitelist::empty(&WHITELIST_RULES);
        fresh.policy.max_fee_wei.remove("sepolia");
        let mut l = Whitelist::empty(&WHITELIST_RULES);
        l.policy.set_max_fee_wei("sepolia", U256::from(7u64));
        let mut lines = vec![];
        reset_lines(
            &mut lines,
            &FileView::List(&l),
            &FileView::Absent,
            &fresh,
            false,
        );
        let text = lines.join("\n");
        assert!(
            text.contains(
                "    sepolia: no default: every send there is refused until `fee-cap` sets one \
                 (whitelist.json says 0.000000000000000007 ETH (7 wei), not verified)"
            ),
            "{text}"
        );
        assert!(text.contains("    base: 0.0005 ETH"), "{text}");
    }

    #[test]
    fn n3_long_name_lists_are_cut_and_the_change_is_repeated_last() {
        let entries: Vec<Entry> = (0..25)
            .map(|i| {
                Entry::new_address(&format!("n{i:02}"), &format!("0x{:040x}", i + 1), &["base"])
            })
            .collect();
        let tmp = home_with(&entries);
        let text = summary(&ListFiles::read(tmp.path()), &Change::Reset, None);
        assert!(text.contains("n18, n19, and 5 more"), "{text}");
        assert!(!text.contains("n20"), "{text}");
        let text = summary(
            &ListFiles::read(tmp.path()),
            &Change::Remove("n03".into()),
            None,
        );
        assert!(text.contains("25 entries (n00, n01,"), "{text}");
        assert_eq!(
            text.lines().last().unwrap(),
            "The password authorizes: remove n03; after the change: the wallet stays locked"
        );
    }
}
