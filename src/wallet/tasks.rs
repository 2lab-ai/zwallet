//! Bounded, advisory task accounting. This module is never used by the key daemon.
//! Local files are claims, RPC replies are observations, and neither is budget authority.
use crate::chains::{self, Chain, Family, Net, evm, tron};
use crate::core::{
    error::{Result, WalletError},
    store,
};
use alloy::primitives::{U256, keccak256};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::Duration,
};

const MAX_RECORDS: usize = 256;
const MAX_TASKS: usize = 256;
const REPORT_TIME: Duration = Duration::from_secs(30);
const METADATA_TIME: Duration = Duration::from_secs(2);

/// Slow files never occupy the async UI loop. A timed-out worker keeps its permit until
/// the syscall ends; a mutating operation may finish later and must not be called cancelled.
async fn metadata<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    static SLOTS: std::sync::LazyLock<std::sync::Arc<tokio::sync::Semaphore>> =
        std::sync::LazyLock::new(|| std::sync::Arc::new(tokio::sync::Semaphore::new(2)));
    let deadline = tokio::time::Instant::now() + METADATA_TIME;
    let late = || {
        WalletError::validation(
            "task metadata operation did not finish in time; it may still complete; inspect task list/report before retrying",
        )
    };
    let permit = tokio::time::timeout_at(deadline, SLOTS.clone().acquire_owned())
        .await
        .map_err(|_| late())?
        .map_err(|_| late())?;
    let worker = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        work()
    });
    let result = tokio::time::timeout_at(deadline, worker)
        .await
        .map_err(|_| late())?
        .map_err(|_| WalletError::validation("task metadata worker stopped"))?;
    if tokio::time::Instant::now() >= deadline {
        return Err(late());
    }
    result
}

