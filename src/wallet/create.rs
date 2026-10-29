//! Creation without terminal input or output. A plan precedes the human's password; the
//! password owner moves into the worker and is dropped after its last use. No result holds a
//! seed, recovery phrase, private key, password, or encrypted wallet envelope.

use super::Session;
use crate::chains::{self, PRIMARY_CHAIN};
use crate::commands::{derive_via_daemon, inert_whitelist, session_index_warning};
use crate::core::error::{ErrorCode, Result, WalletError};
use crate::core::password::MAX_SECRET;
use crate::core::protocol::{self, DaemonExe};
use crate::core::seed::{DEFAULT_WORD_COUNT, VALID_WORD_COUNTS};
use crate::core::store::{self, MasterOptions, WalletFile};
use crate::core::time::parse_duration;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use zeroize::ZeroizeOnDrop;

pub const BACKUP_REMINDER: &str =
    "Write down your recovery phrase now: run `zwallet backup` before funding this wallet.";

/// Prepared before a password prompt. Fields are private so the checked name, master identity
/// and pinned executable cannot be replaced by the caller while the human types.
pub struct CreatePlan {
    home: PathBuf,
    name: String,
    words: usize,
    master: Option<Value>,
    unlock: Option<(Duration, DaemonExe)>,
}

impl CreatePlan {
    /// A first wallet requires a new password entered twice by the calling human interface.
    pub fn creates_master(&self) -> bool {
        self.master.is_none()
    }
}

/// Public creation results. `verified` is false only for the CLI's separate daemon-derived
/// path. A TUI calls [`create`] with its password owner and always receives verified addresses.
#[derive(Clone, Debug, PartialEq)]
pub struct Created {
    pub name: String,
    pub index: u32,
    pub addresses: BTreeMap<String, String>,
    pub active: String,
    pub master_created: bool,
    pub verified: bool,
    pub session: Option<Session>,
    pub backup_reminder: Option<&'static str>,
}

impl Created {
    pub fn address(&self) -> Option<&str> {
        self.addresses
            .get(PRIMARY_CHAIN)
            .or_else(|| self.addresses.values().next())
            .map(String::as_str)
    }
}

/// Validate all non-secret inputs and pin a requested daemon before asking for the password.
/// Does not write files. The caller must not silently replace this plan after prompting.
pub fn prepare_create(
    home: &Path,
    name: &str,
    words: Option<usize>,
    ttl: Option<Duration>,
    allow_ptrace_scope_0: bool,
) -> Result<CreatePlan> {
    store::validate_key_name(name)?;
    let unlock = if let Some(ttl) = ttl {
        if ttl.subsec_nanos() != 0 {
            return Err(WalletError::validation(
                "unlock duration must be whole seconds",
            ));
        }
        parse_duration(&format!("{}s", ttl.as_secs()))?;
        protocol::socket_path(home)?;
        protocol::check_ptrace_scope(allow_ptrace_scope_0)?;
        protocol::check_daemons_findable()?;
        Some((ttl, DaemonExe::resolve()?))
    } else {
        None
    };
    let master = match store::read_wallet_file(home) {
        Err(e) if e.code == ErrorCode::WalletNotFound => None,
        Err(e) => return Err(e),
        Ok(file) => {
            if words.is_some() {
                return Err(WalletError::validation(
                    "--words only applies when creating the master seed",
                ));
            }
            store::plan_new_key(&file, name)?;
            Some(envelope(&file)?)
        }
    };
    let words = words.unwrap_or(DEFAULT_WORD_COUNT);
    if !VALID_WORD_COUNTS.contains(&words) {
        return Err(WalletError::validation(format!(
            "--words must be one of {VALID_WORD_COUNTS:?}"
        )));
    }
    Ok(CreatePlan {
        home: home.to_path_buf(),
        name: name.to_string(),
        words,
        master,
        unlock,
    })
}

/// Create using a password owned by the caller's zeroizing buffer (the CLI's
/// `Zeroizing<String>` or the TUI's fixed `PasswordBuf`). This generic owner avoids copying the
/// TUI password into a second allocation. The owner must implement `ZeroizeOnDrop`; this
/// function neither clones it nor includes it in an error/result. First-run confirmation is
/// the caller's responsibility, before transferring ownership here.
pub async fn create<P>(plan: CreatePlan, password: P) -> Result<Created>
where
    P: AsRef<str> + ZeroizeOnDrop + Send + 'static,
{
    let (plan, password, mut created, file) = tokio::task::spawn_blocking(move || {
        let (file, addresses) = create_local(&plan, password.as_ref())?;
        let created = result(&plan, &file, addresses, true)?;
        Ok::<_, WalletError>((plan, password, created, file))
    })
    .await
    .map_err(|_| WalletError::validation("wallet creation did not finish"))??;
    if let Some((ttl, exe)) = &plan.unlock {
        let ready = protocol::start_daemon(&plan.home, exe, password.as_ref(), *ttl).await?;
        let same = ready["active"] == json!(created.name)
            && ready["address"]
                .as_str()
                .zip(created.address())
                .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b));
        if !same {
            let mismatch = format!(
                "after create the key daemon reports active key {} at {}, not '{}' at {}; \
                 wallet.json changed meanwhile",
                ready["active"],
                ready["address"],
                created.name,
                created.address().unwrap_or("-")
            );
            drop(password);
            let stopped = tokio::task::spawn_blocking(protocol::stop_all_daemons)
                .await
                .map_err(|_| WalletError::validation("locking the wallet again did not finish"))?;
            return Err(relocked(mismatch, stopped));
        }
        created.session = Some(Session {
            whitelist: inert_whitelist(&ready["whitelist"]),
            warning: session_index_warning(&file, &ready),
            ready,
        });
    }
    drop(password);
    Ok(created)
}

