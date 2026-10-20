//! The signed whitelist (`.prd/04-whitelist.md`): a nickname → address list and a spend
//! policy, signed with an ed25519 key kept apart from the seed and anchored in wallet.json's
//! sealed policy ([`WhitelistAnchor`]: the public key, the generation, and the digest of the one
//! list signed at that generation).
//!
//! Files in the home, all 0600:
//! - `whitelist.json`: the list and its signature, plaintext. Readable while locked; trusted
//!   only after [`verify`] against the sealed anchor.
//! - `whitelist.next.json`: an edit staged between signing it and publishing it as
//!   `whitelist.json` (removed once that is done).
//! - `whitelist.key`: the ed25519 secret, sealed under the wallet password with its own salt,
//!   nonce and associated data. Read by the editor only, never by the daemon.
//!
//! What each part guarantees:
//! - The **signature** (ed25519, by a key kept in `whitelist.key` apart from the seed) says the
//!   list was made by whoever holds that key: the editor, with the wallet password. The daemon
//!   only ever holds the public key, so it can check lists but never make one.
//! - The sealed **digest** says this is the one list committed at the sealed generation, so a
//!   list the key signed for an edit that never committed is not accepted in its place.
//! - The sealed **generation** names that commit for humans (`unlock` and `whitelist list` show
//!   it) and makes an older copy fail with a plain reason.
//!
//! An edit ([`edit`], [`reset`]) is linearized under `w.lock`: read `whitelist.json` and
//! `whitelist.next.json` once ([`ListFiles`]; with [`edit_as_shown`] / [`reset_as_shown`], refuse
//! if those bytes are not the ones the human was shown), pick the base from those bytes
//! ([`pick_base_from`]: the current list if it verifies, else a staged list the seal already
//! names; an edit with neither, or with list files but no sealed anchor, refuses,
//! [`base_for_edit`], and only a reset starts over from empty), write a staged sealed list's
//! verified bytes as `whitelist.json`, apply the change at generation + 1,
//! sign, write `whitelist.next.json`, re-seal wallet.json with the new anchor, write the signed
//! bytes from memory as `whitelist.json` and remove `next` (best-effort). An unverified list is
//! never signed. A crash at any point leaves the old
//! generation standing or the staged one promotable. The caller stops every daemon first (a
//! re-seal ends a running session anyway: the daemon notices wallet.json's ciphertext changed).
//!
//! The format is EVM-shaped (20-byte addresses, 4-byte selectors); which chain keys exist and
//! which addresses are registry tokens come from the chains layer through [`ChainRules`].

use crate::core::error::{ErrorCode, Result, WalletError};
use crate::core::store::{self, MetaLock, WhitelistAnchor};
use alloy::hex;
use alloy::primitives::{U256, keccak256};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rand::RngCore;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

pub const WHITELIST_FILE: &str = "whitelist.json";
pub const NEXT_FILE: &str = "whitelist.next.json";
pub const KEY_FILE: &str = "whitelist.key";
/// The list format this zwallet reads and signs.
pub const LIST_VERSION: u32 = 1;
const KEY_FILE_VERSION: u32 = 1;
/// The chains of an address entry usable on every chain (never a contract entry's).
pub const ANY_CHAIN: &str = "*";
/// Prefixed to the canonical bytes before signing, so a signature from this key can never be
/// read as one over anything else.
const SIGNING_DOMAIN: &[u8] = b"zwallet/whitelist/v1\n";

/// What the list's validation needs to know about chains, supplied by the chains layer
/// (`chains::evm::registry::WHITELIST_RULES`) so core stays chain-agnostic.
#[derive(Debug, Clone, Copy)]
pub struct ChainRules {
    /// Whether `key` is exactly a supported chain key (`"base"`).
    pub chain_known: fn(&str) -> bool,
    /// Whether the `0x` address is a registry token on any chain.
    pub is_registry_token: fn(&str) -> bool,
    /// The fee caps a new list starts with: chain key → wei, as a decimal string.
    pub default_fee_caps_wei: &'static [(&'static str, &'static str)],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    /// A payee: native coin and registry tokens may go to it.
    Address,
    /// A contract that may be called with one of its listed selectors.
    Contract,
}

impl EntryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Address => "address",
            Self::Contract => "contract",
        }
    }
}

/// One whitelist entry. Stored exactly in canonical form: `address` and `selectors` lower-case
/// `0x` hex. `selectors` is present on contract entries only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub nickname: String,
    pub kind: EntryKind,
    pub address: String,
    /// Chain keys, or `["*"]` (address entries only).
    pub chains: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selectors: Option<Vec<String>>,
}

impl Entry {
    /// A payee entry; `address` is lower-cased (check it, e.g. its EIP-55 checksum, before).
    pub fn new_address(nickname: &str, address: &str, chains: &[&str]) -> Self {
        Self {
            nickname: nickname.to_string(),
            kind: EntryKind::Address,
            address: address.trim().to_ascii_lowercase(),
            chains: chains.iter().map(|c| c.to_string()).collect(),
            selectors: None,
        }
    }

    /// A contract entry; `address` and `selectors` are lower-cased.
    pub fn new_contract(
        nickname: &str,
        address: &str,
        chains: &[&str],
        selectors: &[&str],
    ) -> Self {
        Self {
            nickname: nickname.to_string(),
            kind: EntryKind::Contract,
            address: address.trim().to_ascii_lowercase(),
            chains: chains.iter().map(|c| c.to_string()).collect(),
            selectors: Some(
                selectors
                    .iter()
                    .map(|s| s.trim().to_ascii_lowercase())
                    .collect(),
            ),
        }
    }

    /// Whether this entry applies on chain `key` (`"*"` covers every chain).
    pub fn covers_chain(&self, key: &str) -> bool {
        self.chains.iter().any(|c| c == key || c == ANY_CHAIN)
    }

    /// Whether `selector` (`0x` + 8 hex digits, any case) is one this contract entry lists.
    pub fn allows_selector(&self, selector: &str) -> bool {
        self.selectors
            .as_deref()
            .unwrap_or_default()
            .iter()
            .any(|s| s.eq_ignore_ascii_case(selector))
    }
}

/// The policy inside the signed list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Policy {
    /// Chain key → the most `gasLimit × max price` may cost, in wei, as a decimal string (wei
    /// amounts exceed 2^53). A chain named twice in a file is refused, not collapsed.
    #[serde(deserialize_with = "unique_keys")]
    pub max_fee_wei: BTreeMap<String, String>,
    /// Whether `sign-message` may sign at all (off by default: some contracts take EIP-191
    /// signatures as spending authority).
    pub allow_message_signing: bool,
}

/// A JSON object of strings into a map, refusing a key that appears twice (a map would keep the
/// last one silently, so two files that read differently would parse alike).
fn unique_keys<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<BTreeMap<String, String>, D::Error> {
    struct Unique;
    impl<'de> serde::de::Visitor<'de> for Unique {
        type Value = BTreeMap<String, String>;

        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("an object of strings")
        }

        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            let mut out = BTreeMap::new();
            while let Some((key, value)) = map.next_entry::<String, String>()? {
                if out.contains_key(&key) {
                    return Err(serde::de::Error::custom(format!(
                        "maxFeeWei names chain {} twice",
                        quote(&key)
                    )));
                }
                out.insert(key, value);
            }
            Ok(out)
        }
    }
    deserializer.deserialize_map(Unique)
}

impl Policy {
    /// The policy a new list starts with: the default fee caps, message signing off.
    pub fn defaults(rules: &ChainRules) -> Self {
        Self {
            max_fee_wei: rules
                .default_fee_caps_wei
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            allow_message_signing: false,
        }
    }

    /// The fee cap on chain `key`, if the list has one (a malformed one reads as none).
    pub fn max_fee_wei(&self, key: &str) -> Option<U256> {
        self.max_fee_wei.get(key).and_then(|v| parse_wei(v).ok())
    }

    pub fn set_max_fee_wei(&mut self, key: &str, wei: U256) {
        self.max_fee_wei.insert(key.to_string(), wei.to_string());
    }
}

/// The signed body of a list: everything but the signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Whitelist {
    /// Starts at 1 and grows by one per edit; the sealed anchor names the current one.
    pub generation: u64,
    pub entries: Vec<Entry>,
    pub policy: Policy,
}

impl Whitelist {
    /// An empty list at generation 0 (an edit makes it 1) with the default policy.
    pub fn empty(rules: &ChainRules) -> Self {
        Self {
            generation: 0,
            entries: Vec::new(),
            policy: Policy::defaults(rules),
        }
    }

    pub fn by_nickname(&self, nickname: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.nickname == nickname)
    }

    /// The entry for `address` (any case), whatever its kind and chains.
    pub fn by_address(&self, address: &str) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|e| e.address.eq_ignore_ascii_case(address.trim()))
    }

    /// The address entry for `address` that applies on chain `key`: a payee.
    pub fn payee(&self, address: &str, key: &str) -> Option<&Entry> {
        self.by_address(address)
            .filter(|e| e.kind == EntryKind::Address && e.covers_chain(key))
    }

    /// The contract entry for `address` that applies on chain `key`.
    pub fn contract(&self, address: &str, key: &str) -> Option<&Entry> {
        self.by_address(address)
            .filter(|e| e.kind == EntryKind::Contract && e.covers_chain(key))
    }

    /// Adds `entry`; its nickname and its address must both be new.
    pub fn add(&mut self, entry: Entry) -> Result<()> {
        if self.by_nickname(&entry.nickname).is_some() {
            return Err(WalletError::validation(format!(
                "the whitelist already has an entry named {}",
                quote(&entry.nickname)
            )));
        }
        if let Some(other) = self.by_address(&entry.address) {
            return Err(WalletError::validation(format!(
                "{} is already in the whitelist as {}",
                quote(&entry.address),
                quote(&other.nickname)
            )));
        }
        self.entries.push(entry);
        Ok(())
    }

    /// Removes the entry named `nickname`, returning it (`None` if there was none).
    pub fn remove(&mut self, nickname: &str) -> Option<Entry> {
        let pos = self.entries.iter().position(|e| e.nickname == nickname)?;
        Some(self.entries.remove(pos))
    }

    /// Everything a list must satisfy before it is signed: [`Whitelist::check_format`], every
    /// chain key known, and no contract entry at a registry token's address (its selectors would
    /// bypass the revoke-only approval rule).
    pub fn validate(&self, rules: &ChainRules) -> Result<()> {
        self.check_format()?;
        for e in &self.entries {
            for c in e.chains.iter().filter(|c| *c != ANY_CHAIN) {
                if !(rules.chain_known)(c) {
                    return Err(entry_error(e, &format!("unknown chain {}", quote(c))));
                }
            }
            if e.kind == EntryKind::Contract && (rules.is_registry_token)(&e.address) {
                return Err(entry_error(
                    e,
                    "a registry token cannot be a contract entry (token transfers to payees and \
                     approvals of 0 are allowed without one)",
                ));
            }
        }
        for c in self.policy.max_fee_wei.keys() {
            if !(rules.chain_known)(c) {
                return Err(WalletError::validation(format!(
                    "whitelist fee cap for unknown chain {}",
                    quote(c)
                )));
            }
        }
        Ok(())
    }

    /// The checks that need no chain knowledge: generation from 1; nicknames per
    /// [`validate_nickname`] and unique; each address `0x` + 40 lower-case hex digits and listed
    /// once; chains non-empty and unique, `"*"` only alone and only on an address entry;
    /// selectors on contract entries only, non-empty, unique, `0x` + 8 lower-case hex digits;
    /// fee caps decimal wei without leading zeros.
    pub fn check_format(&self) -> Result<()> {
        if self.generation == 0 {
            return Err(WalletError::validation(
                "whitelist generation must be at least 1",
            ));
        }
        let (mut nicks, mut addrs) = (BTreeSet::new(), BTreeSet::new());
        for e in &self.entries {
            validate_nickname(&e.nickname)?;
            if !nicks.insert(e.nickname.as_str()) {
                return Err(entry_error(e, "nickname listed twice"));
            }
            if !is_lower_hex(&e.address, 20) {
                return Err(entry_error(
                    e,
                    &format!(
                        "address {} must be 0x and 40 lower-case hex digits",
                        quote(&e.address)
                    ),
                ));
            }
            if !addrs.insert(e.address.as_str()) {
                return Err(entry_error(
                    e,
                    "address already listed under another nickname",
                ));
            }
            if e.chains.is_empty() {
                return Err(entry_error(e, "no chains"));
            }
            let mut seen = BTreeSet::new();
            for c in &e.chains {
                if !seen.insert(c.as_str()) {
                    return Err(entry_error(e, &format!("chain {} listed twice", quote(c))));
                }
                if c == ANY_CHAIN {
                    if e.kind == EntryKind::Contract {
                        return Err(entry_error(
                            e,
                            "a contract entry names its chains, never \"*\"",
                        ));
                    }
                    if e.chains.len() > 1 {
                        return Err(entry_error(e, "\"*\" must be the only chain"));
                    }
                } else if !is_chain_key(c) {
                    return Err(entry_error(e, &format!("{} is not a chain key", quote(c))));
                }
            }
            match (e.kind, &e.selectors) {
                (EntryKind::Address, None) => {}
                (EntryKind::Address, Some(_)) => {
                    return Err(entry_error(e, "an address entry has no selectors"));
                }
                (EntryKind::Contract, None) => {
                    return Err(entry_error(e, "a contract entry needs selectors"));
                }
                (EntryKind::Contract, Some(sels)) => {
                    if sels.is_empty() {
                        return Err(entry_error(e, "a contract entry needs selectors"));
                    }
                    let mut seen = BTreeSet::new();
                    for s in sels {
                        if !is_lower_hex(s, 4) {
                            return Err(entry_error(
                                e,
                                &format!(
                                    "selector {} must be 0x and 8 lower-case hex digits (no \
                                     wildcard)",
                                    quote(s)
                                ),
                            ));
                        }
                        if !seen.insert(s.as_str()) {
                            return Err(entry_error(
                                e,
                                &format!("selector {} listed twice", quote(s)),
                            ));
                        }
                    }
                }
            }
        }
        for (c, v) in &self.policy.max_fee_wei {
            if !is_chain_key(c) {
                return Err(WalletError::validation(format!(
                    "whitelist fee cap key {} is not a chain key",
                    quote(c)
                )));
            }
            parse_wei(v).map_err(|e| {
                WalletError::validation(format!(
                    "whitelist fee cap for {}: {}",
                    quote(c),
                    e.message
                ))
            })?;
        }
        Ok(())
    }
}

fn entry_error(e: &Entry, why: &str) -> WalletError {
    WalletError::validation(format!("whitelist entry {}: {why}", quote(&e.nickname)))
}

/// The most characters of a value read from a file that a message repeats.
const SHOWN_CHARS: usize = 64;

/// A value that may come from a file, for a message a human reads (possibly while typing the
/// password): quoted with control characters escaped (`{:?}`), and cut after [`SHOWN_CHARS`]
/// characters, so a planted terminal escape or a fake instruction is shown inert.
fn quote(value: &str) -> String {
    match value.char_indices().nth(SHOWN_CHARS) {
        Some((cut, _)) => format!("{:?}…", &value[..cut]),
        None => format!("{value:?}"),
    }
}

