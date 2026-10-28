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
/// Key indices stay below 2^31: BIP32 child numbers from 2^31 up are hardened, so a larger index
/// could not be the non-hardened last step of a path such as EVM's `m/44'/60'/0'/0/<index>`.
pub const KEY_INDEX_LIMIT: u32 = 1 << 31;
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
        format!(
            "no key named {} (see `zwallet list`)",
            crate::core::text::shown(name, 64)
        ),
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
            "invalid key name {}: use 1-32 of a-z 0-9 _ - starting with a letter or digit",
            crate::core::text::shown(name, 64)
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
    /// The signing policy's anchor ([`WhitelistAnchor`]); `{}` while there is no whitelist. Keys
    /// this version does not know are carried through intact.
    policy: Value,
}

/// A decrypted wallet: the master seed and the sealed policy object.
pub struct Unsealed {
    pub seed: Seed,
    pub policy: Value,
}

impl Unsealed {
    /// The whitelist anchor sealed in `policy` ([`WhitelistAnchor::from_policy`]): what the
    /// daemon keeps, and what a list is verified against.
    pub fn whitelist_anchor(&self) -> Result<Option<WhitelistAnchor>> {
        WhitelistAnchor::from_policy(&self.policy)
    }

    /// The anchor and the salt sealed in `policy` ([`SealedWhitelist::from_policy`]): what the
    /// editor reads.
    pub fn sealed_whitelist(&self) -> Result<Option<SealedWhitelist>> {
        SealedWhitelist::from_policy(&self.policy)
    }
}

/// Draws a fresh salt and nonce and seals `plaintext` under a key derived from `password`, with
/// `aad` binding the ciphertext to the one file it belongs in.
fn seal(plaintext: &[u8], password: &str, kdf: KdfParams, aad: &[u8]) -> Result<CryptoBox> {
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
                msg: plaintext,
                aad,
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

/// Opens a box [`seal`] made with the same `aad`. `file` names the file in errors. A failed GCM
/// tag check (wrong password, or a box from another file) is `WRONG_PASSWORD`.
fn open(sealed: &CryptoBox, password: &str, aad: &[u8], file: &str) -> Result<Zeroizing<Vec<u8>>> {
    let corrupt = |what: &str| WalletError::validation(format!("{file} is corrupt: {what}"));
    if sealed.cipher != CIPHER || sealed.kdf != KDF {
        return Err(corrupt("unsupported cipher or kdf"));
    }
    // Before anything allocates: bounds the memory a tampered file can demand.
    let kdf = sealed
        .kdf_params()
        .map_err(|e| WalletError::validation(e.message.replacen(WALLET_FILE, file, 1)))?;
    let k = &sealed.kdfparams;
    if k.dklen as usize != KEY_LEN {
        return Err(corrupt("kdf parameters out of range"));
    }
    let salt = hex::decode(&k.salt).map_err(|_| corrupt("salt is not hex"))?;
    let nonce = hex::decode(&sealed.nonce).map_err(|_| corrupt("nonce is not hex"))?;
    let ciphertext =
        hex::decode(&sealed.ciphertext).map_err(|_| corrupt("ciphertext is not hex"))?;
    // Exactly the length `seal` draws (every zwallet has): so the AES key is never scrypt over
    // another input, such as the whitelist key's domain-prefixed salt.
    if salt.len() != SALT_LEN || nonce.len() != NONCE_LEN {
        return Err(corrupt("bad salt or nonce length"));
    }
    let key = derive_key(password, &salt, &kdf)?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key.as_ref()));
    let plain = cipher
        .decrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &ciphertext,
                aad,
            },
        )
        .map_err(|_| {
            WalletError::new(
                ErrorCode::WrongPassword,
                format!("wrong password: {file} could not be decrypted"),
            )
        })?;
    Ok(Zeroizing::new(plain))
}

impl CryptoBox {
    /// The scrypt parameters this box was sealed with, checked as in [`KdfParams::checked`].
    pub fn kdf_params(&self) -> Result<KdfParams> {
        let k = &self.kdfparams;
        KdfParams::checked(k.log_n, k.r, k.p)
    }
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
    seal(&plaintext, password, kdf, AAD)
}

/// Decrypts the sealed payload. A failed GCM tag check is reported as `WRONG_PASSWORD`.
pub fn decrypt(sealed: &CryptoBox, password: &str) -> Result<Unsealed> {
    let corrupt = |what: &str| WalletError::validation(format!("wallet.json is corrupt: {what}"));
    let plain = open(sealed, password, AAD, WALLET_FILE)?;
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

/// Prefixed to the sealed whitelist salt to make the scrypt salt of the whitelist key, so that
/// derivation is its own whatever salt it is given.
const WHITELIST_KEY_DOMAIN: &[u8] = b"zwallet/whitelist-key/v1\n";

/// The whitelist key's 32-byte ed25519 secret, derived from `password` and the wallet's sealed
/// whitelist `salt`: scrypt over `"zwallet/whitelist-key/v1\n" || salt`, with `kdf`, the
/// parameters that opened wallet.json (never a re-read, as in [`MetaLock::reseal`]; they are
/// checked as in [`KdfParams::checked`] before anything allocates). It is a scrypt run of its
/// own, not a step off wallet.json's AES key, so holding that key never yields this one. The same
/// password, salt and parameters always give the same secret, and nothing of it is stored: every
/// whitelist edit derives it again from the password typed.
pub fn derive_whitelist_secret(
    password: &str,
    salt: &[u8; 32],
    kdf: KdfParams,
) -> Result<Zeroizing<[u8; 32]>> {
    let mut input = Zeroizing::new(Vec::with_capacity(WHITELIST_KEY_DOMAIN.len() + salt.len()));
    input.extend_from_slice(WHITELIST_KEY_DOMAIN);
    input.extend_from_slice(salt);
    derive_key(password, &input, &kdf)
}

/// What wallet.json's sealed `policy` pins about the signed whitelist
/// (`.prd/04-whitelist.md`): the ed25519 public key that signs it, its generation, and the
/// digest of the one list signed at that generation. A policy without these keys (`{}`) means
/// there is no whitelist yet. This is all the daemon keeps of the policy: it can check a list,
/// never make one. The salt the key is derived from is sealed beside it ([`SealedWhitelist`]),
/// for the editor only.
///
/// The digest binds the generation to exactly one list: an edit that died after signing
/// `whitelist.next.json` but before re-sealing leaves a signed list at generation + 1 behind, and
/// the next edit signs a different list at that same generation. Without the digest, a copy of
/// the abandoned one would verify too.
///
/// AES-GCM authenticates the anchor but gives it no freshness: an authentic older wallet.json can
/// be restored by anyone who can write the home. One sealed with `{}` reads as "no whitelist",
/// which refuses every value-moving signature (fails closed); one with an older anchor, restored
/// together with its list, is the documented joint rollback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WhitelistAnchor {
    pub public_key: [u8; 32],
    pub generation: u64,
    pub digest: [u8; 32],
}

