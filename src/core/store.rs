//! On-disk custody: `<home>/wallet.json`.
//!
//! The file is chain-agnostic: a versioned envelope holding the BIP39 mnemonic encrypted with
//! AES-256-GCM under a scrypt key derived from a human-held password, plus a non-secret cache of
//! addresses per chain key. No other file on disk can decrypt it. Home is 0700, the file 0600.
//!
//! Every write of wallet.json holds an exclusive flock on `<home>/w.lock` across read, check,
//! mutate and publish, so concurrent writers serialize instead of losing each other's keys. The
//! lock files (`w.lock`, and the daemon's `d.lock` and `u.lock`) are empty and never unlinked: a
//! waiter may hold an open handle on one at any time.
//!
//! wallet.json is only ever read through [`read_wallet_bytes`]: opened without following a link
//! and without blocking, and accepted only as a regular file of this user's, at most 1 MiB.

use crate::core::error::{ErrorCode, Result, WalletError};
use crate::core::seed::Seed;
use crate::core::time;
use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use alloy::hex;
use rand::RngCore;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs::{self, DirBuilder, File, OpenOptions, TryLockError};
use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

pub const WALLET_FILE: &str = "wallet.json";
/// Writers of wallet.json serialize on an exclusive flock of this file.
pub const WRITE_LOCK: &str = "w.lock";
pub const STORE_VERSION: u32 = 1;
pub const MIN_PASSWORD_LEN: usize = 12;
const CIPHER: &str = "aes-256-gcm";
const KDF: &str = "scrypt";
const KEY_LEN: usize = 32;
const SALT_LEN: usize = 32;
const NONCE_LEN: usize = 12;
const AAD: &[u8] = b"zwallet/wallet.json/v1";
pub const DIR_MODE: u32 = 0o700;
pub const FILE_MODE: u32 = 0o600;
/// How long a writer waits for another writer's `w.lock` before giving up.
const WRITE_LOCK_WAIT: Duration = Duration::from_secs(10);
/// The most wallet.json may weigh; a larger file is refused before it is parsed.
const MAX_WALLET_FILE: u64 = 1 << 20;

/// Resolves the wallet home: `$ZWALLET_HOME` if set and non-empty, else `<user home>/.zwallet`.
/// `user_home` is supplied by the caller (`std::env::home_dir`) so this stays testable.
pub fn resolve_home(zwallet_home: Option<OsString>, user_home: Option<PathBuf>) -> Result<PathBuf> {
    let chosen = match zwallet_home.filter(|v| !v.is_empty()) {
        Some(v) => PathBuf::from(v),
        None => match user_home.filter(|p| !p.as_os_str().is_empty()) {
            Some(h) => h.join(".zwallet"),
            None => {
                return Err(WalletError::validation(
                    "cannot locate the wallet home: set ZWALLET_HOME (no home directory found)",
                ));
            }
        },
    };
    std::path::absolute(&chosen).map_err(|e| {
        WalletError::validation(format!("invalid wallet home {}: {e}", chosen.display()))
    })
}

/// Checks a new password chosen at `wallet create`.
pub fn validate_new_password(pw: &str) -> Result<()> {
    if pw.chars().count() < MIN_PASSWORD_LEN {
        return Err(WalletError::validation(format!(
            "password must be at least {MIN_PASSWORD_LEN} characters"
        )));
    }
    if pw.contains(['\n', '\r', '\0']) {
        return Err(WalletError::validation(
            "password must not contain newline or NUL characters",
        ));
    }
    Ok(())
}

/// scrypt cost parameters, stored in the file so they can change without breaking old wallets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdfParams {
    pub log_n: u8,
    pub r: u32,
    pub p: u32,
}

/// The only scrypt parameters zwallet writes or reads: r = 8, p = 1 and
/// 2^15 <= N <= 2^18. Checked before any key derivation, so a tampered wallet.json can make a
/// process allocate at most 128 * r * N = 256 MiB before the GCM tag rejects it.
pub const KDF_R: u32 = 8;
pub const KDF_P: u32 = 1;
pub const KDF_MIN_LOG_N: u8 = 15;
pub const KDF_MAX_LOG_N: u8 = 18;

impl Default for KdfParams {
    /// N = 2^18, r = 8, p = 1: 256 MiB, roughly 0.5 s per unlock in an optimized build.
    fn default() -> Self {
        Self {
            log_n: KDF_MAX_LOG_N,
            r: KDF_R,
            p: KDF_P,
        }
    }
}

impl KdfParams {
    /// Accepts only the parameters described at [`KDF_R`]; anything else is `VALIDATION_ERROR`.
    pub fn checked(log_n: u8, r: u32, p: u32) -> Result<Self> {
        if r != KDF_R || p != KDF_P || !(KDF_MIN_LOG_N..=KDF_MAX_LOG_N).contains(&log_n) {
            return Err(WalletError::validation(format!(
                "wallet.json is corrupt: scrypt parameters log_n={log_n} r={r} p={p} are outside \
                 the accepted range (r={KDF_R}, p={KDF_P}, log_n {KDF_MIN_LOG_N}..={KDF_MAX_LOG_N})"
            )));
        }
        Ok(Self { log_n, r, p })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WalletFile {
    pub version: u32,
    pub created_at: String,
    /// Name of the key every command uses unless `--key` is given.
    pub active: String,
    /// Next derivation index; indices are assigned in creation order and never reused.
    pub next_index: u32,
    pub keys: Vec<KeyEntry>,
    pub crypto: CryptoBox,
}

/// A named subkey of the master seed. Non-secret: its addresses are a cache so locked commands
/// work without the key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyEntry {
    pub name: String,
    pub index: u32,
    pub created_at: String,
    /// chain key → address
    pub addresses: BTreeMap<String, String>,
}

impl WalletFile {
    /// The named key, or the active key when `name` is `None`.
    pub fn key(&self, name: Option<&str>) -> Result<&KeyEntry> {
        let wanted = name.unwrap_or(&self.active);
        self.keys
            .iter()
            .find(|k| k.name == wanted)
            .ok_or_else(|| key_not_found(wanted))
    }
}

fn key_not_found(name: &str) -> WalletError {
    WalletError::new(
        ErrorCode::KeyNotFound,
        format!("no key named '{name}' (see `zwallet list`)"),
    )
}

/// Key names: `^[a-z0-9][a-z0-9_-]{0,31}$`.
pub fn validate_key_name(name: &str) -> Result<()> {
    let bytes = name.as_bytes();
    let ok = !bytes.is_empty()
        && bytes.len() <= 32
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_' || *b == b'-');
    if ok {
        Ok(())
    } else {
        Err(WalletError::validation(format!(
            "invalid key name '{name}': use 1-32 of a-z 0-9 _ - starting with a letter or digit"
        )))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CryptoBox {
    pub cipher: String,
    pub kdf: String,
    pub kdfparams: StoredKdfParams,
    pub nonce: String,
    pub ciphertext: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredKdfParams {
    pub log_n: u8,
    pub r: u32,
    pub p: u32,
    pub dklen: u32,
    pub salt: String,
}

fn derive_key(password: &str, salt: &[u8], kdf: &KdfParams) -> Result<Zeroizing<[u8; KEY_LEN]>> {
    let kdf = KdfParams::checked(kdf.log_n, kdf.r, kdf.p)?;
    let params = scrypt::Params::new(kdf.log_n, kdf.r, kdf.p, KEY_LEN)
        .map_err(|_| WalletError::validation("invalid scrypt parameters"))?;
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    scrypt::scrypt(password.as_bytes(), salt, &params, key.as_mut())
        .map_err(|_| WalletError::validation("scrypt key derivation failed"))?;
    Ok(key)
}

/// Version of the plaintext sealed under the password-derived key.
pub const SEALED_VERSION: u32 = 1;

/// The plaintext under the password-derived key (authenticated by AES-GCM). Anything a signing
/// policy depends on must live here: everything outside it can be edited by the same OS user
/// without the password. Deliberately not `Debug`.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SealedPayload {
    version: u32,
    mnemonic: Zeroizing<String>,
    /// Reserved for a future signing policy; `{}` for now and carried through intact.
    policy: Value,
}

/// A decrypted wallet: the master seed and the sealed policy object.
pub struct Unsealed {
    pub seed: Seed,
    pub policy: Value,
}

/// Seals the mnemonic and `policy` (a JSON object) with a fresh salt and nonce.
pub fn encrypt(seed: &Seed, policy: &Value, password: &str, kdf: KdfParams) -> Result<CryptoBox> {
    if !policy.is_object() {
        return Err(WalletError::validation("policy must be a JSON object"));
    }
    let plaintext = Zeroizing::new(
        serde_json::to_vec(&SealedPayload {
            version: SEALED_VERSION,
            mnemonic: Zeroizing::new(seed.phrase().to_string()),
            policy: policy.clone(),
        })
        .map_err(|_| WalletError::validation("cannot serialize the sealed payload"))?,
    );
    let mut salt = [0u8; SALT_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    OsRng
        .try_fill_bytes(&mut salt)
        .and_then(|_| OsRng.try_fill_bytes(&mut nonce))
        .map_err(|_| WalletError::validation("could not read OS randomness"))?;
    let key = derive_key(password, &salt, &kdf)?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key.as_ref()));
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plaintext.as_slice(),
                aad: AAD,
            },
        )
        .map_err(|_| WalletError::validation("encryption failed"))?;
    Ok(CryptoBox {
        cipher: CIPHER.into(),
        kdf: KDF.into(),
        kdfparams: StoredKdfParams {
            log_n: kdf.log_n,
            r: kdf.r,
            p: kdf.p,
            dklen: KEY_LEN as u32,
            salt: hex::encode(salt),
        },
        nonce: hex::encode(nonce),
        ciphertext: hex::encode(ciphertext),
    })
}

