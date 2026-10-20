//! On-disk custody: `<home>/wallet.json`.
//!
//! The file is chain-agnostic: a versioned envelope holding the BIP39 mnemonic encrypted with
//! AES-256-GCM under a scrypt key derived from a human-held password, plus a non-secret cache of
//! addresses per chain key. No other file on disk can decrypt it. Home is 0700, the file 0600.

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
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

pub const WALLET_FILE: &str = "wallet.json";
pub const STORE_VERSION: u32 = 1;
pub const MIN_PASSWORD_LEN: usize = 8;
const CIPHER: &str = "aes-256-gcm";
const KDF: &str = "scrypt";
const KEY_LEN: usize = 32;
const SALT_LEN: usize = 32;
const NONCE_LEN: usize = 12;
const AAD: &[u8] = b"zwallet/wallet.json/v1";
pub const DIR_MODE: u32 = 0o700;
pub const FILE_MODE: u32 = 0o600;

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

impl Default for KdfParams {
    /// N = 2^15, r = 8, p = 1: 32 MiB, roughly 0.1 s per unlock in an optimized build.
    fn default() -> Self {
        Self {
            log_n: 15,
            r: 8,
            p: 1,
        }
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
    // Bounds keep a tampered file from demanding gigabytes of memory.
    if !(10..=20).contains(&k.log_n)
        || !(1..=32).contains(&k.r)
        || !(1..=16).contains(&k.p)
        || k.dklen as usize != KEY_LEN
    {
        return Err(corrupt("kdf parameters out of range"));
    }
    let salt = hex::decode(&k.salt).map_err(|_| corrupt("salt is not hex"))?;
    let nonce = hex::decode(&sealed.nonce).map_err(|_| corrupt("nonce is not hex"))?;
    let ciphertext =
        hex::decode(&sealed.ciphertext).map_err(|_| corrupt("ciphertext is not hex"))?;
    if salt.len() < 16 || nonce.len() != NONCE_LEN {
        return Err(corrupt("bad salt or nonce length"));
    }
    let key = derive_key(
        password,
        &salt,
        &KdfParams {
            log_n: k.log_n,
            r: k.r,
            p: k.p,
        },
    )?;
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
    let path = home.join(WALLET_FILE);
    let bytes = match fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            return Err(WalletError::new(
                ErrorCode::WalletNotFound,
                format!(
                    "no wallet found in {} (a human runs `zwallet wallet create`)",
                    home.display()
                ),
            ));
        }
        Err(e) => {
            return Err(WalletError::validation(format!(
                "cannot read {}: {e}",
                path.display()
            )));
        }
    };
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