/// A parser's message that may repeat file content: control characters escaped and cut after
/// `max` characters.
fn clip(message: &str, max: usize) -> String {
    let escaped: String = message
        .chars()
        .flat_map(|c| {
            let e: Vec<char> = if c.is_control() {
                c.escape_debug().collect()
            } else {
                vec![c]
            };
            e
        })
        .collect();
    match escaped.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", &escaped[..cut]),
        None => escaped,
    }
}

/// Nicknames: `^[a-z0-9][a-z0-9_-]{0,31}$`.
pub fn validate_nickname(nickname: &str) -> Result<()> {
    let b = nickname.as_bytes();
    let ok = !b.is_empty()
        && b.len() <= 32
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_' || *c == b'-');
    if ok {
        Ok(())
    } else {
        Err(WalletError::validation(format!(
            "invalid whitelist nickname {}: use 1-32 of a-z 0-9 _ - starting with a letter or \
             digit",
            quote(nickname)
        )))
    }
}

/// `0x` followed by exactly `bytes` bytes of lower-case hex.
fn is_lower_hex(s: &str, bytes: usize) -> bool {
    s.strip_prefix("0x").is_some_and(|h| {
        h.len() == bytes * 2 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    })
}

/// The shape of a chain key: 1-32 of a-z 0-9 -, starting with a letter or digit.
fn is_chain_key(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 32
        && b[0] != b'-'
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// A wei amount in canonical decimal: digits only, no leading zero, at most 2^256 - 1.
fn parse_wei(s: &str) -> Result<U256> {
    let canonical =
        !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) && (s == "0" || !s.starts_with('0'));
    if !canonical {
        return Err(WalletError::validation(format!(
            "{} is not a decimal wei amount (digits only, no leading zero)",
            quote(s)
        )));
    }
    U256::from_str_radix(s, 10)
        .map_err(|_| WalletError::validation(format!("{} wei is too large", quote(s))))
}

/// The canonical encoding the signature covers: JSON of `{version, gen, entries, policy}` with
/// every object's keys in byte order, no whitespace, strings as stored (lower-case hex), wei
/// amounts as decimal strings and `gen`/`version` as numbers. Written out by hand rather than
/// through serde, so neither serde's field order nor a map-ordering feature can change it.
pub fn canonical_bytes(list: &Whitelist) -> Vec<u8> {
    let mut out = String::with_capacity(256);
    out.push_str("{\"entries\":[");
    for (i, e) in list.entries.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"address\":");
        push_json_str(&mut out, &e.address);
        out.push_str(",\"chains\":");
        push_json_strs(&mut out, &e.chains);
        out.push_str(",\"kind\":");
        push_json_str(&mut out, e.kind.as_str());
        out.push_str(",\"nickname\":");
        push_json_str(&mut out, &e.nickname);
        if let Some(sels) = &e.selectors {
            out.push_str(",\"selectors\":");
            push_json_strs(&mut out, sels);
        }
        out.push('}');
    }
    out.push_str("],\"gen\":");
    out.push_str(&list.generation.to_string());
    out.push_str(",\"policy\":{\"allowMessageSigning\":");
    out.push_str(if list.policy.allow_message_signing {
        "true"
    } else {
        "false"
    });
    out.push_str(",\"maxFeeWei\":{");
    // BTreeMap<String, _> iterates in byte order of the keys.
    for (i, (k, v)) in list.policy.max_fee_wei.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        push_json_str(&mut out, k);
        out.push(':');
        push_json_str(&mut out, v);
    }
    out.push_str("}},\"version\":");
    out.push_str(&LIST_VERSION.to_string());
    out.push('}');
    out.into_bytes()
}

fn push_json_strs(out: &mut String, items: &[String]) {
    out.push('[');
    for (i, s) in items.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        push_json_str(out, s);
    }
    out.push(']');
}

/// A JSON string literal: `"` and `\` escaped, control characters as `\u00XX`, everything else
/// as is. (Validated lists hold printable ASCII only; this keeps any input unambiguous.)
fn push_json_str(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// The exact bytes signed: a domain prefix, then [`canonical_bytes`].
fn signed_message(list: &Whitelist) -> Vec<u8> {
    let mut m = SIGNING_DOMAIN.to_vec();
    m.extend_from_slice(&canonical_bytes(list));
    m
}

/// The digest the sealed anchor pins: keccak-256 of the signed message.
pub fn digest(list: &Whitelist) -> [u8; 32] {
    keccak256(signed_message(list)).0
}

/// A list with its ed25519 signature, as stored in `whitelist.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedList {
    pub list: Whitelist,
    pub sig: [u8; 64],
}

/// `whitelist.json` / `whitelist.next.json` on disk.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListFile {
    version: u32,
    r#gen: u64,
    entries: Vec<Entry>,
    policy: Policy,
    sig: String,
}

impl SignedList {
    /// The file's bytes: pretty JSON, a trailing newline. (Layout is free: the signature covers
    /// [`canonical_bytes`] of the parsed list.)
    pub fn to_file_bytes(&self) -> Result<Vec<u8>> {
        let file = ListFile {
            version: LIST_VERSION,
            r#gen: self.list.generation,
            entries: self.list.entries.clone(),
            policy: self.list.policy.clone(),
            sig: hex::encode(self.sig),
        };
        let mut bytes = serde_json::to_vec_pretty(&file)
            .map_err(|e| WalletError::validation(format!("cannot serialize the whitelist: {e}")))?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    /// Parses a list file: unknown or repeated keys, another version, or a signature that is not
    /// 64 bytes of lower-case hex are `VALIDATION_ERROR`. The content is not checked here
    /// ([`verify`] does that).
    pub fn from_file_bytes(bytes: &[u8]) -> Result<Self> {
        let file: ListFile = serde_json::from_slice(bytes).map_err(|e| {
            // serde's message can repeat file content (an unknown key, say): clip it.
            let full = e.to_string();
            let what = full
                .rsplit_once(" at line ")
                .map_or(full.as_str(), |(m, _)| m);
            WalletError::validation(format!(
                "not a whitelist file: {} (line {}, column {})",
                clip(what, SHOWN_CHARS + 16),
                e.line(),
                e.column()
            ))
        })?;
        if file.version != LIST_VERSION {
            return Err(WalletError::validation(format!(
                "unsupported whitelist version {} (this zwallet reads version {LIST_VERSION})",
                file.version
            )));
        }
        let mut sig = [0u8; 64];
        let canonical = file.sig.len() == 128
            && file
                .sig
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        if !canonical || hex::decode_to_slice(&file.sig, &mut sig).is_err() {
            return Err(WalletError::validation(
                "whitelist signature must be 64 bytes of lower-case hex",
            ));
        }
        Ok(Self {
            list: Whitelist {
                generation: file.r#gen,
                entries: file.entries,
                policy: file.policy,
            },
            sig,
        })
    }
}

/// The ed25519 key that signs the list. Deliberately not `Debug`; the secret is wiped on drop.
pub struct WhitelistKey(SigningKey);

impl WhitelistKey {
    /// A fresh key from the OS's randomness.
    pub fn generate() -> Result<Self> {
        let mut secret = Zeroizing::new([0u8; 32]);
        OsRng
            .try_fill_bytes(secret.as_mut())
            .map_err(|_| WalletError::validation("could not read OS randomness"))?;
        Ok(Self::from_secret(&secret))
    }

    pub fn from_secret(secret: &[u8; 32]) -> Self {
        Self(SigningKey::from_bytes(secret))
    }

    pub fn public_key(&self) -> [u8; 32] {
        self.0.verifying_key().to_bytes()
    }

    /// Signs `list` as it is (validate it first: an edit never signs an unvalidated list).
    pub fn sign(&self, list: &Whitelist) -> SignedList {
        SignedList {
            list: list.clone(),
            sig: self.0.sign(&signed_message(list)).to_bytes(),
        }
    }