/// Decrypts the sealed payload. A failed GCM tag check is reported as `WRONG_PASSWORD`.
pub fn decrypt(sealed: &CryptoBox, password: &str) -> Result<Unsealed> {
    let corrupt = |what: &str| WalletError::validation(format!("wallet.json is corrupt: {what}"));
    if sealed.cipher != CIPHER || sealed.kdf != KDF {
        return Err(corrupt("unsupported cipher or kdf"));
    }
    let k = &sealed.kdfparams;
    // Before anything allocates: bounds the memory a tampered file can demand.
    let kdf = KdfParams::checked(k.log_n, k.r, k.p)?;
    if k.dklen as usize != KEY_LEN {
        return Err(corrupt("kdf parameters out of range"));
    }
    let salt = hex::decode(&k.salt).map_err(|_| corrupt("salt is not hex"))?;
    let nonce = hex::decode(&sealed.nonce).map_err(|_| corrupt("nonce is not hex"))?;
    let ciphertext =
        hex::decode(&sealed.ciphertext).map_err(|_| corrupt("ciphertext is not hex"))?;
    if salt.len() < 16 || nonce.len() != NONCE_LEN {
        return Err(corrupt("bad salt or nonce length"));
    }
    let key = derive_key(password, &salt, &kdf)?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key.as_ref()));
    let plain = Zeroizing::new(
        cipher
            .decrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &ciphertext,
                    aad: AAD,
                },
            )
            .map_err(|_| {
                WalletError::new(
                    ErrorCode::WrongPassword,
                    "wrong password: wallet.json could not be decrypted",
                )
            })?,
    );
    let payload: SealedPayload =
        serde_json::from_slice(&plain).map_err(|_| corrupt("sealed payload is malformed"))?;
    if payload.version != SEALED_VERSION {
        return Err(corrupt("unsupported sealed payload version"));
    }
    if !payload.policy.is_object() {
        return Err(corrupt("sealed policy is not an object"));
    }
    Ok(Unsealed {
        seed: Seed::from_phrase(payload.mnemonic)?,
        policy: payload.policy,
    })
}

/// Derives one key's address cache (chain key → address) at a derivation index. Supplied by the
/// chain registry so core stays chain-agnostic.
pub type AddressDeriver<'a> = &'a dyn Fn(&Seed, u32) -> Result<BTreeMap<String, String>>;

/// Reads `wallet.json` metadata without decrypting.
pub fn read_wallet_file(home: &Path) -> Result<WalletFile> {
    match parse_wallet_file(&home.join(WALLET_FILE)) {
        Err(e) if e.code == ErrorCode::WalletNotFound => Err(WalletError::new(
            ErrorCode::WalletNotFound,
            format!(
                "no wallet found in {} (a human runs `zwallet create <name>`)",
                home.display()
            ),
        )),
        other => other,
    }
}

fn parse_wallet_file(path: &Path) -> Result<WalletFile> {
    let bytes = read_wallet_bytes(path)?;
    let file: WalletFile = serde_json::from_slice(&bytes)
        .map_err(|e| WalletError::validation(format!("{} is not valid: {e}", path.display())))?;
    if file.version != STORE_VERSION {
        return Err(WalletError::validation(format!(
            "unsupported wallet.json version {} (this zwallet reads version {STORE_VERSION})",
            file.version
        )));
    }
    Ok(file)
}

/// Reads a wallet file without ever blocking or following a link: it is opened with
/// `O_NOFOLLOW | O_NONBLOCK` (so a FIFO planted in its place cannot hang the reader, e.g. the
/// daemon's tick under its session lock), and must then be a regular file owned by this user and
/// at most 1 MiB. A missing file is `WALLET_NOT_FOUND`; anything else wrong is
/// `VALIDATION_ERROR`.
pub fn read_wallet_bytes(path: &Path) -> Result<Vec<u8>> {
    let refuse = |why: String| {
        WalletError::validation(format!("refusing to read {}: {why}", path.display()))
    };
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(f) => f,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            return Err(WalletError::new(
                ErrorCode::WalletNotFound,
                format!("{} does not exist", path.display()),
            ));
        }
        Err(e) if e.raw_os_error() == Some(libc::ELOOP) => {
            return Err(refuse("it is a symbolic link".into()));
        }
        Err(e) => {
            return Err(WalletError::validation(format!(
                "cannot read {}: {e}",
                path.display()
            )));
        }
    };
    let meta = file
        .metadata()
        .map_err(|e| WalletError::validation(format!("cannot inspect {}: {e}", path.display())))?;
    if !meta.file_type().is_file() {
        return Err(refuse("it is not a regular file".into()));
    }
    // SAFETY: geteuid has no preconditions and cannot fail.
    let me = unsafe { libc::geteuid() };
    if meta.uid() != me {
        return Err(refuse(format!(
            "it is owned by uid {}, not this user",
            meta.uid()
        )));
    }
    let too_big = || refuse("it is larger than 1 MiB".into());
    if meta.len() > MAX_WALLET_FILE {
        return Err(too_big());
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    // The file can grow after fstat: never read more than the limit.
    (&file)
        .take(MAX_WALLET_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| WalletError::validation(format!("cannot read {}: {e}", path.display())))?;
    if bytes.len() as u64 > MAX_WALLET_FILE {
        return Err(too_big());
    }
    Ok(bytes)
}