const POLICY_PUB: &str = "whitelistPub";
const POLICY_GEN: &str = "whitelistGen";
const POLICY_DIGEST: &str = "whitelistDigest";
const POLICY_SALT: &str = "whitelistSalt";

impl WhitelistAnchor {
    /// The anchor in a sealed policy object, when its salt is sealed too
    /// ([`SealedWhitelist::from_policy`], then [`SealedWhitelist::salt`]). No whitelist key at
    /// all: `None`. An anchor zwallet 0.1.0 sealed (no salt) is `VALIDATION_ERROR`, as a damaged
    /// one is: never "no whitelist", so the daemon trusts no list under it and signs nothing.
    pub fn from_policy(policy: &Value) -> Result<Option<Self>> {
        SealedWhitelist::from_policy(policy)?
            .map(|sealed| sealed.salt().map(|_| sealed.anchor))
            .transpose()
    }

    /// `policy` (a JSON object) with this anchor's keys and the whitelist `salt` set; every other
    /// key is kept.
    pub fn to_policy(&self, salt: &[u8; 32], policy: &Value) -> Result<Value> {
        let mut obj = policy
            .as_object()
            .cloned()
            .ok_or_else(|| WalletError::validation("policy must be a JSON object"))?;
        obj.insert(POLICY_PUB.into(), Value::from(hex::encode(self.public_key)));
        obj.insert(POLICY_GEN.into(), Value::from(self.generation));
        obj.insert(POLICY_DIGEST.into(), Value::from(hex::encode(self.digest)));
        obj.insert(POLICY_SALT.into(), Value::from(hex::encode(salt)));
        Ok(Value::Object(obj))
    }
}

/// What the whitelist editor reads of the sealed policy: the [`WhitelistAnchor`], and the
/// per-wallet salt (`whitelistSalt`) the whitelist key is derived from with the password
/// ([`derive_whitelist_secret`]). The salt is drawn at random by the first edit and by every
/// reset, and is sealed, so it is no plaintext input anyone can swap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SealedWhitelist {
    pub anchor: WhitelistAnchor,
    /// `None`: an anchor sealed by zwallet 0.1.0, whose key was random and kept in a file of its
    /// own, which is no longer used ([`SealedWhitelist::salt`]).
    salt: Option<[u8; 32]>,
}

impl SealedWhitelist {
    /// The anchor and the salt in a sealed policy object. None of the four keys: `None`. The
    /// three anchor keys well-formed (`whitelistPub` and `whitelistDigest` 32 bytes of lower-case
    /// hex, `whitelistGen` a number from 1), and `whitelistSalt` 32 bytes of lower-case hex or
    /// absent (zwallet 0.1.0): `Some`. Anything else is `VALIDATION_ERROR`, so a damaged anchor
    /// never reads as "no whitelist".
    pub fn from_policy(policy: &Value) -> Result<Option<Self>> {
        let obj = policy.as_object().ok_or_else(|| {
            WalletError::validation("wallet.json is corrupt: sealed policy is not an object")
        })?;
        let keys = [POLICY_PUB, POLICY_GEN, POLICY_DIGEST, POLICY_SALT];
        let present = keys.iter().filter(|k| obj.contains_key(**k)).count();
        if present == 0 {
            return Ok(None);
        }
        let bad = |what: &str| {
            WalletError::validation(format!(
                "wallet.json's sealed whitelist anchor is malformed: {what}"
            ))
        };
        let bytes32 = |key: &str| -> Result<[u8; 32]> {
            let s = obj
                .get(key)
                .and_then(Value::as_str)
                .ok_or_else(|| bad(&format!("{key} is missing or not a string")))?;
            let mut out = [0u8; 32];
            let canonical =
                s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
            if !canonical || hex::decode_to_slice(s, &mut out).is_err() {
                return Err(bad(&format!("{key} is not 32 bytes of lower-case hex")));
            }
            Ok(out)
        };
        let generation = obj
            .get(POLICY_GEN)
            .and_then(Value::as_u64)
            .filter(|g| *g >= 1)
            .ok_or_else(|| bad(&format!("{POLICY_GEN} is missing or not a number from 1")))?;
        let anchor = WhitelistAnchor {
            public_key: bytes32(POLICY_PUB)?,
            generation,
            digest: bytes32(POLICY_DIGEST)?,
        };
        // Absent: sealed by zwallet 0.1.0 ([`SealedWhitelist::salt`]).
        let salt = if obj.contains_key(POLICY_SALT) {
            Some(bytes32(POLICY_SALT)?)
        } else {
            None
        };
        Ok(Some(Self { anchor, salt }))
    }

    /// The salt the whitelist key is derived from. `VALIDATION_ERROR` for an anchor zwallet
    /// 0.1.0 sealed: its key was random and lived in a file this zwallet no longer reads, so the
    /// key cannot be derived. The message names the cause only, as every reason the daemon shows
    /// does; the editor adds what a human can do (`whitelist::edit_salt`).
    pub fn salt(&self) -> Result<[u8; 32]> {
        self.salt.ok_or_else(|| {
            WalletError::validation(
                "this whitelist was made by zwallet 0.1.0, whose key file is no longer used",
            )
        })
    }
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
    // A parser's message can quote the file's text: escaped.
    let file: WalletFile = serde_json::from_slice(&bytes).map_err(|e| {
        WalletError::validation(format!(
            "{} is not valid: {}",
            path.display(),
            crate::core::text::ascii_inert(&e.to_string(), crate::core::text::PROSE_MAX)
        ))
    })?;
    if file.version != STORE_VERSION {
        return Err(WalletError::validation(format!(
            "unsupported wallet.json version {} (this zwallet reads version {STORE_VERSION})",
            file.version
        )));
    }
    // Key names reach messages and human output: every one, and the active one, must be a name
    // `create` would accept. zwallet never writes another, so a file that has one was edited.
    for name in file.keys.iter().map(|k| &k.name).chain([&file.active]) {
        validate_key_name(name).map_err(|e| {
            WalletError::validation(format!(
                "{} is not valid: {}; zwallet never writes such a name, so the file was edited",
                path.display(),
                e.message
            ))
        })?;
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
    verify_key_list(file)?;
    let repairs = cache_repairs(file, seed, derive)?;
    moved_index(file, seed, derive, &repairs)?;
    if let Some(r) = repairs.first() {
        return Err(inconsistent(format!(
            "cached {} address of key '{}' does not match the master seed; run `zwallet address \
             --verify` to check the cache against the seed and repair it",
            r.chain, r.key
        )));
    }
    Ok(())
}

fn inconsistent(what: String) -> WalletError {
    WalletError::validation(format!("wallet.json is inconsistent: {what}"))
}

/// Every key has a distinct name and index.
fn verify_key_list(file: &WalletFile) -> Result<()> {
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
    Ok(())
}

/// One cached address that differs from the seed's: what [`unlock_repairing`] replaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheRepair {
    pub key: String,
    /// A chain key the seed derives an address for.
    pub chain: String,
    /// The cached value, as wallet.json has it (any text: escape it before showing it).
    pub was: String,
    /// The seed's address.
    pub now: String,
}