#[derive(Clone, Debug)]
pub struct TaskCreate {
    pub id: String,
    pub key: String,
    pub allocations: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSummary {
    pub id: String,
    pub chain: String,
    /// Locally cached account identity, not a verified funding address.
    #[serde(skip_serializing)]
    pub owner: String,
    pub created_at: u64,
    pub intent_count: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskReport {
    pub task_id: String,
    pub chain: String,
    pub observed_at: u64,
    pub observation_started_at: u64,
    pub baseline_observed_at: u64,
    pub baseline_source: String,
    pub current_source: Option<String>,
    pub offline: bool,
    pub owner: String,
    pub records: Vec<TaskRecordReport>,
    pub assets: Vec<TaskAssetReport>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRecordReport {
    pub intent_id: u32,
    pub outcome: Outcome,
    pub tx_hash: Option<String>,
    pub nonce: Option<u64>,
    pub replaces: Option<String>,
    pub same_nonce_attempts: Vec<String>,
    pub kind: String,
    pub asset: String,
    pub amount_raw: String,
    pub recipient: String,
    pub calldata_hash: Option<String>,
    pub actual_fee_raw: Option<String>,
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskAssetReport {
    pub asset: String,
    pub symbol: String,
    pub decimals: u8,
    pub allocation_raw: Option<String>,
    pub baseline_balance_raw: Option<String>,
    pub current_balance_raw: Option<String>,
    /// Signed decimal difference; all account activity, not just this task.
    pub balance_change_raw: Option<String>,
    /// Successfully executed intended direct value, not inferred token transfer effects.
    pub confirmed_intent_raw: String,
    pub known_fees_raw: String,
    /// Present only when every submitted attempt has known accounting; still advisory.
    pub remaining_allocation_raw: Option<String>,
    pub accounting_complete: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Intent,
    Submitted,
    Pending,
    Confirmed,
    Reverted,
    Uncertain,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Intent {
    id: u32,
    at: u64,
    kind: String,
    to: String,
    asset: String,
    amount_raw: String,
    calldata_hash: Option<String>,
    nonce: Option<u64>,
    replaces: Option<String>,
    tx_hash: Option<String>,
    state: Outcome,
    dry_run: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Task {
    version: u32,
    id: String,
    chain: String,
    owner: String,
    created_at: u64,
    baseline_source: String,
    allocations: BTreeMap<String, String>,
    baseline: BTreeMap<String, Option<String>>,
    intents: Vec<Intent>,
}

fn now() -> u64 {
    crate::core::time::unix_secs(std::time::SystemTime::now())
}
fn invalid() -> WalletError {
    WalletError::validation(
        "task metadata is malformed or exceeds its bounds; no task data was trusted",
    )
}
pub fn validate_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 32
        || !id.as_bytes()[0].is_ascii_alphanumeric()
        || !id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
    {
        return Err(WalletError::validation(
            "task ID must be 1-32 lowercase letters, digits, _ or -, starting with a letter or digit",
        ));
    }
    Ok(())
}
fn filename(id: &str) -> Result<String> {
    validate_id(id)?;
    Ok(format!("task-{id}.json"))
}

/// Provenance stores the display origin, never the credential-bearing URL. Redacted host
/// labels contain angle brackets and deliberately are not URLs themselves: substitute a
/// harmless label only for structural validation, without recovering or storing a secret.
fn safe_display_origin(source: &str) -> bool {
    if source == "<redacted>" {
        return true;
    }
    if source.len() > 512 || !source.is_ascii() {
        return false;
    }
    let structural = source.replace("<redacted>", "redacted");
    chains::url_origin(&structural) == structural
}
fn raw(s: &str) -> Result<U256> {
    if s.is_empty() || s.len() > 78 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid());
    }
    U256::from_str_radix(s, 10).map_err(|_| invalid())
}
fn address(chain: &Chain, s: &str) -> Result<String> {
    match chain.family {
        Family::Evm(_) => Ok(evm::parse_address(s)?.to_checksum(None)),
        Family::Tron(_) => Ok(tron::address::encode(&tron::address::parse(s)?)),
    }
}
fn same_address(chain: &Chain, a: &str, b: &str) -> bool {
    address(chain, a)
        .ok()
        .zip(address(chain, b).ok())
        .is_some_and(|(a, b)| a == b)
}
fn hash(chain: &Chain, s: &str) -> Result<String> {
    match chain.family {
        Family::Evm(_) => Ok(format!("{:#x}", evm::parse_tx_hash(s)?)),
        Family::Tron(_) => tron::lookup::parse_hash(s),
    }
}
fn asset(chain: &Chain, input: &str) -> Result<(String, String, u8)> {
    if input == "native" {
        return Ok((input.into(), chain.symbol.into(), chain.decimals));
    }
    match chain.family {
        Family::Evm(config) => {
            let a = evm::parse_address(input).map_err(|_| {
                WalletError::validation("task asset is not a valid registry token address")
            })?;
            let t = evm::registry::lookup(config.chain_id, &a).ok_or_else(|| {
                WalletError::validation("task assets must be native or registry tokens")
            })?;
            Ok((t.address.to_ascii_lowercase(), t.symbol.into(), t.decimals))
        }
        Family::Tron(_) => {
            let t = tron::registry::by_symbol(input)
                .or_else(|| {
                    tron::address::parse(input)
                        .ok()
                        .and_then(|a| tron::registry::lookup(&a))
                })
                .ok_or_else(|| {
                    WalletError::validation("task assets must be native or registry tokens")
                })?;
            Ok((t.address.into(), t.symbol.into(), t.decimals))
        }
    }
}

pub fn parse_allocations(chain: &Chain, inputs: &[String]) -> Result<BTreeMap<String, String>> {
    if inputs.len() > 32 {
        return Err(invalid());
    }
    let mut result = BTreeMap::new();
    for entry in inputs {
        let (key, value) = entry
            .split_once('=')
            .ok_or_else(|| WalletError::validation("allocation must be ASSET=BASE_UNITS"))?;
        let key = asset(chain, key)?.0;
        if result.insert(key, raw(value)?.to_string()).is_some() {
            return Err(WalletError::validation("duplicate allocation asset"));
        }
    }
    Ok(result)
}

impl Task {
    fn summary(&self) -> TaskSummary {
        TaskSummary {
            id: self.id.clone(),
            chain: self.chain.clone(),
            owner: self.owner.clone(),
            created_at: self.created_at,
            intent_count: self.intents.len(),
        }
    }
    fn validate(&self, expected: &str) -> Result<()> {
        validate_id(&self.id)?;
        if self.id != expected
            || self.version != 1
            || self.intents.len() > MAX_RECORDS
            || self.allocations.len() > 32
            || self.baseline.len() > 32
        {
            return Err(invalid());
        }
        if !safe_display_origin(&self.baseline_source) {
            return Err(invalid());
        }
        let chain = chains::find(&self.chain).map_err(|_| invalid())?;
        address(chain, &self.owner).map_err(|_| invalid())?;
        for (key, value) in &self.allocations {
            if asset(chain, key).map_err(|_| invalid())?.0 != *key {
                return Err(invalid());
            }
            raw(value)?;
        }
        for (key, value) in &self.baseline {
            if asset(chain, key).map_err(|_| invalid())?.0 != *key {
                return Err(invalid());
            }
            if let Some(v) = value {
                raw(v)?;
            }
        }
        let mut ids = BTreeSet::new();
        for i in &self.intents {
            if !ids.insert(i.id)
                || i.id == 0
                || !["transfer", "token-transfer", "invoke"].contains(&i.kind.as_str())
            {
                return Err(invalid());
            }
            address(chain, &i.to).map_err(|_| invalid())?;
            if asset(chain, &i.asset).map_err(|_| invalid())?.0 != i.asset
                || (i.kind == "token-transfer") == (i.asset == "native")
                || (i.dry_run && i.tx_hash.is_some())
            {
                return Err(invalid());
            }
            raw(&i.amount_raw)?;
            if let Some(h) = &i.calldata_hash {
                evm::parse_tx_hash(h).map_err(|_| invalid())?;
            }
            for h in [&i.tx_hash, &i.replaces].into_iter().flatten() {
                hash(chain, h).map_err(|_| invalid())?;
            }
            if matches!(
                i.state,
                Outcome::Pending | Outcome::Confirmed | Outcome::Reverted
            ) || (i.tx_hash.is_none() && i.state != Outcome::Intent)
            {
                return Err(invalid());
            }
            if matches!(chain.family, Family::Tron(_))
                && (i.nonce.is_some() || i.replaces.is_some() || i.kind == "invoke")
            {
                return Err(invalid());
            }
        }
        Ok(())
    }
}

fn read(home: &Path, id: &str) -> Result<Task> {
    let bytes = store::read_wallet_bytes(&home.join(filename(id)?))?;
    // Never expose serde's quoted malformed input in an error.
    let task: Task = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    task.validate(id)?;
    Ok(task)
}
fn write(lock: &store::MetaLock, task: &Task) -> Result<()> {
    task.validate(&task.id)?;
    let bytes = serde_json::to_vec_pretty(task).map_err(|_| invalid())?;
    if bytes.len() > 1 << 20 {
        return Err(invalid());
    }
    lock.replace_file(&filename(&task.id)?, &bytes)
}
pub fn info(home: &Path, id: &str) -> Result<TaskSummary> {
    Ok(read(home, id)?.summary())
}

pub fn list(home: &Path) -> Result<Vec<TaskSummary>> {
    let mut result = Vec::new();
    let entries = std::fs::read_dir(home)
        .map_err(|_| WalletError::validation("cannot list task metadata"))?;
    for (count, entry) in entries.enumerate() {
        if count > 4096 {
            return Err(WalletError::validation(
                "wallet home has too many entries to list tasks",
            ));
        }
        let entry = entry.map_err(|_| WalletError::validation("cannot list task metadata"))?;
        let name = entry.file_name();
        let Some(id) = name
            .to_str()
            .and_then(|s| s.strip_prefix("task-"))
            .and_then(|s| s.strip_suffix(".json"))
        else {
            continue;
        };
        result.push(info(home, id)?);
        if result.len() > MAX_TASKS {
            return Err(WalletError::validation("too many task records"));
        }
    }
    result.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(result)
}

async fn balances(net: &Net, owner: &str) -> Result<BTreeMap<String, Option<String>>> {
    let read = super::read::read_network_details(net.clone(), owner.into()).await?;
    let mut values = BTreeMap::from([("native".into(), Some(read.native.raw.to_string()))]);
    for t in read.tokens {
        values.insert(
            asset(net.chain, &t.address)?.0,
            t.raw.ok().map(|v| v.to_string()),
        );
    }
    Ok(values)
}

pub async fn create(home: &Path, net: &Net, input: &TaskCreate) -> Result<TaskSummary> {
    validate_id(&input.id)?;
    let mut allocations = BTreeMap::new();
    for (key, value) in &input.allocations {
        if allocations
            .insert(asset(net.chain, key)?.0, raw(value)?.to_string())
            .is_some()
        {
            return Err(invalid());
        }
    }
    let owner_home = home.to_path_buf();
    let key = input.key.clone();
    let chain = net.chain;
    let runtime = tokio::runtime::Handle::current();
    let (owner, _) =
        metadata(move || runtime.block_on(super::address(&owner_home, &key, chain))).await?;
    let baseline = tokio::time::timeout(super::read::NETWORK_TIMEOUT, balances(net, &owner))
        .await
        .map_err(|_| WalletError::rpc("task baseline balance observation timed out"))??;
    let task = Task {
        version: 1,
        id: input.id.clone(),
        chain: net.chain.key.into(),
        owner,
        created_at: now(),
        baseline_source: chains::url_origin(&net.rpc_url),
        allocations,
        baseline,
        intents: Vec::new(),
    };
    let home = home.to_path_buf();
    metadata(move || {
        let lock = store::MetaLock::acquire(&home)?;
        if std::fs::symlink_metadata(home.join(filename(&task.id)?)).is_ok() {
            return Err(WalletError::validation(
                "task already exists; choose a new task ID",
            ));
        }
        if list(&home)?.len() >= MAX_TASKS {
            return Err(WalletError::validation("too many task records"));
        }
        write(&lock, &task)?;
        Ok(task.summary())
    })
    .await
}

pub fn validate_binding(home: &Path, id: &str, chain: &Chain, owner: &str) -> Result<()> {
    let task = read(home, id)?;
    if task.chain != chain.key || !same_address(chain, &task.owner, owner) {
        return Err(WalletError::validation(
            "task belongs to another chain or account; nothing was signed",
        ));
    }
    Ok(())
}

pub async fn validate_binding_async(
    home: &Path,
    id: &str,
    chain: &'static Chain,
    owner: &str,
) -> Result<()> {
    let home = home.to_path_buf();
    let id = id.to_string();
    let owner = owner.to_string();
    metadata(move || validate_binding(&home, &id, chain, &owner)).await
}

pub(crate) async fn begin_async(
    home: &Path,
    chain: &'static Chain,
    owner: &str,
    fields: &Map<String, Value>,
    kind: &str,
    dry_run: bool,
) -> Result<Option<(String, u32)>> {
    if !fields.contains_key("taskId") {
        return Ok(None);
    }
    let home = home.to_path_buf();
    let owner = owner.to_string();
    let fields = fields.clone();
    let kind = kind.to_string();
    metadata(move || begin(&home, chain, &owner, &fields, &kind, dry_run)).await
}

/// A prepared, shaped intent is persisted before any signature request. No calldata or notes.
pub(crate) fn begin(
    home: &Path,
    chain: &Chain,
    owner: &str,
    fields: &Map<String, Value>,
    kind: &str,
    dry_run: bool,
) -> Result<Option<(String, u32)>> {
    let Some(id) = fields.get("taskId").and_then(Value::as_str) else {
        return Ok(None);
    };
    let lock = store::MetaLock::acquire(home)?;
    let mut task = read(home, id)?;
    if task.chain != chain.key || !same_address(chain, &task.owner, owner) {
        return Err(WalletError::validation(
            "task belongs to another chain or account; nothing was signed",
        ));
    }
    if task.intents.len() >= MAX_RECORDS {
        return Err(WalletError::validation(
            "task has reached its 256 intent limit; nothing was signed",
        ));
    }
    let text = |k: &str| fields.get(k).and_then(Value::as_str);
    let asset = asset(chain, text("token").unwrap_or("native"))?.0;
    let amount_raw = raw(text("amountRaw")
        .or_else(|| text("valueRaw"))
        .ok_or_else(invalid)?)?
    .to_string();
    let calldata_hash = text("data")
        .map(|data| {
            let bytes = alloy::hex::decode(data).map_err(|_| invalid())?;
            Ok::<_, WalletError>(format!("{:#x}", keccak256(bytes)))
        })
        .transpose()?;
    let next = task
        .intents
        .iter()
        .map(|i| i.id)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(invalid)?;
    task.intents.push(Intent {
        id: next,
        at: now(),
        kind: kind.into(),
        to: address(chain, text("to").ok_or_else(invalid)?)?,
        asset,
        amount_raw,
        calldata_hash,
        nonce: fields.get("nonce").and_then(Value::as_u64),
        replaces: text("replaces").map(|h| hash(chain, h)).transpose()?,
        tx_hash: None,
        state: Outcome::Intent,
        dry_run,
    });
    write(&lock, &task)?;
    Ok(Some((id.into(), next)))
}

/// Before broadcast this stores uncertain, so interruption can never look like non-payment.
pub(crate) fn submission(
    home: &Path,
    id: &str,
    intent: u32,
    tx_hash: &str,
    nonce: Option<u64>,
    accepted: bool,
) -> Result<()> {
    let lock = store::MetaLock::acquire(home)?;
    let mut task = read(home, id)?;
    let chain = chains::find(&task.chain)?;
    let i = task
        .intents
        .iter_mut()
        .find(|i| i.id == intent)
        .ok_or_else(invalid)?;
    let tx_hash = hash(chain, tx_hash)?;
    if i.tx_hash.as_ref().is_some_and(|h| h != &tx_hash) {
        return Err(invalid());
    }
    i.tx_hash = Some(tx_hash);
    i.nonce = nonce;
    i.state = if accepted {
        Outcome::Submitted
    } else {
        Outcome::Uncertain
    };
    write(&lock, &task)
}

pub(crate) async fn submission_async(
    home: &Path,
    id: &str,
    intent: u32,
    tx_hash: &str,
    nonce: Option<u64>,
    accepted: bool,
) -> Result<()> {
    let home = home.to_path_buf();
    let id = id.to_string();
    let hash = tx_hash.to_string();
    metadata(move || submission(&home, &id, intent, &hash, nonce, accepted)).await
}

fn shown_address(value: &str, reveal: bool) -> String {
    if reveal {
        value.into()
    } else {
        format!(
            "redacted:{:.16}",
            format!("{:x}", keccak256(value.as_bytes()))
        )
    }
}
fn signed_difference(a: U256, b: U256) -> String {
    if a >= b {
        (a - b).to_string()
    } else {
        format!("-{}", b - a)
    }
}

fn classify(observed: &Map<String, Value>) -> Outcome {
    match observed.get("status").and_then(Value::as_str) {
        Some("pending") => Outcome::Pending,
        Some("mined") => match observed.get("success").and_then(Value::as_bool) {
            Some(true) => Outcome::Confirmed,
            Some(false) => Outcome::Reverted,
            None => Outcome::Uncertain,
        },
        _ => Outcome::Uncertain,
    }
}
fn observed_fee(chain: &Chain, o: &Map<String, Value>) -> Option<String> {
    if matches!(chain.family, Family::Tron(_))
        && o.get("feeKnown").and_then(Value::as_bool) != Some(true)
    {
        return None;
    }
    let key = if matches!(chain.family, Family::Tron(_)) {
        "feeSun"
    } else {
        "feeWei"
    };
    o.get(key)
        .and_then(Value::as_str)
        .and_then(|s| raw(s).ok())
        .map(|v| v.to_string())
}

fn observed_intent(chain: &Chain, i: &Intent, o: &Map<String, Value>) -> bool {
    let text = |key: &str| o.get(key).and_then(Value::as_str);
    let to = text("to");
    let value = text("valueRaw").and_then(|s| raw(s).ok());
    let amount = raw(&i.amount_raw).ok();
    match chain.family {
        Family::Tron(_) => {
            to.is_some_and(|to| same_address(chain, to, &i.to))
                && value == amount
                && match text("token") {
                    None => i.asset == "native",
                    Some(t) => asset(chain, t).is_ok_and(|a| a.0 == i.asset),
                }
        }
        Family::Evm(_) if i.kind == "token-transfer" => {
            let input = evm::parse_address(&i.to)
                .ok()
                .zip(amount)
                .map(|(to, amount)| {
                    format!(
                        "{:#x}",
                        keccak256(evm::tx::erc20_transfer_calldata(to, amount))
                    )
                });
            to.is_some_and(|to| same_address(chain, to, &i.asset))
                && value == Some(U256::ZERO)
                && text("inputHash") == input.as_deref()
        }
        Family::Evm(_) => {
            let input = i
                .calldata_hash
                .clone()
                .unwrap_or_else(|| format!("{:#x}", keccak256([])));
            to.is_some_and(|to| same_address(chain, to, &i.to))
                && value == amount
                && text("inputHash") == Some(input.as_str())
        }
    }
}

async fn observe(net: &Net, hash: &str) -> Result<Map<String, Value>> {
    match net.chain.family {
        Family::Evm(_) => {
            let p = evm::rpc::connect(net).await?;
            evm::rpc::lookup_tx(&p, net, evm::parse_tx_hash(hash)?).await
        }
        Family::Tron(_) => tron::lookup::lookup_tx(net, hash, None).await,
    }
}

async fn before<T>(
    deadline: tokio::time::Instant,
    future: impl std::future::Future<Output = Result<T>>,
) -> Option<T> {
    let result = tokio::time::timeout_at(deadline, future)
        .await
        .ok()
        .and_then(std::result::Result::ok);
    (tokio::time::Instant::now() < deadline)
        .then_some(result)
        .flatten()
}

/// One bounded pass, never polling to finality. Failed/late observations remain uncertain.
pub async fn report(
    home: &Path,
    id: &str,
    net: &Net,
    offline: bool,
    reveal_addresses: bool,
) -> Result<TaskReport> {
    let deadline = tokio::time::Instant::now() + REPORT_TIME;
    let started = now();
    let home = home.to_path_buf();
    let id = id.to_string();
    let task = metadata(move || read(&home, &id)).await?;
    if task.chain != net.chain.key {
        return Err(WalletError::validation(
            "task report RPC chain differs from its task",
        ));
    }
    let mut observations = BTreeMap::new();
    if !offline {
        for i in &task.intents {
            if let Some(hash) = &i.tx_hash
                && !observations.contains_key(hash)
            {
                let value = before(
                    deadline.min(tokio::time::Instant::now() + super::read::NETWORK_TIMEOUT),
                    observe(net, hash),
                )
                .await;
                observations.insert(hash.clone(), value);
            }
        }
    }
    let current = if offline {
        None
    } else {
        before(deadline, balances(net, &task.owner)).await
    };
    let mut result = build_report(
        &task,
        net.chain,
        offline,
        reveal_addresses,
        &observations,
        current.as_ref(),
    )?;
    result.observation_started_at = started;
    result.current_source = (!offline).then(|| chains::url_origin(&net.rpc_url));
    Ok(result)
}

fn build_report(
    task: &Task,
    chain: &Chain,
    offline: bool,
    reveal: bool,
    observations: &BTreeMap<String, Option<Map<String, Value>>>,
    current: Option<&BTreeMap<String, Option<String>>>,
) -> Result<TaskReport> {
    let mut records = Vec::new();
    let mut seen = BTreeSet::new();
    let mut paid = BTreeMap::<String, U256>::new();
    let mut fees = U256::ZERO;
    let mut complete = true;
    let mut mined_nonces = BTreeMap::<u64, BTreeSet<&str>>::new();
    for i in &task.intents {
        if let (Some(nonce), Some(hash)) = (i.nonce, i.tx_hash.as_deref())
            && observations
                .get(hash)
                .and_then(Option::as_ref)
                .is_some_and(|o| o.get("status").and_then(Value::as_str) == Some("mined"))
        {
            mined_nonces.entry(nonce).or_default().insert(hash);
        }
    }
    let mut warnings=vec![
        "Local task/intent/submission records are mutable claims, not authorization or a complete account history. Untagged sends and missing/deleted task files are not discovered.".into(),
        "A confirmed outcome means this node reports successful inclusion, not finality or a verified payment/delivery. Reorgs can change it. Token/contract effects are not inferred from intended value.".into(),
        "Balances are separate latest node observations, not an atomic snapshot. Their change includes all account activity. Fees and allocations never include fiat valuation or establish profit.".into(),
        "Redaction omits account/payee addresses by default, but transaction hashes and task IDs remain linkable public evidence. Do not use personal data in task IDs.".into(),
    ];
    for i in &task.intents {
        let mut outcome = i.state;
        let mut evidence = vec![format!(
            "local intent {} recorded at {}; {}",
            i.id,
            i.at,
            if i.dry_run {
                "dry run, no submission recorded"
            } else {
                "agent-writable claim"
            }
        )];
        let mut fee = None;
        let mut consistent = false;
        if let Some(hash) = &i.tx_hash {
            if let Some(Some(o)) = observations.get(hash) {
                // A node reply must bind back to the task account and nonce. Missing fields do
                // not turn local recipient/amount claims into verified transaction semantics.
                let owner = o
                    .get("from")
                    .and_then(Value::as_str)
                    .is_some_and(|a| same_address(chain, a, &task.owner));
                let nonce = matches!(chain.family, Family::Tron(_))
                    || i.nonce
                        .zip(o.get("nonce").and_then(Value::as_u64))
                        .is_some_and(|(a, b)| a == b);
                let tx_hash = o
                    .get("txHash")
                    .and_then(Value::as_str)
                    .is_some_and(|s| s == hash);
                let nonce_conflict = i
                    .nonce
                    .is_some_and(|n| mined_nonces.get(&n).is_some_and(|hashes| hashes.len() > 1));
                let receipt = matches!(chain.family, Family::Tron(_))
                    || o.get("status").and_then(Value::as_str) != Some("mined")
                    || o.get("receiptConsistent").and_then(Value::as_bool) == Some(true);
                consistent = owner
                    && nonce
                    && tx_hash
                    && observed_intent(chain, i, o)
                    && !nonce_conflict
                    && receipt;
                if nonce_conflict {
                    evidence.push("different transaction hashes at one nonce were reported mined; observations conflict or span a reorg, so none is counted".into());
                }
                if consistent {
                    outcome = classify(o);
                    if o.get("status").and_then(Value::as_str) == Some("mined") {
                        fee = observed_fee(chain, o);
                    }
                    evidence.push("RPC hash, account, nonce and executable recipient/value/calldata match local attribution; node observation, not consensus proof".into());
                } else {
                    outcome = Outcome::Uncertain;
                    evidence.push(
                        "RPC transaction identity or executable intent is missing or different from task attribution".into(),
                    );
                }
                if let Some(block) = o.get("blockNumber").and_then(Value::as_u64) {
                    evidence.push(format!("node block {block}; no finality guarantee"));
                }
            } else if !offline {
                outcome = Outcome::Uncertain;
                evidence.push("RPC observation missing, invalid, failed or exceeded the bounded deadline; absence is not non-payment".into());
            } else {
                evidence.push("offline: submission state is only the last local claim".into());
            }
            if !seen.insert(hash.clone()) {
                complete = false;
                evidence.push("duplicate transaction hash: excluded from totals".into());
            } else {
                if outcome == Outcome::Confirmed && consistent {
                    // These totals deliberately retain their local-intent label, even after a
                    // receipt reports successful execution. Balance deltas are separate.
                    let total = paid.entry(i.asset.clone()).or_default();
                    *total = total.checked_add(raw(&i.amount_raw)?).ok_or_else(invalid)?;
                    if i.kind == "invoke" || i.asset != "native" {
                        complete = false;
                    }
                }
                if let Some(f) = &fee {
                    fees = fees.checked_add(raw(f)?).ok_or_else(invalid)?;
                }
                if fee.is_none() || !matches!(outcome, Outcome::Confirmed | Outcome::Reverted) {
                    complete = false;
                }
            }
        }
        let same_nonce_attempts = if let Some(n) = i.nonce {
            task.intents
                .iter()
                .filter(|other| other.id != i.id && other.nonce == Some(n))
                .filter_map(|other| other.tx_hash.clone())
                .collect()
        } else {
            Vec::new()
        };
        records.push(TaskRecordReport {
            intent_id: i.id,
            outcome,
            tx_hash: i.tx_hash.clone(),
            nonce: i.nonce,
            replaces: i.replaces.clone(),
            same_nonce_attempts,
            kind: i.kind.clone(),
            asset: i.asset.clone(),
            amount_raw: i.amount_raw.clone(),
            recipient: shown_address(&i.to, reveal),
            calldata_hash: i.calldata_hash.clone(),
            actual_fee_raw: fee,
            evidence,
        });
    }
    if current.is_none() && !offline {
        warnings.push("Current balances could not be observed within the report deadline; null does not mean zero.".into());
    }
    let keys = task
        .baseline
        .keys()
        .chain(task.allocations.keys())
        .chain(paid.keys())
        .cloned()
        .chain(std::iter::once("native".into()))
        .collect::<BTreeSet<_>>();
    let mut assets = Vec::new();
    for key in keys {
        let (_, symbol, decimals) = asset(chain, &key)?;
        let baseline = task.baseline.get(&key).cloned().flatten();
        let current = current.and_then(|c| c.get(&key)).cloned().flatten();
        let delta = baseline
            .as_ref()
            .zip(current.as_ref())
            .map(|(b, c)| Ok::<_, WalletError>(signed_difference(raw(c)?, raw(b)?)))
            .transpose()?;
        let payment = paid.get(&key).copied().unwrap_or_default();
        let fee = if key == "native" { fees } else { U256::ZERO };
        let allocation = task.allocations.get(&key).cloned();
        let remaining = if complete {
            allocation
                .as_ref()
                .map(|a| {
                    Ok::<_, WalletError>(signed_difference(
                        raw(a)?,
                        payment.checked_add(fee).ok_or_else(invalid)?,
                    ))
                })
                .transpose()?
        } else {
            None
        };
        assets.push(TaskAssetReport {
            asset: key,
            symbol,
            decimals,
            allocation_raw: allocation,
            baseline_balance_raw: baseline,
            current_balance_raw: current,
            balance_change_raw: delta,
            confirmed_intent_raw: payment.to_string(),
            known_fees_raw: fee.to_string(),
            remaining_allocation_raw: remaining,
            accounting_complete: complete,
        });
    }
    Ok(TaskReport {
        task_id: task.id.clone(),
        chain: task.chain.clone(),
        observed_at: now(),
        observation_started_at: now(),
        baseline_observed_at: task.created_at,
        baseline_source: task.baseline_source.clone(),
        current_source: None,
        offline,
        owner: shown_address(&task.owner, reveal),
        records,
        assets,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    const OWNER: &str = "0x1111111111111111111111111111111111111111";
    const PAYEE: &str = "0x2222222222222222222222222222222222222222";
    const TRON_OWNER: &str = "TWer2Ygk5TEheHp3TPuYeqxmB6SsGZmaL6";
    const TRON_PAYEE: &str = "TPjjvMwjPoDC32V2dGDYTkLH4E5LAtBZ6C";
    fn task() -> Task {
        Task {
            version: 1,
            id: "build-17".into(),
            chain: "ethereum".into(),
            owner: OWNER.into(),
            created_at: 1,
            baseline_source: "http://127.0.0.1:1".into(),
            allocations: BTreeMap::from([("native".into(), "10000".into())]),
            baseline: BTreeMap::from([("native".into(), Some("90071992547409931234".into()))]),
            intents: vec![],
        }
    }
    fn intent(id: u32, state: Outcome) -> Intent {
        Intent {
            id,
            at: 2,
            kind: "transfer".into(),
            to: PAYEE.into(),
            asset: "native".into(),
            amount_raw: "10".into(),
            calldata_hash: None,
            nonce: Some(id.into()),
            replaces: None,
            tx_hash: (state != Outcome::Intent).then(|| format!("0x{id:064x}")),
            state,
            dry_run: false,
        }
    }
    fn observation(i: &Intent, status: &str, success: Option<bool>) -> Map<String, Value> {
        json!({"txHash":i.tx_hash,"from":OWNER,"to":PAYEE,"nonce":i.nonce,"valueRaw":i.amount_raw,
            "inputHash":format!("{:#x}",keccak256([])),"status":status,"success":success,
            "feeWei":"3","blockNumber":7,"receiptConsistent":true})
        .as_object()
        .unwrap()
        .clone()
    }
    fn report_with(
        t: &Task,
        obs: BTreeMap<String, Option<Map<String, Value>>>,
        offline: bool,
    ) -> TaskReport {
        build_report(
            t,
            chains::find(&t.chain).unwrap(),
            offline,
            false,
            &obs,
            None,
        )
        .unwrap()
    }
    #[test]
    fn six_outcomes_are_distinct_and_missing_observation_never_means_failed() {
        let mut t = task();
        t.intents = (1..=6)
            .map(|n| {
                intent(
                    n,
                    if n == 1 {
                        Outcome::Intent
                    } else {
                        Outcome::Submitted
                    },
                )
            })
            .collect();
        let offline = report_with(&t, BTreeMap::new(), true);
        assert_eq!(offline.records[0].outcome, Outcome::Intent);
        assert_eq!(offline.records[1].outcome, Outcome::Submitted);
        let obs = [
            (2, "pending", None),
            (3, "mined", Some(true)),
            (4, "mined", Some(false)),
            (5, "mined", None),
        ]
        .into_iter()
        .map(|(id, status, success)| {
            let i = &t.intents[id - 1];
            (
                i.tx_hash.clone().unwrap(),
                Some(observation(i, status, success)),
            )
        })
        .collect();
        let r = report_with(&t, obs, false);
        assert_eq!(
            r.records.iter().map(|r| r.outcome).collect::<Vec<_>>(),
            vec![
                Outcome::Intent,
                Outcome::Pending,
                Outcome::Confirmed,
                Outcome::Reverted,
                Outcome::Uncertain,
                Outcome::Uncertain
            ]
        );
        assert_eq!(r.assets[0].confirmed_intent_raw, "10");
        // Unknown execution can still have an observed fee; all three mined fees count.
        assert_eq!(r.assets[0].known_fees_raw, "9");
        assert_eq!(r.assets[0].remaining_allocation_raw, None);
        assert_eq!(
            r.records[1].actual_fee_raw, None,
            "pending has no actual receipt fee"
        );
    }
    #[test]
    fn exact_units_reverts_and_balance_delta_are_not_conflated() {
        let mut t = task();
        t.intents = vec![intent(1, Outcome::Submitted), intent(2, Outcome::Submitted)];
        t.intents[0].amount_raw = "9007199254740993".into();
        t.allocations
            .insert("native".into(), "9007199254741000".into());
        let obs = t
            .intents
            .iter()
            .enumerate()
            .map(|(n, i)| {
                (
                    i.tx_hash.clone().unwrap(),
                    Some(observation(i, "mined", Some(n == 0))),
                )
            })
            .collect();
        let current = BTreeMap::from([("native".into(), Some("90071992547409931230".into()))]);
        let r = build_report(
            &t,
            chains::find("ethereum").unwrap(),
            false,
            false,
            &obs,
            Some(&current),
        )
        .unwrap();
        assert_eq!(r.assets[0].confirmed_intent_raw, "9007199254740993");
        assert_eq!(r.assets[0].known_fees_raw, "6");
        assert_eq!(r.assets[0].remaining_allocation_raw.as_deref(), Some("1"));
        assert_eq!(r.assets[0].balance_change_raw.as_deref(), Some("-4"));
        let text = serde_json::to_string(&r).unwrap();
        assert!(!text.contains(OWNER) && !text.contains(PAYEE));
        assert!(r.records[0].recipient.starts_with("redacted:"));
    }
    #[test]
    fn forged_attribution_and_unknown_fees_never_yield_available_allocation() {
        for key in ["txHash", "from", "to", "nonce", "valueRaw", "inputHash"] {
            let mut t = task();
            let i = intent(1, Outcome::Submitted);
            let mut o = observation(&i, "mined", Some(true));
            o.insert(key.into(), Value::Null);
            let hash = i.tx_hash.clone().unwrap();
            t.intents.push(i);
            let r = report_with(&t, BTreeMap::from([(hash, Some(o))]), false);
            assert_eq!(r.records[0].outcome, Outcome::Uncertain, "{key}");
            assert_eq!(r.assets[0].remaining_allocation_raw, None);
        }
        let mut t = task();
        let i = intent(1, Outcome::Submitted);
        let mut o = observation(&i, "mined", Some(true));
        o.remove("feeWei");
        let hash = i.tx_hash.clone().unwrap();
        t.intents.push(i);
        let r = report_with(&t, BTreeMap::from([(hash, Some(o))]), false);
        assert_eq!(r.records[0].outcome, Outcome::Confirmed);
        assert!(!r.assets[0].accounting_complete);
    }
    #[test]
    fn replacement_identity_is_preserved_and_duplicate_hash_is_not_double_counted() {
        let mut t = task();
        let i = intent(1, Outcome::Submitted);
        let mut second = i.clone();
        second.id = 2;
        second.replaces = Some(format!("0x{:064x}", 99));
        t.intents = vec![i.clone(), second];
        let r = report_with(
            &t,
            BTreeMap::from([(
                i.tx_hash.clone().unwrap(),
                Some(observation(&i, "mined", Some(true))),
            )]),
            false,
        );
        assert_eq!(r.assets[0].confirmed_intent_raw, "10");
        assert_eq!(r.assets[0].known_fees_raw, "3");
        assert_eq!(r.records[0].same_nonce_attempts, vec![i.tx_hash.unwrap()]);
        assert!(r.records[1].replaces.is_some());
        assert!(!r.assets[0].accounting_complete);
    }

    #[test]
    fn conflicting_mined_replacements_and_missing_receipt_proof_stay_uncertain() {
        let mut t = task();
        let a = intent(1, Outcome::Submitted);
        let mut b = intent(2, Outcome::Submitted);
        b.nonce = a.nonce;
        t.intents = vec![a, b];
        let obs = t
            .intents
            .iter()
            .map(|i| {
                (
                    i.tx_hash.clone().unwrap(),
                    Some(observation(i, "mined", Some(true))),
                )
            })
            .collect();
        let r = report_with(&t, obs, false);
        assert!(r.records.iter().all(|r| r.outcome == Outcome::Uncertain));
        assert_eq!(r.assets[0].confirmed_intent_raw, "0");
        assert_eq!(r.assets[0].known_fees_raw, "0");
        assert!(!r.assets[0].accounting_complete);
        t.intents.truncate(1);
        let i = &t.intents[0];
        let mut o = observation(i, "mined", Some(true));
        o.remove("receiptConsistent");
        let r = report_with(
            &t,
            BTreeMap::from([(i.tx_hash.clone().unwrap(), Some(o))]),
            false,
        );
        assert_eq!(r.records[0].outcome, Outcome::Uncertain);
    }

    #[test]
    fn summary_serialization_redacts_owner_and_tron_omitted_fee_is_unknown() {
        let t = task();
        let v = serde_json::to_string(&t.summary()).unwrap();
        assert!(!v.contains(OWNER));
        let tron = chains::find("tron").unwrap();
        let fee = json!({"feeSun":"0"}).as_object().unwrap().clone();
        assert_eq!(observed_fee(tron, &fee), None);
        let fee = json!({"feeSun":"0","feeKnown":true})
            .as_object()
            .unwrap()
            .clone();
        assert_eq!(observed_fee(tron, &fee).as_deref(), Some("0"));
        let r = report_with(&t, BTreeMap::new(), true);
        assert_eq!(r.baseline_observed_at, 1);
        assert_eq!(r.baseline_source, "http://127.0.0.1:1");
    }

    #[test]
    fn redacted_source_survives_persistence_without_storing_credentials() {
        let source = chains::url_origin(
            "https://fake-login:fake-password@deadbeef0123.eth.example/v2/fake-key?token=fake-query",
        );
        assert_eq!(source, "https://<redacted>.eth.example");
        let home = tempfile::Builder::new()
            .prefix("zwt-source-")
            .tempdir_in("/tmp")
            .unwrap();
        let mut t = task();
        t.baseline_source = source.clone();
        write(&store::MetaLock::acquire(home.path()).unwrap(), &t).unwrap();
        let restored = read(home.path(), &t.id).unwrap();
        assert_eq!(restored.baseline_source, source);
        assert_eq!(
            report_with(&restored, BTreeMap::new(), true).baseline_source,
            source
        );
        let bytes = std::fs::read_to_string(home.path().join("task-build-17.json")).unwrap();
        for secret in [
            "deadbeef0123",
            "fake-login",
            "fake-password",
            "fake-key",
            "fake-query",
        ] {
            assert!(!bytes.contains(secret));
        }
        for valid in ["<redacted>", "https://rpc.example", "http://127.0.0.1:8545"] {
            assert!(safe_display_origin(valid));
        }
        for invalid in [
            "https://user:password@rpc.example",
            "https://rpc.example/secret",
            "https://rpc.example?secret",
            "https://rpc.example#secret",
            "https://rpc.example\n",
            "https://rpc.example\u{1b}[2J",
            "https://<unknown>.example",
        ] {
            assert!(!safe_display_origin(invalid));
        }
    }

    #[test]
    fn overflowing_totals_and_oversized_task_do_not_wrap_or_truncate() {
        let mut t = task();
        t.intents = vec![intent(1, Outcome::Submitted), intent(2, Outcome::Submitted)];
        t.intents[0].amount_raw = U256::MAX.to_string();
        let observations = t
            .intents
            .iter()
            .map(|i| {
                (
                    i.tx_hash.clone().unwrap(),
                    Some(observation(i, "mined", Some(true))),
                )
            })
            .collect();
        assert!(
            build_report(
                &t,
                chains::find("ethereum").unwrap(),
                false,
                false,
                &observations,
                None
            )
            .is_err()
        );
        t.intents = (1..=257).map(|n| intent(n, Outcome::Intent)).collect();
        assert!(t.validate(&t.id).is_err());
        t.intents.truncate(256);
        assert!(t.validate(&t.id).is_ok());
    }

    #[tokio::test]
    async fn ready_but_late_observation_is_not_accepted() {
        let deadline = tokio::time::Instant::now() + Duration::from_millis(1);
        let late = before(deadline, async {
            std::thread::sleep(Duration::from_millis(5));
            Ok(7)
        })
        .await;
        assert!(late.is_none());
        assert_eq!(
            before(
                tokio::time::Instant::now() + Duration::from_secs(1),
                async { Ok(7) }
            )
            .await,
            Some(7)
        );
    }
    #[test]
    fn tron_native_and_token_reports_use_separate_exact_assets() {
        let mut t = task();
        t.chain = "tron".into();
        t.owner = TRON_OWNER.into();
        let mut i = intent(1, Outcome::Submitted);
        i.to = TRON_PAYEE.into();
        i.tx_hash = Some("aa".repeat(32));
        i.nonce = None;
        let mut token = i.clone();
        token.id = 2;
        token.kind = "token-transfer".into();
        token.asset = tron::registry::TOKENS[0].address.into();
        token.tx_hash = Some("bb".repeat(32));
        token.amount_raw = "1234567".into();
        t.intents = vec![i, token];
        let obs=t.intents.iter().map(|i| {
            let mut o=json!({"txHash":i.tx_hash,"from":TRON_OWNER,"to":TRON_PAYEE,"valueRaw":i.amount_raw,"status":"mined","success":true,"feeSun":"420","feeKnown":true}).as_object().unwrap().clone();
            if i.asset!="native" {o.insert("token".into(),json!(i.asset));}
            (i.tx_hash.clone().unwrap(),Some(o))
        }).collect();
        t.validate(&t.id).unwrap();
        let r = report_with(&t, obs, false);
        assert!(r.records.iter().all(|r| r.outcome == Outcome::Confirmed));
        assert!(r.records.iter().all(|r| r.nonce.is_none()));
        let native = r.assets.iter().find(|a| a.asset == "native").unwrap();
        let token = r.assets.iter().find(|a| a.asset != "native").unwrap();
        assert_eq!(native.known_fees_raw, "840");
        assert_eq!(token.confirmed_intent_raw, "1234567");
        assert_eq!(token.known_fees_raw, "0");
        assert!(
            !native.accounting_complete,
            "token execution alone does not prove a token balance effect"
        );
    }
    #[test]
    fn persistence_preserves_intent_uncertainty_and_serializes_concurrent_writers() {
        let home = tempfile::Builder::new()
            .prefix("zwt-tasks-")
            .tempdir_in("/tmp")
            .unwrap();
        write(&store::MetaLock::acquire(home.path()).unwrap(), &task()).unwrap();
        let fields = json!({"taskId":"build-17","to":PAYEE,"amountRaw":"10"})
            .as_object()
            .unwrap()
            .clone();
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let h = home.path();
                let f = &fields;
                scope.spawn(move || {
                    begin(
                        h,
                        chains::find("ethereum").unwrap(),
                        OWNER,
                        f,
                        "transfer",
                        false,
                    )
                    .unwrap()
                });
            }
        });
        let t = read(home.path(), "build-17").unwrap();
        assert_eq!(t.intents.len(), 8);
        assert_eq!(
            t.intents
                .iter()
                .map(|i| i.id)
                .collect::<BTreeSet<_>>()
                .len(),
            8
        );
        submission(
            home.path(),
            "build-17",
            1,
            &format!("0x{:064x}", 1),
            Some(7),
            false,
        )
        .unwrap();
        assert_eq!(
            read(home.path(), "build-17").unwrap().intents[0].state,
            Outcome::Uncertain
        );
        submission(
            home.path(),
            "build-17",
            1,
            &format!("0x{:064x}", 1),
            Some(7),
            true,
        )
        .unwrap();
        assert_eq!(
            read(home.path(), "build-17").unwrap().intents[0].state,
            Outcome::Submitted
        );
        assert!(
            submission(
                home.path(),
                "build-17",
                1,
                &format!("0x{:064x}", 2),
                Some(7),
                true
            )
            .is_err()
        );
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(home.path().join("task-build-17.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    #[test]
    fn hostile_task_files_names_and_oversized_metadata_are_refused_without_echoing() {
        for id in ["../wallet", "", "UPPER", "a\nsecret", "a/b"] {
            assert!(validate_id(id).is_err());
        }
        let home = tempfile::Builder::new()
            .prefix("zwt-task-bad-")
            .tempdir_in("/tmp")
            .unwrap();
        let path = home.path().join("task-build-17.json");
        let mut v = serde_json::to_value(task()).unwrap();
        v["private-note"] = json!("secret-input-must-not-be-echoed");
        std::fs::write(&path, v.to_string()).unwrap();
        assert!(
            !read(home.path(), "build-17")
                .unwrap_err()
                .message
                .contains("secret-input")
        );
        std::fs::write(&path, vec![b'x'; (1 << 20) + 1]).unwrap();
        assert!(read(home.path(), "build-17").is_err());
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink("wallet.json", &path).unwrap();
        assert!(read(home.path(), "build-17").is_err());
    }
    #[test]
    fn task_binding_and_allocation_schema_do_not_expand_signing_authority() {
        let chain = chains::find("ethereum").unwrap();
        assert!(parse_allocations(chain, &["native=1".into(), "native=2".into()]).is_err());
        assert!(parse_allocations(chain, &["native=1.5".into()]).is_err());
        assert!(parse_allocations(chain, &[format!("{PAYEE}=1")]).is_err());
        let home = tempfile::Builder::new()
            .prefix("zwt-task-bind-")
            .tempdir_in("/tmp")
            .unwrap();
        write(&store::MetaLock::acquire(home.path()).unwrap(), &task()).unwrap();
        assert!(validate_binding(home.path(), "build-17", chain, PAYEE).is_err());
        assert!(
            validate_binding(
                home.path(),
                "build-17",
                chains::find("base").unwrap(),
                OWNER
            )
            .is_err()
        );
    }
}
