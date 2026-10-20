//! Command handlers. Everything here runs in the one-shot CLI process; the only key access is
//! through the daemon socket (or, for `create`/`backup`, a password typed by the human).

use crate::chains::evm::{self, Evm, abi, rpc as evm_rpc};
use crate::chains::{
    self, Broadcast, CHAINS, Chain, ChainFamily, Family, Net, PRIMARY_CHAIN, PreparedTx,
    TransferRequest, redact_url,
};
use crate::cli::{
    AddressArgs, BalanceArgs, CallArgs, Cli, Command, CreateArgs, InvokeArgs, KeyArg, Out,
    SendArgs, SignMessageArgs, TransferArgs, UnlockArgs, render_human,
};
use crate::core::amount::{format_units, parse_units};
use crate::core::error::{ErrorCode, Result, WalletError};
use crate::core::password::{disable_core_dumps, read_password};
use crate::core::protocol::{self, Request, SIGN_TIMEOUT};
use crate::core::seed::{DEFAULT_WORD_COUNT, VALID_WORD_COUNTS};
use crate::core::store::{self, KeyEntry, MasterOptions, WalletFile};
use crate::core::time::parse_duration;
use crate::daemon;
use alloy::hex;
use alloy::primitives::U256;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::Duration;
use zeroize::Zeroizing;

/// How long `--wait` polls for a receipt before reporting `status: "pending"`.
const WAIT_TIMEOUT: Duration = Duration::from_secs(300);

fn home() -> Result<PathBuf> {
    store::resolve_home(std::env::var_os("ZWALLET_HOME"), std::env::home_dir())
}

pub async fn run(cli: Cli) -> Result<Out> {
    match cli.command {
        Command::Chains => Ok(chains_list()),
        Command::Create(a) => create(&home()?, a).await,
        Command::List => list(&home()?).await,
        Command::Change { name } => change(&home()?, &name).await,
        Command::Address(a) => address(&home()?, a).await,
        Command::Addresses(k) => addresses(&home()?, k).await,
        Command::Balance(a) => balance(&home()?, a).await,
        Command::Call(a) => call(&home()?, a).await,
        Command::Invoke(a) => invoke(&home()?, a).await,
        Command::Transfer(a) => transfer(&home()?, a).await,
        Command::SignMessage(a) => sign_message(&home()?, a).await,
        Command::Status => status(&home()?).await,
        Command::Backup => backup(&home()?),
        Command::Unlock(a) => unlock(&home()?, a).await,
        Command::Lock => lock(&home()?).await,
        Command::Daemon => {
            daemon::run(home()?).await?;
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
        return Ok(a.clone());
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
    v["address"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| WalletError::validation("daemon returned no address"))
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
            let a = v["address"]
                .as_str()
                .ok_or_else(|| WalletError::validation("daemon returned no address"))?;
            per_family.insert(family, a.to_string());
        }
        out.insert(chain.key.to_string(), per_family[family].clone());
    }
    Ok(out)
}

fn primary_address(key: &KeyEntry) -> Option<&String> {
    key.addresses
        .get(PRIMARY_CHAIN)
        .or_else(|| key.addresses.values().next())
}

/// The key daemon's own derivation of key `index` on `chain`, or `None` while the wallet is
/// locked (no daemon, or one that just locked itself).
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
    reply["address"]
        .as_str()
        .map(|a| Some(a.to_string()))
        .ok_or_else(|| WalletError::validation("daemon returned no address"))
}

/// wallet.json's cached address for `key` on `chain` must be the one derived from the seed; the
/// cache is plain metadata that anyone running as this user can edit.
fn check_cache(key: &KeyEntry, chain: &str, derived: &str) -> Result<()> {
    match key.addresses.get(chain) {
        Some(cached) if !cached.eq_ignore_ascii_case(derived) => {
            Err(WalletError::validation(format!(
                "wallet.json's cached {chain} address for key '{}' ({cached}) does not match \
                 the address the key daemon derives from the seed ({derived}); the cache was \
                 modified, do not send funds to it",
                key.name
            )))
        }
        _ => Ok(()),
    }
}