/// Whether a cached address is the one derived. An address the deriver writes as `0x` and hex
/// digits names its bytes in either case (an EVM address, EIP-55 or not), so it compares
/// case-insensitively; any other text (Tron's base58check `T…`, where the case is part of the
/// value and a case variant is another string) compares byte for byte. Core knows the encoding,
/// not the chain.
fn same_cached(derived: &str, cached: &str) -> bool {
    let hex = |s: &str| {
        s.strip_prefix("0x")
            .is_some_and(|h| !h.is_empty() && h.bytes().all(|b| b.is_ascii_hexdigit()))
    };
    if hex(derived) {
        derived.eq_ignore_ascii_case(cached)
    } else {
        derived == cached
    }
}

/// Every cached address in `file` that is not the one `seed` derives for its key and chain
/// ([`same_cached`]). Chain keys this build derives nothing for are left alone.
fn cache_repairs(
    file: &WalletFile,
    seed: &Seed,
    derive: AddressDeriver<'_>,
) -> Result<Vec<CacheRepair>> {
    let mut repairs = Vec::new();
    for key in &file.keys {
        let derived = derive(seed, key.index)?;
        for (chain, cached) in &key.addresses {
            if let Some(actual) = derived.get(chain)
                && !same_cached(actual, cached)
            {
                repairs.push(CacheRepair {
                    key: key.name.clone(),
                    chain: chain.clone(),
                    was: cached.clone(),
                    now: actual.clone(),
                });
            }
        }
    }
    Ok(repairs)
}

/// The highest derivation index [`moved_index`] looks at (besides every key's own).
const INDEX_SCAN: u32 = 256;

/// Refuses when a cached address that differs from its key's is the seed's address at another
/// index: the key list's indices were changed (or two swapped), not the cache. Repairing that
/// would make the key show another account than the one its cache showed, which may be the one
/// the human funded, so it is never repaired: the human restores the indices. Looks at every
/// key's index and at 0 to `nextIndex` (at most [`INDEX_SCAN`]); only when there is a mismatch.
fn moved_index(
    file: &WalletFile,
    seed: &Seed,
    derive: AddressDeriver<'_>,
    repairs: &[CacheRepair],
) -> Result<()> {
    if repairs.is_empty() {
        return Ok(());
    }
    let top = file
        .keys
        .iter()
        .map(|k| k.index.saturating_add(1))
        .chain([file.next_index])
        .max()
        .unwrap_or(0)
        .min(INDEX_SCAN);
    let mut candidates: BTreeSet<u32> = (0..top).collect();
    candidates.extend(file.keys.iter().map(|k| k.index));
    let mut derived: BTreeMap<u32, BTreeMap<String, String>> = BTreeMap::new();
    for r in repairs {
        let Some(key) = file.keys.iter().find(|k| k.name == r.key) else {
            continue;
        };
        for &j in candidates.iter().filter(|&&j| j != key.index) {
            let at_j = match derived.entry(j) {
                std::collections::btree_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::btree_map::Entry::Vacant(e) => e.insert(derive(seed, j)?),
            };
            if !at_j.get(&r.chain).is_some_and(|a| same_cached(a, &r.was)) {
                continue;
            }
            // `r.key`'s cache holds index `j`'s address. If a key legitimately sits at `j` with a
            // correct cache there, `r.key` just holds a stray copy of that address: repair it,
            // this is not a moved index. Only when no key owns `j`, or the key at `j` also
            // caches the wrong address (two indices swapped), is this a move, and refused.
            let owner = file.keys.iter().find(|k| k.index == j);
            let owner_correct = owner.is_some_and(|k| {
                k.addresses
                    .get(&r.chain)
                    .is_some_and(|c| at_j.get(&r.chain).is_some_and(|a| same_cached(a, c)))
            });
            if owner_correct {
                continue;
            }
            let owner = owner
                .map(|k| format!(", key '{}''s", k.name))
                .unwrap_or_default();
            return Err(inconsistent(format!(
                "key '{}' has index {} but caches the {} address of index {j}{owner}: the key \
                 list's indices were changed. Not repaired: that would make '{}' show another \
                 account than the one its cache showed, which may be the one you funded. Restore \
                 the indices in wallet.json, or delete key '{}''s cached address, from a copy you \
                 trust",
                key.name, key.index, r.chain, key.name, key.name
            )));
        }
    }
    Ok(())
}

/// [`unlock`], but cached addresses that differ from the seed are repaired instead of refused:
/// with the password the seed is authoritative and the cache is only metadata. Every other check
/// (distinct names and indices) still refuses. The repair is written under the writer lock, only
/// while wallet.json's crypto envelope is still the one the password opened, through the same
/// atomic replace as every metadata write; the encrypted master is never touched. Returns the
/// repairs made (none when the cache was right, and then nothing is written).
pub fn unlock_repairing(
    home: &Path,
    password: &str,
    derive: AddressDeriver<'_>,
) -> Result<(Unsealed, WalletFile, Vec<CacheRepair>)> {
    let file = read_wallet_file(home)?;
    let unsealed = decrypt(&file.crypto, password)?;
    verify_key_list(&file)?;
    let repairs = cache_repairs(&file, &unsealed.seed, derive)?;
    if repairs.is_empty() {
        verify_key_cache(&file, &unsealed.seed, derive)?;
        return Ok((unsealed, file, Vec::new()));
    }
    moved_index(&file, &unsealed.seed, derive, &repairs)?;
    let lock = MetaLock::acquire(home)?;
    let (repaired, repairs) = repair_under_lock(&lock, &file.crypto, &unsealed.seed, derive)?;
    Ok((unsealed, repaired, repairs))
}

/// [`repair_under_lock`] with no test hook.
fn repair_under_lock(
    lock: &MetaLock,
    opened: &CryptoBox,
    seed: &Seed,
    derive: AddressDeriver<'_>,
) -> Result<(WalletFile, Vec<CacheRepair>)> {
    repair_under_lock_hooked(lock, opened, seed, derive, || {})
}

/// The write of [`unlock_repairing`], under `lock`: only while wallet.json's crypto envelope is
/// still `opened` (the one the password decrypted); the key list checked again and the repairs
/// computed again as the file is now, and nothing written when there are none (another process
/// repaired first). The envelope is then **read back from disk** and must still be `opened`, so a
/// same-user process that swapped the file around the write (`after_write`, a test seam) is
/// caught: the repair writes only cached addresses and must never be the path that lets a
/// different master through.
fn repair_under_lock_hooked(
    lock: &MetaLock,
    opened: &CryptoBox,
    seed: &Seed,
    derive: AddressDeriver<'_>,
    after_write: impl FnOnce(),
) -> Result<(WalletFile, Vec<CacheRepair>)> {
    let opened = serde_json::to_value(opened).ok();
    let mut repairs = Vec::new();
    lock.update_if(|current| {
        if serde_json::to_value(&current.crypto).ok() != opened {
            return Err(WalletError::validation(
                "wallet.json's master changed while the password was checked; retry",
            ));
        }
        verify_key_list(current)?;
        repairs = cache_repairs(current, seed, derive)?;
        moved_index(current, seed, derive, &repairs)?;
        for r in &repairs {
            if let Some(key) = current.keys.iter_mut().find(|k| k.name == r.key) {
                key.addresses.insert(r.chain.clone(), r.now.clone());
            }
        }
        Ok(!repairs.is_empty())
    })?;
    after_write();
    let on_disk = read_wallet_file(lock.home())?;
    if serde_json::to_value(&on_disk.crypto).ok() != opened {
        return Err(WalletError::validation(
            "wallet.json's master changed during the cache repair; run `zwallet address --verify` \
             again",
        ));
    }
    verify_key_cache(&on_disk, seed, derive)?;
    Ok((on_disk, repairs))
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
/// index at or above [`KEY_INDEX_LIMIT`] is `VALIDATION_ERROR`.
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
    if index >= KEY_INDEX_LIMIT {
        return Err(no_index_left());
    }
    Ok(index)
}