fn verify_key_cache(file: &WalletFile, seed: &Seed, derive: AddressDeriver<'_>) -> Result<()> {
    for key in &file.keys {
        let derived = derive(seed, key.index)?;
        for (chain, cached) in &key.addresses {
            if let Some(actual) = derived.get(chain)
                && !actual.eq_ignore_ascii_case(cached)
            {
                return Err(WalletError::validation(format!(
                    "wallet.json is inconsistent: cached {chain} address of key '{}' does not \
                     match the master seed",
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

/// Points in [`create_master`] where tests can inject a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateStage {
    WalletWritten,
}

/// Paths this invocation created; on failure exactly these are removed, nothing else.
#[derive(Default)]
struct Journal {
    files: Vec<PathBuf>,
    dir: Option<PathBuf>,
}

impl Journal {
    fn rollback(&self) {
        for f in self.files.iter().rev() {
            let _ = fs::remove_file(f);
        }
        if let Some(d) = &self.dir {
            // Only succeeds if empty, which is what we want.
            let _ = fs::remove_dir(d);
        }
    }
}

/// Creates the master seed in `home`, encrypted under `password`, with its first key
/// `first_key` at index 0 (active).
///
/// Writes `wallet.json` atomically, then reads it back from disk, decrypts it with the same
/// password, and checks the derived addresses before reporting success. On any failure only the
/// paths this call created are removed. Refuses if `wallet.json` already exists.
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
    hook: &dyn Fn(CreateStage) -> Result<()>,
) -> Result<WalletFile> {
    validate_key_name(first_key)?;
    validate_new_password(password)?;
    let mut journal = Journal::default();
    let result = create_steps(home, password, first_key, opts, derive, hook, &mut journal);
    if result.is_err() {
        journal.rollback();
    }
    result
}

fn create_steps(
    home: &Path,
    password: &str,
    first_key: &str,
    opts: MasterOptions,
    derive: AddressDeriver<'_>,
    hook: &dyn Fn(CreateStage) -> Result<()>,
    journal: &mut Journal,
) -> Result<WalletFile> {
    let wallet_path = home.join(WALLET_FILE);
    if fs::symlink_metadata(&wallet_path).is_ok() {
        return Err(wallet_exists(home));
    }
    ensure_home_dir(home, journal)?;

    let seed = Seed::generate(opts.words)?;
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
            addresses: derive(&seed, 0)?,
        }],
        crypto: encrypt(
            &seed,
            &Value::Object(Default::default()),
            password,
            opts.kdf,
        )?,
    };
    drop(seed);
    publish_new_file(home, &wallet_path, &to_json_bytes(&file)?, journal)?;
    hook(CreateStage::WalletWritten)?;

    // Verify exactly what a later process will see: the file as read back from disk.
    let verify = || -> Result<()> {
        let (Unsealed { seed, .. }, on_disk) = unlock(home, password, derive)?;
        if on_disk.keys != file.keys || seed.word_count() != opts.words {
            return Err(WalletError::validation("read-back mismatch"));
        }
        Ok(())
    };
    verify().map_err(|e| {
        WalletError::validation(format!(
            "post-write verification failed, wallet not kept: {}",
            e.message
        ))
    })?;
    Ok(file)
}

/// The derivation index the next [`add_key`] will use, after checking the name is free.
pub fn plan_new_key(file: &WalletFile, name: &str) -> Result<u32> {
    validate_key_name(name)?;
    if file.keys.iter().any(|k| k.name == name) {
        return Err(WalletError::new(
            ErrorCode::KeyExists,
            format!("a key named '{name}' already exists"),
        ));
    }
    Ok(file.next_index)
}

/// Adds key `name` at `index` (from [`plan_new_key`]) with its derived addresses and makes it
/// active. The encrypted master is carried over byte-for-byte.
pub fn add_key(
    home: &Path,
    name: &str,
    index: u32,
    addresses: BTreeMap<String, String>,
) -> Result<WalletFile> {
    update_metadata(home, |file| {
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
        file.next_index = index + 1;
        file.active = name.to_string();
        Ok(())
    })
}

/// Makes `name` the active key (`KEY_NOT_FOUND` if there is none).
pub fn set_active(home: &Path, name: &str) -> Result<WalletFile> {
    update_metadata(home, |file| {
        file.key(Some(name))?;
        file.active = name.to_string();
        Ok(())
    })
}

/// Read-modify-write of the non-secret metadata. The crypto box must come out unchanged, and the
/// new file replaces the old one atomically (temp file + rename), so the encrypted master is never
/// at risk even if this process dies midway.
fn update_metadata(
    home: &Path,
    mutate: impl FnOnce(&mut WalletFile) -> Result<()>,
) -> Result<WalletFile> {
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
        fs::rename(&tmp, &path)
            .map_err(|e| WalletError::validation(format!("cannot replace {}: {e}", path.display())))
    });
    if written.is_err() {
        journal.rollback();
    }
    written?;
    sync_dir(home);
    let on_disk = read_wallet_file(home)?;
    if on_disk.keys != file.keys || on_disk.active != file.active {
        return Err(WalletError::validation(
            "wallet.json read-back mismatch after update",
        ));
    }
    Ok(on_disk)
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

fn sync_dir(dir: &Path) {
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
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

/// Creates `home` with 0700 (recording it if new) or tightens an existing one to 0700.
fn ensure_home_dir(home: &Path, journal: &mut Journal) -> Result<()> {
    let fail = |e: std::io::Error| {
        WalletError::validation(format!(
            "cannot prepare wallet home {}: {e}",
            home.display()
        ))
    };
    match fs::metadata(home) {
        Ok(meta) => {
            if !meta.is_dir() {
                return Err(WalletError::validation(format!(
                    "wallet home {} exists and is not a directory",
                    home.display()
                )));
            }
            if meta.permissions().mode() & 0o777 != DIR_MODE {
                fs::set_permissions(home, fs::Permissions::from_mode(DIR_MODE)).map_err(fail)?;
            }
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {
            DirBuilder::new()
                .recursive(true)
                .mode(DIR_MODE)
                .create(home)
                .map_err(fail)?;
            journal.dir = Some(home.to_path_buf());
            // umask can only clear bits; set the mode explicitly anyway.
            fs::set_permissions(home, fs::Permissions::from_mode(DIR_MODE)).map_err(fail)?;
        }
        Err(e) => return Err(fail(e)),
    }
    Ok(())
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
    journal.files.push(path.to_path_buf());
    f.set_permissions(fs::Permissions::from_mode(FILE_MODE))
        .map_err(fail)?;
    f.write_all(bytes).map_err(fail)?;
    f.sync_all().map_err(fail)?;
    Ok(())
}

/// Atomic publish: write a temp file, then link(2) it into place. Unlike rename(2), link fails
/// instead of clobbering if `final_path` appeared concurrently.
fn publish_new_file(
    home: &Path,
    final_path: &Path,
    bytes: &[u8],
    journal: &mut Journal,
) -> Result<()> {
    let tmp = temp_path(home);
    write_new_file(&tmp, bytes, journal)?;
    let linked = match fs::hard_link(&tmp, final_path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::AlreadyExists => Err(wallet_exists(home)),
        // Filesystems without hard links: fall back to rename after re-checking.
        Err(_) if fs::symlink_metadata(final_path).is_err() => fs::rename(&tmp, final_path)
            .map_err(|e| {
                WalletError::validation(format!("cannot write {}: {e}", final_path.display()))
            }),
        Err(_) => Err(wallet_exists(home)),
    };
    let _ = fs::remove_file(&tmp);
    linked?;
    journal.files.push(final_path.to_path_buf());
    sync_dir(home);
    Ok(())
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

    #[test]
    fn encrypt_decrypt_round_trip() {
        let sealed = encrypt(&test_seed(), &empty(), PW, KdfParams::default()).unwrap();
        let opened = decrypt(&sealed, PW).unwrap();
        assert_eq!(opened.seed.phrase(), TEST_MNEMONIC);
        assert_eq!(opened.policy, empty());
        let again = encrypt(&test_seed(), &empty(), PW, KdfParams::default()).unwrap();
        assert_ne!(sealed.nonce, again.nonce);
        assert_ne!(sealed.kdfparams.salt, again.kdfparams.salt);
        assert!(!sealed.ciphertext.contains(&hex::encode("test test")));
    }

    #[test]
    fn wrong_password_is_wrong_password_error() {
        let sealed = encrypt(&test_seed(), &empty(), PW, KdfParams::default()).unwrap();
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
        let mut sealed = encrypt(&test_seed(), &empty(), PW, KdfParams::default()).unwrap();
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
            vec![OsString::from(WALLET_FILE)],
            "no password or temp file"
        );
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
            ..MasterOptions::default()
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
            ..MasterOptions::default()
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
        create_master(&home, PW, "a", MasterOptions::default(), &fake_derive).unwrap();
        assert_eq!(mode(&home), 0o700, "pre-existing home must be tightened");
        let before = fs::read(home.join(WALLET_FILE)).unwrap();
        let err =
            create_master(&home, PW, "b", MasterOptions::default(), &fake_derive).unwrap_err();
        assert_eq!(err.code, ErrorCode::WalletExists);
        assert_eq!(fs::read(home.join(WALLET_FILE)).unwrap(), before);
    }

    #[test]
    fn subkeys_get_increasing_indices_and_switch_active() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        create_master(home, PW, "a", MasterOptions::default(), &fake_derive).unwrap();
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
        assert_eq!(names(home), vec![OsString::from(WALLET_FILE)]);

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
        create_master(home, PW, "a", MasterOptions::default(), &fake_derive).unwrap();
        // Re-seal with a non-empty policy (as a future `policy set` would, with the password).
        let path = home.join(WALLET_FILE);
        let mut file = read_wallet_file(home).unwrap();
        let seed = decrypt(&file.crypto, PW).unwrap().seed;
        let policy = serde_json::json!({ "v": 1, "rules": [{ "maxPerTx": "0.1" }] });
        file.crypto = encrypt(&seed, &policy, PW, KdfParams::default()).unwrap();
        fs::write(&path, serde_json::to_vec_pretty(&file).unwrap()).unwrap();

        let idx = plan_new_key(&file, "b").unwrap();
        add_key(home, "b", idx, fake_derive(&seed, idx).unwrap()).unwrap();
        set_active(home, "a").unwrap();
        let (opened, _) = unlock(home, PW, &fake_derive).unwrap();
        assert_eq!(opened.policy, policy);

        // Not a JSON object: refused when sealing.
        assert!(encrypt(&seed, &serde_json::json!([1]), PW, KdfParams::default()).is_err());
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
        let err =
            create_master(&home, "short", "a", MasterOptions::default(), &fake_derive).unwrap_err();
        assert_eq!(err.code, ErrorCode::ValidationError);
        assert!(!home.exists());
    }

    #[test]
    fn failure_after_write_removes_only_created_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let boom = |stage: CreateStage| match stage {
            CreateStage::WalletWritten => Err(WalletError::validation("injected")),
        };
        let opts = MasterOptions::default();
        let home = tmp.path().join("fresh");
        let err = create_master_with_hook(&home, PW, "a", opts, &fake_derive, &boom).unwrap_err();
        assert_eq!(err.message, "injected");
        assert!(
            !home.exists(),
            "home created by this call should be removed"
        );

        let home2 = tmp.path().join("existing");
        fs::create_dir(&home2).unwrap();
        fs::write(home2.join("unrelated"), "keep").unwrap();
        create_master_with_hook(&home2, PW, "a", opts, &fake_derive, &boom).unwrap_err();
        assert_eq!(names(&home2), vec![OsString::from("unrelated")]);
    }

    #[test]
    fn inconsistent_address_cache_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        create_master(tmp.path(), PW, "a", MasterOptions::default(), &fake_derive).unwrap();
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