fn create_local(
    plan: &CreatePlan,
    password: &str,
) -> Result<(WalletFile, BTreeMap<String, String>)> {
    if password.len() > MAX_SECRET || password.contains(['\r', '\n', '\0']) {
        return Err(WalletError::validation("invalid password input"));
    }
    if plan.creates_master() {
        let file = store::create_master(
            &plan.home,
            password,
            &plan.name,
            MasterOptions {
                words: plan.words,
                ..MasterOptions::default()
            },
            &chains::derive_addresses,
        )?;
        let addresses = file.key(Some(&plan.name))?.addresses.clone();
        return Ok((file, addresses));
    }
    // The KDF runs before taking the writer lock. Other creates may publish metadata meanwhile,
    // but cannot change the master this prompt was for. Plan the actual index under the lock.
    let (opened, file) = store::unlock(&plan.home, password, &chains::derive_addresses)?;
    check_master(plan, &file)?;
    let lock = store::MetaLock::acquire(&plan.home)?;
    let file = lock.read()?;
    check_master(plan, &file)?;
    let index = store::plan_new_key(&file, &plan.name)?;
    let addresses = chains::derive_addresses(&opened.seed, index)?;
    check_master(plan, &lock.read()?)?;
    let file = lock.add_key(&plan.name, index, addresses.clone())?;
    Ok((file, addresses))
}

fn envelope(file: &WalletFile) -> Result<Value> {
    serde_json::to_value(&file.crypto)
        .map_err(|_| WalletError::validation("cannot compare the wallet master"))
}

fn check_master(plan: &CreatePlan, file: &WalletFile) -> Result<()> {
    if plan.master.as_ref() != Some(&envelope(file)?) {
        return Err(WalletError::validation(
            "wallet.json's master changed while the password was checked; retry",
        ));
    }
    Ok(())
}

fn result(
    plan: &CreatePlan,
    file: &WalletFile,
    addresses: BTreeMap<String, String>,
    verified: bool,
) -> Result<Created> {
    let key = file.key(Some(&plan.name))?;
    Ok(Created {
        name: key.name.clone(),
        index: key.index,
        addresses,
        active: file.active.clone(),
        master_created: plan.creates_master(),
        verified,
        session: None,
        backup_reminder: plan.creates_master().then_some(BACKUP_REMINDER),
    })
}

/// The legacy non-interactive CLI path only: no password and no warm-up. Every address remains
/// explicitly unverified. The TUI must use [`create`] instead.
pub(crate) async fn create_via_daemon(plan: CreatePlan) -> Result<Created> {
    if plan.creates_master() || plan.unlock.is_some() {
        return Err(WalletError::validation(
            "this create requires a human password",
        ));
    }
    let (plan, lock, index) = tokio::task::spawn_blocking(move || {
        let lock = store::MetaLock::acquire(&plan.home)?;
        let file = lock.read()?;
        check_master(&plan, &file)?;
        let index = store::plan_new_key(&file, &plan.name)?;
        Ok::<_, WalletError>((plan, lock, index))
    })
    .await
    .map_err(|_| WalletError::validation("new key preparation did not finish"))??;
    let addresses = derive_via_daemon(&plan.home, index).await?;
    tokio::task::spawn_blocking(move || {
        check_master(&plan, &lock.read()?)?;
        let file = lock.add_key(&plan.name, index, addresses.clone())?;
        result(&plan, &file, addresses, false)
    })
    .await
    .map_err(|_| WalletError::validation("new key creation did not finish"))?
}

/// Preserve the distinction between a confirmed relock and a failed attempt.
pub(crate) fn relocked(mismatch: String, relock: Result<Vec<i32>>) -> WalletError {
    match relock {
        Ok(_) => WalletError::validation(format!("{mismatch}, so the wallet was locked again")),
        Err(e) => WalletError::validation(format!(
            "{mismatch}, and locking the wallet again failed: {}. The wallet may still be \
             unlocked: run `zwallet lock`",
            e.message
        )),
    }
}