fn no_index_left() -> WalletError {
    WalletError::validation(
        "wallet.json has no derivation index left for a new key: indices stay below 2^31, where \
         BIP32's hardened range starts",
    )
}

/// Exclusive writer lock on `<home>/w.lock`, held across read → check → mutate → publish.
pub struct MetaLock {
    home: PathBuf,
    _file: File,
}

/// wallet.json as [`MetaLock::open`] decrypted it: the file, its sealed contents, and the
/// password that opened them (zeroized on drop). Read-only, so what [`MetaLock::reseal`] seals is
/// exactly what was opened. Deliberately not `Debug`.
pub struct Opened {
    file: WalletFile,
    unsealed: Unsealed,
    password: Zeroizing<String>,
}

impl Opened {
    /// wallet.json as it was when opened. Its crypto envelope is authenticated: these KDF
    /// parameters, salt and nonce are the ones that decrypted it.
    pub fn file(&self) -> &WalletFile {
        &self.file
    }

    /// The seed and the sealed policy.
    pub fn unsealed(&self) -> &Unsealed {
        &self.unsealed
    }

    /// The password that opened this wallet.json (the whitelist key is derived from it).
    pub fn password(&self) -> &str {
        &self.password
    }
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

    /// The home this lock guards.
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// Decrypts wallet.json as it is under this lock. The result is the proof [`MetaLock::reseal`]
    /// needs: that `password` opens the master the file holds right now.
    pub fn open(&self, password: &str) -> Result<Opened> {
        let file = read_wallet_file(&self.home)?;
        let unsealed = decrypt(&file.crypto, password)?;
        Ok(Opened {
            file,
            unsealed,
            password: Zeroizing::new(password.to_string()),
        })
    }

    /// Re-seals the master in wallet.json with `policy` under the password that opened it (the
    /// whitelist editor's commit point). If wallet.json's crypto envelope is no longer exactly
    /// the one `opened` came from (cipher, KDF and every KDF parameter, salt, nonce,
    /// ciphertext), nothing is written. The mnemonic, the password and the KDF parameters carry
    /// over from `opened`, never from a re-read: a file can be edited on disk without the
    /// password, so a re-read could downgrade the KDF. The salt and nonce are fresh; the metadata
    /// is untouched.
    ///
    /// That comparison is a snapshot check among writers that take `w.lock`, not an atomic
    /// compare-and-swap: a same-user process that ignores the lock can still race the rename
    /// (the same-user limits in README §Known limits).
    ///
    /// The new file is written to a temp file, read back from disk and decrypted (the phrase and
    /// the policy must come out as sealed, the metadata as it was), and only then renamed over
    /// wallet.json, so a crash leaves either the old file or the new one. A failure before the
    /// rename removes the temp file.
    pub fn reseal(&self, opened: &Opened, policy: &Value) -> Result<WalletFile> {
        let home = &self.home;
        let before = read_wallet_file(home)?;
        if serde_json::to_value(&before.crypto).ok()
            != serde_json::to_value(&opened.file.crypto).ok()
        {
            return Err(WalletError::validation(
                "wallet.json's master changed while the password was checked; retry",
            ));
        }
        let (seed, password) = (&opened.unsealed.seed, opened.password.as_str());
        let kdf = opened.file.crypto.kdf_params()?;
        let mut file = before.clone();
        file.crypto = encrypt(seed, policy, password, kdf)?;
        let bytes = to_json_bytes(&file)?;
        let path = home.join(WALLET_FILE);
        let tmp = temp_path(home);
        let mut journal = Journal::default();
        let verify = || -> Result<()> {
            let on_disk = parse_wallet_file(&tmp)?;
            let reopened = decrypt(&on_disk.crypto, password)?;
            let same_meta = on_disk.keys == before.keys
                && on_disk.active == before.active
                && on_disk.next_index == before.next_index
                && on_disk.created_at == before.created_at;
            if !same_meta || reopened.seed.phrase() != seed.phrase() || reopened.policy != *policy {
                return Err(WalletError::validation("read-back mismatch"));
            }
            Ok(())
        };
        let written = write_new_file(&tmp, &bytes, &mut journal)
            .and_then(|()| {
                verify().map_err(|e| {
                    WalletError::validation(format!(
                        "post-write verification failed, wallet.json not replaced: {}",
                        e.message
                    ))
                })
            })
            .and_then(|()| {
                fs::rename(&tmp, &path).map_err(|e| {
                    WalletError::validation(format!("cannot replace {}: {e}", path.display()))
                })
            });
        if written.is_err() {
            journal.rollback();
        }
        written?;
        sync_dir(home)?;
        match read_wallet_bytes(&path) {
            Ok(b) if b == bytes => Ok(file),
            Ok(_) => Err(WalletError::validation(
                "wallet.json changed right after it was re-sealed",
            )),
            Err(e) => Err(WalletError::validation(format!(
                "cannot read wallet.json back after re-sealing it: {}",
                e.message
            ))),
        }
    }

    /// Atomically replaces `<home>/<name>` with `bytes`: a fresh 0600 temp file, fsync, rename,
    /// then a directory fsync. For the whitelist's files; wallet.json and the lock files are
    /// refused (they have their own writers).
    pub(crate) fn replace_file(&self, name: &str, bytes: &[u8]) -> Result<()> {
        check_plain_name(name)?;
        let home = &self.home;
        let path = home.join(name);
        let tmp = temp_path_for(home, name);
        let mut journal = Journal::default();
        let written = write_new_file(&tmp, bytes, &mut journal).and_then(|()| {
            fs::rename(&tmp, &path).map_err(|e| {
                WalletError::validation(format!("cannot replace {}: {e}", path.display()))
            })
        });
        if written.is_err() {
            journal.rollback();
        }
        written?;
        sync_dir(home)
    }