    /// The anchor that makes `signed` (signed by this key) the current list.
    pub fn anchor_for(&self, signed: &SignedList) -> WhitelistAnchor {
        WhitelistAnchor {
            public_key: self.public_key(),
            generation: signed.list.generation,
            digest: digest(&signed.list),
        }
    }
}

/// Whether `signed` is the list the sealed `anchor` names: well-formed
/// ([`Whitelist::check_format`]), at the sealed generation, signed (strict ed25519) by the sealed
/// public key, and the very list sealed at that generation (its digest). `VALIDATION_ERROR` with
/// the reason otherwise; the daemon reports it as `WHITELIST_INVALID`.
///
/// It checks the format, not [`Whitelist::validate`]'s chain rules: a list signed before the
/// registry gained a token stays valid as a whole, and the signer instead refuses any contract
/// call to a registry token at sign time.
pub fn verify(signed: &SignedList, anchor: &WhitelistAnchor) -> Result<()> {
    let list = &signed.list;
    list.check_format()?;
    if list.generation != anchor.generation {
        return Err(WalletError::validation(format!(
            "whitelist generation {} is not the sealed generation {} (an older or a foreign copy)",
            list.generation, anchor.generation
        )));
    }
    let key = VerifyingKey::from_bytes(&anchor.public_key)
        .ok()
        .filter(|k| !k.is_weak())
        .ok_or_else(|| {
            WalletError::validation("wallet.json's sealed whitelist key is not a valid ed25519 key")
        })?;
    if !signed_by(signed, &key) {
        return Err(WalletError::validation(
            "whitelist signature does not verify against this wallet's whitelist key (edited by \
             hand, or signed by another key)",
        ));
    }
    let message = signed_message(list);
    if keccak256(&message).0 != anchor.digest {
        return Err(WalletError::validation(format!(
            "whitelist is not the list sealed at generation {} (an abandoned edit's copy)",
            anchor.generation
        )));
    }
    Ok(())
}

/// Whether `signed`'s signature verifies (strict ed25519) under `key`.
fn signed_by(signed: &SignedList, key: &VerifyingKey) -> bool {
    key.verify_strict(
        &signed_message(&signed.list),
        &Signature::from_bytes(&signed.sig),
    )
    .is_ok()
}

/// Why a `whitelist.json` above the sealed generation is there, by its generation and its signer
/// (`None` at or below the seal, or with a sealed key that is not a valid key). A list is
/// published as `whitelist.json` only after the re-seal that names it, so the seal is the first
/// suspect: an older `whitelist.json` would verify under a rolled-back seal and complete a joint
/// rollback, so it is offered only behind the highest generation the human remembers. Within one
/// seal lineage the seal only moves forward, so with no rollback that is the sealed generation;
/// after any rollback it is higher. A reset or first edit made while wallet.json sealed no
/// whitelist starts again at gen 1, so only generations seen since then count.
/// - two or more edits above the seal, under the sealed key: only a rolled-back wallet.json
///   explains it, and no copy is offered;
/// - one edit above, under the sealed key: that, or an edit that never finished sealing (its
///   staged copy put in place of `whitelist.json`);
/// - under another key: wallet.json rolled back across a reset, a reset that never finished
///   sealing, or a foreign file.
fn above_seal(signed: &SignedList, anchor: &WhitelistAnchor) -> Option<String> {
    let (n, m) = (signed.list.generation, anchor.generation);
    if n <= m {
        return None;
    }
    let key = VerifyingKey::from_bytes(&anchor.public_key)
        .ok()
        .filter(|k| !k.is_weak())?;
    let restore = format!(
        "restore the current wallet.json (zwallet keeps no copy of it), never an older \
         {WHITELIST_FILE}"
    );
    let highest = format!("the highest generation you remember seeing {SEEN_SINCE}");
    Some(if !signed_by(signed, &key) {
        format!(
            "{WHITELIST_FILE} (gen {n}) is newer than the seal (gen {m}) but not signed by this \
             wallet's whitelist key: a wallet.json rolled back across a reset, a reset that never \
             finished sealing, or a foreign file. If gen {m} is {highest}, it is one of the last \
             two: put back a genuine copy of {WHITELIST_FILE} at gen {m}; if you remember a \
             higher one, wallet.json was rolled back across a reset: {restore}"
        )
    } else if n == m.saturating_add(1) {
        format!(
            "{WHITELIST_FILE} (gen {n}) is one edit newer than the seal (gen {m}), under this \
             wallet's whitelist key: an edit to gen {n} that never finished sealing, or a \
             rolled-back wallet.json. If gen {m} is {highest}, the edit to gen {n} never \
             reported: put back a genuine copy of {WHITELIST_FILE} at gen {m}; if you remember \
             gen {n} or higher, wallet.json was rolled back: {restore}"
        )
    } else {
        format!(
            "{WHITELIST_FILE} (gen {n}) is newer than the seal (gen {m}): wallet.json was rolled \
             back; restore the current wallet.json (zwallet keeps no copy of it), never an older \
             {WHITELIST_FILE}"
        )
    })
}

/// Which generations the human's memory counts, and where they saw them: generations restart at
/// 1 with a reset or first edit made while wallet.json sealed no whitelist, and plain `whitelist
/// list` prints the unverified generation, so it is no source.
pub const SEEN_SINCE: &str = "since the last start at gen 1 (an edit's report, or unlock, status \
                              or `zwallet whitelist list --verify`)";

/// What the human can do about a `whitelist.json` that does not verify and is not above the seal
/// ([`above_seal`] says what to do then): a genuine copy at the sealed generation, which a
/// rolled-back wallet.json would also accept, so only when that is the highest generation the
/// human remembers ([`SEEN_SINCE`]). zwallet keeps no counter outside the home to check it
/// against.
pub fn genuine_copy_remedy(sealed_generation: u64) -> String {
    let m = sealed_generation;
    format!(
        "put back a genuine copy of {WHITELIST_FILE} at gen {m} only if gen {m} is the highest \
         generation you remember seeing {SEEN_SINCE}: it verifies again without a reset, \
         because the seal pins its digest; if you remember a higher one, wallet.json was rolled \
         back: restore the current wallet.json instead (zwallet keeps no copy of it), never an \
         older {WHITELIST_FILE}"
    )
}

/// [`genuine_copy_remedy`] for `files` under `anchor`, or `None` when `whitelist.json` is above
/// the seal (its reason carries what to do).
pub fn unverified_remedy(files: &ListFiles, anchor: &WhitelistAnchor) -> Option<String> {
    (!files.current.above(anchor)).then(|| genuine_copy_remedy(anchor.generation))
}

/// Reads a list file through the wallet's safe reader (no symlink, no FIFO, a regular file of
/// this user's, at most 1 MiB) and parses it ([`SignedList::from_file_bytes`]). A missing file is
/// `Ok(None)`. Nothing is verified.
pub fn read_list(path: &Path) -> Result<Option<SignedList>> {
    ListFileRead::read(path).parsed()
}

/// Reads and verifies the list file at `path` against `anchor`.
fn read_verified(path: &Path, anchor: &WhitelistAnchor) -> Result<Whitelist> {
    ListFileRead::read(path).verified(anchor)
}

/// One list file read once through the safe reader, so that what is shown, compared and verified
/// are the same bytes.
#[derive(Debug, Clone)]
pub struct ListFileRead {
    path: PathBuf,
    read: FileRead,
}

#[derive(Debug, Clone)]
enum FileRead {
    Absent,
    /// The safe reader refused it or the read failed (a symlink, a FIFO, another user's file,
    /// more than 1 MiB, an I/O error).
    Unreadable(WalletError),
    Bytes(Vec<u8>),
}

impl ListFileRead {
    pub fn read(path: &Path) -> Self {
        let read = match store::read_wallet_bytes(path) {
            Ok(b) => FileRead::Bytes(b),
            Err(e) if e.code == ErrorCode::WalletNotFound => FileRead::Absent,
            Err(e) => FileRead::Unreadable(e),
        };
        Self {
            path: path.to_path_buf(),
            read,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn exists(&self) -> bool {
        !matches!(self.read, FileRead::Absent)
    }

    /// The file parsed, unverified ([`SignedList::from_file_bytes`]): `Ok(None)` if there is no
    /// file; `VALIDATION_ERROR` if it could not be read or is not a list.
    pub fn parsed(&self) -> Result<Option<SignedList>> {
        match &self.read {
            FileRead::Absent => Ok(None),
            FileRead::Unreadable(e) => Err(e.clone()),
            FileRead::Bytes(b) => SignedList::from_file_bytes(b).map(Some).map_err(|e| {
                WalletError::validation(format!("{}: {}", self.path.display(), e.message))
            }),
        }
    }

    /// The file's `entries` as found (raw JSON, unverified), or `null` if there is no file, it
    /// could not be read, or it is not JSON with entries.
    pub fn raw_entries(&self) -> Value {
        match &self.read {
            FileRead::Bytes(b) => serde_json::from_slice::<Value>(b)
                .ok()
                .and_then(|v| v.get("entries").cloned())
                .unwrap_or(Value::Null),
            _ => Value::Null,
        }
    }

    /// The list in this file if it verifies against `anchor`.
    fn verified(&self, anchor: &WhitelistAnchor) -> Result<Whitelist> {
        let name = self.name();
        let signed = self
            .parsed()?
            .ok_or_else(|| WalletError::validation(format!("there is no {name}")))?;
        verify(&signed, anchor).map_err(|e| {
            // whitelist.json is published only after the re-seal that names it: above the seal,
            // the seal is the first suspect.
            match above_seal(&signed, anchor).filter(|_| name == WHITELIST_FILE) {
                Some(why) => WalletError::validation(why),
                None => WalletError::validation(format!("{name}: {}", e.message)),
            }
        })?;
        Ok(signed.list)
    }

    /// Whether this file holds a list above the seal ([`above_seal`]).
    fn above(&self, anchor: &WhitelistAnchor) -> bool {
        matches!(self.parsed(), Ok(Some(s)) if above_seal(&s, anchor).is_some())
    }

    /// Whether this file and `other` were both read, with the very same bytes (a staged copy
    /// left after a change, say).
    pub fn same_bytes(&self, other: &ListFileRead) -> bool {
        matches!((self.bytes(), other.bytes()), (Some(a), Some(b)) if a == b)
    }

    /// The bytes read, if the file was there and readable.
    fn bytes(&self) -> Option<&[u8]> {
        match &self.read {
            FileRead::Bytes(b) => Some(b),
            _ => None,
        }
    }

    fn name(&self) -> String {
        self.path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    }

    fn seen(&self) -> Seen {
        match &self.read {
            FileRead::Absent => Seen::Absent,
            FileRead::Unreadable(_) => Seen::Unreadable,
            FileRead::Bytes(b) => Seen::Bytes(keccak256(b).0),
        }
    }
}

/// `whitelist.json` and `whitelist.next.json` of a home, each read once ([`ListFileRead`]).
#[derive(Debug, Clone)]
pub struct ListFiles {
    pub current: ListFileRead,
    pub next: ListFileRead,
}

impl ListFiles {
    pub fn read(home: &Path) -> Self {
        Self {
            current: ListFileRead::read(&home.join(WHITELIST_FILE)),
            next: ListFileRead::read(&home.join(NEXT_FILE)),
        }
    }

    /// What a human was shown of the two files, to bind an edit to it ([`edit_as_shown`]).
    pub fn fingerprint(&self) -> Fingerprint {
        Fingerprint {
            current: self.current.seen(),
            next: self.next.seen(),
        }
    }
}

/// The two list files as a human saw them: each absent, unreadable, or the keccak-256 of its
/// bytes. An edit made [`edit_as_shown`] refuses before writing anything when the files it reads
/// are not these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fingerprint {
    current: Seen,
    next: Seen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Seen {
    Absent,
    Unreadable,
    Bytes([u8; 32]),
}

/// `VALIDATION_ERROR` if the list files of `home` are not, right now, the ones `shown`: for a
/// caller that wants to refuse before doing anything an edit would undo (stopping daemons). The
/// edit checks again under `w.lock` ([`edit_as_shown`]).
pub fn check_unchanged(home: &Path, shown: &Fingerprint) -> Result<()> {
    shown.check(&ListFiles::read(home).fingerprint())
}

impl Fingerprint {
    /// `VALIDATION_ERROR` naming the first file that is not as shown.
    fn check(&self, now: &Fingerprint) -> Result<()> {
        let changed = if self.current != now.current {
            WHITELIST_FILE
        } else if self.next != now.next {
            NEXT_FILE
        } else {
            return Ok(());
        };
        Err(WalletError::validation(format!(
            "{changed} changed after it was shown; nothing in the whitelist changed. Run the \
             command again to see the list as it is now"
        )))
    }
}

/// The list a signer may trust: `whitelist.json` if it verifies against `anchor`, else
/// `whitelist.next.json` if that does (an edit that re-sealed wallet.json and died before it
/// published `whitelist.json`; the anchor's digest names exactly that file). Otherwise
/// `VALIDATION_ERROR` with the reason, including when there is no list.
pub fn load_verified(home: &Path, anchor: &WhitelistAnchor) -> Result<Whitelist> {
    match read_verified(&home.join(WHITELIST_FILE), anchor) {
        Ok(list) => Ok(list),
        Err(current) => read_verified(&home.join(NEXT_FILE), anchor).map_err(|_| current),
    }
}

/// Where an edit's base came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseSource {
    /// `whitelist.json` verified.
    Current,
    /// `whitelist.json` did not verify but `whitelist.next.json` did: a committed edit that never
    /// published `whitelist.json`.
    PromotedNext,
    /// Nothing verified (or there is no whitelist yet): an empty list with the default policy.
    Empty,
}

/// Why an edit could not start from the sealed list, with the unverified `whitelist.json`'s
/// entries for the human to look at (they are never signed).
#[derive(Debug, Clone, PartialEq)]
pub struct Discarded {
    pub reason: String,
    /// `whitelist.json`'s `entries` as found (raw JSON, unverified), or `null` if there is no
    /// such file or it could not be read.
    pub entries: Value,
}

/// What an edit starts from.
#[derive(Debug, Clone, PartialEq)]
pub struct Base {
    pub list: Whitelist,
    pub source: BaseSource,
    pub discarded: Option<Discarded>,
}

/// [`pick_base_from`] on the files of `home` as they are now.
pub fn pick_base(home: &Path, anchor: Option<&WhitelistAnchor>, rules: &ChainRules) -> Base {
    pick_base_from(&ListFiles::read(home), anchor, rules)
}

/// Step 3 of the edit protocol, from the bytes in `files`. The current list if it verifies
/// against `anchor`; else the staged `whitelist.next.json` if it does; else an empty list with
/// the default policy, with the reason and the unverified `whitelist.json`'s entries in
/// `discarded` for the human to look at. An unverified list is never the base, so it is never
/// signed; only a reset starts from the empty one when there is an anchor ([`base_for_edit`]).
/// With no anchor (no whitelist yet) the base is empty, and `discarded` is set only if a
/// `whitelist.json` is there anyway. The base keeps the generation it verified at (the sealed
/// one, or 0 without an anchor).
pub fn pick_base_from(
    files: &ListFiles,
    anchor: Option<&WhitelistAnchor>,
    rules: &ChainRules,
) -> Base {
    let why = match anchor {
        Some(a) => match files.current.verified(a) {
            Ok(list) => {
                return Base {
                    list,
                    source: BaseSource::Current,
                    discarded: None,
                };
            }
            Err(e) => match files.next.verified(a) {
                Ok(list) => {
                    return Base {
                        list,
                        source: BaseSource::PromotedNext,
                        discarded: None,
                    };
                }
                Err(_) => e.message,
            },
        },
        None => format!("{WHITELIST_FILE}: wallet.json seals no whitelist key yet"),
    };
    let mut list = Whitelist::empty(rules);
    list.generation = anchor.map_or(0, |a| a.generation);
    let discarded = (anchor.is_some() || files.current.exists()).then(|| Discarded {
        reason: why,
        entries: files.current.raw_entries(),
    });
    Base {
        list,
        source: BaseSource::Empty,
        discarded,
    }
}

/// What `whitelist reset` does, for a message that offers it: it is no free recovery.
pub const RESET_CAVEAT: &str = "`zwallet whitelist reset` starts over with a new key and an empty \
                                list: it sets every fee cap back to the built-in default and \
                                lists no payee and no contract, so set `zwallet whitelist \
                                fee-cap` again before `--unlock`";

/// The base an edit (not a reset) builds on: [`pick_base_from`], refusing (`VALIDATION_ERROR`,
/// nothing written) when that would mean building on anything but the list wallet.json seals:
/// - a sealed anchor and neither file verifying: rebuilding from empty would drop every entry and
///   loosen every stricter fee cap on the strength of whoever damaged the file;
/// - no sealed anchor but a list file present: a wallet.json restored from before the first edit
///   (the real list is still there, and a new key at generation 1 would drop it), or a planted
///   file (whose values a change would seem to tighten or loosen). A crash in the first edit
///   leaves no `whitelist.json` before the re-seal (publishing comes after it), but one between
///   staging and re-seal leaves `whitelist.next.json`: that is refused too, and `reset` (which
///   at that point loses nothing, the caps being the defaults) recovers it.
///
/// Starting over is `reset`, a choice the human makes by name.
pub fn base_for_edit(
    files: &ListFiles,
    anchor: Option<&WhitelistAnchor>,
    rules: &ChainRules,
) -> Result<Base> {
    if anchor.is_none()
        && let Some(note) = unsealed_files_note(files)
    {
        return Err(WalletError::validation(format!(
            "refusing to edit the whitelist: {note}; nothing in the whitelist changed. Check it \
             with `zwallet whitelist list --verify`; {UNSEALED_REMEDY} {RESET_CAVEAT}"
        )));
    }
    let base = pick_base_from(files, anchor, rules);
    if let (Some(a), BaseSource::Empty) = (anchor, base.source) {
        let reason = base.discarded.map(|d| d.reason).unwrap_or_default();
        // Above the seal, the reason says what to do; putting back the copy the seal names
        // could complete a rollback.
        let remedy = unverified_remedy(files, a)
            .map(|r| format!(" If whitelist.json was only damaged, replaced or deleted, {r}."))
            .unwrap_or_default();
        return Err(WalletError::validation(format!(
            "refusing to edit the whitelist: {reason}. An edit builds only on the list wallet.json \
             seals, so nothing in the whitelist changed. Check it with `zwallet whitelist list \
             --verify`.{remedy} {RESET_CAVEAT}"
        )));
    }
    Ok(base)
}

/// What the human can do when wallet.json seals no whitelist but a list file exists, after
/// [`unsealed_files_note`] (then [`RESET_CAVEAT`]).
pub const UNSEALED_REMEDY: &str = "if wallet.json was put back from an older copy, restore the \
                                   current one instead (zwallet keeps no copy of it).";

/// When wallet.json seals no whitelist but a list file exists, which files and what can cause it
/// (`None` otherwise, e.g. a fresh wallet): the refusal of every edit ([`base_for_edit`]), and
/// what `whitelist list --verify` explains, each followed by [`UNSEALED_REMEDY`]. A first edit
/// interrupted between staging and re-seal leaves only `whitelist.next.json`, at generation 1,
/// signed by the key in `whitelist.key`.
pub fn unsealed_files_note(files: &ListFiles) -> Option<String> {
    let found: Vec<String> = [&files.current, &files.next]
        .into_iter()
        .filter(|f| f.exists())
        .map(ListFileRead::name)
        .collect();
    if found.is_empty() {
        return None;
    }
    let interrupted = if files.current.exists() {
        ""
    } else {
        "a first edit interrupted before it was sealed, "
    };
    Some(format!(
        "wallet.json seals no whitelist, but {} {} ({interrupted}a wallet.json restored from \
         before the first edit, or a planted file)",
        found.join(" and "),
        if found.len() == 1 { "exists" } else { "exist" }
    ))
}

/// Points in an edit where a test can stop it as a crash would: after each write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditStage<'a> {
    /// The list files are read under `w.lock` and the base is picked; nothing is written yet (the
    /// path is the home).
    FilesRead(&'a Path),
    /// A staged list the seal names is in place as `whitelist.json` (see [`BaseSource::PromotedNext`]).
    Promoted(&'a Path),
    /// A new `whitelist.key` is in place (first edit, or [`reset`]).
    KeyWritten(&'a Path),
    /// The signed `whitelist.next.json` is in place.
    NextWritten(&'a Path),
    /// wallet.json is re-sealed with the new anchor.
    Resealed(&'a Path),
    /// The new list is published as `whitelist.json`, written from the signed bytes after the
    /// re-seal (the staged `whitelist.next.json` is removed right after, best-effort).
    Published(&'a Path),
}

/// What an edit did.
#[derive(Debug, Clone, PartialEq)]
pub struct Edited {
    /// The list now in `whitelist.json`, verified against the new seal.
    pub list: Whitelist,
    /// The sealed generation before the edit (`None`: there was no whitelist).
    pub previous_generation: Option<u64>,
    pub base: BaseSource,
    /// The unverified list a reset dropped (one that did not verify, or a `whitelist.json` found
    /// while wallet.json sealed no whitelist). An edit never has one: it refuses instead
    /// ([`base_for_edit`]).
    pub discarded: Option<Discarded>,
    /// A new `whitelist.key` was made (the first edit, or [`reset`]).
    pub key_created: bool,
    /// Something the edit could not tidy up after it landed (the change itself is done): the
    /// staged `whitelist.next.json` not removed, or its removal not synced.
    pub warning: Option<String>,
}

/// `whitelist.key` on disk.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyFile {
    version: u32,
    crypto: store::CryptoBox,
}

/// Decrypts `whitelist.key` with the password that opened wallet.json and checks it is the key
/// the seal names.
fn load_key(home: &Path, password: &str, anchor: &WhitelistAnchor) -> Result<WhitelistKey> {
    let reset_hint = RESET_CAVEAT;
    let path = home.join(KEY_FILE);
    let bytes = store::read_wallet_bytes(&path).map_err(|e| {
        WalletError::validation(format!(
            "cannot use {KEY_FILE}: {}; {reset_hint}",
            e.message
        ))
    })?;
    let file: KeyFile = serde_json::from_slice(&bytes)
        .ok()
        .filter(|f: &KeyFile| f.version == KEY_FILE_VERSION)
        .ok_or_else(|| {
            WalletError::validation(format!(
                "{KEY_FILE} is not a whitelist key file; {reset_hint}"
            ))
        })?;
    // wallet.json just opened with this password, so a key file that does not was sealed
    // under another one, or was damaged or replaced.
    let secret = store::open_whitelist_key(&file.crypto, password).map_err(|e| {
        WalletError::validation(format!(
            "{KEY_FILE} does not open with the wallet password ({}); {reset_hint}",
            e.message
        ))
    })?;
    let key = WhitelistKey::from_secret(&secret);
    if key.public_key() != anchor.public_key {
        return Err(WalletError::validation(format!(
            "{KEY_FILE} is not the key wallet.json seals for the whitelist; {reset_hint}"
        )));
    }
    Ok(key)
}

/// Runs one whitelist edit (`.prd/04-whitelist.md` §Edits, steps 2–4) with the typed
/// `password`: under `w.lock`, decrypt wallet.json, pick the base ([`pick_base`]), then decrypt
/// `whitelist.key` (the first edit makes the key), apply `mutate`, set generation + 1, validate,
/// sign, write `whitelist.next.json`, re-seal wallet.json with the new anchor, publish the signed
/// bytes as `whitelist.json` (then remove `next`, best-effort: [`Edited::warning`] if that
/// fails), and read it back verified. `mutate` runs before anything is written, so its error (or
/// a validation error) leaves every file as it was. Stopping daemons before, and unlocking after,
/// is the caller's. This binds nothing to what a human saw: it is for tests and seeding; the CLI
/// uses [`edit_as_shown`].
pub fn edit(
    home: &Path,
    password: &str,
    rules: &ChainRules,
    mutate: impl FnOnce(&mut Whitelist) -> Result<()>,
) -> Result<Edited> {
    edit_with_hook(home, password, rules, mutate, &|_| Ok(()))
}

/// [`edit`] with a hook after each write. A hook error stops the edit right there with every
/// file as it is, as a crash would; nothing is rolled back.
pub fn edit_with_hook(
    home: &Path,
    password: &str,
    rules: &ChainRules,
    mutate: impl FnOnce(&mut Whitelist) -> Result<()>,
    hook: &dyn Fn(EditStage<'_>) -> Result<()>,
) -> Result<Edited> {
    transact(home, password, rules, Some(Box::new(mutate)), None, hook)
}

/// [`edit`] bound to what the human was `shown` ([`ListFiles::fingerprint`] of the files the
/// summary was made from): the files are read once under `w.lock`, and if they are not those
/// bytes the edit refuses before writing anything. So the change lands on exactly the list
/// shown, or nothing changes.
pub fn edit_as_shown(
    home: &Path,
    password: &str,
    rules: &ChainRules,
    shown: &Fingerprint,
    mutate: impl FnOnce(&mut Whitelist) -> Result<()>,
) -> Result<Edited> {
    transact(
        home,
        password,
        rules,
        Some(Box::new(mutate)),
        Some(shown),
        &|_| Ok(()),
    )
}

/// `whitelist reset`: a new key and an empty list with the default policy (every fee cap back to
/// the built-in default, no payee and no contract), at the next generation, for when
/// `whitelist.key` is lost or damaged or the list cannot be recovered. The old list stops
/// verifying. This binds nothing to what a human saw (tests and seeding); the CLI uses
/// [`reset_as_shown`].
pub fn reset(home: &Path, password: &str, rules: &ChainRules) -> Result<Edited> {
    reset_with_hook(home, password, rules, &|_| Ok(()))
}

/// [`reset`] bound to what the human was `shown`, as [`edit_as_shown`].
pub fn reset_as_shown(
    home: &Path,
    password: &str,
    rules: &ChainRules,
    shown: &Fingerprint,
) -> Result<Edited> {
    transact(home, password, rules, None, Some(shown), &|_| Ok(()))
}

/// [`reset`] with the hook of [`edit_with_hook`].
pub fn reset_with_hook(
    home: &Path,
    password: &str,
    rules: &ChainRules,
    hook: &dyn Fn(EditStage<'_>) -> Result<()>,
) -> Result<Edited> {
    transact(home, password, rules, None, None, hook)
}

type Mutation<'a> = Box<dyn FnOnce(&mut Whitelist) -> Result<()> + 'a>;

/// One edit; `mutate: None` is a reset; `shown: Some` binds it to those files.
fn transact(
    home: &Path,
    password: &str,
    rules: &ChainRules,
    mutate: Option<Mutation<'_>>,
    shown: Option<&Fingerprint>,
    hook: &dyn Fn(EditStage<'_>) -> Result<()>,
) -> Result<Edited> {
    store::read_wallet_file(home)?;
    let lock = MetaLock::acquire(home)?;
    let opened = lock.open(password)?;
    let anchor = opened.unsealed().whitelist_anchor()?;
    let previous_generation = anchor.map(|a| a.generation);

    // Everything that can fail on its own happens before the first write. The list files are
    // read once, here: every decision below is made from these bytes.
    let files = ListFiles::read(home);
    if let Some(shown) = shown {
        shown.check(&files.fingerprint())?;
    }
    // The list first: what is wrong with it (a rollback, say) explains a key that does not match
    // the seal too, and says what to do better than the key alone would.
    let picked = match &mutate {
        Some(_) => base_for_edit(&files, anchor.as_ref(), rules)?,
        None => pick_base_from(&files, anchor.as_ref(), rules),
    };
    let (key, key_created) = match (&mutate, &anchor) {
        (Some(_), Some(a)) => (load_key(home, password, a)?, false),
        _ => (WhitelistKey::generate()?, true),
    };
    hook(EditStage::FilesRead(home))?;
    // The sealed list may still sit in `whitelist.next.json` (an edit that re-sealed and died
    // before it published `whitelist.json`, or an agent's copy). Staging this edit overwrites
    // that file, so its verified bytes are first written as `whitelist.json`, by reset too;
    // otherwise a crash after staging would leave no file the seal names, and the committed list
    // would be lost.
    let promote = picked.source == BaseSource::PromotedNext;
    let base = if mutate.is_some() {
        picked
    } else {
        let mut list = Whitelist::empty(rules);
        list.generation = previous_generation.unwrap_or(0);
        Base {
            list,
            source: BaseSource::Empty,
            // What the reset drops without it having verified, for the human to look at.
            discarded: picked.discarded,
        }
    };
    let mut list = base.list.clone();
    if let Some(mutate) = mutate {
        mutate(&mut list)?;
    }
    list.generation = previous_generation
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| WalletError::validation("whitelist generation overflow"))?;
    list.validate(rules)?;
    let signed = key.sign(&list);
    let new_anchor = key.anchor_for(&signed);
    let policy = new_anchor.to_policy(&opened.unsealed().policy)?;

    if promote {
        // From memory: the bytes that verified, never what the path holds by now.
        let verified = files
            .next
            .bytes()
            .ok_or_else(|| WalletError::validation("the staged whitelist was not read"))?;
        lock.replace_file(WHITELIST_FILE, verified)?;
        hook(EditStage::Promoted(&home.join(WHITELIST_FILE)))?;
    }
    if key_created {
        // The KDF parameters that opened wallet.json, never a re-read.
        let kdf = opened.file().crypto.kdf_params()?;
        let file = KeyFile {
            version: KEY_FILE_VERSION,
            crypto: store::seal_whitelist_key(&Zeroizing::new(key.0.to_bytes()), password, kdf)?,
        };
        let mut bytes = serde_json::to_vec_pretty(&file)
            .map_err(|_| WalletError::validation("cannot serialize whitelist.key"))?;
        bytes.push(b'\n');
        lock.replace_file(KEY_FILE, &bytes)?;
        hook(EditStage::KeyWritten(&home.join(KEY_FILE)))?;
    }
    let staged = signed.to_file_bytes()?;
    lock.replace_file(NEXT_FILE, &staged)?;
    hook(EditStage::NextWritten(&home.join(NEXT_FILE)))?;
    lock.reseal(&opened, &policy)?;
    hook(EditStage::Resealed(&home.join(store::WALLET_FILE)))?;
    // Published from memory, as a promotion is: never whatever the staged path holds by now.
    // The change has landed here. The staged copy is then removed, best-effort: left in place
    // by a crash it is the same verified bytes, which the next staging replaces; a removal that
    // fails is a warning (something else may sit at the path, and a later staging replaces only
    // a plain file).
    lock.replace_file(WHITELIST_FILE, &staged)?;
    hook(EditStage::Published(&home.join(WHITELIST_FILE)))?;
    let warning = lock.remove_file(NEXT_FILE).err().map(|e| {
        format!(
            "the change landed, but zwallet could not remove {NEXT_FILE}, or could not sync its \
             directory after removing it ({}). If {NEXT_FILE} is still there it is not needed \
             ({WHITELIST_FILE} holds the list just signed): remove it; a later change can stage \
             its list only where there is a plain file or nothing",
            e.message
        )
    });

    let list = read_verified(&home.join(WHITELIST_FILE), &new_anchor).map_err(|e| {
        WalletError::validation(format!(
            "the whitelist was written but does not read back verified: {}",
            e.message
        ))
    })?;
    Ok(Edited {
        list,
        previous_generation,
        base: base.source,
        discarded: base.discarded,
        key_created,
        warning,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::seed::Seed;
    use crate::core::store::{KDF_MIN_LOG_N, KdfParams, MasterOptions};
    use std::ffi::OsString;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    const PW: &str = "correct horse battery";
    const TOKEN: &str = "0x00000000000000000000000000000000000000aa";
    const ALICE: &str = "0x1111111111111111111111111111111111111111";
    const BOB: &str = "0x3333333333333333333333333333333333333333";
    const CAROL: &str = "0x4444444444444444444444444444444444444444";
    const ROUTER: &str = "0x2222222222222222222222222222222222222222";
    const DAN: &str = "0x5555555555555555555555555555555555555555";

    fn known(key: &str) -> bool {
        ["base", "ethereum", "polygon", "sepolia"].contains(&key)
    }

    fn is_token(address: &str) -> bool {
        address == TOKEN
    }

    /// Stand-in chain facts so these tests do not depend on the chains layer.
    const RULES: ChainRules = ChainRules {
        chain_known: known,
        is_registry_token: is_token,
        default_fee_caps_wei: &[
            ("base", "500000000000000"),
            ("ethereum", "3000000000000000"),
        ],
    };

    fn fake_derive(_: &Seed, index: u32) -> Result<BTreeMap<String, String>> {
        Ok(BTreeMap::from([(
            "alpha".to_string(),
            format!("addr-{index}"),
        )]))
    }

    /// A wallet with the cheapest accepted KDF, and no whitelist yet.
    fn wallet() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let opts = MasterOptions {
            kdf: KdfParams {
                log_n: KDF_MIN_LOG_N,
                ..KdfParams::default()
            },
            ..MasterOptions::default()
        };
        store::create_master(tmp.path(), PW, "a", opts, &fake_derive).unwrap();
        tmp
    }

    /// The anchor wallet.json seals right now.
    fn sealed(home: &Path) -> Option<WhitelistAnchor> {
        let file = store::read_wallet_file(home).unwrap();
        store::decrypt(&file.crypto, PW)
            .unwrap()
            .whitelist_anchor()
            .unwrap()
    }

    fn add(entry: Entry) -> impl FnOnce(&mut Whitelist) -> Result<()> {
        move |l: &mut Whitelist| l.add(entry)
    }

    fn alice() -> Entry {
        Entry::new_address("alice", ALICE, &["base"])
    }

    fn nicknames(list: &Whitelist) -> Vec<&str> {
        list.entries.iter().map(|e| e.nickname.as_str()).collect()
    }

    /// Every file in the home but the writer lock, with its bytes.
    fn snapshot(home: &Path) -> BTreeMap<OsString, Vec<u8>> {
        fs::read_dir(home)
            .unwrap()
            .map(|e| e.unwrap())
            .filter(|e| e.file_name() != store::WRITE_LOCK)
            .map(|e| (e.file_name(), fs::read(e.path()).unwrap()))
            .collect()
    }

    fn golden_list() -> Whitelist {
        Whitelist {
            generation: 7,
            entries: vec![
                Entry::new_address("alice", ALICE, &["base", "ethereum"]),
                Entry::new_contract("router", ROUTER, &["base"], &["0x12345678", "0xABCDEF01"]),
                Entry::new_address("bob", BOB, &[ANY_CHAIN]),
            ],
            policy: Policy {
                max_fee_wei: BTreeMap::from([
                    ("polygon".to_string(), "2000000000000000000".to_string()),
                    ("base".to_string(), "500000000000000".to_string()),
                ]),
                allow_message_signing: true,
            },
        }
    }

    #[test]
    fn nickname_rules() {
        for ok in ["a", "0", "alice", "pay-1", "x_y", &"a".repeat(32)] {
            validate_nickname(ok).unwrap();
        }
        for bad in [
            "",
            "-a",
            "_a",
            "Alice",
            "a b",
            "a.b",
            "@a",
            "ä",
            &"a".repeat(33),
        ] {
            assert!(validate_nickname(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn canonical_bytes_and_signature_are_pinned() {
        let list = golden_list();
        let expected = concat!(
            r#"{"entries":["#,
            r#"{"address":"0x1111111111111111111111111111111111111111","chains":["base","ethereum"],"kind":"address","nickname":"alice"},"#,
            r#"{"address":"0x2222222222222222222222222222222222222222","chains":["base"],"kind":"contract","nickname":"router","selectors":["0x12345678","0xabcdef01"]},"#,
            r#"{"address":"0x3333333333333333333333333333333333333333","chains":["*"],"kind":"address","nickname":"bob"}"#,
            r#"],"gen":7,"policy":{"allowMessageSigning":true,"maxFeeWei":{"base":"500000000000000","polygon":"2000000000000000000"}},"version":1}"#,
        );
        assert_eq!(String::from_utf8(canonical_bytes(&list)).unwrap(), expected);
        list.validate(&RULES).unwrap();
        // ed25519 is deterministic: a fixed key gives a fixed signature over the domain-prefixed
        // bytes, and the anchor pins keccak-256 of those same bytes.
        let key = WhitelistKey::from_secret(&[1u8; 32]);
        let signed = key.sign(&list);
        assert_eq!(hex::encode(key.public_key()), GOLDEN_PUB);
        assert_eq!(hex::encode(signed.sig), GOLDEN_SIG);
        assert_eq!(hex::encode(digest(&list)), GOLDEN_DIGEST);
    }

    // Computed independently of this code (Python `cryptography` Ed25519 and OpenSSL
    // keccak-256 over the same domain-prefixed canonical bytes), not copied from its output.
    const GOLDEN_PUB: &str = "8a88e3dd7409f195fd52db2d3cba5d72ca6709bf1d94121bf3748801b40f6f5c";
    const GOLDEN_SIG: &str = concat!(
        "777721a9888559de6415b06b4de03a193952543d44535bda86826d954d790c74",
        "a4891723616b0ab24c04313d272cdcb5c728e309d61f27aba33f6002518a4205"
    );
    const GOLDEN_DIGEST: &str = "feba551260a9bcae17b4c138b4e24d4ecc65a6fbe1b981f1cd88f7b8ce8847af";

    #[test]
    fn file_layout_is_free_but_content_is_signed() {
        let key = WhitelistKey::from_secret(&[1u8; 32]);
        let signed = key.sign(&golden_list());
        let anchor = key.anchor_for(&signed);
        let bytes = signed.to_file_bytes().unwrap();
        let parsed = SignedList::from_file_bytes(&bytes).unwrap();
        assert_eq!(parsed, signed);
        verify(&parsed, &anchor).unwrap();
        // Another key order and no whitespace: the same list, the same signature.
        let mut v: Value = serde_json::from_slice(&bytes).unwrap();
        let reordered = serde_json::json!({
            "sig": v["sig"].take(), "policy": v["policy"].take(),
            "entries": v["entries"].take(), "gen": 7, "version": 1,
        });
        let compact = SignedList::from_file_bytes(reordered.to_string().as_bytes()).unwrap();
        verify(&compact, &anchor).unwrap();
        // Unknown or repeated keys, another version, a non-canonical signature: not a list.
        let text = String::from_utf8(bytes).unwrap();
        let sig = hex::encode(signed.sig);
        for bad in [
            text.replacen("\"version\": 1", "\"version\": 1, \"extra\": 0", 1),
            text.replacen("\"version\": 1", "\"version\": 1, \"gen\": 8", 1),
            text.replacen("\"version\": 1", "\"version\": 2", 1),
            text.replacen(
                "\"nickname\": \"bob\"",
                "\"nickname\": \"bob\", \"note\": \"x\"",
                1,
            ),
            text.replacen(
                "\"allowMessageSigning\": true",
                "\"allowMessageSigning\": 1",
                1,
            ),
            text.replacen(&sig, &sig.to_ascii_uppercase(), 1),
            text.replacen(&sig, &sig[..126], 1),
        ] {
            assert_ne!(bad, text);
            let err = SignedList::from_file_bytes(bad.as_bytes()).unwrap_err();
            assert_eq!(err.code, ErrorCode::ValidationError, "{bad}");
        }
    }

    #[test]
    fn malformed_lists_are_refused_before_signing() {
        let good = || {
            let mut l = Whitelist::empty(&RULES);
            l.generation = 1;
            l.entries = vec![
                alice(),
                Entry::new_contract("router", ROUTER, &["base"], &["0x12345678"]),
            ];
            l
        };
        good().validate(&RULES).unwrap();
        type Edit = fn(&mut Whitelist);
        let cases: Vec<(&str, Edit)> = vec![
            ("generation 0", |l| l.generation = 0),
            ("bad nickname", |l| l.entries[0].nickname = "Alice".into()),
            ("nickname twice", |l| l.entries[1].nickname = "alice".into()),
            ("address twice", |l| l.entries[1].address = ALICE.into()),
            ("upper-case address", |l| {
                l.entries[0].address = ALICE.replace('1', "A").replacen("0A", "0x", 1)
            }),
            ("short address", |l| l.entries[0].address = "0x1111".into()),
            ("no 0x", |l| {
                l.entries[0].address = ALICE[2..].to_string() + "11"
            }),
            ("no chains", |l| l.entries[0].chains.clear()),
            ("chain twice", |l| {
                l.entries[0].chains = vec!["base".into(), "base".into()]
            }),
            ("unknown chain", |l| {
                l.entries[0].chains = vec!["solana".into()]
            }),
            ("upper-case chain", |l| {
                l.entries[0].chains = vec!["Base".into()]
            }),
            ("* with others", |l| {
                l.entries[0].chains = vec![ANY_CHAIN.into(), "base".into()]
            }),
            ("contract on *", |l| {
                l.entries[1].chains = vec![ANY_CHAIN.into()]
            }),
            ("address with selectors", |l| {
                l.entries[0].selectors = Some(vec![])
            }),
            ("contract without selectors", |l| {
                l.entries[1].selectors = None
            }),
            ("contract with empty selectors", |l| {
                l.entries[1].selectors = Some(vec![])
            }),
            ("wildcard selector", |l| {
                l.entries[1].selectors = Some(vec!["0x*".into()])
            }),
            ("upper-case selector", |l| {
                l.entries[1].selectors = Some(vec!["0xABCDEF01".into()])
            }),
            ("selector twice", |l| {
                l.entries[1].selectors = Some(vec!["0x12345678".into(), "0x12345678".into()])
            }),
            ("contract at a registry token", |l| {
                l.entries[1].address = TOKEN.into()
            }),
            ("fee cap leading zero", |l| {
                l.policy.max_fee_wei.insert("base".into(), "0500".into());
            }),
            ("fee cap not decimal", |l| {
                l.policy.max_fee_wei.insert("base".into(), "5e14".into());
            }),
            ("fee cap past 2^256", |l| {
                l.policy.max_fee_wei.insert("base".into(), "9".repeat(80));
            }),
            ("fee cap on unknown chain", |l| {
                l.policy.max_fee_wei.insert("solana".into(), "1".into());
            }),
        ];
        for (what, edit) in cases {
            let mut l = good();
            edit(&mut l);
            let err = l.validate(&RULES).unwrap_err();
            assert_eq!(err.code, ErrorCode::ValidationError, "{what}");
        }
        // A registry token may still be a payee (the daemon refuses token transfers to it).
        let mut l = good();
        l.entries[0].address = TOKEN.into();
        l.validate(&RULES).unwrap();
        // A fee cap of 0 is well-formed (it refuses every send).
        l.policy.max_fee_wei.insert("base".into(), "0".into());
        l.validate(&RULES).unwrap();
    }

    #[test]
    fn verify_accepts_only_the_list_the_seal_names() {
        let key = WhitelistKey::from_secret(&[1u8; 32]);
        let signed = key.sign(&golden_list());
        let anchor = key.anchor_for(&signed);
        verify(&signed, &anchor).unwrap();
        let reason = |s: &SignedList, a: &WhitelistAnchor| verify(s, a).unwrap_err().message;

        // Another generation: what an older copy looks like.
        let older = key.sign(&Whitelist {
            generation: 6,
            ..golden_list()
        });
        assert!(
            reason(&older, &anchor).contains("generation 6"),
            "{}",
            reason(&older, &anchor)
        );
        // An edited entry, the signature kept.
        let mut edited = signed.clone();
        edited.list.entries[2].address = CAROL.into();
        assert!(reason(&edited, &anchor).contains("does not verify"));
        // Signed by another key.
        let other = WhitelistKey::from_secret(&[2u8; 32]).sign(&golden_list());
        assert!(reason(&other, &anchor).contains("does not verify"));
        // A damaged signature.
        let mut bad_sig = signed.clone();
        bad_sig.sig[63] ^= 0x10;
        assert!(reason(&bad_sig, &anchor).contains("does not verify"));
        // Validly signed at the sealed generation, but not the list sealed there.
        let mut abandoned = golden_list();
        abandoned.entries.pop();
        let abandoned = key.sign(&abandoned);
        assert!(reason(&abandoned, &anchor).contains("abandoned"));
        // A weak (small-order) sealed key verifies nothing.
        let mut identity = [0u8; 32];
        identity[0] = 1;
        let weak = WhitelistAnchor {
            public_key: identity,
            ..anchor
        };
        assert!(reason(&signed, &weak).contains("not a valid ed25519 key"));
        // A malformed list never verifies, even if signed.
        let mut malformed = golden_list();
        malformed.entries[0].nickname = "Alice".into();
        let malformed = key.sign(&malformed);
        let a = key.anchor_for(&malformed);
        assert!(reason(&malformed, &a).contains("nickname"));
    }

    #[test]
    fn list_files_are_read_safely() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(WHITELIST_FILE);
        assert_eq!(read_list(&path).unwrap(), None, "missing is no list");
        let signed = WhitelistKey::from_secret(&[1u8; 32]).sign(&golden_list());
        let real = tmp.path().join("real.json");
        fs::write(&real, signed.to_file_bytes().unwrap()).unwrap();
        std::os::unix::fs::symlink(&real, &path).unwrap();
        let err = read_list(&path).unwrap_err();
        assert!(err.message.contains("symbolic link"), "{}", err.message);
        fs::remove_file(&path).unwrap();
        let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: mkfifo with a valid NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        let err = read_list(&path).unwrap_err();
        assert!(
            err.message.contains("not a regular file"),
            "{}",
            err.message
        );
        fs::remove_file(&path).unwrap();
        fs::write(&path, b"{").unwrap();
        assert_eq!(
            read_list(&path).unwrap_err().code,
            ErrorCode::ValidationError
        );
        fs::rename(&real, &path).unwrap();
        assert_eq!(read_list(&path).unwrap(), Some(signed));
    }

    #[test]
    fn first_edit_makes_the_key_and_seals_generation_1() {
        // W1 at the core level.
        let tmp = wallet();
        let home = tmp.path();
        assert_eq!(sealed(home), None);
        let edited = edit(home, PW, &RULES, add(alice())).unwrap();
        assert!(edited.key_created);
        assert_eq!(edited.previous_generation, None);
        assert_eq!(edited.base, BaseSource::Empty);
        assert_eq!(edited.discarded, None);
        assert_eq!(edited.list.generation, 1);
        assert_eq!(nicknames(&edited.list), ["alice"]);
        assert_eq!(edited.list.policy, Policy::defaults(&RULES));

        let anchor = sealed(home).unwrap();
        assert_eq!(anchor.generation, 1);
        assert_eq!(anchor.digest, digest(&edited.list));
        assert_eq!(load_verified(home, &anchor).unwrap(), edited.list);
        let names: Vec<_> = snapshot(home).into_keys().collect();
        assert_eq!(names, ["wallet.json", "whitelist.json", "whitelist.key"]);
        for f in [WHITELIST_FILE, KEY_FILE, store::WALLET_FILE] {
            let mode = fs::metadata(home.join(f)).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{f}");
        }
        // The key file opens with the wallet password, is the sealed key, and is no wallet.json.
        let key = load_key(home, PW, &anchor).unwrap();
        assert_eq!(key.public_key(), anchor.public_key);
        let text = fs::read_to_string(home.join(KEY_FILE)).unwrap();
        assert!(
            !text.contains(&hex::encode(key.0.to_bytes())),
            "secret is sealed"
        );
        let file: KeyFile = serde_json::from_str(&text).unwrap();
        assert_eq!(
            file.crypto.kdfparams.log_n, KDF_MIN_LOG_N,
            "the wallet's KDF"
        );
        assert_eq!(
            store::decrypt(&file.crypto, PW).err().unwrap().code,
            ErrorCode::WrongPassword
        );
        // Readable while locked, unverified: the plain file parses on its own.
        let plain = read_list(&home.join(WHITELIST_FILE)).unwrap().unwrap();
        assert_eq!(plain.list, edited.list);
    }

    #[test]
    fn each_edit_advances_one_generation_with_the_same_key() {
        let tmp = wallet();
        let home = tmp.path();
        edit(home, PW, &RULES, add(alice())).unwrap();
        let first = sealed(home).unwrap();
        let edited = edit(home, PW, &RULES, |l| {
            l.add(Entry::new_contract(
                "router",
                ROUTER,
                &["base"],
                &["0x12345678"],
            ))?;
            l.policy.allow_message_signing = true;
            l.policy.set_max_fee_wei("base", U256::from(7u64));
            Ok(())
        })
        .unwrap();
        assert!(!edited.key_created);
        assert_eq!(edited.previous_generation, Some(1));
        assert_eq!(edited.base, BaseSource::Current);
        assert_eq!(edited.list.generation, 2);
        let second = sealed(home).unwrap();
        assert_eq!(second.public_key, first.public_key);
        assert_eq!(second.generation, 2);
        let list = load_verified(home, &second).unwrap();
        assert_eq!(nicknames(&list), ["alice", "router"]);
        assert!(list.policy.allow_message_signing);
        assert_eq!(list.policy.max_fee_wei("base"), Some(U256::from(7u64)));
        let edited = edit(home, PW, &RULES, |l| {
            l.remove("alice").unwrap();
            Ok(())
        })
        .unwrap();
        assert_eq!(nicknames(&edited.list), ["router"]);
        assert_eq!(sealed(home).unwrap().generation, 3);
    }

    #[test]
    fn a_refused_edit_writes_nothing() {
        let tmp = wallet();
        let home = tmp.path();
        let before = snapshot(home);
        // First edit refused by validation: no key file either.
        let at_token = Entry::new_contract("t", TOKEN, &["base"], &["0x12345678"]);
        let err = edit(home, PW, &RULES, add(at_token)).unwrap_err();
        assert!(err.message.contains("registry token"), "{}", err.message);
        assert_eq!(snapshot(home), before);
        let err = edit(home, "not the password", &RULES, add(alice())).unwrap_err();
        assert_eq!(err.code, ErrorCode::WrongPassword);
        assert_eq!(snapshot(home), before);

        edit(home, PW, &RULES, add(alice())).unwrap();
        let before = snapshot(home);
        let err = edit(
            home,
            PW,
            &RULES,
            add(Entry::new_address("alice", BOB, &["base"])),
        );
        assert!(err.unwrap_err().message.contains("already has"));
        let err = edit(
            home,
            PW,
            &RULES,
            add(Entry::new_address("al2", ALICE, &["base"])),
        );
        assert!(
            err.unwrap_err()
                .message
                .contains("already in the whitelist")
        );
        let err = edit(home, PW, &RULES, |_| {
            Err(WalletError::validation("refused by the caller"))
        });
        assert_eq!(err.unwrap_err().message, "refused by the caller");
        assert_eq!(
            edit(home, "nope", &RULES, add(alice())).unwrap_err().code,
            ErrorCode::WrongPassword
        );
        assert_eq!(snapshot(home), before);
    }

    #[test]
    fn w4_a_hand_edited_list_is_invalid_and_never_signed() {
        let tmp = wallet();
        let home = tmp.path();
        edit(home, PW, &RULES, add(alice())).unwrap();
        let anchor = sealed(home).unwrap();
        // Add an address by hand, keeping the signature.
        let path = home.join(WHITELIST_FILE);
        let mut v: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        v["entries"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::to_value(Entry::new_address("mallory", CAROL, &["*"])).unwrap());
        fs::write(&path, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
        let err = load_verified(home, &anchor).unwrap_err();
        assert!(err.message.contains("does not verify"), "{}", err.message);

        // It is never a base: pick_base shows it, an edit refuses and writes nothing.
        let base = pick_base(home, Some(&anchor), &RULES);
        assert_eq!(base.source, BaseSource::Empty);
        assert_eq!(base.list.entries, vec![]);
        let shown = base.discarded.unwrap();
        assert!(shown.reason.contains("does not verify"), "{}", shown.reason);
        assert_eq!(shown.entries, v["entries"]);
        let before = snapshot(home);
        let err = edit(
            home,
            PW,
            &RULES,
            add(Entry::new_address("bob", BOB, &["base"])),
        )
        .unwrap_err();
        for want in [
            "does not verify",
            "zwallet whitelist list --verify",
            "put back a genuine copy of whitelist.json at gen 1 only if gen 1 is the highest \
             generation you remember seeing",
            "if you remember a higher one, wallet.json was rolled back: restore the current \
             wallet.json instead",
            "nothing in the whitelist changed",
            "zwallet whitelist reset",
            "sets every fee cap back to the built-in default and lists no payee and no contract",
        ] {
            assert!(err.message.contains(want), "{want}: {}", err.message);
        }
        assert_eq!(snapshot(home), before, "a refused edit wrote something");

        // A deleted list: the same refusal, saying why.
        fs::remove_file(home.join(WHITELIST_FILE)).unwrap();
        let shown = pick_base(home, sealed(home).as_ref(), &RULES)
            .discarded
            .unwrap();
        assert!(
            shown.reason.contains("there is no whitelist.json"),
            "{}",
            shown.reason
        );
        assert_eq!(shown.entries, Value::Null);
        let before = snapshot(home);
        let err = edit(
            home,
            PW,
            &RULES,
            add(Entry::new_address("bob", BOB, &["base"])),
        )
        .unwrap_err();
        assert!(
            err.message.contains("there is no whitelist.json"),
            "{}",
            err.message
        );
        assert_eq!(snapshot(home), before);

        // Only reset starts over; it reports what it dropped without it having verified.
        fs::write(home.join(WHITELIST_FILE), serde_json::to_vec(&v).unwrap()).unwrap();
        let done = reset(home, PW, &RULES).unwrap();
        assert_eq!(done.list.generation, 2);
        assert_eq!(done.list.entries, vec![]);
        let dropped = done.discarded.unwrap();
        assert!(
            dropped.reason.contains("does not verify"),
            "{}",
            dropped.reason
        );
        assert_eq!(dropped.entries, v["entries"]);
        let edited = edit(
            home,
            PW,
            &RULES,
            add(Entry::new_address("bob", BOB, &["base"])),
        )
        .unwrap();
        assert_eq!((edited.base, edited.discarded), (BaseSource::Current, None));
        assert_eq!(
            nicknames(&edited.list),
            ["bob"],
            "neither alice nor mallory was signed"
        );

        // A list planted before there is any whitelist: no edit builds on it or near it; reset
        // starts over and shows it as dropped.
        let tmp = wallet();
        let home = tmp.path();
        fs::write(home.join(WHITELIST_FILE), serde_json::to_vec(&v).unwrap()).unwrap();
        let before = snapshot(home);
        let err = edit(
            home,
            PW,
            &RULES,
            add(Entry::new_address("bob", BOB, &["base"])),
        )
        .unwrap_err();
        assert!(
            err.message
                .contains("wallet.json seals no whitelist, but whitelist.json exists"),
            "{}",
            err.message
        );
        assert_eq!(snapshot(home), before);
        let done = reset(home, PW, &RULES).unwrap();
        assert_eq!(nicknames(&done.list), Vec::<&str>::new());
        assert!(
            done.discarded
                .unwrap()
                .reason
                .contains("no whitelist key yet")
        );
    }

    #[test]
    fn an_edit_bound_to_what_was_shown_refuses_any_other_bytes() {
        let tmp = wallet();
        let home = tmp.path();
        // Before any whitelist: both files absent.
        let shown = ListFiles::read(home).fingerprint();
        edit_as_shown(home, PW, &RULES, &shown, add(alice())).unwrap();
        let path = home.join(WHITELIST_FILE);
        let genuine = fs::read(&path).unwrap();
        let shown = ListFiles::read(home).fingerprint();

        // The same signed content in other bytes, a staged file appearing, or the list going
        // missing: refused before anything is written, naming the file.
        let compact =
            serde_json::to_vec(&serde_json::from_slice::<Value>(&genuine).unwrap()).unwrap();
        let cases: [(&str, &dyn Fn()); 3] = [
            (WHITELIST_FILE, &|| fs::write(&path, &compact).unwrap()),
            (NEXT_FILE, &|| {
                fs::write(home.join(NEXT_FILE), &genuine).unwrap()
            }),
            (WHITELIST_FILE, &|| fs::remove_file(&path).unwrap()),
        ];
        for (file, change) in cases {
            change();
            let before = snapshot(home);
            for result in [
                edit_as_shown(
                    home,
                    PW,
                    &RULES,
                    &shown,
                    add(Entry::new_address("bob", BOB, &["base"])),
                ),
                reset_as_shown(home, PW, &RULES, &shown),
            ] {
                let err = result.unwrap_err();
                assert_eq!(
                    err.message,
                    format!(
                        "{file} changed after it was shown; nothing in the whitelist changed. \
                         Run the command again to see the list as it is now"
                    )
                );
            }
            assert_eq!(snapshot(home), before, "{file}: something was written");
            let _ = fs::remove_file(home.join(NEXT_FILE));
            fs::write(&path, &genuine).unwrap();
        }
        // As shown: the edit goes through, and the reset after it.
        let edited = edit_as_shown(
            home,
            PW,
            &RULES,
            &shown,
            add(Entry::new_address("bob", BOB, &["base"])),
        )
        .unwrap();
        assert_eq!(nicknames(&edited.list), ["alice", "bob"]);
        let shown = ListFiles::read(home).fingerprint();
        assert_eq!(
            reset_as_shown(home, PW, &RULES, &shown)
                .unwrap()
                .list
                .generation,
            3
        );
    }

    #[test]
    fn w5_an_older_signed_copy_is_invalid() {
        let tmp = wallet();
        let home = tmp.path();
        edit(home, PW, &RULES, add(alice())).unwrap();
        let gen1 = fs::read(home.join(WHITELIST_FILE)).unwrap();
        edit(home, PW, &RULES, |l| {
            l.remove("alice");
            Ok(())
        })
        .unwrap();
        let anchor = sealed(home).unwrap();
        assert_eq!(anchor.generation, 2);
        fs::write(home.join(WHITELIST_FILE), &gen1).unwrap();
        let err = load_verified(home, &anchor).unwrap_err();
        assert!(
            err.message
                .contains("generation 1 is not the sealed generation 2"),
            "{}",
            err.message
        );
        // The same copy as a staged edit is no better.
        fs::write(home.join(NEXT_FILE), &gen1).unwrap();
        assert!(load_verified(home, &anchor).is_err());
        assert_eq!(
            pick_base(home, Some(&anchor), &RULES).source,
            BaseSource::Empty
        );
    }

    /// W7: an edit adding bob to [alice] at generation 1, stopped by a crash after `stage`.
    fn crash_after(stage: fn(&EditStage) -> bool) -> (tempfile::TempDir, Whitelist, Whitelist) {
        let tmp = wallet();
        let home = tmp.path();
        let old = edit(home, PW, &RULES, add(alice())).unwrap().list;
        let mut new = old.clone();
        new.add(Entry::new_address("bob", BOB, &["base"])).unwrap();
        new.generation = 2;
        let hook = |s: EditStage| {
            if stage(&s) {
                Err(WalletError::validation("crash"))
            } else {
                Ok(())
            }
        };
        let err = edit_with_hook(
            home,
            PW,
            &RULES,
            add(Entry::new_address("bob", BOB, &["base"])),
            &hook,
        )
        .unwrap_err();
        assert_eq!(err.message, "crash");
        (tmp, old, new)
    }

    /// Every list file that verifies against the seal is `old` or `new`; the signer's view and
    /// the next edit's base are the same one.
    fn assert_never_invalid_and_signed(home: &Path, old: &Whitelist, new: &Whitelist) -> Whitelist {
        let anchor = sealed(home).unwrap();
        for f in [WHITELIST_FILE, NEXT_FILE] {
            if let Some(s) = read_list(&home.join(f)).unwrap()
                && verify(&s, &anchor).is_ok()
            {
                assert!(
                    s.list == *old || s.list == *new,
                    "{f} verifies but is neither"
                );
            }
        }
        let trusted = load_verified(home, &anchor).unwrap();
        assert!(trusted == *old || trusted == *new);
        assert_eq!(pick_base(home, Some(&anchor), &RULES).list, trusted);
        trusted
    }

    #[test]
    fn w7_a_crash_after_any_write_leaves_the_old_or_the_promotable_new_generation() {
        // After the staged write: the old generation stands.
        let (tmp, old, new) = crash_after(|s| matches!(s, EditStage::NextWritten(_)));
        let home = tmp.path();
        assert_eq!(assert_never_invalid_and_signed(home, &old, &new), old);
        assert_eq!(
            pick_base(home, sealed(home).as_ref(), &RULES).source,
            BaseSource::Current
        );
        // The abandoned staged list is signed at generation 2; the retry signs another list at
        // generation 2. The abandoned copy must not verify afterwards.
        let abandoned = fs::read(home.join(NEXT_FILE)).unwrap();
        let retry = edit(
            home,
            PW,
            &RULES,
            add(Entry::new_address("carol", CAROL, &["base"])),
        )
        .unwrap();
        assert_eq!(retry.base, BaseSource::Current);
        assert_eq!(retry.list.generation, 2);
        assert_eq!(nicknames(&retry.list), ["alice", "carol"]);
        fs::write(home.join(WHITELIST_FILE), &abandoned).unwrap();
        let err = load_verified(home, &sealed(home).unwrap()).unwrap_err();
        assert!(err.message.contains("abandoned"), "{}", err.message);

        // After the re-seal: the staged list is the sealed one; the signer trusts it and the next
        // edit promotes it.
        let (tmp, old, new) = crash_after(|s| matches!(s, EditStage::Resealed(_)));
        let home = tmp.path();
        assert_eq!(assert_never_invalid_and_signed(home, &old, &new), new);
        assert_eq!(
            pick_base(home, sealed(home).as_ref(), &RULES).source,
            BaseSource::PromotedNext
        );
        let retry = edit(
            home,
            PW,
            &RULES,
            add(Entry::new_address("carol", CAROL, &["base"])),
        )
        .unwrap();
        assert_eq!(retry.base, BaseSource::PromotedNext);
        assert_eq!(retry.list.generation, 3);
        assert_eq!(nicknames(&retry.list), ["alice", "bob", "carol"]);
        assert!(!home.join(NEXT_FILE).exists());

        // After the publish: done, but for the read-back.
        let (tmp, old, new) = crash_after(|s| matches!(s, EditStage::Published(_)));
        let home = tmp.path();
        assert_eq!(assert_never_invalid_and_signed(home, &old, &new), new);
        assert_eq!(
            pick_base(home, sealed(home).as_ref(), &RULES).source,
            BaseSource::Current
        );
        let retry = edit(
            home,
            PW,
            &RULES,
            add(Entry::new_address("carol", CAROL, &["base"])),
        )
        .unwrap();
        assert_eq!(retry.list.generation, 3);
    }

    fn crash_at_next_written(stage: EditStage) -> Result<()> {
        match stage {
            EditStage::NextWritten(_) => Err(WalletError::validation("crash")),
            _ => Ok(()),
        }
    }

    /// The "re-sealed, not published" state left by a crash: the seal names [alice, bob] at
    /// generation 2, held in `whitelist.next.json`; `whitelist.json` is still [alice] at 1.
    fn promoted_by_a_crash() -> (tempfile::TempDir, Whitelist) {
        let (tmp, _, committed) = crash_after(|s| matches!(s, EditStage::Resealed(_)));
        (tmp, committed)
    }

    /// The same state made by hand, no crash needed: an agent copies the current list to
    /// `whitelist.next.json` and puts an older signed copy in its place.
    fn promoted_by_hand() -> (tempfile::TempDir, Whitelist) {
        let tmp = wallet();
        let home = tmp.path();
        edit(home, PW, &RULES, add(alice())).unwrap();
        let gen1 = fs::read(home.join(WHITELIST_FILE)).unwrap();
        let committed = edit(
            home,
            PW,
            &RULES,
            add(Entry::new_address("bob", BOB, &["base"])),
        )
        .unwrap()
        .list;
        fs::copy(home.join(WHITELIST_FILE), home.join(NEXT_FILE)).unwrap();
        fs::write(home.join(WHITELIST_FILE), &gen1).unwrap();
        (tmp, committed)
    }

    #[test]
    fn m1_a_second_crash_after_a_promotable_edit_keeps_the_sealed_list() {
        for (how, (tmp, committed)) in [
            ("crash", promoted_by_a_crash()),
            ("by hand", promoted_by_hand()),
        ] {
            let home = tmp.path();
            let anchor = sealed(home).unwrap();
            let base = pick_base(home, Some(&anchor), &RULES);
            assert_eq!(base.source, BaseSource::PromotedNext, "{how}");

            // The next edit dies after staging its own list.
            let carol = Entry::new_address("carol", CAROL, &["base"]);
            let err =
                edit_with_hook(home, PW, &RULES, add(carol), &crash_at_next_written).unwrap_err();
            assert_eq!(err.message, "crash");
            let anchor = sealed(home).unwrap();
            assert_eq!(anchor.generation, 2, "{how}: not re-sealed");
            let trusted = load_verified(home, &anchor)
                .unwrap_or_else(|e| panic!("{how}: the committed list was lost: {}", e.message));
            assert_eq!(trusted, committed, "{how}");
            let base = pick_base(home, Some(&anchor), &RULES);
            assert_eq!(
                (base.source, &base.list),
                (BaseSource::Current, &committed),
                "{how}"
            );

            let edited = edit(
                home,
                PW,
                &RULES,
                add(Entry::new_address("dan", DAN, &["base"])),
            );
            let edited = edited.unwrap();
            assert_eq!(edited.list.generation, 3, "{how}");
            assert_eq!(nicknames(&edited.list), ["alice", "bob", "dan"], "{how}");
        }
    }

    #[test]
    fn m1_a_reset_that_crashes_from_a_promotable_state_keeps_the_sealed_list() {
        for (how, (tmp, committed)) in [
            ("crash", promoted_by_a_crash()),
            ("by hand", promoted_by_hand()),
        ] {
            let home = tmp.path();
            let err = reset_with_hook(home, PW, &RULES, &crash_at_next_written).unwrap_err();
            assert_eq!(err.message, "crash");
            let anchor = sealed(home).unwrap();
            assert_eq!(anchor.generation, 2, "{how}: not re-sealed");
            let trusted = load_verified(home, &anchor)
                .unwrap_or_else(|e| panic!("{how}: the committed list was lost: {}", e.message));
            assert_eq!(trusted, committed, "{how}");
            // Run again, the reset completes.
            let done = reset(home, PW, &RULES).unwrap();
            assert_eq!(done.list.generation, 3, "{how}");
            load_verified(home, &sealed(home).unwrap()).unwrap();
        }
    }

    #[test]
    fn r3_1_a_promotion_writes_the_verified_staged_bytes_and_leaves_next_alone() {
        for (how, (tmp, committed)) in [
            ("crash", promoted_by_a_crash()),
            ("by hand", promoted_by_hand()),
        ] {
            let home = tmp.path();
            let staged = fs::read(home.join(NEXT_FILE)).unwrap();
            let stop = |s: EditStage| match s {
                EditStage::Promoted(_) => Err(WalletError::validation("crash")),
                _ => Ok(()),
            };
            let carol = Entry::new_address("carol", CAROL, &["base"]);
            let err = edit_with_hook(home, PW, &RULES, add(carol), &stop).unwrap_err();
            assert_eq!(err.message, "crash", "{how}");
            // whitelist.json is the very bytes that verified; next is where it was, untouched.
            assert_eq!(
                fs::read(home.join(WHITELIST_FILE)).unwrap(),
                staged,
                "{how}"
            );
            assert_eq!(
                fs::read(home.join(NEXT_FILE)).ok(),
                Some(staged.clone()),
                "{how}: next was moved"
            );
            let anchor = sealed(home).unwrap();
            assert_eq!(load_verified(home, &anchor).unwrap(), committed, "{how}");
            assert_eq!(
                pick_base(home, Some(&anchor), &RULES).source,
                BaseSource::Current
            );
        }
    }

    #[test]
    fn r3_1_an_edit_from_a_promotable_state_is_bound_to_what_was_shown() {
        let (tmp, committed) = promoted_by_hand();
        let home = tmp.path();
        let shown = ListFiles::read(home).fingerprint();
        // The staged file swapped after it was shown: refused by the fingerprint.
        let staged = fs::read(home.join(NEXT_FILE)).unwrap();
        let mut other: Value = serde_json::from_slice(&staged).unwrap();
        other["entries"][1]["chains"] = Value::from(vec!["*"]);
        fs::write(home.join(NEXT_FILE), serde_json::to_vec(&other).unwrap()).unwrap();
        let before = snapshot(home);
        let err = edit_as_shown(
            home,
            PW,
            &RULES,
            &shown,
            add(Entry::new_address("dan", DAN, &["base"])),
        )
        .unwrap_err();
        assert!(
            err.message
                .starts_with("whitelist.next.json changed after it was shown"),
            "{}",
            err.message
        );
        assert_eq!(snapshot(home), before, "the refused edit wrote something");
        // As shown: the change lands on the staged list, now whitelist.json.
        fs::write(home.join(NEXT_FILE), &staged).unwrap();
        let edited = edit_as_shown(
            home,
            PW,
            &RULES,
            &shown,
            add(Entry::new_address("dan", DAN, &["base"])),
        )
        .unwrap();
        assert_eq!(edited.base, BaseSource::PromotedNext);
        assert_eq!(edited.list.generation, committed.generation + 1);
        assert_eq!(nicknames(&edited.list), ["alice", "bob", "dan"]);
        assert_eq!(
            load_verified(home, &sealed(home).unwrap()).unwrap(),
            edited.list
        );
        assert_eq!(
            read_list(&home.join(WHITELIST_FILE)).unwrap().unwrap().list,
            edited.list
        );
        assert!(!home.join(NEXT_FILE).exists());
    }

    #[test]
    fn r3_2_a_list_file_without_a_sealed_whitelist_refuses_an_edit() {
        // A wallet.json from before the first edit, put back under a real list; or a planted
        // file on a fresh wallet; or only a staged one. An edit would make a new key at gen 1 and
        // drop the list, or present a loosening as a change of a planted value.
        let tmp = wallet();
        let home = tmp.path();
        let fresh = fs::read(home.join(store::WALLET_FILE)).unwrap();
        edit(home, PW, &RULES, |l| {
            l.add(alice())?;
            l.policy.set_max_fee_wei("base", U256::from(1u64));
            Ok(())
        })
        .unwrap();
        let list = fs::read(home.join(WHITELIST_FILE)).unwrap();
        fs::write(home.join(store::WALLET_FILE), &fresh).unwrap();
        assert_eq!(sealed(home), None);
        let as_is = || {};
        let staged_only = || fs::rename(home.join(WHITELIST_FILE), home.join(NEXT_FILE)).unwrap();
        let unreadable = || {
            let _ = fs::remove_file(home.join(NEXT_FILE));
            fs::write(home.join(WHITELIST_FILE), b"not a list").unwrap()
        };
        let cases: [(&str, &dyn Fn()); 3] = [
            (WHITELIST_FILE, &as_is),
            (NEXT_FILE, &staged_only),
            (WHITELIST_FILE, &unreadable),
        ];
        for (file, setup) in cases {
            setup();
            let before = snapshot(home);
            let err = edit(
                home,
                PW,
                &RULES,
                add(Entry::new_address("bob", BOB, &["base"])),
            )
            .unwrap_err();
            for want in [
                &format!("wallet.json seals no whitelist, but {file} exists") as &str,
                "a wallet.json restored from before the first edit, or a planted file",
                "nothing in the whitelist changed",
                "zwallet whitelist list --verify",
                "zwallet whitelist reset",
                "sets every fee cap back to the built-in default",
            ] {
                assert!(
                    err.message.contains(want),
                    "{file}: {want}: {}",
                    err.message
                );
            }
            assert_eq!(
                snapshot(home),
                before,
                "{file}: the refused edit wrote something"
            );
        }
        // Reset still works, and says what it dropped.
        fs::write(home.join(WHITELIST_FILE), &list).unwrap();
        let done = reset(home, PW, &RULES).unwrap();
        assert_eq!(done.list.generation, 1);
        assert_eq!(done.list.policy, Policy::defaults(&RULES));
        let dropped = done.discarded.unwrap();
        assert!(
            dropped.reason.contains("no whitelist key yet"),
            "{}",
            dropped.reason
        );
        assert!(!home.join(NEXT_FILE).exists());
        // A fresh wallet with no list file: the first edit works as before.
        let fresh = wallet();
        let first = edit(fresh.path(), PW, &RULES, add(alice())).unwrap();
        assert_eq!((first.list.generation, first.discarded), (1, None));
    }

    #[test]
    fn r4_2_a_list_newer_than_the_seal_names_the_rollback() {
        let tmp = wallet();
        let home = tmp.path();
        edit(home, PW, &RULES, add(alice())).unwrap();
        let gen1_wallet = fs::read(home.join(store::WALLET_FILE)).unwrap();
        edit(
            home,
            PW,
            &RULES,
            add(Entry::new_address("bob", BOB, &["base"])),
        )
        .unwrap();
        edit(home, PW, &RULES, |l| {
            l.remove("bob");
            Ok(())
        })
        .unwrap();
        // wallet.json alone put back to generation 1, under the current (gen 3) list: two edits
        // newer than the seal, under the sealed key, is a rollback and nothing else.
        fs::write(home.join(store::WALLET_FILE), &gen1_wallet).unwrap();
        let anchor = sealed(home).unwrap();
        let rolled = "whitelist.json (gen 3) is newer than the seal (gen 1): wallet.json was rolled \
                      back; restore the current wallet.json (zwallet keeps no copy of it), never \
                      an older whitelist.json";
        // The daemon's reason, list --verify's and the edit's all say it.
        assert_eq!(load_verified(home, &anchor).unwrap_err().message, rolled);
        let base = pick_base(home, Some(&anchor), &RULES);
        assert_eq!(base.discarded.unwrap().reason, rolled);
        let before = snapshot(home);
        let err = edit(
            home,
            PW,
            &RULES,
            add(Entry::new_address("carol", CAROL, &["base"])),
        )
        .unwrap_err();
        assert!(err.message.contains(rolled), "{}", err.message);
        assert!(!err.message.contains("genuine copy"), "{}", err.message);
        assert_eq!(snapshot(home), before);
        // An older copy, or another key's list, is still "an older or a foreign copy".
        let tmp = wallet();
        let home = tmp.path();
        edit(home, PW, &RULES, add(alice())).unwrap();
        let gen1 = fs::read(home.join(WHITELIST_FILE)).unwrap();
        edit(
            home,
            PW,
            &RULES,
            add(Entry::new_address("bob", BOB, &["base"])),
        )
        .unwrap();
        fs::write(home.join(WHITELIST_FILE), &gen1).unwrap();
        let err = load_verified(home, &sealed(home).unwrap()).unwrap_err();
        assert!(
            err.message.contains("an older or a foreign copy"),
            "{}",
            err.message
        );
        assert!(!err.message.contains("rolled back"), "{}", err.message);
    }

    #[test]
    fn r5_1_a_newer_list_under_another_key_offers_the_copy_only_behind_the_gate() {
        let tmp = wallet();
        let home = tmp.path();
        edit(home, PW, &RULES, add(alice())).unwrap();
        let gen1_wallet = fs::read(home.join(store::WALLET_FILE)).unwrap();
        // A reset (a new key, gen 2) and an edit under it (gen 3); then wallet.json alone is put
        // back to before the reset.
        reset(home, PW, &RULES).unwrap();
        edit(
            home,
            PW,
            &RULES,
            add(Entry::new_address("bob", BOB, &["base"])),
        )
        .unwrap();
        fs::write(home.join(store::WALLET_FILE), &gen1_wallet).unwrap();
        let anchor = sealed(home).unwrap();
        let why = "whitelist.json (gen 3) is newer than the seal (gen 1) but not signed by this \
                   wallet's whitelist key: a wallet.json rolled back across a reset, a reset that \
                   never finished sealing, or a foreign file";
        // The copy only behind the highest-generation gate: the reset reported gen 2.
        let gate = "If gen 1 is the highest generation you remember seeing since the last start \
                    at gen 1 (an edit's report, or unlock, status or `zwallet whitelist list \
                    --verify`), it is one of the last two: put back a genuine copy of \
                    whitelist.json at gen 1; if you remember a higher one, wallet.json was rolled \
                    back across a reset: restore the current wallet.json (zwallet keeps no copy \
                    of it), never an older whitelist.json";
        let reason = load_verified(home, &anchor).unwrap_err().message;
        assert!(reason.contains(why), "{reason}");
        assert!(reason.contains(gate), "{reason}");
        let before = snapshot(home);
        let err = edit(
            home,
            PW,
            &RULES,
            add(Entry::new_address("carol", CAROL, &["base"])),
        )
        .unwrap_err();
        assert!(err.message.contains(why), "{}", err.message);
        assert!(err.message.contains(gate), "{}", err.message);
        // No second copy remedy, unconditional or not.
        assert_eq!(
            err.message.matches("put back").count(),
            1,
            "{}",
            err.message
        );
        assert_eq!(snapshot(home), before);
        // One generation above the seal under another key reads the same (no "one edit").
        reset(home, PW, &RULES).unwrap();
        let gen2_wallet = fs::read(home.join(store::WALLET_FILE)).unwrap();
        reset(home, PW, &RULES).unwrap();
        fs::write(home.join(store::WALLET_FILE), &gen2_wallet).unwrap();
        let reason = load_verified(home, &sealed(home).unwrap())
            .unwrap_err()
            .message;
        assert!(
            reason.contains("but not signed by this wallet's whitelist key"),
            "{reason}"
        );
    }

    #[test]
    fn r5_2_one_edit_above_the_seal_under_the_sealed_key_names_both_causes() {
        // An edit to gen 2 whose re-seal never happened: its staged copy, put in place of
        // whitelist.json, is one edit newer than the seal and signed by the sealed key, exactly
        // as a wallet.json rolled back by one edit would leave it.
        let (tmp, _, _) = crash_after(|s| matches!(s, EditStage::NextWritten(_)));
        let home = tmp.path();
        fs::copy(home.join(NEXT_FILE), home.join(WHITELIST_FILE)).unwrap();
        let anchor = sealed(home).unwrap();
        assert_eq!(anchor.generation, 1);
        let reason = load_verified(home, &anchor).unwrap_err().message;
        for want in [
            "whitelist.json (gen 2) is one edit newer than the seal (gen 1)",
            "an edit to gen 2 that never finished sealing, or a rolled-back wallet.json",
            "If gen 1 is the highest generation you remember seeing since the last start at gen 1 \
             (an edit's report, or unlock, status or `zwallet whitelist list --verify`), the edit \
             to gen 2 never reported: put back a genuine copy of whitelist.json at gen 1; if you \
             remember gen 2 or higher, wallet.json was rolled back: restore the current \
             wallet.json",
        ] {
            assert!(reason.contains(want), "{want:?} not in {reason}");
        }
        assert!(!reason.contains("Did the last edit"), "{reason}");
        let err = edit(
            home,
            PW,
            &RULES,
            add(Entry::new_address("carol", CAROL, &["base"])),
        )
        .unwrap_err();
        assert!(
            err.message.contains("never finished sealing"),
            "{}",
            err.message
        );
        // The edit adds no second, unconditional copy remedy.
        assert_eq!(
            err.message.matches("put back").count(),
            1,
            "{}",
            err.message
        );
    }

    #[test]
    fn r6_2_one_edit_above_after_two_edits_never_offers_the_older_copy_unconditionally() {
        // History gen 1 -> 2 -> 3 (bob added at 2, removed at 3): the human saw gen 3. Then
        // wallet.json from gen 1 and the gen-2 list are put back. The list is one edit above the
        // seal and signed by the sealed key, as an edit that never finished sealing would be.
        let tmp = wallet();
        let home = tmp.path();
        edit(home, PW, &RULES, add(alice())).unwrap();
        let gen1_wallet = fs::read(home.join(store::WALLET_FILE)).unwrap();
        edit(
            home,
            PW,
            &RULES,
            add(Entry::new_address("bob", BOB, &["base"])),
        )
        .unwrap();
        let gen2_list = fs::read(home.join(WHITELIST_FILE)).unwrap();
        edit(home, PW, &RULES, |l| {
            l.remove("bob");
            Ok(())
        })
        .unwrap();
        fs::write(home.join(store::WALLET_FILE), &gen1_wallet).unwrap();
        fs::write(home.join(WHITELIST_FILE), &gen2_list).unwrap();
        let reason = load_verified(home, &sealed(home).unwrap())
            .unwrap_err()
            .message;
        // A human who remembers gen 3 is sent to the current wallet.json, never to gen 1 (which
        // verifies, and would bring back bob).
        assert!(
            reason.contains(
                "if you remember gen 2 or higher, wallet.json was rolled back: restore the \
                 current wallet.json (zwallet keeps no copy of it), never an older whitelist.json"
            ),
            "{reason}"
        );
        assert!(
            reason.contains("If gen 1 is the highest generation you remember seeing"),
            "{reason}"
        );
        assert!(!reason.contains("Did the last edit"), "{reason}");
    }

    #[test]
    fn r5_3_a_staged_file_left_after_the_publish_is_a_warning_not_a_failure() {
        let tmp = wallet();
        let home = tmp.path();
        edit(home, PW, &RULES, add(alice())).unwrap();
        // whitelist.next.json cannot be removed once whitelist.json is published: a directory
        // with something in it stands at its path.
        let plant = |s: EditStage| {
            if let EditStage::Published(_) = s {
                fs::remove_file(home.join(NEXT_FILE)).unwrap();
                fs::create_dir(home.join(NEXT_FILE)).unwrap();
                fs::write(home.join(NEXT_FILE).join("x"), b"x").unwrap();
            }
            Ok(())
        };
        let edited = edit_with_hook(
            home,
            PW,
            &RULES,
            add(Entry::new_address("bob", BOB, &["base"])),
            &plant,
        )
        .unwrap();
        assert_eq!(nicknames(&edited.list), ["alice", "bob"]);
        let warning = edited.warning.unwrap();
        assert!(
            warning.contains(
                "the change landed, but zwallet could not remove whitelist.next.json, or could \
                 not sync its directory after removing it"
            ) && warning.ends_with(
                "remove it; a later change can stage its list only where there is a plain file \
                 or nothing"
            ),
            "{warning}"
        );
        assert!(!warning.contains("overwrites"), "{warning}");
        let anchor = sealed(home).unwrap();
        assert_eq!(anchor.generation, 2);
        assert_eq!(
            nicknames(&load_verified(home, &anchor).unwrap()),
            ["alice", "bob"]
        );
        // A normal edit leaves no warning.
        fs::remove_dir_all(home.join(NEXT_FILE)).unwrap();
        let edited = edit(home, PW, &RULES, |l| {
            l.remove("bob");
            Ok(())
        })
        .unwrap();
        assert_eq!(edited.warning, None);
    }

    #[test]
    fn n_e_the_new_list_is_written_from_memory_after_the_reseal() {
        let tmp = wallet();
        let home = tmp.path();
        edit(home, PW, &RULES, add(alice())).unwrap();
        // Between the re-seal and the publish, something else is put at whitelist.next.json.
        let swap = |s: EditStage| {
            if let EditStage::Resealed(_) = s {
                fs::write(home.join(NEXT_FILE), b"{ planted }").unwrap();
            }
            Ok(())
        };
        let edited = edit_with_hook(
            home,
            PW,
            &RULES,
            add(Entry::new_address("bob", BOB, &["base"])),
            &swap,
        )
        .unwrap();
        assert_eq!(nicknames(&edited.list), ["alice", "bob"]);
        assert_eq!(
            load_verified(home, &sealed(home).unwrap()).unwrap(),
            edited.list
        );
        assert!(!home.join(NEXT_FILE).exists(), "the staged file was left");
    }

    #[test]
    fn n_g_a_staged_list_swapped_after_the_read_is_not_what_is_promoted() {
        let (tmp, committed) = promoted_by_hand();
        let home = tmp.path();
        let staged = fs::read(home.join(NEXT_FILE)).unwrap();
        let hook = |s: EditStage| match s {
            EditStage::FilesRead(_) => {
                fs::write(home.join(NEXT_FILE), b"{ swapped }").unwrap();
                Ok(())
            }
            EditStage::Promoted(_) => Err(WalletError::validation("crash")),
            _ => Ok(()),
        };
        let carol = Entry::new_address("carol", CAROL, &["base"]);
        assert_eq!(
            edit_with_hook(home, PW, &RULES, add(carol), &hook)
                .unwrap_err()
                .message,
            "crash"
        );
        assert_eq!(fs::read(home.join(WHITELIST_FILE)).unwrap(), staged);
        assert_eq!(
            load_verified(home, &sealed(home).unwrap()).unwrap(),
            committed
        );
    }

    #[test]
    fn n3_a_repeated_fee_cap_chain_is_refused() {
        let signed = WhitelistKey::from_secret(&[1u8; 32]).sign(&golden_list());
        let text = String::from_utf8(signed.to_file_bytes().unwrap()).unwrap();
        let cap = "\"base\": \"500000000000000\"";
        let twice = text.replacen(cap, &format!("\"base\": \"1\", {cap}"), 1);
        assert_ne!(twice, text);
        let err = SignedList::from_file_bytes(twice.as_bytes()).unwrap_err();
        assert!(err.message.contains("base"), "{}", err.message);
        // The same key once is fine.
        SignedList::from_file_bytes(text.as_bytes()).unwrap();
    }

    /// No control character (a terminal escape) and never more than 64 characters of a value
    /// that came from a file (the planted value ends in a run of 200 `x`).
    fn assert_clean(msg: &str) {
        assert!(
            !msg.chars().any(char::is_control),
            "control character in {msg:?}"
        );
        assert!(!msg.contains(&"x".repeat(65)), "unbounded: {msg:?}");
    }

    #[test]
    fn n6_text_from_a_file_reaches_messages_escaped_and_bounded() {
        let evil = format!(
            "\u{1b}[2J\u{1b}[H run `zwallet whitelist add evil {CAROL}` {}",
            "x".repeat(200)
        );
        type Plant = fn(&mut Whitelist, &str);
        let plants: [(&str, Plant); 6] = [
            ("nickname", |l, v| l.entries[0].nickname = v.into()),
            ("address", |l, v| l.entries[0].address = v.into()),
            ("chain", |l, v| l.entries[0].chains = vec![v.into()]),
            ("selector", |l, v| {
                l.entries[1].selectors = Some(vec![v.into()])
            }),
            ("fee cap chain", |l, v| {
                l.policy.max_fee_wei.insert(v.into(), "1".into());
            }),
            ("fee cap", |l, v| {
                l.policy.max_fee_wei.insert("base".into(), v.into());
            }),
        ];
        for (what, plant) in plants {
            let mut l = golden_list();
            plant(&mut l, &evil);
            let msg = l.check_format().unwrap_err().message;
            assert!(
                msg.contains("\\u{1b}"),
                "{what}: the value is still named: {msg}"
            );
            assert_clean(&msg);
        }
        assert_clean(&validate_nickname(&evil).unwrap_err().message);

        // Through the edit path: a hand-edited list's reason is shown while the human types.
        let tmp = wallet();
        let home = tmp.path();
        edit(home, PW, &RULES, add(alice())).unwrap();
        let path = home.join(WHITELIST_FILE);
        let mut v: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        v["entries"][0]["nickname"] = Value::from(evil.as_str());
        fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
        let shown = pick_base(home, sealed(home).as_ref(), &RULES)
            .discarded
            .unwrap();
        assert_clean(&shown.reason);
        assert_eq!(shown.entries, v["entries"], "the entries stay raw JSON");
        // A key the parser does not know is quoted by the parser's own message.
        v.as_object_mut()
            .unwrap()
            .insert(evil.clone(), Value::from(1));
        fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
        assert_clean(&read_list(&path).unwrap_err().message);
        let shown = pick_base(home, sealed(home).as_ref(), &RULES)
            .discarded
            .unwrap();
        assert_clean(&shown.reason);
    }

    #[test]
    fn a_crash_while_making_the_key_is_recovered_by_running_again() {
        // Every stage a first edit passes, in order (a first edit promotes nothing).
        for stage in 0..5 {
            let tmp = wallet();
            let home = tmp.path();
            let hook = |s: EditStage| {
                let at = match s {
                    EditStage::FilesRead(_) => 0,
                    EditStage::KeyWritten(_) => 1,
                    EditStage::NextWritten(_) => 2,
                    EditStage::Resealed(_) => 3,
                    EditStage::Published(_) => 4,
                    EditStage::Promoted(_) => 5,
                };
                if at == stage {
                    Err(WalletError::validation("crash"))
                } else {
                    Ok(())
                }
            };
            edit_with_hook(home, PW, &RULES, add(alice()), &hook).unwrap_err();
            match sealed(home) {
                // Before the re-seal: still no whitelist, so every send stays refused.
                None => assert!(stage < 3, "{stage}"),
                // From the re-seal on: the first list is the sealed one.
                Some(a) => {
                    assert!(stage >= 3, "{stage}");
                    assert_eq!(nicknames(&load_verified(home, &a).unwrap()), ["alice"]);
                }
            }
            if stage == 0 {
                assert_eq!(
                    fs::read_dir(home)
                        .unwrap()
                        .filter_map(|e| e.ok())
                        .filter(|e| e.file_name().to_string_lossy().starts_with("whitelist"))
                        .count(),
                    0,
                    "a crash before any write wrote a whitelist file"
                );
            }
            if stage == 2 {
                // Staged, not sealed: the leftover first list is refused like any list file
                // without a sealed whitelist, and reset (losing nothing yet) clears the way.
                let err = edit(
                    home,
                    PW,
                    &RULES,
                    add(Entry::new_address("bob", BOB, &["base"])),
                )
                .unwrap_err();
                assert!(
                    err.message.contains(
                        "but whitelist.next.json exists (a first edit interrupted before it was \
                         sealed"
                    ),
                    "{}",
                    err.message
                );
                reset(home, PW, &RULES).unwrap();
            }
            let edited = edit(
                home,
                PW,
                &RULES,
                add(Entry::new_address("bob", BOB, &["base"])),
            );
            let edited = edited.unwrap();
            let expect: &[&str] = if stage < 3 {
                &["bob"]
            } else {
                &["alice", "bob"]
            };
            assert_eq!(nicknames(&edited.list), expect, "{stage}");
            // Before the key was written, or after it with nothing staged, this edit is a first
            // edit again; after stage 2 the reset made the key this edit used.
            assert_eq!(edited.key_created, stage <= 1, "{stage}");
            load_verified(home, &sealed(home).unwrap()).unwrap();
        }
    }

    #[test]
    fn a_crash_at_any_stage_of_an_edit_that_promotes_is_recovered_by_running_again() {
        // Every stage an edit passes when the sealed list is still staged: read, promote, stage,
        // re-seal, publish.
        let stages: [fn(&EditStage) -> bool; 5] = [
            |s| matches!(s, EditStage::FilesRead(_)),
            |s| matches!(s, EditStage::Promoted(_)),
            |s| matches!(s, EditStage::NextWritten(_)),
            |s| matches!(s, EditStage::Resealed(_)),
            |s| matches!(s, EditStage::Published(_)),
        ];
        for (i, stage) in stages.into_iter().enumerate() {
            let (tmp, committed) = promoted_by_a_crash();
            let home = tmp.path();
            let hook = |s: EditStage| {
                if stage(&s) {
                    Err(WalletError::validation("crash"))
                } else {
                    Ok(())
                }
            };
            let carol = Entry::new_address("carol", CAROL, &["base"]);
            let err = edit_with_hook(home, PW, &RULES, add(carol), &hook).unwrap_err();
            assert_eq!(err.message, "crash", "{i}");
            // The committed list until the re-seal, the new one from it: never neither.
            let anchor = sealed(home).unwrap();
            let trusted = load_verified(home, &anchor)
                .unwrap_or_else(|e| panic!("{i}: nothing verifies: {}", e.message));
            if i < 3 {
                assert_eq!((anchor.generation, &trusted), (2, &committed), "{i}");
            } else {
                assert_eq!(anchor.generation, 3, "{i}");
                assert_eq!(nicknames(&trusted), ["alice", "bob", "carol"], "{i}");
            }
            // The next edit continues from it.
            let dan = Entry::new_address(
                "dan",
                "0x5555555555555555555555555555555555555555",
                &["base"],
            );
            let edited = edit(home, PW, &RULES, add(dan)).unwrap();
            assert_eq!(edited.list.generation, anchor.generation + 1, "{i}");
            assert!(nicknames(&edited.list).contains(&"dan"), "{i}");
            load_verified(home, &sealed(home).unwrap()).unwrap();
        }
    }

    #[test]
    fn a_missing_or_foreign_key_file_needs_a_reset() {
        let tmp = wallet();
        let home = tmp.path();
        edit(home, PW, &RULES, add(alice())).unwrap();
        let key_path = home.join(KEY_FILE);
        let real_key = fs::read(&key_path).unwrap();
        let before = snapshot(home);

        fs::remove_file(&key_path).unwrap();
        let err = edit(
            home,
            PW,
            &RULES,
            add(Entry::new_address("bob", BOB, &["base"])),
        )
        .unwrap_err();
        assert!(err.message.contains("whitelist reset"), "{}", err.message);
        assert!(err.message.contains(RESET_CAVEAT), "{}", err.message);

        // Another wallet's key file, sealed under the same password.
        let other = wallet();
        edit(other.path(), PW, &RULES, add(alice())).unwrap();
        fs::copy(other.path().join(KEY_FILE), &key_path).unwrap();
        let err = edit(
            home,
            PW,
            &RULES,
            add(Entry::new_address("bob", BOB, &["base"])),
        )
        .unwrap_err();
        assert!(err.message.contains("whitelist reset"), "{}", err.message);

        fs::write(&key_path, b"{}").unwrap();
        let err = edit(
            home,
            PW,
            &RULES,
            add(Entry::new_address("bob", BOB, &["base"])),
        )
        .unwrap_err();
        assert!(err.message.contains("whitelist reset"), "{}", err.message);

        // This wallet's own key, sealed under another password.
        let kdf = KdfParams {
            log_n: KDF_MIN_LOG_N,
            ..KdfParams::default()
        };
        let secret = store::open_whitelist_key(
            &serde_json::from_slice::<KeyFile>(&real_key).unwrap().crypto,
            PW,
        )
        .unwrap();
        let resealed = KeyFile {
            version: KEY_FILE_VERSION,
            crypto: store::seal_whitelist_key(&secret, "another password", kdf).unwrap(),
        };
        fs::write(&key_path, serde_json::to_vec(&resealed).unwrap()).unwrap();
        let err = edit(
            home,
            PW,
            &RULES,
            add(Entry::new_address("bob", BOB, &["base"])),
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::ValidationError);
        assert!(
            err.message
                .contains("does not open with the wallet password"),
            "{}",
            err.message
        );
        assert!(err.message.contains("whitelist reset"), "{}", err.message);

        fs::write(&key_path, &real_key).unwrap();
        assert_eq!(snapshot(home), before, "nothing else was written");
        edit(
            home,
            PW,
            &RULES,
            add(Entry::new_address("bob", BOB, &["base"])),
        )
        .unwrap();
    }

    #[test]
    fn reset_makes_a_new_key_and_an_empty_list() {
        let tmp = wallet();
        let home = tmp.path();
        edit(home, PW, &RULES, |l| {
            l.add(alice())?;
            l.policy.allow_message_signing = true;
            Ok(())
        })
        .unwrap();
        let old_anchor = sealed(home).unwrap();
        let old_list = fs::read(home.join(WHITELIST_FILE)).unwrap();
        fs::remove_file(home.join(KEY_FILE)).unwrap();

        let done = reset(home, PW, &RULES).unwrap();
        assert!(done.key_created);
        assert_eq!(done.previous_generation, Some(1));
        assert_eq!(done.list.generation, 2);
        assert_eq!(done.list.entries, vec![]);
        assert_eq!(done.list.policy, Policy::defaults(&RULES));
        let anchor = sealed(home).unwrap();
        assert_ne!(anchor.public_key, old_anchor.public_key);
        load_verified(home, &anchor).unwrap();
        // The old list no longer verifies, even at a matching generation, and no edit builds on
        // it.
        let reset_list = fs::read(home.join(WHITELIST_FILE)).unwrap();
        fs::write(home.join(WHITELIST_FILE), &old_list).unwrap();
        assert!(load_verified(home, &anchor).is_err());
        let before = snapshot(home);
        let err = edit(home, PW, &RULES, add(alice())).unwrap_err();
        assert!(
            err.message.starts_with("refusing to edit the whitelist: "),
            "{}",
            err.message
        );
        assert!(
            err.message
                .contains("generation 1 is not the sealed generation 2"),
            "{}",
            err.message
        );
        assert_eq!(snapshot(home), before, "the refused edit wrote something");
        // Edits work again with the new key, on the list the reset made.
        fs::write(home.join(WHITELIST_FILE), &reset_list).unwrap();
        let edited = edit(home, PW, &RULES, add(alice())).unwrap();
        assert_eq!(edited.base, BaseSource::Current);
        assert_eq!(edited.list.generation, 3);

        // A reset that dies after writing its key: the old list still verifies; an edit asks for
        // the reset again, which then completes.
        let crash = |s: EditStage| match s {
            EditStage::KeyWritten(_) => Err(WalletError::validation("crash")),
            _ => Ok(()),
        };
        reset_with_hook(home, PW, &RULES, &crash).unwrap_err();
        let anchor = sealed(home).unwrap();
        assert_eq!(nicknames(&load_verified(home, &anchor).unwrap()), ["alice"]);
        let err = edit(
            home,
            PW,
            &RULES,
            add(Entry::new_address("bob", BOB, &["base"])),
        )
        .unwrap_err();
        assert!(err.message.contains("whitelist reset"), "{}", err.message);
        assert_eq!(reset(home, PW, &RULES).unwrap().list.generation, 4);
    }

    #[test]
    fn lookups_follow_kind_and_chain() {
        let mut l = golden_list();
        l.entries
            .push(Entry::new_address("dan", CAROL, &["ethereum"]));
        assert_eq!(l.by_nickname("router").unwrap().address, ROUTER);
        assert_eq!(
            l.by_address(&ALICE.to_ascii_uppercase().replace("0X", "0x"))
                .unwrap()
                .nickname,
            "alice"
        );
        assert!(l.payee(ALICE, "base").is_some());
        assert!(l.payee(ALICE, "polygon").is_none(), "chain scope");
        assert!(l.payee(BOB, "polygon").is_some(), "\"*\"");
        assert!(l.payee(ROUTER, "base").is_none(), "a contract is no payee");
        assert!(l.contract(ROUTER, "base").is_some());
        assert!(l.contract(ROUTER, "ethereum").is_none());
        assert!(
            l.contract(ALICE, "base").is_none(),
            "a payee is no contract"
        );
        let router = l.by_nickname("router").unwrap();
        assert!(router.allows_selector("0xABCDEF01"));
        assert!(!router.allows_selector("0xabcdef02"));
        assert!(
            !l.by_nickname("alice")
                .unwrap()
                .allows_selector("0x12345678")
        );
        assert_eq!(
            l.policy.max_fee_wei("polygon"),
            Some(U256::from(2_000_000_000_000_000_000u128))
        );
        assert_eq!(l.policy.max_fee_wei("ethereum"), None);
        l.policy.max_fee_wei.insert("ethereum".into(), "01".into());
        assert_eq!(
            l.policy.max_fee_wei("ethereum"),
            None,
            "malformed reads as none"
        );
    }
}