/// A key's address on `chain` for display, and whether it was verified: cross-checked against
/// the key daemon while unlocked (a mismatch is `VALIDATION_ERROR`), the unverified cache while
/// locked.
async fn verified_address(home: &Path, key: &KeyEntry, chain: &Chain) -> Result<(String, bool)> {
    if let Some(derived) = daemon_derive(home, key.index, chain).await? {
        check_cache(key, chain.key, &derived)?;
        return Ok((derived, true));
    }
    Ok((key_address(home, key, chain).await?, false))
}

const UNVERIFIED_NOTE: &str = "note: address read from wallet.json's cache, not verified \
                               against the key (the wallet is locked)";

/// Human text for a result object, with the unverified note when it applies.
fn with_note(value: Value, verified: bool) -> Out {
    let mut text = render_human(&value);
    if !verified {
        text.push('\n');
        text.push_str(UNVERIFIED_NOTE);
    }
    Out::with_text(value, text)
}

async fn start_session(home: &Path, password: &str, ttl: Duration) -> Result<Value> {
    protocol::start_daemon(home, password, ttl).await
}

async fn create(home: &Path, a: CreateArgs) -> Result<Out> {
    disable_core_dumps();
    store::validate_key_name(&a.name)?;
    let ttl = a.unlock.as_deref().map(parse_duration).transpose()?;
    if ttl.is_some() {
        protocol::socket_path(home)?;
    }
    let (file, addresses, master, password) = match store::read_wallet_file(home) {
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
            (file, addresses, "created", Some(pw))
        }
        Err(e) => return Err(e),
        Ok(_) => {
            if a.words.is_some() {
                return Err(WalletError::validation(
                    "--words only applies when creating the master seed",
                ));
            }
            let (file, addresses, pw) = add_subkey(home, &a, ttl.is_none()).await?;
            (file, addresses, "existing", pw)
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
    });
    if let (Some(ttl), Some(pw)) = (ttl, password.as_deref()) {
        let session = start_session(home, pw, ttl).await?;
        // The daemon derived the active key's address from the seed on its own; it must be the
        // key this call just created.
        let same = session["active"] == json!(a.name)
            && session["address"]
                .as_str()
                .zip(address.as_deref())
                .is_some_and(|(d, c)| d.eq_ignore_ascii_case(c));
        if !same {
            let _ = protocol::lock(home).await;
            return Err(WalletError::validation(format!(
                "after create the key daemon reports active key {} at {}, not '{}' at {}; \
                 wallet.json changed meanwhile, so the wallet was locked again",
                session["active"],
                session["address"],
                a.name,
                address.as_deref().unwrap_or("-")
            )));
        }
        out["locked"] = json!(false);
        out["expiresAt"] = session["expiresAt"].clone();
        out["daemonPid"] = session["pid"].clone();
    }
    if master == "created" && std::io::stderr().is_terminal() {
        eprintln!(
            "Write down your recovery phrase now: run `zwallet backup` before funding this wallet."
        );
    }
    Ok(Out::json(out))
}