    /// Removes the plain file `name` from the home (gone already is fine), then syncs the
    /// directory.
    pub(crate) fn remove_file(&self, name: &str) -> Result<()> {
        check_plain_name(name)?;
        let path = self.home.join(name);
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => {
                return Err(WalletError::validation(format!(
                    "cannot remove {}: {e}",
                    path.display()
                )));
            }
        }
        sync_dir(&self.home)
    }

    /// Read-modify-write of the non-secret metadata. The crypto box must come out unchanged,
    /// and the new file replaces the old one atomically (temp file + rename), so the encrypted
    /// master is never at risk even if this process dies midway.
    fn update(&self, mutate: impl FnOnce(&mut WalletFile) -> Result<()>) -> Result<WalletFile> {
        self.update_if(|file| mutate(file).map(|()| true))
    }

    /// [`MetaLock::update`], written only when `mutate` returns `true`; otherwise the file as
    /// read is returned and nothing is written.
    fn update_if(
        &self,
        mutate: impl FnOnce(&mut WalletFile) -> Result<bool>,
    ) -> Result<WalletFile> {
        let home = &self.home;
        let mut file = read_wallet_file(home)?;
        let crypto_before = serde_json::to_value(&file.crypto).ok();
        if !mutate(&mut file)? {
            return Ok(file);
        }
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
    temp_path_for(home, WALLET_FILE)
}

/// A private temp name next to `<home>/<name>`: dot-prefixed, with this process's id and a
/// random suffix.
fn temp_path_for(home: &Path, name: &str) -> PathBuf {
    home.join(format!(
        ".{name}.tmp-{}-{:08x}",
        std::process::id(),
        OsRng.next_u32()
    ))
}