/// Decrypts the master seed and checks every key's cached addresses against it.
pub fn unlock(
    home: &Path,
    password: &str,
    derive: AddressDeriver<'_>,
) -> Result<(Unsealed, WalletFile)> {
    let file = read_wallet_file(home)?;
    let unsealed = decrypt(&file.crypto, password)?;
    verify_key_cache(&file, &unsealed.seed, derive)?;
    Ok((unsealed, file))
}

/// Every key has a distinct name and index, and every cached address is the one the seed
/// derives at that index.
fn verify_key_cache(file: &WalletFile, seed: &Seed, derive: AddressDeriver<'_>) -> Result<()> {
    let inconsistent =
        |what: String| WalletError::validation(format!("wallet.json is inconsistent: {what}"));
    let (mut names, mut indices) = (BTreeSet::new(), BTreeSet::new());
    for key in &file.keys {
        if !names.insert(key.name.as_str()) {
            return Err(inconsistent(format!("two keys are named '{}'", key.name)));
        }
        if !indices.insert(key.index) {
            return Err(inconsistent(format!(
                "key '{}' reuses derivation index {}",
                key.name, key.index
            )));
        }
    }
    for key in &file.keys {
        let derived = derive(seed, key.index)?;
        for (chain, cached) in &key.addresses {
            if let Some(actual) = derived.get(chain)
                && !actual.eq_ignore_ascii_case(cached)
            {
                return Err(inconsistent(format!(
                    "cached {chain} address of key '{}' does not match the master seed",
                    key.name
                )));
            }
        }
    }
    Ok(())
}

/// Options for creating the master seed.
#[derive(Debug, Clone, Copy)]
pub struct MasterOptions {
    pub words: usize,
    pub kdf: KdfParams,
}

impl Default for MasterOptions {
    fn default() -> Self {
        Self {
            words: crate::core::seed::DEFAULT_WORD_COUNT,
            kdf: KdfParams::default(),
        }
    }
}