/// Adds key `a.name` to an existing master: derived through the unlocked daemon (no password)
/// when `may_use_daemon`, else from the seed decrypted with a one-off password. The password and
/// scrypt run before the writer lock is taken, so a slow human never blocks other writers; the
/// index is planned, derived and published under the lock, so concurrent creates get distinct
/// indices.
async fn add_subkey(
    home: &Path,
    a: &CreateArgs,
    may_use_daemon: bool,
) -> Result<(
    WalletFile,
    BTreeMap<String, String>,
    Option<Zeroizing<String>>,
)> {
    let local = if may_use_daemon && unlocked(home).await? {
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

async fn list(home: &Path) -> Result<Out> {
    let file = store::read_wallet_file(home)?;
    let primary = chains::find(PRIMARY_CHAIN)?;
    let mut verified = true;
    let mut keys = Vec::new();
    for k in &file.keys {
        // Once the wallet answers as locked, the rest is the cache too.
        let derived = if verified {
            daemon_derive(home, k.index, primary).await?
        } else {
            None
        };
        let address = match derived {
            Some(d) => {
                check_cache(k, primary.key, &d)?;
                Some(d)
            }
            None => {
                verified = false;
                primary_address(k).cloned()
            }
        };
        keys.push(json!({
            "name": k.name,
            "index": k.index,
            "address": address,
            "active": k.name == file.active,
        }));
    }
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
    Ok(Out::with_text(
        json!({ "active": file.active, "keys": keys, "verified": verified }),
        text,
    ))
}

async fn change(home: &Path, name: &str) -> Result<Out> {
    let primary = chains::find(PRIMARY_CHAIN)?;
    // Check the target before switching to it, so a poisoned cache never becomes active.
    let file = store::read_wallet_file(home)?;
    let (address, verified) = verified_address(home, file.key(Some(name))?, primary).await?;
    let file = store::set_active(home, name)?;
    Ok(with_note(
        json!({ "active": file.active, "address": address, "verified": verified }),
        verified,
    ))
}

async fn address(home: &Path, a: AddressArgs) -> Result<Out> {
    let chain = chains::find(&a.chain)?;
    let file = store::read_wallet_file(home)?;
    let key = file.key(a.key.key.as_deref())?;
    let (address, verified) = verified_address(home, key, chain).await?;
    Ok(with_note(
        json!({ "chain": chain.key, "key": key.name, "address": address, "verified": verified }),
        verified,
    ))
}

async fn addresses(home: &Path, k: KeyArg) -> Result<Out> {
    let file = store::read_wallet_file(home)?;
    let key = file.key(k.key.as_deref())?;
    // One daemon derivation per family (EVM chains share an address), checked per chain.
    let mut derived: BTreeMap<&str, Option<String>> = BTreeMap::new();
    let mut verified = true;
    let mut rows = Vec::new();
    for chain in CHAINS {
        let family = chain.family.name();
        if !derived.contains_key(family) {
            let d = if verified {
                daemon_derive(home, key.index, chain).await?
            } else {
                None
            };
            derived.insert(family, d);
        }
        let address = match &derived[family] {
            Some(d) => {
                check_cache(key, chain.key, d)?;
                d.clone()
            }
            None => {
                verified = false;
                key_address(home, key, chain).await?
            }
        };
        rows.push(json!({ "chainId": chain.key, "address": address }));
    }
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
    if !verified {
        text.push('\n');
        text.push_str(UNVERIFIED_NOTE);
    }
    Ok(Out::with_text(
        json!({ "key": key.name, "addresses": rows, "verified": verified }),
        text,
    ))
}

async fn balance(home: &Path, a: BalanceArgs) -> Result<Out> {
    let chain = chains::find(&a.chain)?;
    let file = store::read_wallet_file(home)?;
    let key = file.key(a.key.key.as_deref())?;
    let owner = key_address(home, key, chain).await?;
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
    if let Some(token) = &a.token {
        out["token"] = json!(Evm.normalize_address(token)?);
    }
    Ok(Out::json(out))
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
    let to = evm::parse_address(&a.to)?;
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
    let (from, index, key_name) = signing_key(home, &a.key, chain).await?;
    require_unlocked(home).await?;
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
        max_fee_per_gas,
    };
    let prepared = evm_rpc::prepare(&p, &net, intent).await?;
    let mut base = Map::new();
    base.insert("chain".into(), json!(chain.key));
    base.insert("key".into(), json!(key_name));
    base.insert("from".into(), json!(from));
    base.insert("to".into(), json!(to.to_checksum(None)));
    base.insert("sig".into(), json!(sig));
    base.insert("args".into(), json!(a.args));
    base.insert("data".into(), json!(format!("0x{}", hex::encode(&input))));
    base.insert("value".into(), json!(format_units(value, chain.decimals)));
    base.insert("valueRaw".into(), json!(value.to_string()));
    sign_and_send(&Evm, &net, home, index, &from, prepared, &a.send, base).await
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
    let to = fam.normalize_address(&a.to)?;
    if let Some(t) = &a.token {
        fam.normalize_address(t)?;
    } else {
        parse_units(&a.amount, chain.decimals)?;
    }
    let max_fee_per_gas = max_fee_wei(&a.send)?;
    let (from, index, key_name) = signing_key(home, &a.key, chain).await?;
    require_unlocked(home).await?;
    let req = TransferRequest {
        from: from.clone(),
        to: to.clone(),
        amount: a.amount.clone(),
        token: a.token.clone(),
        dry_run: a.send.dry_run,
        nonce: a.send.nonce,
        max_fee_per_gas,
        force: a.force,
    };
    let prepared = fam.prepare_transfer(net, &req).await?;
    let mut base = Map::new();
    base.insert("chain".into(), json!(chain.key));
    base.insert("key".into(), json!(key_name));
    base.insert("from".into(), json!(from));
    base.insert("to".into(), json!(to));
    sign_and_send(fam, net, home, index, &from, prepared, &a.send, base).await
}

/// Shared tail of `transfer` and `invoke`: sign through the daemon, check the raw bytes really
/// come from `from`, then either report (dry-run) or broadcast (and optionally wait).
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
) -> Result<Out> {
    let signed_value = protocol::send_unlocked(
        home,
        &Request::SignTx {
            chain: net.chain.key.into(),
            index,
            tx: prepared.unsigned.clone(),
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
    for (k, v) in prepared.fields {
        out.entry(k).or_insert(v);
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
        // No rawTx: a dry-run must not hand out a transaction that can be broadcast without
        // zwallet.
        out.insert("recoveredFrom".into(), json!(signed.signer));
        return Ok(Out::json(Value::Object(out)));
    }
    let outcome = fam.broadcast(net, &signed).await?;
    out.insert(
        "explorerUrl".into(),
        json!(net.chain.explorer_tx_url(&signed.tx_hash)),
    );
    if let Broadcast::KnownDespite(why) = outcome {
        out.insert(
            "broadcastWarning".into(),
            json!(format!(
                "{why}; the node reports the transaction, so it was sent"
            )),
        );
    }
    if send.wait {
        let receipt = fam
            .wait_for_receipt(net, &signed.tx_hash, WAIT_TIMEOUT)
            .await;
        chains::record_receipt(&mut out, receipt);
    }
    Ok(Out::json(Value::Object(out)))
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

async fn status(home: &Path) -> Result<Out> {
    let file = match store::read_wallet_file(home) {
        Ok(f) => Some(f),
        Err(e) if e.code == ErrorCode::WalletNotFound => None,
        Err(e) => return Err(e),
    };
    let daemon = protocol::status(home).await?;
    let locked = !daemon.as_ref().is_some_and(|s| s["locked"] == json!(false));
    let mut out = json!({
        "wallet": if file.is_some() { "configured" } else { "not configured" },
        "locked": locked,
        "home": home.display().to_string(),
    });
    if let Some(d) = &daemon {
        out["daemonPid"] = d["pid"].clone();
        if !locked {
            out["expiresAt"] = d["expiresAt"].clone();
        }
    }
    let mut verified = true;
    if let Some(file) = &file {
        out["active"] = json!(file.active);
        if let Ok(key) = file.key(None) {
            let primary = chains::find(PRIMARY_CHAIN)?;
            let derived = if locked {
                None
            } else {
                daemon_derive(home, key.index, primary).await?
            };
            verified = derived.is_some();
            out["address"] = match derived {
                Some(d) => {
                    check_cache(key, primary.key, &d)?;
                    json!(d)
                }
                None => json!(primary_address(key)),
            };
            out["verified"] = json!(verified);
        }
    }
    Ok(with_note(out, verified))
}

async fn unlock(home: &Path, a: UnlockArgs) -> Result<Out> {
    disable_core_dumps();
    let ttl = parse_duration(&a.duration)?;
    protocol::socket_path(home)?;
    // Fail before prompting when there is nothing to unlock. Nothing from this read is reported:
    // the human is shown what the daemon derived from the decrypted seed.
    store::read_wallet_file(home)?;
    let pw = read_password(a.password.password_stdin, "Wallet password: ", false)?;
    let session = start_session(home, &pw, ttl).await?;
    drop(pw);
    Ok(Out::json(json!({
        "locked": false,
        "duration": a.duration.trim(),
        "expiresAt": session["expiresAt"],
        "active": session["active"],
        "address": session["address"],
        "daemonPid": session["pid"],
    })))
}

async fn lock(home: &Path) -> Result<Out> {
    let was_unlocked = protocol::lock(home).await?;
    Ok(Out::json(
        json!({ "locked": true, "wasUnlocked": was_unlocked }),
    ))
}

fn backup(home: &Path) -> Result<Out> {
    disable_core_dumps();
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