/// A file name directly in the home, other than wallet.json, the lock files and the socket.
fn check_plain_name(name: &str) -> Result<()> {
    use crate::core::protocol::{LOCK_NAME, SOCKET_NAME, UNLOCK_LOCK_NAME};
    let reserved = [
        WALLET_FILE,
        WRITE_LOCK,
        LOCK_NAME,
        UNLOCK_LOCK_NAME,
        SOCKET_NAME,
    ];
    if name.is_empty() || name.contains('/') || name.starts_with('.') || reserved.contains(&name) {
        return Err(WalletError::validation(format!(
            "refusing to write '{name}' through the plain-file writer"
        )));
    }
    Ok(())
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
/// fsynced so the entry survives a crash, also when the home already existed (an earlier run may
/// have created it and died before its fsync).
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
            sync_dir(parent)
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

    /// A fresh wallet in a temp home with keys `a` (0) and `b` (1), rewritten by `edit`.
    fn two_keys(edit: impl FnOnce(&mut WalletFile)) -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        create_master(tmp.path(), PW, "a", fast(), &fake_derive).unwrap();
        let seed = unlock(tmp.path(), PW, &fake_derive).unwrap().0.seed;
        add_key(tmp.path(), "b", 1, fake_derive(&seed, 1).unwrap()).unwrap();
        let path = tmp.path().join(WALLET_FILE);
        let mut file: WalletFile = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        edit(&mut file);
        fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();
        tmp
    }

    #[test]
    fn a_forged_cache_is_repaired_from_the_seed_once() {
        let tmp = two_keys(|f| {
            f.keys[1].addresses.insert("alpha".into(), "forged".into());
        });
        let err = unlock(tmp.path(), PW, &fake_derive).err().unwrap();
        assert!(
            err.message.contains("run `zwallet address --verify`"),
            "{}",
            err.message
        );
        let (_, file, repairs) = unlock_repairing(tmp.path(), PW, &fake_derive).unwrap();
        assert_eq!(repairs.len(), 1, "{repairs:?}");
        assert_eq!(
            (repairs[0].key.as_str(), repairs[0].was.as_str()),
            ("b", "forged")
        );
        assert_eq!(file.keys[1].addresses["alpha"], repairs[0].now);
        unlock(tmp.path(), PW, &fake_derive).unwrap();
        // Nothing left to repair: nothing is written.
        let before = fs::read(tmp.path().join(WALLET_FILE)).unwrap();
        let (_, _, again) = unlock_repairing(tmp.path(), PW, &fake_derive).unwrap();
        assert!(again.is_empty());
        assert_eq!(fs::read(tmp.path().join(WALLET_FILE)).unwrap(), before);
    }

    /// A deriver with two address encodings: `hex` gives `0x` and hex digits in mixed case (the
    /// EVM form), `b58` a base58check string (the Tron form), both from the index.
    fn two_encodings(_: &Seed, index: u32) -> Result<BTreeMap<String, String>> {
        let (hex, b58) = match index {
            0 => (
                "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
                "TWer2Ygk5TEheHp3TPuYeqxmB6SsGZmaL6",
            ),
            _ => (
                "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
                "TPjjvMwjPoDC32V2dGDYTkLH4E5LAtBZ6C",
            ),
        };
        Ok(BTreeMap::from([
            ("hex".to_string(), hex.to_string()),
            ("b58".to_string(), b58.to_string()),
        ]))
    }

    #[test]
    fn hex_caches_compare_in_any_case_and_every_other_text_byte_for_byte() {
        let tmp = tempfile::tempdir().unwrap();
        create_master(tmp.path(), PW, "a", fast(), &two_encodings).unwrap();
        let path = tmp.path().join(WALLET_FILE);
        // `nextIndex` 2: index 1, which no key owns, is among the indices a mismatch is checked
        // against for a moved index.
        let edit = |hex: &str, b58: &str| {
            let mut file: WalletFile = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            file.next_index = 2;
            file.keys[0].addresses.insert("hex".into(), hex.into());
            file.keys[0].addresses.insert("b58".into(), b58.into());
            fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();
        };
        // The hex address in lower case is the same 20 bytes: no repair.
        edit(
            "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266",
            "TWer2Ygk5TEheHp3TPuYeqxmB6SsGZmaL6",
        );
        unlock(tmp.path(), PW, &two_encodings).unwrap();
        // A base58 address with one letter's case flipped is another string: repaired.
        edit(
            "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266",
            "tWer2Ygk5TEheHp3TPuYeqxmB6SsGZmaL6",
        );
        assert!(unlock(tmp.path(), PW, &two_encodings).is_err());
        let (_, file, repairs) = unlock_repairing(tmp.path(), PW, &two_encodings).unwrap();
        assert_eq!(repairs.len(), 1, "{repairs:?}");
        assert_eq!(
            (repairs[0].chain.as_str(), repairs[0].was.as_str()),
            ("b58", "tWer2Ygk5TEheHp3TPuYeqxmB6SsGZmaL6")
        );
        assert_eq!(
            file.keys[0].addresses["b58"],
            "TWer2Ygk5TEheHp3TPuYeqxmB6SsGZmaL6"
        );
        // A case variant of index 1's base58 address is not index 1's address: a repair, not a
        // moved index (which the exact compare alone tells apart).
        edit(
            "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
            "tPjjvMwjPoDC32V2dGDYTkLH4E5LAtBZ6C",
        );
        let (_, _, repairs) = unlock_repairing(tmp.path(), PW, &two_encodings).unwrap();
        assert_eq!(repairs.len(), 1, "{repairs:?}");
        // Index 1's address itself, with no key at index 1: a moved index, refused.
        edit(
            "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
            "TPjjvMwjPoDC32V2dGDYTkLH4E5LAtBZ6C",
        );
        let e = unlock_repairing(tmp.path(), PW, &two_encodings)
            .err()
            .unwrap();
        assert!(e.message.contains("indices were changed"), "{}", e.message);
    }

    #[test]
    fn a_changed_index_is_refused_not_repaired() {
        // `b`'s index changed from 1 to 5: its cache still holds index 1's address. Repairing
        // would make `b` show index 5's account; refused, naming the index, for unlock and repair.
        let tmp = two_keys(|f| {
            f.keys[1].index = 5;
            f.next_index = 6;
        });
        for err in [
            unlock(tmp.path(), PW, &fake_derive).err().unwrap(),
            unlock_repairing(tmp.path(), PW, &fake_derive)
                .err()
                .unwrap(),
        ] {
            assert!(
                err.message
                    .contains("key 'b' has index 5 but caches the alpha address of index 1")
                    && err.message.contains("Not repaired"),
                "{}",
                err.message
            );
            // Two remedies, each named; no vague "(or the file)".
            assert!(
                err.message.contains(
                    "Restore the indices in wallet.json, or delete key 'b''s cached address"
                ) && !err.message.contains("(or the file)"),
                "{}",
                err.message
            );
            assert!(
                !err.message.contains("run `zwallet address --verify`"),
                "{}",
                err.message
            );
        }
        // Two keys' indices swapped: the other key is named.
        let tmp = two_keys(|f| {
            f.keys[0].index = 1;
            f.keys[1].index = 0;
        });
        let err = unlock_repairing(tmp.path(), PW, &fake_derive)
            .err()
            .unwrap();
        assert!(
            err.message
                .contains("key 'a' has index 1 but caches the alpha address of index 0, key 'b''s"),
            "{}",
            err.message
        );
        let bytes = fs::read(tmp.path().join(WALLET_FILE)).unwrap();
        let file: WalletFile = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            (file.keys[0].index, file.keys[1].index),
            (1, 0),
            "nothing written"
        );
    }

    #[test]
    fn a_repair_refuses_duplicate_names_and_indices() {
        for (edit, why) in [
            (
                (|f: &mut WalletFile| {
                    f.keys[1].name = "a".into();
                    f.keys[1].addresses.insert("alpha".into(), "forged".into());
                }) as fn(&mut WalletFile),
                "two keys are named 'a'",
            ),
            (
                |f: &mut WalletFile| {
                    f.keys[1].index = 0;
                    f.keys[1].addresses.insert("alpha".into(), "forged".into());
                },
                "key 'b' reuses derivation index 0",
            ),
        ] {
            let tmp = two_keys(edit);
            let err = unlock_repairing(tmp.path(), PW, &fake_derive)
                .err()
                .unwrap();
            assert!(err.message.contains(why), "{}", err.message);
        }
    }

    #[test]
    fn a_repair_refuses_a_master_that_changed_after_the_password() {
        let tmp = two_keys(|f| {
            f.keys[1].addresses.insert("alpha".into(), "forged".into());
        });
        let file = read_wallet_file(tmp.path()).unwrap();
        let seed = decrypt(&file.crypto, PW).unwrap().seed;
        // The envelope the password opened is not the one on disk any more.
        let mut opened = file.crypto.clone();
        opened.nonce = "AAAAAAAAAAAAAAAA".into();
        let lock = MetaLock::acquire(tmp.path()).unwrap();
        let err = repair_under_lock(&lock, &opened, &seed, &fake_derive)
            .err()
            .unwrap();
        assert!(
            err.message
                .contains("master changed while the password was checked"),
            "{}",
            err.message
        );
        let now = read_wallet_file(tmp.path()).unwrap();
        assert_eq!(now.keys[1].addresses["alpha"], "forged", "nothing written");
    }

    #[test]
    fn a_stray_copy_of_another_keys_address_is_repaired_not_called_a_move() {
        // `a` (index 0) caches the address `b` (index 1) correctly owns: `a` holds a stray copy,
        // not a moved index. Because a key sits at index 1 with a correct cache there, this is a
        // repair of `a`, not a refusal.
        let tmp = two_keys(|f| {
            let b_addr = f.keys[1].addresses["alpha"].clone();
            f.keys[0].addresses.insert("alpha".into(), b_addr);
        });
        let (unsealed, file, repairs) = unlock_repairing(tmp.path(), PW, &fake_derive).unwrap();
        let want0 = fake_derive(&unsealed.seed, 0).unwrap()["alpha"].clone();
        let want1 = fake_derive(&unsealed.seed, 1).unwrap()["alpha"].clone();
        assert_eq!(repairs.len(), 1, "{repairs:?}");
        assert_eq!(
            (repairs[0].key.as_str(), repairs[0].now.as_str()),
            ("a", want0.as_str())
        );
        assert_eq!(file.keys[0].addresses["alpha"], want0);
        // `b`, which legitimately sits at index 1, is left as it is.
        assert_eq!(file.keys[1].addresses["alpha"], want1);
    }

    #[test]
    fn a_repair_under_lock_writes_nothing_when_the_cache_is_already_right() {
        // Another process repaired first: under the lock the on-disk cache already matches the
        // seed, so the write is skipped and wallet.json is untouched (S5).
        let tmp = two_keys(|_| {});
        let file = read_wallet_file(tmp.path()).unwrap();
        let seed = decrypt(&file.crypto, PW).unwrap().seed;
        let before = fs::read(tmp.path().join(WALLET_FILE)).unwrap();
        let lock = MetaLock::acquire(tmp.path()).unwrap();
        let (_, repairs) = repair_under_lock(&lock, &file.crypto, &seed, &fake_derive).unwrap();
        assert!(repairs.is_empty(), "{repairs:?}");
        drop(lock);
        assert_eq!(
            fs::read(tmp.path().join(WALLET_FILE)).unwrap(),
            before,
            "nothing written"
        );
    }

    #[test]
    fn the_repair_catches_a_master_swapped_in_after_the_write() {
        // A same-user process swaps wallet.json for one with a different master right after the
        // repair writes: the read-back from disk no longer matches the envelope the password
        // opened, so the repair errors instead of leaving a cache written under a master this
        // password never opened (S3).
        let tmp = two_keys(|f| {
            f.keys[1].addresses.insert("alpha".into(), "forged".into());
        });
        let file = read_wallet_file(tmp.path()).unwrap();
        let seed = decrypt(&file.crypto, PW).unwrap().seed;
        let path = tmp.path().join(WALLET_FILE);
        let lock = MetaLock::acquire(tmp.path()).unwrap();
        let swap = || {
            let mut swapped = read_wallet_file(tmp.path()).unwrap();
            swapped.crypto.nonce = "AAAAAAAAAAAAAAAA".into();
            fs::write(&path, serde_json::to_vec(&swapped).unwrap()).unwrap();
        };
        let err = repair_under_lock_hooked(&lock, &file.crypto, &seed, &fake_derive, swap)
            .err()
            .unwrap();
        assert!(
            err.message
                .contains("master changed during the cache repair"),
            "{}",
            err.message
        );
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
    fn plan_new_key_stays_below_the_hardened_range() {
        let tmp = tempfile::tempdir().unwrap();
        create_master(tmp.path(), PW, "a", fast(), &fake_derive).unwrap();
        let base = read_wallet_file(tmp.path()).unwrap();
        let last = (1u32 << 31) - 1;
        // The last non-hardened index is still free.
        let mut file = base.clone();
        file.next_index = last;
        assert_eq!(plan_new_key(&file, "b").unwrap(), last);
        // From 2^31 on, by the counter or past the highest key, nothing is left.
        let mut at_limit = base.clone();
        at_limit.next_index = 1 << 31;
        let mut past_last = base.clone();
        let mut far = past_last.keys[0].clone();
        far.name = "far".into();
        far.index = last;
        past_last.keys.push(far);
        for file in [at_limit, past_last] {
            let err = plan_new_key(&file, "b").unwrap_err();
            assert_eq!(err.code, ErrorCode::ValidationError);
            assert!(err.message.contains("2^31"), "{}", err.message);
        }
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
    fn existing_home_still_fsyncs_its_parent() {
        // R6: a home left by an earlier run (whose parent fsync may never have happened) gets
        // its parent fsynced too. Observable through a parent that cannot be opened to sync.
        let tmp = tempfile::tempdir().unwrap();
        let parent = tmp.path().join("p");
        let home = parent.join("zw");
        fs::create_dir(&parent).unwrap();
        ensure_home_dir(&home).unwrap();
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o300)).unwrap();
        let existing = ensure_home_dir(&home);
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
        let err = existing.unwrap_err();
        assert!(err.message.contains("cannot sync"), "{}", err.message);
        assert_eq!(mode(&home), 0o700);
    }

    #[test]
    fn sync_dir_reports_errors() {
        let tmp = tempfile::tempdir().unwrap();
        sync_dir(tmp.path()).unwrap();
        let err = sync_dir(&tmp.path().join("missing")).unwrap_err();
        assert!(err.message.contains("cannot sync"), "{}", err.message);
    }

    #[test]
    fn the_whitelist_secret_is_its_own_scrypt_of_the_password_and_the_salt() {
        let salt = [7u8; 32];
        let secret = derive_whitelist_secret(PW, &salt, fast_kdf()).unwrap();
        // Deterministic: nothing needs storing.
        assert_eq!(
            *derive_whitelist_secret(PW, &salt, fast_kdf()).unwrap(),
            *secret
        );
        // Another salt, another password or other parameters give another secret.
        let others = [
            derive_whitelist_secret(PW, &[8u8; 32], fast_kdf()).unwrap(),
            derive_whitelist_secret("another password", &salt, fast_kdf()).unwrap(),
            derive_whitelist_secret(
                PW,
                &salt,
                KdfParams {
                    log_n: KDF_MIN_LOG_N + 1,
                    ..fast_kdf()
                },
            )
            .unwrap(),
        ];
        for other in &others {
            assert_ne!(**other, *secret);
        }
        // Domain-separated: never the key scrypt gives for the bare salt (the AES key a box
        // sealed with that salt would use).
        assert_ne!(*derive_key(PW, &salt, &fast_kdf()).unwrap(), *secret);
        let mut prefixed = b"zwallet/whitelist-key/v1\n".to_vec();
        prefixed.extend_from_slice(&salt);
        assert_eq!(*derive_key(PW, &prefixed, &fast_kdf()).unwrap(), *secret);
        // Parameters outside the accepted range are refused before anything allocates.
        let err = derive_whitelist_secret(
            PW,
            &salt,
            KdfParams {
                log_n: 30,
                ..fast_kdf()
            },
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::ValidationError);
    }

    #[test]
    fn a_wallet_salt_of_any_length_but_32_bytes_is_refused_before_the_kdf() {
        // Every zwallet seals wallet.json with a 32-byte salt. Any other length is refused as
        // corrupt before scrypt runs, so wallet.json's AES key can never be scrypt over the
        // whitelist key's domain-prefixed salt (25 + 32 bytes).
        let sealed = encrypt(&test_seed(), &empty(), PW, fast_kdf()).unwrap();
        assert_eq!(hex::decode(&sealed.kdfparams.salt).unwrap().len(), 32);
        let mut prefixed = b"zwallet/whitelist-key/v1\n".to_vec();
        prefixed.extend_from_slice(&[7u8; 32]);
        for salt in [vec![0u8; 16], vec![0u8; 31], vec![0u8; 33], prefixed] {
            let mut bad = sealed.clone();
            bad.kdfparams.salt = hex::encode(&salt);
            let err = decrypt(&bad, PW).err().unwrap();
            assert_eq!(err.code, ErrorCode::ValidationError, "{} bytes", salt.len());
            assert!(
                err.message.contains("bad salt or nonce length"),
                "{} bytes: {}",
                salt.len(),
                err.message
            );
        }
        decrypt(&sealed, PW).unwrap();
    }

    #[test]
    fn whitelist_anchor_rides_in_the_sealed_policy() {
        assert_eq!(WhitelistAnchor::from_policy(&empty()).unwrap(), None);
        assert_eq!(SealedWhitelist::from_policy(&empty()).unwrap(), None);
        let anchor = WhitelistAnchor {
            public_key: [0xab; 32],
            generation: 3,
            digest: [0x01; 32],
        };
        let salt = [0x5a; 32];
        let other = serde_json::json!({ "future": [1, 2] });
        let policy = anchor.to_policy(&salt, &other).unwrap();
        assert_eq!(
            policy["future"],
            serde_json::json!([1, 2]),
            "other keys kept"
        );
        assert_eq!(policy["whitelistPub"], hex::encode([0xab; 32]));
        assert_eq!(policy["whitelistGen"], 3);
        assert_eq!(policy["whitelistDigest"], hex::encode([0x01; 32]));
        assert_eq!(policy["whitelistSalt"], hex::encode(salt));
        assert_eq!(WhitelistAnchor::from_policy(&policy).unwrap(), Some(anchor));
        let sealed = SealedWhitelist::from_policy(&policy).unwrap().unwrap();
        assert_eq!((sealed.anchor, sealed.salt().unwrap()), (anchor, salt));

        // Through the seal: what the daemon reads at unlock, and what the editor reads.
        let sealed = encrypt(&test_seed(), &policy, PW, fast_kdf()).unwrap();
        let opened = decrypt(&sealed, PW).unwrap();
        assert_eq!(opened.whitelist_anchor().unwrap(), Some(anchor));
        let read = opened.sealed_whitelist().unwrap().unwrap();
        assert_eq!((read.anchor, read.salt().unwrap()), (anchor, salt));

        // An anchor zwallet 0.1.0 sealed (no salt): the editor sees it, and only a reset goes on
        // from it; to every other reader it is unusable, never "no whitelist".
        let mut old = policy.clone();
        old.as_object_mut().unwrap().remove("whitelistSalt");
        let read = SealedWhitelist::from_policy(&old).unwrap().unwrap();
        assert_eq!(read.anchor, anchor);
        let err = read.salt().unwrap_err();
        assert_eq!(err.code, ErrorCode::ValidationError);
        // The cause only: the daemon shows it as is, and the editor adds the recovery.
        assert_eq!(
            err.message,
            "this whitelist was made by zwallet 0.1.0, whose key file is no longer used"
        );
        assert_eq!(
            WhitelistAnchor::from_policy(&old).unwrap_err().message,
            err.message
        );

        // A damaged anchor never reads as "no whitelist".
        let mut damaged = Vec::new();
        for key in ["whitelistPub", "whitelistGen", "whitelistDigest"] {
            let mut p = policy.clone();
            p.as_object_mut().unwrap().remove(key);
            damaged.push(p);
        }
        // A salt alone.
        damaged.push(serde_json::json!({ "whitelistSalt": hex::encode(salt) }));
        for (key, value) in [
            ("whitelistPub", Value::from("AB".repeat(32))),
            ("whitelistPub", Value::from("ab".repeat(31))),
            ("whitelistDigest", Value::from(7)),
            ("whitelistGen", Value::from(0)),
            ("whitelistGen", Value::from("3")),
            ("whitelistGen", Value::from(-1)),
            ("whitelistSalt", Value::from("5A".repeat(32))),
            ("whitelistSalt", Value::from("5a".repeat(33))),
            ("whitelistSalt", Value::Null),
        ] {
            let mut p = policy.clone();
            p[key] = value;
            damaged.push(p);
        }
        for p in damaged {
            let err = WhitelistAnchor::from_policy(&p).unwrap_err();
            assert_eq!(err.code, ErrorCode::ValidationError, "{p}");
            assert!(err.message.contains("malformed"), "{p}: {}", err.message);
            assert!(SealedWhitelist::from_policy(&p).is_err(), "{p}");
        }
        assert!(anchor.to_policy(&salt, &serde_json::json!([])).is_err());
    }

    #[test]
    fn reseal_changes_only_the_master_and_checks_what_it_opened() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        create_master(home, PW, "a", fast(), &fake_derive).unwrap();
        let lock = MetaLock::acquire(home).unwrap();
        let opened = lock.open(PW).unwrap();
        assert_eq!(
            lock.open("not the password").err().unwrap().code,
            ErrorCode::WrongPassword
        );
        let before = read_wallet_file(home).unwrap();
        let policy = serde_json::json!({ "whitelistGen": 1, "x": "y" });
        let after = lock.reseal(&opened, &policy).unwrap();
        assert_ne!(after.crypto.ciphertext, before.crypto.ciphertext);
        assert_ne!(after.crypto.kdfparams.salt, before.crypto.kdfparams.salt);
        assert_eq!(after.crypto.kdfparams.log_n, before.crypto.kdfparams.log_n);
        let on_disk = read_wallet_file(home).unwrap();
        assert_eq!(on_disk.keys, before.keys);
        assert_eq!(on_disk.active, before.active);
        assert_eq!(on_disk.created_at, before.created_at);
        assert_eq!(mode(&home.join(WALLET_FILE)), 0o600);
        assert_eq!(
            data_names(home),
            vec![OsString::from(WALLET_FILE)],
            "no temp left"
        );
        let reopened = decrypt(&on_disk.crypto, PW).unwrap();
        assert_eq!(reopened.policy, policy);
        assert_eq!(reopened.seed.phrase(), opened.unsealed().seed.phrase());

        // `opened` is now stale: wallet.json holds another ciphertext. Nothing is written.
        let bytes = fs::read(home.join(WALLET_FILE)).unwrap();
        let err = lock.reseal(&opened, &empty()).unwrap_err();
        assert!(err.message.contains("changed"), "{}", err.message);
        assert_eq!(fs::read(home.join(WALLET_FILE)).unwrap(), bytes);
        drop(lock);
        // The metadata writers carry the new master through untouched.
        set_active(home, "a").unwrap();
        assert_eq!(unlock(home, PW, &fake_derive).unwrap().0.policy, policy);
    }

    #[test]
    fn reseal_refuses_an_envelope_changed_on_disk_after_open() {
        // M2: the default KDF (log_n 18), so a downgrade to 15 is visible.
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        create_master(home, PW, "a", MasterOptions::default(), &fake_derive).unwrap();
        let path = home.join(WALLET_FILE);
        let original = fs::read(&path).unwrap();
        let lock = MetaLock::acquire(home).unwrap();
        let opened = lock.open(PW).unwrap();
        let policy = serde_json::json!({ "whitelistGen": 1 });
        type Tamper = fn(&mut CryptoBox);
        let tampers: [(&str, Tamper); 5] = [
            ("log_n 18 -> 15", |c| c.kdfparams.log_n = 15),
            ("salt", |c| c.kdfparams.salt = "00".repeat(32)),
            ("nonce", |c| c.nonce = "00".repeat(12)),
            ("dklen", |c| c.kdfparams.dklen = 16),
            ("kdf name", |c| c.kdf = "pbkdf2".into()),
        ];
        for (what, tamper) in tampers {
            // Only the envelope changes; the ciphertext stays the one that was opened.
            let mut file: WalletFile = serde_json::from_slice(&original).unwrap();
            tamper(&mut file.crypto);
            fs::write(&path, to_json_bytes(&file).unwrap()).unwrap();
            let tampered = fs::read(&path).unwrap();
            let err = lock.reseal(&opened, &policy).unwrap_err();
            assert!(err.message.contains("changed"), "{what}: {}", err.message);
            assert_eq!(
                fs::read(&path).unwrap(),
                tampered,
                "{what}: nothing written"
            );
        }
        fs::write(&path, &original).unwrap();
        let after = lock.reseal(&opened, &policy).unwrap();
        assert_eq!(
            after.crypto.kdfparams.log_n, KDF_MAX_LOG_N,
            "the opened KDF carries over"
        );
        assert_eq!(decrypt(&after.crypto, PW).unwrap().policy, policy);
    }

    #[test]
    fn plain_file_writer_replaces_atomically_and_refuses_reserved_names() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        create_master(home, PW, "a", fast(), &fake_derive).unwrap();
        let wallet = fs::read(home.join(WALLET_FILE)).unwrap();
        let lock = MetaLock::acquire(home).unwrap();
        lock.replace_file("w.next", b"one").unwrap();
        lock.replace_file("w.next", b"two").unwrap();
        assert_eq!(fs::read(home.join("w.next")).unwrap(), b"two");
        assert_eq!(mode(&home.join("w.next")), 0o600);
        lock.replace_file("w.cur", b"two").unwrap();
        lock.remove_file("w.next").unwrap();
        lock.remove_file("w.next").unwrap();
        assert_eq!(fs::read(home.join("w.cur")).unwrap(), b"two");
        assert!(!home.join("w.next").exists());
        for name in [
            WALLET_FILE,
            WRITE_LOCK,
            "d.lock",
            "u.lock",
            "d.sock",
            "",
            "a/b",
            ".x",
        ] {
            assert!(lock.replace_file(name, b"x").is_err(), "{name:?}");
            assert!(lock.remove_file(name).is_err(), "{name:?}");
        }
        assert_eq!(fs::read(home.join(WALLET_FILE)).unwrap(), wallet);
        assert_eq!(
            data_names(home),
            vec![OsString::from("w.cur"), OsString::from(WALLET_FILE)]
        );
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