/// Points in [`create_master`] where tests can inject a failure (or tamper with a file).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateStage<'a> {
    /// The temp file is written and fsynced, not yet read back or published.
    TempWritten(&'a Path),
    /// wallet.json is in place (linked from the verified temp file).
    Published(&'a Path),
}

/// Private temp files this invocation created; on a failure before publication exactly these are
/// removed. Never the home or a lock file: another creator may already be waiting on `w.lock`
/// in that home, and unlinking it would let a third one lock a different file.
#[derive(Default)]
struct Journal {
    temps: Vec<PathBuf>,
}

impl Journal {
    fn rollback(&self) {
        for f in self.temps.iter().rev() {
            let _ = fs::remove_file(f);
        }
    }
}

/// Creates the master seed in `home`, encrypted under `password`, with its first key
/// `first_key` at index 0 (active). The returned file's addresses were derived from the
/// in-memory seed this call generated.
///
/// Writes a temp file, reads it back from disk, decrypts it with the same password, checks the
/// derived addresses and the phrase, and only then links it into place as `wallet.json` (link,
/// unlike rename, never clobbers a concurrent wallet). A failure before that removes only this
/// call's temp files; a home directory it created stays, with its lock file. Once published,
/// wallet.json is never removed: a later failure returns an error naming the file and saying
/// recovery is needed. Refuses if `wallet.json` already exists.
pub fn create_master(
    home: &Path,
    password: &str,
    first_key: &str,
    opts: MasterOptions,
    derive: AddressDeriver<'_>,
) -> Result<WalletFile> {
    create_master_with_hook(home, password, first_key, opts, derive, &|_| Ok(()))
}

pub fn create_master_with_hook(
    home: &Path,
    password: &str,
    first_key: &str,
    opts: MasterOptions,
    derive: AddressDeriver<'_>,
    hook: &dyn Fn(CreateStage<'_>) -> Result<()>,
) -> Result<WalletFile> {
    validate_key_name(first_key)?;
    validate_new_password(password)?;
    if !crate::core::seed::VALID_WORD_COUNTS.contains(&opts.words) {
        return Err(WalletError::validation(format!(
            "--words must be one of {:?}, got {}",
            crate::core::seed::VALID_WORD_COUNTS,
            opts.words
        )));
    }
    let mut journal = Journal::default();
    let wallet_path = home.join(WALLET_FILE);
    match create_steps(home, password, first_key, opts, derive, hook, &mut journal) {
        Ok(file) => Ok(file),
        Err(Failure::Before(e)) => {
            journal.rollback();
            Err(e)
        }
        Err(Failure::After(e)) => Err(published_but_failed(&wallet_path, &e)),
    }
}

/// Where a create failed relative to publishing wallet.json.
enum Failure {
    Before(WalletError),
    After(WalletError),
}

fn published_but_failed(path: &Path, cause: &WalletError) -> WalletError {
    WalletError::new(
        cause.code,
        format!(
            "{} was published, then create failed: {}. The file was left in place; recovery \
             is needed: check it with `zwallet list` and `zwallet unlock`, or move it aside \
             before creating again",
            path.display(),
            cause.message
        ),
    )
}

fn create_steps(
    home: &Path,
    password: &str,
    first_key: &str,
    opts: MasterOptions,
    derive: AddressDeriver<'_>,
    hook: &dyn Fn(CreateStage<'_>) -> Result<()>,
    journal: &mut Journal,
) -> std::result::Result<WalletFile, Failure> {
    let wallet_path = home.join(WALLET_FILE);
    if fs::symlink_metadata(&wallet_path).is_ok() {
        return Err(Failure::Before(wallet_exists(home)));
    }
    ensure_home_dir(home).map_err(Failure::Before)?;
    let _lock = MetaLock::acquire(home).map_err(Failure::Before)?;
    let before = |e| Failure::Before(e);
    if fs::symlink_metadata(&wallet_path).is_ok() {
        return Err(before(wallet_exists(home)));
    }

    let seed = Seed::generate(opts.words).map_err(before)?;
    let created_at = time::now_rfc3339();
    let file = WalletFile {
        version: STORE_VERSION,
        created_at: created_at.clone(),
        active: first_key.to_string(),
        next_index: 1,
        keys: vec![KeyEntry {
            name: first_key.to_string(),
            index: 0,
            created_at,
            addresses: derive(&seed, 0).map_err(before)?,
        }],
        crypto: encrypt(
            &seed,
            &Value::Object(Default::default()),
            password,
            opts.kdf,
        )
        .map_err(before)?,
    };
    let bytes = to_json_bytes(&file).map_err(before)?;
    let tmp = temp_path(home);
    write_new_file(&tmp, &bytes, journal).map_err(before)?;
    hook(CreateStage::TempWritten(&tmp)).map_err(before)?;

    // Verify exactly what a later process will see, before it can see it: the temp file as read
    // back from disk.
    let verify = || -> Result<()> {
        let on_disk = parse_wallet_file(&tmp)?;
        let opened = decrypt(&on_disk.crypto, password)?;
        verify_key_cache(&on_disk, &opened.seed, derive)?;
        if on_disk.keys != file.keys
            || on_disk.active != file.active
            || opened.seed.phrase() != seed.phrase()
        {
            return Err(WalletError::validation("read-back mismatch"));
        }
        Ok(())
    };
    verify().map_err(|e| {
        before(WalletError::validation(format!(
            "post-write verification failed, wallet not kept: {}",
            e.message
        )))
    })?;
    drop(seed);

    publish_new_file(home, &tmp, &wallet_path).map_err(before)?;
    // Published: from here on wallet.json is never removed, whatever fails.
    let after = |e| Failure::After(e);
    if let Some(pos) = journal.temps.iter().position(|t| *t == tmp) {
        journal.temps.remove(pos);
        let _ = fs::remove_file(&tmp);
    }
    sync_dir(home).map_err(after)?;
    hook(CreateStage::Published(&wallet_path)).map_err(after)?;
    match read_wallet_bytes(&wallet_path) {
        Ok(b) if b == bytes => Ok(file),
        Ok(_) => Err(after(WalletError::validation(
            "wallet.json changed right after it was written",
        ))),
        Err(e) => Err(after(WalletError::validation(format!(
            "cannot read it back: {}",
            e.message
        )))),
    }
}

/// The derivation index the next [`add_key`] will use, after checking the name is free: past
/// both `nextIndex` and every index in use, so a hand-edited counter can never reuse a key. An
/// index with no successor to store as the next `nextIndex` is `VALIDATION_ERROR`.
pub fn plan_new_key(file: &WalletFile, name: &str) -> Result<u32> {
    validate_key_name(name)?;
    if file.keys.iter().any(|k| k.name == name) {
        return Err(WalletError::new(
            ErrorCode::KeyExists,
            format!("a key named '{name}' already exists"),
        ));
    }
    let past_used = match file.keys.iter().map(|k| k.index).max() {
        Some(i) => i.checked_add(1).ok_or_else(no_index_left)?,
        None => 0,
    };
    let index = file.next_index.max(past_used);
    index.checked_add(1).ok_or_else(no_index_left)?;
    Ok(index)
}

fn no_index_left() -> WalletError {
    WalletError::validation("wallet.json has no derivation index left for a new key")
}

/// Exclusive writer lock on `<home>/w.lock`, held across read → check → mutate → publish.
pub struct MetaLock {
    home: PathBuf,
    _file: File,
}

impl MetaLock {
    /// Waits (bounded) for the lock. `home` must exist.
    pub fn acquire(home: &Path) -> Result<Self> {
        let file = lock_exclusive(
            &home.join(WRITE_LOCK),
            WRITE_LOCK_WAIT,
            "another zwallet is writing wallet.json",
        )?;
        Ok(Self {
            home: home.to_path_buf(),
            _file: file,
        })
    }

    /// wallet.json as of now; it cannot change under this lock except by a process that ignores
    /// it.
    pub fn read(&self) -> Result<WalletFile> {
        read_wallet_file(&self.home)
    }

    /// Adds key `name` at `index` (planned with [`plan_new_key`] under this lock) with its
    /// derived addresses and makes it active. The encrypted master is carried over byte-for-byte.
    pub fn add_key(
        &self,
        name: &str,
        index: u32,
        addresses: BTreeMap<String, String>,
    ) -> Result<WalletFile> {
        self.update(|file| {
            if plan_new_key(file, name)? != index {
                return Err(WalletError::validation(
                    "wallet.json changed while the key was being derived; retry",
                ));
            }
            file.keys.push(KeyEntry {
                name: name.to_string(),
                index,
                created_at: time::now_rfc3339(),
                addresses: addresses.clone(),
            });
            file.next_index = index.checked_add(1).ok_or_else(no_index_left)?;
            file.active = name.to_string();
            Ok(())
        })
    }

    /// Makes `name` the active key (`KEY_NOT_FOUND` if there is none).
    pub fn set_active(&self, name: &str) -> Result<WalletFile> {
        self.update(|file| {
            file.key(Some(name))?;
            file.active = name.to_string();
            Ok(())
        })
    }

    /// Read-modify-write of the non-secret metadata. The crypto box must come out unchanged,
    /// and the new file replaces the old one atomically (temp file + rename), so the encrypted
    /// master is never at risk even if this process dies midway.
    fn update(&self, mutate: impl FnOnce(&mut WalletFile) -> Result<()>) -> Result<WalletFile> {
        let home = &self.home;
        let mut file = read_wallet_file(home)?;
        let crypto_before = serde_json::to_value(&file.crypto).ok();
        mutate(&mut file)?;
        if serde_json::to_value(&file.crypto).ok() != crypto_before {
            return Err(WalletError::validation(
                "refusing to rewrite the encrypted master",
            ));
        }
        let path = home.join(WALLET_FILE);
        let tmp = temp_path(home);
        let mut journal = Journal::default();
        let written = write_new_file(&tmp, &to_json_bytes(&file)?, &mut journal).and_then(|()| {
            fs::rename(&tmp, &path).map_err(|e| {
                WalletError::validation(format!("cannot replace {}: {e}", path.display()))
            })
        });
        if written.is_err() {
            journal.rollback();
        }
        written?;
        sync_dir(home)?;
        let on_disk = read_wallet_file(home)?;
        if on_disk.keys != file.keys || on_disk.active != file.active {
            return Err(WalletError::validation(
                "wallet.json read-back mismatch after update",
            ));
        }
        Ok(on_disk)
    }
}

/// [`MetaLock::add_key`] under a lock taken for just this call.
pub fn add_key(
    home: &Path,
    name: &str,
    index: u32,
    addresses: BTreeMap<String, String>,
) -> Result<WalletFile> {
    read_wallet_file(home)?;
    MetaLock::acquire(home)?.add_key(name, index, addresses)
}

/// [`MetaLock::set_active`] under a lock taken for just this call.
pub fn set_active(home: &Path, name: &str) -> Result<WalletFile> {
    read_wallet_file(home)?;
    MetaLock::acquire(home)?.set_active(name)
}

/// Opens (creating 0600 if needed, never following a symlink) and exclusively flocks `path`,
/// retrying until `wait` runs out. The lock lasts as long as the returned file is open.
pub fn lock_exclusive(path: &Path, wait: Duration, busy: &str) -> Result<File> {
    let file = open_lock_file(path)?;
    let deadline = Instant::now() + wait;
    while !try_flock(&file, path)? {
        if Instant::now() >= deadline {
            return Err(WalletError::validation(format!(
                "{busy}: {} is locked (waited {}s)",
                path.display(),
                wait.as_secs()
            )));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(file)
}

/// [`lock_exclusive`] without waiting: `None` while another holder has the lock.
pub fn try_lock_exclusive(path: &Path) -> Result<Option<File>> {
    let file = open_lock_file(path)?;
    Ok(try_flock(&file, path)?.then_some(file))
}

fn open_lock_file(path: &Path) -> Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(FILE_MODE)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|e| WalletError::validation(format!("cannot open {}: {e}", path.display())))
}

/// Takes `file`'s exclusive flock if it is free; `false` while another holder has it.
fn try_flock(file: &File, path: &Path) -> Result<bool> {
    match file.try_lock() {
        Ok(()) => Ok(true),
        Err(TryLockError::WouldBlock) => Ok(false),
        Err(TryLockError::Error(e)) => Err(WalletError::validation(format!(
            "cannot lock {}: {e}",
            path.display()
        ))),
    }
}

fn to_json_bytes(file: &WalletFile) -> Result<Vec<u8>> {
    let mut json = serde_json::to_vec_pretty(file)
        .map_err(|e| WalletError::validation(format!("cannot serialize wallet: {e}")))?;
    json.push(b'\n');
    Ok(json)
}

fn temp_path(home: &Path) -> PathBuf {
    home.join(format!(
        ".{WALLET_FILE}.tmp-{}-{:08x}",
        std::process::id(),
        OsRng.next_u32()
    ))
}

/// fsyncs a directory so a new or renamed entry in it survives a crash.
fn sync_dir(dir: &Path) -> Result<()> {
    File::open(dir)
        .and_then(|d| d.sync_all())
        .map_err(|e| WalletError::validation(format!("cannot sync {}: {e}", dir.display())))
}

fn wallet_exists(home: &Path) -> WalletError {
    WalletError::new(
        ErrorCode::WalletExists,
        format!(
            "a wallet already exists in {}; refusing to overwrite it",
            home.display()
        ),
    )
}

/// Creates `home` with 0700, or tightens an existing one to 0700. Only the final directory is
/// created: its parent must already exist (`VALIDATION_ERROR` naming it otherwise), and is
/// fsynced so the new entry survives a crash.
fn ensure_home_dir(home: &Path) -> Result<()> {
    let fail = |e: std::io::Error| {
        WalletError::validation(format!(
            "cannot prepare wallet home {}: {e}",
            home.display()
        ))
    };
    let parent = home
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| {
            WalletError::validation(format!("wallet home {} has no parent", home.display()))
        })?;
    match DirBuilder::new().mode(DIR_MODE).create(home) {
        Ok(()) => {
            // umask can only clear bits; set the mode explicitly anyway.
            fs::set_permissions(home, fs::Permissions::from_mode(DIR_MODE)).map_err(fail)?;
            sync_dir(parent)
        }
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {
            let meta = fs::metadata(home).map_err(fail)?;
            if !meta.is_dir() {
                return Err(WalletError::validation(format!(
                    "wallet home {} exists and is not a directory",
                    home.display()
                )));
            }
            if meta.permissions().mode() & 0o777 != DIR_MODE {
                fs::set_permissions(home, fs::Permissions::from_mode(DIR_MODE)).map_err(fail)?;
            }
            Ok(())
        }
        Err(e) if e.kind() == ErrorKind::NotFound => Err(WalletError::validation(format!(
            "the parent of wallet home {}, {}, does not exist; create it first",
            home.display(),
            parent.display()
        ))),
        Err(e) => Err(fail(e)),
    }
}

/// Creates `path` exclusively with mode 0600, writes `bytes`, and fsyncs.
fn write_new_file(path: &Path, bytes: &[u8], journal: &mut Journal) -> Result<()> {
    let fail = |e: std::io::Error| {
        WalletError::validation(format!("cannot write {}: {e}", path.display()))
    };
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(FILE_MODE)
        .open(path)
        .map_err(fail)?;
    journal.temps.push(path.to_path_buf());
    f.set_permissions(fs::Permissions::from_mode(FILE_MODE))
        .map_err(fail)?;
    f.write_all(bytes).map_err(fail)?;
    f.sync_all().map_err(fail)?;
    Ok(())
}

/// Atomic publish of a verified temp file: link(2) it into place. Unlike rename(2), link fails
/// instead of clobbering if `final_path` appeared concurrently. There is no fallback: a
/// filesystem without hard links gets an error, never a clobbering rename.
fn publish_new_file(home: &Path, tmp: &Path, final_path: &Path) -> Result<()> {
    match fs::hard_link(tmp, final_path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::AlreadyExists => Err(wallet_exists(home)),
        Err(e) => Err(WalletError::validation(format!(
            "cannot publish {} (hard link from {}): {e}",
            final_path.display(),
            tmp.display()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::seed::TEST_MNEMONIC;

    const PW: &str = "correct horse battery";

    fn mode(p: &Path) -> u32 {
        fs::metadata(p).unwrap().permissions().mode() & 0o777
    }

    /// Stand-in deriver so core tests do not depend on a chain family: a fingerprint of the
    /// phrase plus the index.
    fn fake_derive(seed: &Seed, index: u32) -> Result<BTreeMap<String, String>> {
        let fp: u32 = seed.phrase().bytes().map(u32::from).sum();
        Ok(BTreeMap::from([(
            "alpha".to_string(),
            format!("addr-{fp}-{index}"),
        )]))
    }

    fn test_seed() -> Seed {
        Seed::from_phrase(Zeroizing::new(TEST_MNEMONIC.into())).unwrap()
    }

    fn names(dir: &Path) -> Vec<OsString> {
        let mut v: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        v.sort();
        v
    }

    fn empty() -> Value {
        Value::Object(Default::default())
    }

    /// The cheapest accepted KDF, for tests where KDF strength is not the point.
    fn fast_kdf() -> KdfParams {
        KdfParams {
            log_n: KDF_MIN_LOG_N,
            ..KdfParams::default()
        }
    }

    fn fast() -> MasterOptions {
        MasterOptions {
            kdf: fast_kdf(),
            ..MasterOptions::default()
        }
    }

    /// Directory entries other than the lock files.
    fn data_names(dir: &Path) -> Vec<OsString> {
        let mut v = names(dir);
        v.retain(|n| n != WRITE_LOCK);
        v
    }

    #[test]
    fn encrypt_decrypt_round_trip() {
        let sealed = encrypt(&test_seed(), &empty(), PW, fast_kdf()).unwrap();
        let opened = decrypt(&sealed, PW).unwrap();
        assert_eq!(opened.seed.phrase(), TEST_MNEMONIC);
        assert_eq!(opened.policy, empty());
        let again = encrypt(&test_seed(), &empty(), PW, fast_kdf()).unwrap();
        assert_ne!(sealed.nonce, again.nonce);
        assert_ne!(sealed.kdfparams.salt, again.kdfparams.salt);
        assert!(!sealed.ciphertext.contains(&hex::encode("test test")));
    }

    #[test]
    fn wrong_password_is_wrong_password_error() {
        let sealed = encrypt(&test_seed(), &empty(), PW, fast_kdf()).unwrap();
        let err = decrypt(&sealed, "wrong password").err().unwrap();
        assert_eq!(err.code, ErrorCode::WrongPassword);
        assert_eq!(
            err.to_json(),
            serde_json::json!({
                "error": "wrong password: wallet.json could not be decrypted",
                "errorCode": "WRONG_PASSWORD"
            })
        );
    }

    #[test]
    fn tampered_file_fails_closed() {
        let mut sealed = encrypt(&test_seed(), &empty(), PW, fast_kdf()).unwrap();
        let mut ct = hex::decode(&sealed.ciphertext).unwrap();
        ct[0] ^= 1;
        sealed.ciphertext = hex::encode(ct);
        assert!(decrypt(&sealed, PW).is_err());
        sealed.kdfparams.log_n = 30;
        assert_eq!(
            decrypt(&sealed, PW).err().unwrap().code,
            ErrorCode::ValidationError
        );
    }

    #[test]
    fn create_master_writes_only_wallet_json_with_strict_modes() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("zw");
        let file =
            create_master(&home, PW, "main", MasterOptions::default(), &fake_derive).unwrap();
        assert_eq!(mode(&home), 0o700);
        assert_eq!(mode(&home.join(WALLET_FILE)), 0o600);
        assert_eq!(
            names(&home),
            vec![OsString::from(WRITE_LOCK), OsString::from(WALLET_FILE)],
            "no password or temp file; only the empty writer lock"
        );
        assert_eq!(mode(&home.join(WRITE_LOCK)), 0o600);
        assert_eq!(fs::metadata(home.join(WRITE_LOCK)).unwrap().len(), 0);
        // The default KDF: scrypt N = 2^18, r = 8, p = 1.
        let k = &file.crypto.kdfparams;
        assert_eq!((k.log_n, k.r, k.p), (18, 8, 1));
        assert_eq!(file.active, "main");
        assert_eq!(file.next_index, 1);
        assert_eq!(file.keys[0].index, 0);

        // Default master is 24 words; counted from the decrypted store, never printed.
        let (Unsealed { seed, policy }, on_disk) = unlock(&home, PW, &fake_derive).unwrap();
        assert_eq!(policy, empty(), "policy slot starts empty");
        assert_eq!(seed.word_count(), 24);
        assert_eq!(on_disk.keys, file.keys);
        assert_eq!(
            unlock(&home, "not the password", &fake_derive)
                .err()
                .unwrap()
                .code,
            ErrorCode::WrongPassword
        );
    }

    #[test]
    fn word_count_option_is_honoured() {
        let tmp = tempfile::tempdir().unwrap();
        let opts = MasterOptions {
            words: 12,
            ..fast()
        };
        create_master(tmp.path(), PW, "a", opts, &fake_derive).unwrap();
        assert_eq!(
            unlock(tmp.path(), PW, &fake_derive)
                .unwrap()
                .0
                .seed
                .word_count(),
            12
        );
        let bad = MasterOptions {
            words: 13,
            ..fast()
        };
        let other = tmp.path().join("other");
        assert!(create_master(&other, PW, "a", bad, &fake_derive).is_err());
        assert!(!other.exists());
    }

    #[test]
    fn second_master_is_wallet_exists_and_file_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_path_buf();
        fs::set_permissions(&home, fs::Permissions::from_mode(0o755)).unwrap();
        create_master(&home, PW, "a", fast(), &fake_derive).unwrap();
        assert_eq!(mode(&home), 0o700, "pre-existing home must be tightened");
        let before = fs::read(home.join(WALLET_FILE)).unwrap();
        let err = create_master(&home, PW, "b", fast(), &fake_derive).unwrap_err();
        assert_eq!(err.code, ErrorCode::WalletExists);
        assert_eq!(fs::read(home.join(WALLET_FILE)).unwrap(), before);
    }

    #[test]
    fn subkeys_get_increasing_indices_and_switch_active() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        create_master(home, PW, "a", fast(), &fake_derive).unwrap();
        let (Unsealed { seed, .. }, file) = unlock(home, PW, &fake_derive).unwrap();
        let crypto_before = serde_json::to_value(&file.crypto).unwrap();

        let idx = plan_new_key(&file, "b").unwrap();
        assert_eq!(idx, 1);
        let file = add_key(home, "b", idx, fake_derive(&seed, idx).unwrap()).unwrap();
        assert_eq!(file.active, "b");
        assert_eq!(file.key(None).unwrap().index, 1);
        assert_eq!(file.next_index, 2);
        assert_eq!(serde_json::to_value(&file.crypto).unwrap(), crypto_before);
        assert_eq!(mode(&home.join(WALLET_FILE)), 0o600);
        assert_eq!(data_names(home), vec![OsString::from(WALLET_FILE)]);

        assert_eq!(
            plan_new_key(&file, "a").unwrap_err().code,
            ErrorCode::KeyExists
        );
        assert_eq!(
            plan_new_key(&file, "Bad!").unwrap_err().code,
            ErrorCode::ValidationError
        );
        // A stale plan (index already used) is refused rather than clobbering.
        assert!(add_key(home, "c", 1, fake_derive(&seed, 1).unwrap()).is_err());

        let file = set_active(home, "a").unwrap();
        assert_eq!(file.key(None).unwrap().name, "a");
        assert_eq!(
            set_active(home, "nope").unwrap_err().code,
            ErrorCode::KeyNotFound
        );
        assert_eq!(
            file.key(Some("nope")).unwrap_err().code,
            ErrorCode::KeyNotFound
        );
        // The metadata rewrite kept the master decryptable and the caches consistent.
        unlock(home, PW, &fake_derive).unwrap();
    }

    #[test]
    fn sealed_policy_survives_metadata_rewrites() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        create_master(home, PW, "a", fast(), &fake_derive).unwrap();
        // Re-seal with a non-empty policy (as a future `policy set` would, with the password).
        let path = home.join(WALLET_FILE);
        let mut file = read_wallet_file(home).unwrap();
        let seed = decrypt(&file.crypto, PW).unwrap().seed;
        let policy = serde_json::json!({ "v": 1, "rules": [{ "maxPerTx": "0.1" }] });
        file.crypto = encrypt(&seed, &policy, PW, fast_kdf()).unwrap();
        fs::write(&path, serde_json::to_vec_pretty(&file).unwrap()).unwrap();

        let idx = plan_new_key(&file, "b").unwrap();
        add_key(home, "b", idx, fake_derive(&seed, idx).unwrap()).unwrap();
        set_active(home, "a").unwrap();
        let (opened, _) = unlock(home, PW, &fake_derive).unwrap();
        assert_eq!(opened.policy, policy);

        // Not a JSON object: refused when sealing.
        assert!(encrypt(&seed, &serde_json::json!([1]), PW, fast_kdf()).is_err());
    }

    #[test]
    fn key_name_rules() {
        for ok in ["a", "0", "main", "hot-1", "x_y", &"a".repeat(32)] {
            validate_key_name(ok).unwrap();
        }
        for bad in ["", "-a", "_a", "A", "a b", "a.b", "ä", &"a".repeat(33)] {
            assert!(validate_key_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn missing_wallet_is_wallet_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(
            read_wallet_file(tmp.path()).unwrap_err().code,
            ErrorCode::WalletNotFound
        );
    }

    #[test]
    fn weak_password_rejected_before_anything_is_written() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("zw");
        let err = create_master(&home, "short", "a", fast(), &fake_derive).unwrap_err();
        assert_eq!(err.code, ErrorCode::ValidationError);
        assert!(!home.exists());
    }

    #[test]
    fn failure_before_publication_removes_only_created_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let boom = |stage: CreateStage| match stage {
            CreateStage::TempWritten(_) => Err(WalletError::validation("injected")),
            CreateStage::Published(_) => Ok(()),
        };
        let opts = fast();
        let home = tmp.path().join("fresh");
        let err = create_master_with_hook(&home, PW, "a", opts, &fake_derive, &boom).unwrap_err();
        assert_eq!(err.message, "injected");
        // The temp file is gone; the home and its lock file stay (another creator may wait on
        // that lock).
        assert!(data_names(&home).is_empty(), "{:?}", names(&home));
        assert!(home.join(WRITE_LOCK).exists());

        let home2 = tmp.path().join("existing");
        fs::create_dir(&home2).unwrap();
        fs::write(home2.join("unrelated"), "keep").unwrap();
        create_master_with_hook(&home2, PW, "a", opts, &fake_derive, &boom).unwrap_err();
        assert_eq!(data_names(&home2), vec![OsString::from("unrelated")]);
    }

    #[test]
    fn inconsistent_address_cache_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        create_master(tmp.path(), PW, "a", fast(), &fake_derive).unwrap();
        let path = tmp.path().join(WALLET_FILE);
        let mut file: WalletFile = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        file.keys[0]
            .addresses
            .insert("alpha".into(), "forged".into());
        fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();
        let err = unlock(tmp.path(), PW, &fake_derive).err().unwrap();
        assert_eq!(err.code, ErrorCode::ValidationError);
    }

    #[test]
    fn decrypt_rejects_kdf_params_outside_policy() {
        let kdf = KdfParams {
            log_n: 15,
            r: 8,
            p: 1,
        };
        let sealed = encrypt(&test_seed(), &empty(), PW, kdf).unwrap();
        for (log_n, r, p) in [(14, 8, 1), (15, 4, 1), (15, 8, 2)] {
            let mut s = sealed.clone();
            s.kdfparams.log_n = log_n;
            s.kdfparams.r = r;
            s.kdfparams.p = p;
            let err = decrypt(&s, PW).err().unwrap();
            assert_eq!(err.code, ErrorCode::ValidationError, "{log_n} {r} {p}");
        }
    }

    #[test]
    fn plan_new_key_skips_past_the_highest_index() {
        let tmp = tempfile::tempdir().unwrap();
        create_master(tmp.path(), PW, "a", fast(), &fake_derive).unwrap();
        let mut file = read_wallet_file(tmp.path()).unwrap();
        let mut stale = file.keys[0].clone();
        stale.name = "far".into();
        stale.index = 5;
        file.keys.push(stale);
        assert_eq!(file.next_index, 1);
        assert_eq!(plan_new_key(&file, "b").unwrap(), 6);
    }

    #[test]
    fn unlock_rejects_duplicate_indices() {
        let tmp = tempfile::tempdir().unwrap();
        create_master(tmp.path(), PW, "a", fast(), &fake_derive).unwrap();
        let path = tmp.path().join(WALLET_FILE);
        let mut file = read_wallet_file(tmp.path()).unwrap();
        let mut dup = file.keys[0].clone();
        dup.name = "twin".into();
        file.keys.push(dup);
        fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();
        let err = unlock(tmp.path(), PW, &fake_derive).err().unwrap();
        assert_eq!(err.code, ErrorCode::ValidationError);
        assert!(err.message.contains("index"), "{}", err.message);
    }

    #[test]
    fn published_wallet_is_never_unlinked_on_a_later_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("zw");
        let path = home.join(WALLET_FILE);
        let replace = |stage: CreateStage| match stage {
            CreateStage::TempWritten(_) => Ok(()),
            CreateStage::Published(p) => {
                fs::remove_file(p).unwrap();
                fs::write(p, b"replacement").unwrap();
                Err(WalletError::validation("injected"))
            }
        };
        let err =
            create_master_with_hook(&home, PW, "a", fast(), &fake_derive, &replace).unwrap_err();
        assert_eq!(fs::read(&path).unwrap(), b"replacement");
        assert!(err.message.contains("recovery"), "{}", err.message);
        assert!(err.message.contains(&path.display().to_string()));
        assert_eq!(data_names(&home), vec![OsString::from(WALLET_FILE)]);

        // Changed after publication without the hook failing: still an error, still kept.
        let home = tmp.path().join("zw2");
        let path = home.join(WALLET_FILE);
        let edit = |stage: CreateStage| {
            if let CreateStage::Published(p) = stage {
                fs::write(p, b"edited").unwrap();
            }
            Ok(())
        };
        let err = create_master_with_hook(&home, PW, "a", fast(), &fake_derive, &edit).unwrap_err();
        assert!(err.message.contains("recovery"), "{}", err.message);
        assert_eq!(fs::read(&path).unwrap(), b"edited");
    }

    #[test]
    fn rewritten_cache_before_publication_fails_and_leaves_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("zw");
        let poison = |stage: CreateStage| {
            if let CreateStage::TempWritten(p) = stage {
                let mut f: WalletFile = serde_json::from_slice(&fs::read(p).unwrap()).unwrap();
                f.keys[0]
                    .addresses
                    .insert("alpha".into(), "addr-forged".into());
                fs::write(p, serde_json::to_vec(&f).unwrap()).unwrap();
            }
            Ok(())
        };
        let err =
            create_master_with_hook(&home, PW, "a", fast(), &fake_derive, &poison).unwrap_err();
        assert!(
            err.message.contains("verification failed"),
            "{}",
            err.message
        );
        assert!(
            data_names(&home).is_empty(),
            "nothing left behind but the home and its lock: {:?}",
            names(&home)
        );
    }

    #[test]
    fn kdf_policy_bounds_memory_before_any_allocation() {
        // Pure parameter checks: nothing here runs scrypt.
        for log_n in KDF_MIN_LOG_N..=KDF_MAX_LOG_N {
            KdfParams::checked(log_n, 8, 1).unwrap();
        }
        for (log_n, r, p) in [
            (14, 8, 1),
            (19, 8, 1),
            (20, 8, 1),
            (30, 8, 1),
            (u8::MAX, 8, 1),
            (15, 1, 1),
            (15, 16, 1),
            (15, u32::MAX, 1),
            (15, 8, 0),
            (15, 8, 2),
            (15, 8, u32::MAX),
        ] {
            let err = KdfParams::checked(log_n, r, p).unwrap_err();
            assert_eq!(err.code, ErrorCode::ValidationError, "{log_n} {r} {p}");
        }
        // The worst accepted case is 128 * r * N = 256 MiB.
        assert_eq!(128 * KDF_R as u64 * (1u64 << KDF_MAX_LOG_N), 256 << 20);
        // And decrypt applies it before deriving anything (log_n = 30 would need 128 GiB).
        let mut sealed = encrypt(&test_seed(), &empty(), PW, fast_kdf()).unwrap();
        sealed.kdfparams.log_n = 30;
        assert_eq!(
            decrypt(&sealed, PW).err().unwrap().code,
            ErrorCode::ValidationError
        );
    }

    #[test]
    fn writers_serialize_on_the_write_lock() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        create_master(home, PW, "a", fast(), &fake_derive).unwrap();
        let held = MetaLock::acquire(home).unwrap();
        let start = Instant::now();
        let waiter = std::thread::spawn({
            let home = home.to_path_buf();
            move || set_active(&home, "a").map(|_| start.elapsed())
        });
        std::thread::sleep(Duration::from_millis(300));
        drop(held);
        let waited = waiter.join().unwrap().unwrap();
        assert!(waited >= Duration::from_millis(250), "{waited:?}");

        // Planned and published under one lock from several threads: distinct indices.
        let (seed, _) = (test_seed(), ());
        let names = ["b", "c", "d", "e"];
        std::thread::scope(|s| {
            for name in names {
                let seed = &seed;
                s.spawn(move || {
                    let lock = MetaLock::acquire(home).unwrap();
                    let file = lock.read().unwrap();
                    let index = plan_new_key(&file, name).unwrap();
                    lock.add_key(name, index, fake_derive(seed, index).unwrap())
                        .unwrap();
                });
            }
        });
        let file = read_wallet_file(home).unwrap();
        let mut indices: Vec<u32> = file.keys.iter().map(|k| k.index).collect();
        indices.sort_unstable();
        assert_eq!(indices, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn failed_creator_never_unlinks_the_lock_a_waiter_holds() {
        // K7: creator A made the home and holds w.lock; creator B waits on that same lock file.
        // A then fails before publishing. Its rollback must leave w.lock and the home in place,
        // so B, once it gets the lock, creates the wallet in that home.
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("zw");
        let (holding_tx, holding_rx) = std::sync::mpsc::channel::<()>();
        let (fail_tx, fail_rx) = std::sync::mpsc::channel::<()>();
        let fail_rx = std::sync::Mutex::new(fail_rx);
        let hook = |stage: CreateStage| match stage {
            CreateStage::TempWritten(_) => {
                holding_tx.send(()).unwrap();
                fail_rx.lock().unwrap().recv().unwrap();
                Err(WalletError::validation("injected"))
            }
            CreateStage::Published(_) => Ok(()),
        };
        let b = std::thread::scope(|s| {
            let a =
                s.spawn(|| create_master_with_hook(&home, PW, "a", fast(), &fake_derive, &hook));
            holding_rx.recv().unwrap();
            let b = s.spawn(|| create_master(&home, PW, "b", fast(), &fake_derive));
            // B is now waiting on w.lock (it polls every 20 ms).
            std::thread::sleep(Duration::from_millis(300));
            fail_tx.send(()).unwrap();
            assert_eq!(a.join().unwrap().unwrap_err().message, "injected");
            b.join().unwrap()
        });
        let file = b.unwrap_or_else(|e| panic!("the waiting creator failed: {}", e.message));
        assert_eq!(file.active, "b");
        assert_eq!(
            data_names(&home),
            vec![OsString::from(WALLET_FILE)],
            "no temp file left"
        );
        assert!(home.join(WRITE_LOCK).exists());
        unlock(&home, PW, &fake_derive).unwrap();
    }

    #[test]
    fn missing_home_parent_is_a_validation_error_naming_it() {
        // K8: only the final directory is created; its parent must already exist.
        let tmp = tempfile::tempdir().unwrap();
        let parent = tmp.path().join("missing");
        let home = parent.join("zw");
        let err = create_master(&home, PW, "a", fast(), &fake_derive).unwrap_err();
        assert_eq!(err.code, ErrorCode::ValidationError);
        assert!(
            err.message.contains(&parent.display().to_string()),
            "{}",
            err.message
        );
        assert!(!parent.exists(), "no ancestor directory was created");
    }

    #[test]
    fn exhausted_index_counter_is_a_validation_error_and_leaves_the_file() {
        // K9: a hand-edited nextIndex at u32::MAX has no successor to store.
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        create_master(home, PW, "a", fast(), &fake_derive).unwrap();
        let path = home.join(WALLET_FILE);
        let mut file = read_wallet_file(home).unwrap();
        file.next_index = u32::MAX;
        fs::write(&path, serde_json::to_vec_pretty(&file).unwrap()).unwrap();
        let before = fs::read(&path).unwrap();
        let err = plan_new_key(&file, "b").unwrap_err();
        assert_eq!(err.code, ErrorCode::ValidationError, "{}", err.message);
        let seed = test_seed();
        let err = add_key(home, "b", u32::MAX, fake_derive(&seed, u32::MAX).unwrap()).unwrap_err();
        assert_eq!(err.code, ErrorCode::ValidationError, "{}", err.message);
        assert_eq!(fs::read(&path).unwrap(), before, "wallet.json unchanged");

        // The same through the highest index in use rather than the counter.
        file.next_index = 1;
        file.keys[0].index = u32::MAX - 1;
        assert_eq!(
            plan_new_key(&file, "b").unwrap_err().code,
            ErrorCode::ValidationError
        );
    }

    #[test]
    fn wallet_file_reads_refuse_fifos_links_and_oversized_files() {
        // K10: a FIFO in place of wallet.json must fail fast instead of blocking the reader.
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_path_buf();
        let path = home.join(WALLET_FILE);
        let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: mkfifo with a valid NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        let (tx, rx) = std::sync::mpsc::channel();
        let reader = {
            let home = home.clone();
            std::thread::spawn(move || {
                let _ = tx.send(read_wallet_file(&home).map(|_| ()));
            })
        };
        let got = rx.recv_timeout(Duration::from_secs(2));
        if got.is_err() {
            // Unblock the stuck reader so the test can end.
            let _ = OpenOptions::new().write(true).open(&path);
        }
        let _ = reader.join();
        let err = got
            .expect("reading a FIFO wallet.json blocked")
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::ValidationError, "{}", err.message);
        fs::remove_file(&path).unwrap();

        // A symlink to a real wallet is refused, not followed.
        let real = tmp.path().join("real");
        create_master(&real, PW, "a", fast(), &fake_derive).unwrap();
        std::os::unix::fs::symlink(real.join(WALLET_FILE), &path).unwrap();
        assert_eq!(
            read_wallet_file(&home).unwrap_err().code,
            ErrorCode::ValidationError
        );
        fs::remove_file(&path).unwrap();

        // Larger than 1 MiB: refused before it is parsed.
        fs::write(&path, vec![b' '; (1 << 20) + 1]).unwrap();
        let err = read_wallet_file(&home).unwrap_err();
        assert!(err.message.contains("1 MiB"), "{}", err.message);
    }

    #[test]
    fn sync_dir_reports_errors() {
        let tmp = tempfile::tempdir().unwrap();
        sync_dir(tmp.path()).unwrap();
        let err = sync_dir(&tmp.path().join("missing")).unwrap_err();
        assert!(err.message.contains("cannot sync"), "{}", err.message);
    }

    #[test]
    fn home_resolution() {
        assert_eq!(
            resolve_home(Some("/x/y".into()), Some("/home/u".into())).unwrap(),
            PathBuf::from("/x/y")
        );
        assert_eq!(
            resolve_home(Some("".into()), Some("/home/u".into())).unwrap(),
            PathBuf::from("/home/u/.zwallet")
        );
        assert_eq!(
            resolve_home(None, None).unwrap_err().code,
            ErrorCode::ValidationError
        );
    }
}
