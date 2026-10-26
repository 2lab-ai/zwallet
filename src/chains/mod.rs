//! Chain registry and the chain-family boundary.
//!
//! A family (EVM today; Solana and TON later) owns key derivation from the shared seed, signing,
//! and its chain I/O. The split is deliberate: [`FamilyKeys`] is everything the unlock daemon may
//! do (no network), [`ChainFamily`] adds what the one-shot CLI does over RPC.

pub mod evm;

use crate::chains::evm::policy::ListView;
use crate::core::error::{ErrorCode, Result, WalletError};
use crate::core::seed::Seed;
use crate::core::store;
use alloy::primitives::U256;
use alloy::transports::http::reqwest::Url;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::ops::RangeInclusive;
use std::os::unix::fs::{FileExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Family plus its family-specific configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Evm(evm::EvmChain),
}

impl Family {
    pub fn name(&self) -> &'static str {
        match self {
            Family::Evm(_) => "evm",
        }
    }

    /// Key-side operations for this family (the only part the daemon can reach).
    pub fn keys(&self) -> &'static dyn FamilyKeys {
        match self {
            Family::Evm(_) => &evm::Evm,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chain {
    pub key: &'static str,
    pub name: &'static str,
    pub symbol: &'static str,
    pub decimals: u8,
    pub family: Family,
}

const fn evm_chain(
    key: &'static str,
    name: &'static str,
    symbol: &'static str,
    chain_id: u64,
    default_rpc: &'static str,
    explorer: &'static str,
) -> Chain {
    Chain {
        key,
        name,
        symbol,
        decimals: 18,
        family: Family::Evm(evm::EvmChain {
            chain_id,
            default_rpc,
            explorer,
            op_stack: matches!(chain_id, 8453 | 10),
        }),
    }
}

pub const CHAINS: &[Chain] = &[
    evm_chain(
        "ethereum",
        "Ethereum",
        "ETH",
        1,
        "https://ethereum-rpc.publicnode.com",
        "https://etherscan.io",
    ),
    evm_chain(
        "base",
        "Base",
        "ETH",
        8453,
        "https://base-rpc.publicnode.com",
        "https://basescan.org",
    ),
    evm_chain(
        "arbitrum",
        "Arbitrum One",
        "ETH",
        42161,
        "https://arbitrum-one-rpc.publicnode.com",
        "https://arbiscan.io",
    ),
    evm_chain(
        "optimism",
        "OP Mainnet",
        "ETH",
        10,
        "https://optimism-rpc.publicnode.com",
        "https://optimistic.etherscan.io",
    ),
    evm_chain(
        "polygon",
        "Polygon PoS",
        "POL",
        137,
        "https://polygon-bor-rpc.publicnode.com",
        "https://polygonscan.com",
    ),
    evm_chain(
        "bsc",
        "BNB Smart Chain",
        "BNB",
        56,
        "https://bsc-rpc.publicnode.com",
        "https://bscscan.com",
    ),
    evm_chain(
        "avalanche",
        "Avalanche C-Chain",
        "AVAX",
        43114,
        "https://avalanche-c-chain-rpc.publicnode.com",
        "https://snowtrace.io",
    ),
    evm_chain(
        "sepolia",
        "Sepolia",
        "ETH",
        11155111,
        "https://ethereum-sepolia-rpc.publicnode.com",
        "https://sepolia.etherscan.io",
    ),
];

/// The chain whose cached address is reported as the wallet's primary `address`.
pub const PRIMARY_CHAIN: &str = "ethereum";

/// Test networks among [`CHAINS`]. A listing of every chain puts them after the mainnets.
const TESTNETS: &[&str] = &["sepolia"];

pub fn find(key: &str) -> Result<&'static Chain> {
    let wanted = key.trim().to_ascii_lowercase();
    CHAINS.iter().find(|c| c.key == wanted).ok_or_else(|| {
        let keys: Vec<&str> = CHAINS.iter().map(|c| c.key).collect();
        WalletError::new(
            ErrorCode::ChainUnsupported,
            format!("unsupported chain '{key}'; supported: {}", keys.join(", ")),
        )
    })
}

impl Chain {
    /// A test network ([`TESTNETS`]), whose coins have no value.
    pub fn is_testnet(&self) -> bool {
        TESTNETS.contains(&self.key)
    }

    /// Environment variable that overrides this chain's RPC URL.
    pub fn rpc_env_var(&self) -> String {
        format!("ZWALLET_RPC_{}", self.key.to_ascii_uppercase())
    }

    pub fn default_rpc(&self) -> &'static str {
        match &self.family {
            Family::Evm(c) => c.default_rpc,
        }
    }

    /// RPC precedence: `--rpc` flag > `ZWALLET_RPC_<KEY>` env > built-in default.
    pub fn resolve_rpc(&self, flag: Option<&str>, env_value: Option<&str>) -> String {
        fn pick(v: Option<&str>) -> Option<&str> {
            v.map(str::trim).filter(|s| !s.is_empty())
        }
        pick(flag)
            .or_else(|| pick(env_value))
            .unwrap_or(self.default_rpc())
            .to_string()
    }

    /// Family-specific chain id for display (`1` for Ethereum).
    pub fn chain_id_json(&self) -> Value {
        match &self.family {
            Family::Evm(c) => Value::from(c.chain_id),
        }
    }

    pub fn explorer_tx_url(&self, tx_hash: &str) -> String {
        match &self.family {
            Family::Evm(c) => format!("{}/tx/{tx_hash}", c.explorer),
        }
    }

    pub fn explorer_address_url(&self, address: &str) -> String {
        match &self.family {
            Family::Evm(c) => format!("{}/address/{address}", c.explorer),
        }
    }
}

/// Address cache for the key at `index`: chain key → address. Derives once per family.
pub fn derive_addresses(seed: &Seed, index: u32) -> Result<BTreeMap<String, String>> {
    let mut per_family: BTreeMap<&'static str, String> = BTreeMap::new();
    let mut out = BTreeMap::new();
    for chain in CHAINS {
        let family = chain.family.name();
        let address = match per_family.get(family) {
            Some(a) => a.clone(),
            None => {
                let a = chain.family.keys().derive_address(seed, index)?;
                per_family.insert(family, a.clone());
                a
            }
        };
        out.insert(chain.key.to_string(), address);
    }
    Ok(out)
}

/// Key-side operations. Runs inside the unlock daemon; must never touch the network.
///
/// `index` is the wallet's key index (creation order); each family maps it to its own path, e.g.
/// EVM `m/44'/60'/0'/0/{index}` (later: Solana `m/44'/501'/{index}'/0'`, TON `m/44'/607'/{index}'`).
pub trait FamilyKeys: Sync {
    fn derivation_path(&self, index: u32) -> String;
    fn derive_address(&self, seed: &Seed, index: u32) -> Result<String>;
    /// Returns `{ address, signature, digest }` (family-defined encodings). Refused unless `list`
    /// allows message signing ([`evm::policy::check_message`]).
    fn sign_message(
        &self,
        seed: &Seed,
        index: u32,
        chain: &Chain,
        message: &[u8],
        list: &ListView<'_>,
    ) -> Result<Value>;
    /// Signs a family-specific unsigned transaction; returns `{ rawTx, txHash }`. Refused unless
    /// `list` allows what the transaction does ([`evm::policy::check_tx`]).
    fn sign_tx(
        &self,
        seed: &Seed,
        index: u32,
        chain: &Chain,
        unsigned: &Value,
        list: &ListView<'_>,
    ) -> Result<Value>;
}

/// An RPC endpoint for one chain. `Debug` prints the URL through [`redact_url`].
#[derive(Clone)]
pub struct Net {
    pub chain: &'static Chain,
    /// May carry an API key; print it only through [`redact_url`], and text that came back from
    /// the endpoint only through [`Net::redact`].
    pub rpc_url: String,
}

impl fmt::Debug for Net {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Net")
            .field("chain", &self.chain.key)
            .field("rpc_url", &redact_url(&self.rpc_url))
            .finish()
    }
}

impl Net {
    /// Text from or about this endpoint (a transport error, a JSON-RPC error message) as it may
    /// be printed: every URL through [`redact_url`], then every [`url_secrets`] part of the
    /// configured URL replaced with `<redacted>`, because the endpoint can echo the key without
    /// the URL around it. A part of 8 or more characters is replaced wherever it occurs; a
    /// shorter one (a short query value or userinfo part) only as a whole word, so that a value
    /// like `1` does not shred every number in the text.
    pub fn redact(&self, text: &str) -> String {
        let mut out = redact_urls(text);
        for secret in url_secrets(&self.rpc_url) {
            out = if secret.chars().count() >= SECRET_MIN_LEN {
                out.replace(&secret, REDACTED)
            } else {
                replace_word(&out, &secret, REDACTED)
            };
        }
        out
    }
}

const REDACTED: &str = "<redacted>";

/// Shortest path segment or fragment [`url_secrets`] treats as a possible key on its own.
const SECRET_MIN_LEN: usize = 8;

/// `text` with every occurrence of `needle` that is a whole word (not preceded or followed by a
/// letter, digit or `_` where `needle` itself begins or ends with one) replaced by `with`.
fn replace_word(text: &str, needle: &str, with: &str) -> String {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let (Some(first), Some(last)) = (needle.chars().next(), needle.chars().last()) else {
        return text.to_string();
    };
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut prev: Option<char> = None;
    while let Some(i) = rest.find(needle) {
        let before = rest[..i]
            .chars()
            .last()
            .or(if i == 0 { prev } else { None });
        let after = rest[i + needle.len()..].chars().next();
        let starts = !word(first) || !before.is_some_and(word);
        let ends = !word(last) || !after.is_some_and(word);
        let (take, keep) = if starts && ends {
            (i + needle.len(), Some(with))
        } else {
            (i + first.len_utf8(), None)
        };
        match keep {
            Some(w) => {
                out.push_str(&rest[..i]);
                out.push_str(w);
            }
            None => out.push_str(&rest[..take]),
        }
        prev = rest[..take].chars().last();
        rest = &rest[take..];
    }
    out.push_str(rest);
    out
}

/// `%XX` escapes decoded (and `+` as a space when `plus`), as a server would read them;
/// invalid escapes and non-UTF-8 results are kept as they are.
fn percent_decode(s: &str, plus: bool) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    let hex = |c: u8| (c as char).to_digit(16);
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => match (hex(b[i + 1]), hex(b[i + 2])) {
                (Some(h), Some(l)) => {
                    out.push((h * 16 + l) as u8);
                    i += 3;
                    continue;
                }
                _ => out.push(b'%'),
            },
            b'+' if plus => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// Shortest hyphen-free run of a host label that is a possible key on its own.
const KEY_LABEL_RUN: usize = 20;
/// Shortest host label that is a possible key when it mixes letters and digits.
const KEY_LABEL_MIXED: usize = 12;

/// Does a host name label look like an API key (some providers put the key in a subdomain)?
/// Yes when it has a hyphen-free run of 20 or more characters, or has 12 or more characters with
/// both letters and digits. Word-like labels made of shorter runs, such as the default
/// `avalanche-c-chain-rpc` (21 characters), stay readable.
fn key_like_label(label: &str) -> bool {
    let letters = label.chars().any(|c| c.is_ascii_alphabetic());
    let digits = label.chars().any(|c| c.is_ascii_digit());
    (label.chars().count() >= KEY_LABEL_MIXED && letters && digits)
        || label
            .split('-')
            .any(|run| run.chars().count() >= KEY_LABEL_RUN)
}

/// The host of `u` as it may be printed: each key-like label ([`key_like_label`]) of a domain
/// replaced with `<redacted>`. IP addresses print as they are.
fn shown_host(u: &Url) -> Option<String> {
    Some(match u.domain() {
        Some(d) => d
            .split('.')
            .map(|l| if key_like_label(l) { REDACTED } else { l })
            .collect::<Vec<_>>()
            .join("."),
        None => u.host_str()?.to_string(),
    })
}

/// The parts of an RPC URL that may carry an API key, longest first, each as written in the URL
/// and percent-decoded (`+` also as a space in the query), parsed the way the HTTP client parses
/// the URL:
/// - credential slots, at any length: the userinfo whole and each of its parts, the query string
///   whole and each parameter value;
/// - the path whole (unless empty or `/`), and each path segment of 8 or more characters;
/// - the fragment, at 8 or more characters (it is never sent, so a short one is noise);
/// - each key-like host label ([`key_like_label`]).
///
/// Text that does not parse as a URL is one secret as a whole (as in [`redact_url`]).
pub fn url_secrets(url: &str) -> Vec<String> {
    let raw = url.trim();
    let Ok(u) = Url::parse(raw) else {
        return if raw.is_empty() {
            Vec::new()
        } else {
            vec![raw.to_string()]
        };
    };
    let mut any: Vec<String> = Vec::new();
    let mut long: Vec<String> = Vec::new();
    let both = |list: &mut Vec<String>, s: &str, plus: bool| {
        list.push(s.to_string());
        list.push(percent_decode(s, false));
        if plus {
            list.push(percent_decode(s, true));
        }
    };
    let (user, pass) = (u.username(), u.password());
    if !user.is_empty() || pass.is_some() {
        both(&mut any, user, false);
        if let Some(pass) = pass {
            both(&mut any, pass, false);
            any.push(format!("{user}:{pass}"));
        }
    }
    if let Some(query) = u.query() {
        any.push(query.to_string());
        for pair in query.split('&') {
            let value = pair.split_once('=').map_or(pair, |(_, v)| v);
            both(&mut any, value, true);
        }
    }
    if u.path() != "/" {
        both(&mut any, u.path(), false);
    }
    for segment in u.path_segments().into_iter().flatten() {
        both(&mut long, segment, false);
    }
    if let Some(fragment) = u.fragment() {
        both(&mut long, fragment, false);
    }
    if let Some(domain) = u.domain() {
        any.extend(
            domain
                .split('.')
                .filter(|l| key_like_label(l))
                .map(String::from),
        );
    }
    long.retain(|s| s.chars().count() >= SECRET_MIN_LEN);
    let mut secrets: Vec<String> = any
        .into_iter()
        .chain(long)
        .filter(|s| !s.is_empty())
        .collect();
    secrets.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    secrets.dedup();
    secrets
}

/// `scheme://host[:port]` of an RPC URL as the HTTP client parses it, key-like host labels
/// redacted ([`shown_host`]), or `None` if it does not parse or has no host.
fn origin(u: &Url) -> Option<String> {
    let host = shown_host(u)?;
    Some(match u.port() {
        Some(port) => format!("{}://{host}:{port}", u.scheme()),
        None => format!("{}://{host}", u.scheme()),
    })
}

/// `scheme://host[:port]` of an RPC URL (as the HTTP client parses it, key-like host labels
/// redacted), or `<redacted>` if it has none: what an error about the endpoint itself may name.
pub fn url_origin(url: &str) -> String {
    Url::parse(url.trim())
        .ok()
        .as_ref()
        .and_then(origin)
        .unwrap_or_else(|| REDACTED.to_string())
}

/// An RPC URL as it may be printed: scheme and host (with port) only, parsed the way the HTTP
/// client parses it. Providers put API keys in the path, the query or the userinfo
/// (`https://eth-mainnet.g.alchemy.com/v2/<key>`), so anything after the host becomes
/// `/<redacted>`; some put it in a subdomain, so a key-like host label becomes `<redacted>` too.
/// `https://host` and `https://host/` print as they are. Text that is not a URL with a host is
/// redacted whole.
pub fn redact_url(url: &str) -> String {
    let raw = url.trim();
    let Ok(u) = Url::parse(raw) else {
        return REDACTED.to_string();
    };
    let Some(shown) = origin(&u) else {
        return REDACTED.to_string();
    };
    let extra = !u.username().is_empty()
        || u.password().is_some()
        || u.query().is_some()
        || u.fragment().is_some()
        || u.path() != "/";
    if extra {
        format!("{shown}/{REDACTED}")
    } else if raw.ends_with('/') {
        format!("{shown}/")
    } else {
        shown
    }
}

/// Applies [`redact_url`] to every `http://` / `https://` URL inside free text, such as a
/// transport error (reqwest quotes the full request URL). A URL ends at whitespace, a quote, `)`
/// or `>`.
pub fn redact_urls(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = [rest.find("http://"), rest.find("https://")]
        .into_iter()
        .flatten()
        .min()
    {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let end = tail
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ')' | '>' | '`'))
            .unwrap_or(tail.len());
        out.push_str(&redact_url(&tail[..end]));
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Balance {
    pub symbol: String,
    pub decimals: u8,
    pub raw: U256,
}

/// A registry token's balance read: its contract as the registry writes it, the registry's
/// symbol and decimals, and the raw balance or why it could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenBalance {
    pub address: String,
    pub symbol: String,
    pub decimals: u8,
    pub raw: Result<U256>,
}

#[derive(Debug, Clone)]
pub struct TransferRequest {
    pub from: String,
    pub to: String,
    pub amount: String,
    pub token: Option<String>,
    pub dry_run: bool,
    /// Use this account sequence number instead of the next free one (to replace a pending tx).
    pub nonce: Option<u64>,
    /// `--bump` ([`parse_bump`]): what a replacement pays over the fees it must beat.
    pub bump: Option<u128>,
    /// `--replace`: the hash of the pending transaction this one replaces (its fees are read).
    pub replace: Option<String>,
    /// Explicit ceiling on the price per gas unit, in base units, replacing the automatic one.
    pub max_fee_per_gas: Option<u128>,
    /// Allow recipients that are almost certainly mistakes (zero address, the token itself).
    pub force: bool,
    /// `--allow-pending`: a send without `nonce` goes ahead while an earlier transaction from
    /// this key is still pending.
    pub allow_pending: bool,
    /// The account's send journal, for naming earlier transactions in errors.
    pub journal: Option<SendJournal>,
}

/// A replacement's fee bump in ten-thousandths: 12_500 = 1.25x. Nodes drop a replacement that
/// does not outbid the pending transaction by their bump rule (geth and most clients: +10% on
/// both the tip and the fee cap), hence the 1.1x minimum.
pub const DEFAULT_BUMP: u128 = 12_500;
pub const BUMP_SCALE: u128 = 10_000;
const BUMP_MIN: u128 = 11_000;
const BUMP_MAX: u128 = 30_000;

/// `--bump <factor>` in ten-thousandths: a plain decimal from 1.1 to 3, at most 4 decimals.
pub fn parse_bump(input: &str) -> Result<u128> {
    let bad = || {
        WalletError::validation(format!(
            "--bump '{input}': expected a factor from 1.1 to 3, e.g. 1.25"
        ))
    };
    let v = crate::core::amount::parse_units(input, 4).map_err(|_| bad())?;
    u128::try_from(v)
        .ok()
        .filter(|v| (BUMP_MIN..=BUMP_MAX).contains(v))
        .ok_or_else(bad)
}

/// A bump as printed: `1.25x`.
pub fn bump_label(bump: u128) -> String {
    format!(
        "{}x",
        crate::core::amount::format_units(U256::from(bump), 4)
    )
}

/// An unsigned transaction plus the output fields describing it (nonce, fees, shortfall, ...).
#[derive(Debug, Clone)]
pub struct PreparedTx {
    pub unsigned: Value,
    pub fields: Map<String, Value>,
}

/// A signed transaction as decoded from its raw bytes (not as claimed by the signer).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedTx {
    pub raw: String,
    /// Computed from `raw`; never taken from the node.
    pub tx_hash: String,
    pub signer: String,
    /// The account sequence number the transaction consumes, where the family has one (EVM).
    pub nonce: Option<u64>,
}

/// A broadcast that reached a node which holds the transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Broadcast {
    /// The node accepted it and answered with the transaction's own hash.
    Accepted,
    /// The send call failed or answered another hash (`warning` says how), but the node reports
    /// the transaction by its own hash, so it was sent. `rpc_tx_hash` is the other hash, if the
    /// node answered one.
    KnownDespite {
        warning: String,
        rpc_tx_hash: Option<String>,
    },
}

/// One send at a time per account and chain within a home: held from reading the next nonce
/// until the command returns (through `--wait`), so two concurrent sends never sign the same
/// nonce. A second send fails at once instead of waiting: it may be a second attempt at the same
/// payment, and after waiting it would sign the next nonce and land as well. An exclusive flock
/// on `<home>/nonce-<chainId>-<address>.lock`, released when this value drops, or by the OS when
/// the process dies.
pub struct SendLock {
    _file: File,
}

impl SendLock {
    /// Takes the lock on `address` (as printed by the family) on `chain`, or fails with
    /// `VALIDATION_ERROR` without waiting if another send holds it.
    pub fn acquire(home: &Path, chain: &Chain, address: &str) -> Result<Self> {
        let path = home.join(format!(
            "nonce-{}-{}.lock",
            chain.chain_id_json(),
            address.to_ascii_lowercase()
        ));
        match store::try_lock_exclusive(&path)? {
            Some(file) => Ok(Self { _file: file }),
            None => Err(WalletError::validation(format!(
                "another send from this key on {} is in progress (lock {}); nothing was signed \
                 or broadcast. If it is an earlier attempt at this same payment, wait for it to \
                 finish and look up its result before sending again: its txHash is in its output \
                 and, once it is broadcast, in the send journal {} (look it up with {}); for a \
                 different payment, retry after it finishes",
                chain.key,
                path.display(),
                journal_path(home, chain, address).display(),
                lookup_hint(chain, "<txHash>")
            ))),
        }
    }
}

/// The send journal of one account on one chain: `<home>/sends-<chainId>-<address>.jsonl`, one
/// JSON line `{ ts, chain, from, nonce, txHash, to, value, amountRaw, kind, token? }` per
/// transaction,
/// appended and fsynced right before it is broadcast. If the process dies after the broadcast,
/// this is where the transaction is still named. Not secret, but created 0600 like every file in
/// the home.
#[derive(Debug, Clone)]
pub struct SendJournal {
    path: PathBuf,
    chain: &'static Chain,
    address: String,
}

/// How many journal entries an error message names at most (the newest).
const JOURNAL_SHOWN: usize = 5;
/// How many characters the send journal gives one message in all: every shown entry's hash,
/// nonce, time, kind, amount and recipient together. Entries are shown whole, newest first, while
/// they fit; the journal is a file any process of this user can write.
const JOURNAL_TEXT_MAX: usize = crate::core::text::PROSE_MAX;
/// The most digits an amount in the journal shows on either side of its point (a U256 has 78).
const AMOUNT_DIGITS: usize = 78;
/// The most characters of the symbol in a journal amount (zwallet keeps 32 of an on-chain one).
const SYMBOL_CHARS: usize = 32;
/// The kinds of send zwallet journals: what `transfer` and `invoke` pass as `kind`.
const SEND_KINDS: [&str; 3] = ["transfer", "token-transfer", "invoke"];

/// A journal entry's `txHash` when it is a transaction hash (32 bytes of hex), as alloy writes
/// one; anything else in that field is not shown, nor made into a lookup command.
fn entry_hash(e: &Value) -> Option<String> {
    e["txHash"]
        .as_str()
        .and_then(|h| h.parse::<alloy::primitives::B256>().ok())
        .map(|h| format!("{h:#x}"))
}

/// `0x` and 40 hex digits: an address as zwallet writes one.
fn is_address(s: &str) -> bool {
    s.len() == 42 && s.starts_with("0x") && s[2..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// `<digits>[.<digits>] <symbol>`, as zwallet writes a journal amount: at most
/// [`AMOUNT_DIGITS`] digits on either side of the point, then one space and a symbol of letters,
/// digits and `₮` (one point inside allowed, as in `USDC.e`), at most [`SYMBOL_CHARS`] long.
fn is_amount_value(s: &str) -> bool {
    let digits = |d: &str| {
        !d.is_empty() && d.len() <= AMOUNT_DIGITS && d.bytes().all(|b| b.is_ascii_digit())
    };
    let word = |w: &str| !w.is_empty() && w.chars().all(|c| c.is_ascii_alphanumeric() || c == '₮');
    let Some((amount, symbol)) = s.split_once(' ') else {
        return false;
    };
    let amount_ok = match amount.split_once('.') {
        Some((int, frac)) => digits(int) && digits(frac),
        None => digits(amount),
    };
    let symbol_ok = symbol.chars().count() <= SYMBOL_CHARS
        && match symbol.split_once('.') {
            Some((a, b)) => word(a) && word(b),
            None => word(symbol),
        };
    amount_ok && symbol_ok
}

/// A journal entry's fields as a message shows them: each only in the shape zwallet itself writes
/// it, else [`INVALID`](crate::core::text::INVALID); the hash only when it is one (`?` else); the
/// time re-rendered from the instant it names, never the file's bytes.
struct EntryText {
    hash: String,
    nonce: String,
    ts: String,
    kind: String,
    value: String,
    to: String,
}

impl EntryText {
    fn of(e: &Value) -> Self {
        use crate::core::text::INVALID;
        use crate::core::time::{parse_rfc3339_utc, rfc3339_utc};
        let shaped = |k: &str, ok: fn(&str) -> bool| {
            e[k].as_str()
                .filter(|v| ok(v))
                .map_or_else(|| INVALID.to_string(), str::to_string)
        };
        Self {
            hash: entry_hash(e).unwrap_or_else(|| "?".to_string()),
            nonce: e["nonce"]
                .as_u64()
                .map_or_else(|| "?".to_string(), |n| n.to_string()),
            ts: e["ts"]
                .as_str()
                .and_then(parse_rfc3339_utc)
                .map_or_else(|| INVALID.to_string(), rfc3339_utc),
            kind: shaped("kind", |k| SEND_KINDS.contains(&k)),
            value: shaped("value", is_amount_value),
            to: shaped("to", is_address),
        }
    }
}

/// The newest of `entries` (at most [`JOURNAL_SHOWN`]) as `text` renders each, oldest first: as
/// many as fit one [`JOURNAL_TEXT_MAX`] budget of journal text (`text` also says how much of the
/// rendering is the journal's), each whole, so a hash is never cut. Then `(and N more there)` for
/// the entries not shown.
fn entries_shown(entries: &[Value], text: impl Fn(&EntryText) -> (String, usize)) -> String {
    let mut shown = Vec::new();
    let mut used = 0;
    for e in entries.iter().rev().take(JOURNAL_SHOWN) {
        let (line, len) = text(&EntryText::of(e));
        if used + len > JOURNAL_TEXT_MAX {
            break;
        }
        used += len;
        shown.push(line);
    }
    let more = entries.len() - shown.len();
    shown.reverse();
    let mut out = shown.join("; ");
    if more > 0 {
        out.push_str(&format!(" (and {more} more there)"));
    }
    out
}

/// A journal entry as a message names it: `0xab.. (nonce 6, sent 2026-10-20T04:05:06Z: transfer
/// 0.01 ETH to 0x..)`, and how many of its characters are the journal's.
fn describe_entry(e: &EntryText) -> (String, usize) {
    let line = format!(
        "{} (nonce {}, sent {}: {} {} to {})",
        e.hash, e.nonce, e.ts, e.kind, e.value, e.to
    );
    let len = [&e.hash, &e.nonce, &e.ts, &e.kind, &e.value, &e.to]
        .iter()
        .map(|s| s.len())
        .sum();
    (line, len)
}

/// How to look up the hashes a journal message names: one command and one link, with the hash as
/// a placeholder, so the journal's text is not repeated.
fn journal_lookup(chain: &Chain) -> String {
    format!("look each hash up with {}", lookup_hint(chain, "<txHash>"))
}

/// Journal entries for an error's `--json` fields: every string and key escaped
/// ([`crate::core::text::inert_json`]), structure kept.
fn inert_entries(entries: &[Value]) -> Value {
    Value::Array(entries.iter().map(crate::core::text::inert_json).collect())
}
/// How much of the journal's end is read; older entries are past every window that matters.
const JOURNAL_TAIL: u64 = 1 << 20;
/// A send without `--nonce` that matches a journal entry this recent is refused as a likely
/// repeat ([`SendJournal::recent_duplicates`]).
pub const DUPLICATE_WINDOW_SECS: u64 = 600;

/// What a send does, as its journal line records it and the duplicate check compares it:
/// `kind` (`transfer`, `token-transfer`, `invoke`), `to` (the recipient, or the contract for
/// `invoke`), `value` (the amount with its symbol, e.g. `0.01 ETH`, for people), `amount_raw`
/// (the same amount in base units: wei, or the token's smallest unit), and `token` (the token
/// contract of a token transfer).
///
/// The duplicate check compares `amount_raw`, never `value`: a token's symbol comes from the
/// node, and a `symbol()` call that fails once (`?`) must not make a repeat look new.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendSummary {
    pub kind: String,
    pub to: String,
    pub value: String,
    pub amount_raw: Option<String>,
    pub token: Option<String>,
}

/// Where [`SendJournal`] keeps `address`'s sends on `chain` in `home`.
fn journal_path(home: &Path, chain: &Chain, address: &str) -> PathBuf {
    home.join(format!(
        "sends-{}-{}.jsonl",
        chain.chain_id_json(),
        address.to_ascii_lowercase()
    ))
}

impl SendJournal {
    pub fn new(home: &Path, chain: &'static Chain, address: &str) -> Self {
        Self {
            path: journal_path(home, chain, address),
            chain,
            address: address.to_string(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn display(&self) -> String {
        self.path.display().to_string()
    }

    /// Opens the journal with `opts` plus `O_NOFOLLOW | O_NONBLOCK` (a symlink is refused, and a
    /// FIFO planted in its place cannot block the open), then requires a regular file owned by
    /// this user, as `wallet.json` is read. Returns the file and its length.
    fn open_checked(
        &self,
        opts: &mut OpenOptions,
    ) -> std::result::Result<(File, u64), JournalOpen> {
        let f = opts
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&self.path)
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => JournalOpen::Missing,
                _ if e.raw_os_error() == Some(libc::ELOOP) => {
                    JournalOpen::Refused("it is a symbolic link".into())
                }
                _ => JournalOpen::Refused(e.to_string()),
            })?;
        let meta = f
            .metadata()
            .map_err(|e| JournalOpen::Refused(format!("cannot inspect it: {e}")))?;
        if !meta.file_type().is_file() {
            return Err(JournalOpen::Refused("it is not a regular file".into()));
        }
        // SAFETY: geteuid has no preconditions and cannot fail.
        let me = unsafe { libc::geteuid() };
        if meta.uid() != me {
            return Err(JournalOpen::Refused(format!(
                "it is owned by uid {}, not this user",
                meta.uid()
            )));
        }
        Ok((f, meta.len()))
    }

    /// Appends `entry` as one line and fsyncs it (and the home, when the file is new). The file
    /// is opened as [`Self::open_checked`] says. If it does not end in a newline (a run killed in
    /// the middle of an append), a newline goes first, so that the cut-off record cannot swallow
    /// this one. Any failure, including reading that last byte, is an error: the caller then
    /// broadcasts nothing.
    pub fn append(&self, entry: &Value) -> Result<()> {
        let fail = |why: String| {
            WalletError::validation(format!(
                "cannot write the send journal {}: {why}",
                self.display()
            ))
        };
        let io = |e: std::io::Error| fail(e.to_string());
        let (f, len) = self
            .open_checked(
                OpenOptions::new()
                    .read(true)
                    .append(true)
                    .create(true)
                    .mode(store::FILE_MODE),
            )
            .map_err(|e| fail(e.why()))?;
        f.set_permissions(std::fs::Permissions::from_mode(store::FILE_MODE))
            .map_err(io)?;
        let mut line = String::new();
        if len > 0 {
            let mut last = [0u8; 1];
            f.read_exact_at(&mut last, len - 1)
                .map_err(|e| fail(format!("cannot read its last byte: {e}")))?;
            if last[0] != b'\n' {
                line.push('\n');
            }
        }
        line.push_str(&entry.to_string());
        line.push('\n');
        (&f).write_all(line.as_bytes()).map_err(io)?;
        f.sync_all().map_err(io)?;
        if len == 0
            && let Some(dir) = self.path.parent()
        {
            File::open(dir).and_then(|d| d.sync_all()).map_err(io)?;
        }
        Ok(())
    }

    /// The journal's entries, oldest first: at most its last [`JOURNAL_TAIL`] bytes (from the
    /// first whole line in them; the read stops there even if the file grows meanwhile), skipping
    /// lines that are not JSON objects. A missing journal has none. It is opened as
    /// [`Self::open_checked`] says; anything else wrong with it is an error, so that a check
    /// relying on it fails closed instead of seeing no earlier sends.
    fn entries(&self) -> Result<Vec<Value>> {
        use std::io::{Read, Seek, SeekFrom};
        let fail = |why: String| {
            WalletError::validation(format!(
                "cannot read the send journal {}: {why}",
                self.display()
            ))
        };
        let (mut f, len) = match self.open_checked(OpenOptions::new().read(true)) {
            Ok(opened) => opened,
            Err(JournalOpen::Missing) => return Ok(Vec::new()),
            Err(e) => return Err(fail(e.why())),
        };
        let start = len.saturating_sub(JOURNAL_TAIL);
        let mut bytes = Vec::new();
        f.seek(SeekFrom::Start(start))
            .and_then(|_| f.take(JOURNAL_TAIL).read_to_end(&mut bytes))
            .map_err(|e| fail(e.to_string()))?;
        // Mid-file, the first line is cut: drop it.
        let whole = match (start > 0, bytes.iter().position(|&b| b == b'\n')) {
            (false, _) => &bytes[..],
            (true, Some(i)) => &bytes[i + 1..],
            (true, None) => &[][..],
        };
        Ok(String::from_utf8_lossy(whole)
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter(Value::is_object)
            .collect())
    }

    /// The entries whose nonce is in `nonces`, oldest first, minus the one for `except` (a hash
    /// known not to be the one sought). An error as [`Self::entries`].
    pub fn entries_at(
        &self,
        nonces: RangeInclusive<u64>,
        except: Option<&str>,
    ) -> Result<Vec<Value>> {
        Ok(self
            .entries()?
            .into_iter()
            .filter(|e| e["nonce"].as_u64().is_some_and(|n| nonces.contains(&n)))
            .filter(|e| {
                except.is_none_or(|h| {
                    !e["txHash"]
                        .as_str()
                        .is_some_and(|t| t.eq_ignore_ascii_case(h))
                })
            })
            .collect())
    }

    /// Where to find the earlier transactions at `nonces`, as one sentence for an error message
    /// (no final period): the journal's newest entries there (minus `except`), or that it has
    /// none (or cannot be read) and where else to look. Also returns the entries, for the error's
    /// `--json` fields.
    pub fn pointer(
        &self,
        nonces: RangeInclusive<u64>,
        except: Option<&str>,
    ) -> (String, Vec<Value>) {
        let at = nonce_label(&nonces);
        let entries = match self.entries_at(nonces.clone(), except) {
            Ok(entries) => entries,
            Err(e) => {
                let text = format!(
                    "{}, so it cannot name the transaction at {at}; find it on the explorer: {}",
                    e.message,
                    self.chain.explorer_address_url(&self.address)
                );
                return (text, Vec::new());
            }
        };
        if entries.is_empty() {
            let text = format!(
                "The send journal {} has no {}entry at {at}; find it on the explorer: {}",
                self.display(),
                if except.is_some() { "other " } else { "" },
                self.chain.explorer_address_url(&self.address)
            );
            return (text, entries);
        }
        let text = format!(
            "The send journal {} has at {at}: {}; {}",
            self.display(),
            entries_shown(&entries, describe_entry),
            journal_lookup(self.chain)
        );
        // The entries for an error's `pendingSends`: escaped, as the text is.
        let entries = entries.iter().map(crate::core::text::inert_json).collect();
        (text, entries)
    }

    /// The entries from the last [`DUPLICATE_WINDOW_SECS`] before `now` (UNIX seconds) that do
    /// what `send` does: same chain, kind and `to`, the same amount in base units (`amountRaw`;
    /// a line written before zwallet journaled it compares its `value` text instead), and the
    /// same token contract (or none on both). Addresses compare without case. An entry stamped after `now` (a clock set back)
    /// counts as recent; one whose time does not parse does not. A journal that cannot be read
    /// is an error that says nothing was signed: without it a repeat would go unnoticed.
    pub fn recent_duplicates(&self, send: &SendSummary, now: u64) -> Result<Vec<Value>> {
        let same_addr = |v: &Value, want: Option<&str>| match (v.as_str(), want) {
            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
            (None, None) => v.is_null(),
            _ => false,
        };
        let entries = self.entries().map_err(|e| {
            WalletError::new(
                e.code,
                format!(
                    "{}; the check for a repeat of a recent send needs it (it must be a regular \
                     file owned by this user), so nothing was signed",
                    e.message
                ),
            )
            .with_detail("journal", self.display())
        })?;
        Ok(entries
            .into_iter()
            .filter(|e| {
                e["ts"]
                    .as_str()
                    .and_then(crate::core::time::parse_rfc3339_utc)
                    .is_some_and(|ts| now.saturating_sub(ts) <= DUPLICATE_WINDOW_SECS)
            })
            .filter(|e| {
                let same_amount = match (e["amountRaw"].as_str(), send.amount_raw.as_deref()) {
                    (Some(a), Some(b)) => a == b,
                    _ => e["value"] == send.value.as_str(),
                };
                e["chain"] == self.chain.key
                    && e["kind"] == send.kind.as_str()
                    && same_amount
                    && same_addr(&e["to"], Some(&send.to))
                    && same_addr(&e["token"], send.token.as_deref())
            })
            .collect())
    }

    /// The refusal of a send without `--nonce` that repeats `duplicates` (from
    /// [`Self::recent_duplicates`], not empty): nothing was signed.
    pub fn duplicate_error(&self, duplicates: Vec<Value>) -> WalletError {
        let shown = entries_shown(&duplicates, |e| {
            let line = format!("{} (nonce {}, sent {})", e.hash, e.nonce, e.ts);
            (line, e.hash.len() + e.nonce.len() + e.ts.len())
        });
        WalletError::validation(format!(
            "this looks like a repeat of a recent send; look those hashes up first. If this \
             really is a second payment, pass --allow-duplicate. The send journal {} has the same \
             send (chain, kind, recipient, amount and token) from the last {} minutes: {shown}; \
             {}; nothing was signed",
            self.display(),
            DUPLICATE_WINDOW_SECS / 60,
            journal_lookup(self.chain)
        ))
        .with_detail("duplicateSends", inert_entries(&duplicates))
        .with_detail("journal", self.display())
    }
}

/// Why [`SendJournal::open_checked`] did not open the journal.
enum JournalOpen {
    /// There is no journal yet.
    Missing,
    /// It cannot or must not be used (the text says why).
    Refused(String),
}

impl JournalOpen {
    fn why(self) -> String {
        match self {
            Self::Missing => "it does not exist".to_string(),
            Self::Refused(why) => why,
        }
    }
}

/// `nonce 6` or `nonces 5 to 7`.
pub fn nonce_label(nonces: &RangeInclusive<u64>) -> String {
    if nonces.start() == nonces.end() {
        format!("nonce {}", nonces.start())
    } else {
        format!("nonces {} to {}", nonces.start(), nonces.end())
    }
}

/// A signed transaction about to be broadcast, and its journal line.
pub struct SendRecord {
    pub journal: SendJournal,
    pub entry: Value,
}

impl SendRecord {
    /// The journal line for `signed`, which does what `send` says; `token` only for a token
    /// transfer.
    pub fn new(journal: SendJournal, signed: &SignedTx, send: &SendSummary) -> Self {
        let mut entry = json!({
            "ts": crate::core::time::now_rfc3339(),
            "chain": journal.chain.key,
            "from": signed.signer,
            "nonce": signed.nonce,
            "txHash": signed.tx_hash,
            "to": send.to,
            "value": send.value,
            "kind": send.kind,
        });
        if let Some(raw) = &send.amount_raw {
            entry["amountRaw"] = json!(raw);
        }
        if let Some(token) = &send.token {
            entry["token"] = json!(token);
        }
        Self { journal, entry }
    }

    /// Call right before the transaction is broadcast: appends the journal line (fsynced) and
    /// prints the same fields as one line on stderr, so that a run cut off after the broadcast
    /// still leaves the hash behind. A failure here means nothing was broadcast.
    pub fn write(&self) -> Result<()> {
        self.journal.append(&self.entry)?;
        let hash = self.entry["txHash"].as_str().unwrap_or("?");
        let chain = self.journal.chain.key;
        let lookup = lookup_hint(self.journal.chain, hash);
        let line = match self.entry["nonce"].as_u64() {
            Some(n) => format!(
                "broadcasting {hash} (nonce {n} on {chain}); if this command does not finish, \
                 look this hash up before sending again ({lookup}); a retry must pass --nonce \
                 {n} --replace {hash}"
            ),
            None => format!(
                "broadcasting {hash} (on {chain}); if this command does not finish, look this \
                 hash up before sending again ({lookup})"
            ),
        };
        // A closed stderr must not panic the send (`eprint_line` ignores it).
        crate::core::terminal::eprint_line(&format!(
            "{line}. Journal {}: {}",
            self.journal.display(),
            self.entry
        ));
        Ok(())
    }
}

/// The error for a transaction that may have reached the network: it carries `txHash`, `from`
/// and `nonce` so the caller can look the transaction up, or retry with the same nonce and
/// `--replace` so that the retry replaces it instead of becoming a second send.
pub fn after_send_error(
    code: ErrorCode,
    chain: &Chain,
    signed: &SignedTx,
    what: &str,
) -> WalletError {
    let hash = &signed.tx_hash;
    let mut message = format!(
        "{what}. The transaction {hash} may have reached the network; do not resend blindly: \
         look it up first ({})",
        lookup_hint(chain, hash)
    );
    if let Some(n) = signed.nonce {
        message.push_str(&format!(
            ", or retry with --nonce {n} --replace {hash} so that the retry replaces it instead \
             of sending twice"
        ));
    }
    with_tx_details(WalletError::new(code, message), signed)
}

/// What to do about a retry once a transaction is signed but was not broadcast: reuse its
/// nonce, so that the retry can only take this transaction's place, never add a second one. No
/// `--replace`: nothing was broadcast, so the nonce is still free and there is nothing to name.
fn retry_hint(signed: &SignedTx) -> String {
    match signed.nonce {
        Some(n) => {
            format!("If you retry, pass --nonce {n} so that the retry cannot become a second send")
        }
        None => "Look the transaction up before you retry".to_string(),
    }
}

/// The error for a transaction the node refused outright (`why` is its answer): nothing was
/// broadcast. It carries the same `txHash`, `from` and `nonce` fields as [`after_send_error`].
/// `hint` is a family-specific addition.
pub fn rejected_send_error(signed: &SignedTx, why: &str, hint: Option<&str>) -> WalletError {
    let mut message = format!(
        "the node rejected the transaction; nothing was broadcast ({why}). {}",
        retry_hint(signed)
    );
    if let Some(h) = hint {
        message.push_str(". ");
        message.push_str(h);
    }
    with_tx_details(WalletError::rpc(message), signed)
}

/// `e` (code kept) for a failure after signing but before the send request went out: nothing
/// was broadcast, but the signed transaction exists, so a retry still reuses its nonce.
pub fn before_send_error(signed: &SignedTx, e: WalletError) -> WalletError {
    with_tx_details(
        WalletError::new(
            e.code,
            format!(
                "{}; nothing was broadcast. {}",
                e.message,
                retry_hint(signed)
            ),
        ),
        signed,
    )
}

/// The node says the transaction's nonce is already used (`why` is its answer) and does not
/// report this transaction by its hash: another transaction from this key, pending or mined,
/// holds the nonce. This one was refused and will never be found, so its hash is
/// `refusedTxHash`, never `txHash`. The holder may be an earlier send of this same payment, so
/// the message sends the agent to that earlier send (`journal` names its entries) and never
/// invites a plain resend. `BROADCAST_UNCERTAIN` when this send was a `--nonce` retry
/// (`explicit_nonce`), else `RPC_ERROR`. `hint` is a family-specific addition.
pub fn nonce_taken_error(
    signed: &SignedTx,
    why: &str,
    explicit_nonce: bool,
    hint: Option<&str>,
    journal: Option<&SendJournal>,
) -> WalletError {
    let (nonce, n) = match signed.nonce {
        Some(n) => (format!("nonce {n}"), n.to_string()),
        None => ("its nonce".to_string(), "<its nonce>".to_string()),
    };
    // The journal also holds this refused transaction's own line: it is not the earlier send.
    let pointer = match (journal, signed.nonce) {
        (Some(j), Some(n)) => {
            let (text, _) = j.pointer(n..=n, Some(&signed.tx_hash));
            format!(" {text}.")
        }
        _ => String::new(),
    };
    let (code, mut message) = if explicit_nonce {
        (
            ErrorCode::BroadcastUncertain,
            format!(
                "{nonce} is taken by another transaction from this key, pending or mined \
                 ({why}). This retry was refused and will not be found; look up the txHash of \
                 your earlier send at {nonce} from its own output (or the send journal \
                 below).{pointer} While it may be pending or mined, do not send this payment \
                 without --nonce; to replace it use --nonce {n} --replace <earlier txHash>"
            ),
        )
    } else {
        (
            ErrorCode::RpcError,
            format!(
                "{nonce} is taken by another transaction from this key, pending or mined, so the \
                 node refused this one ({why}); it will not be found. If you were retrying an \
                 earlier send, look up that send's txHash before sending again (its own output, \
                 or the send journal below).{pointer}"
            ),
        )
    };
    if let Some(h) = hint {
        message.push_str(if message.ends_with('.') { " " } else { ". " });
        message.push_str(h);
    }
    let mut e = WalletError::new(code, message)
        .with_detail("refusedTxHash", signed.tx_hash.as_str())
        .with_detail("from", signed.signer.as_str());
    if let Some(n) = signed.nonce {
        e = e.with_detail("nonce", n);
    }
    if let Some(j) = journal {
        e = e.with_detail("journal", j.display());
    }
    e
}

fn with_tx_details(e: WalletError, signed: &SignedTx) -> WalletError {
    let mut e = e
        .with_detail("txHash", signed.tx_hash.as_str())
        .with_detail("from", signed.signer.as_str());
    if let Some(n) = signed.nonce {
        e = e.with_detail("nonce", n);
    }
    e
}

/// How to look `hash` up on `chain`: the `zwallet tx` command and the explorer page, for every
/// message that sends the agent to a transaction before it sends anything again.
pub fn lookup_hint(chain: &Chain, hash: &str) -> String {
    format!(
        "`zwallet tx --chain {} {hash}` or {}",
        chain.key,
        chain.explorer_tx_url(hash)
    )
}

/// What to do about a sent transaction that may still be pending (`state` says how it stands):
/// replace it at its own nonce, never send the payment again without one.
pub fn pending_hint(state: &str, nonce: u64, hash: &str) -> String {
    format!(
        "{state}; to speed up or cancel use --nonce {nonce} --replace {hash}; a run without \
         --nonce is a second payment"
    )
}

/// Adds a `--wait` result to a broadcast's output (which carries `txHash` and `nonce`). The
/// transaction is already sent, so a receipt problem never becomes a failure (a retry would send
/// again): no receipt in time is `status: "pending"`, an RPC error is `status: "unknown"` with
/// `receiptError`. Both add a `hint` that keeps the next step on the same nonce and says how to
/// look the transaction up later ([`lookup_hint`]).
pub fn record_receipt(out: &mut Map<String, Value>, chain: &Chain, receipt: Result<Option<Value>>) {
    let state = match receipt {
        Ok(Some(Value::Object(r))) => {
            out.extend(r);
            return;
        }
        Ok(Some(_)) | Ok(None) => {
            out.insert("status".into(), Value::from("pending"));
            "still pending"
        }
        Err(e) => {
            out.insert("status".into(), Value::from("unknown"));
            out.insert("receiptError".into(), Value::from(e.message));
            "receipt unknown, it may still be pending"
        }
    };
    let hash = out
        .get("txHash")
        .and_then(Value::as_str)
        .unwrap_or("<txHash>")
        .to_string();
    let lookup = lookup_hint(chain, &hash);
    let hint = match out.get("nonce").and_then(Value::as_u64) {
        Some(n) => format!(
            "{}; look it up with {lookup}",
            pending_hint(state, n, &hash)
        ),
        None => format!("{state}; look it up with {lookup} before sending again"),
    };
    out.insert("hint".into(), Value::from(hint));
}

/// Network-side operations, run by the one-shot CLI process (no key access).
#[allow(async_fn_in_trait)]
pub trait ChainFamily: FamilyKeys {
    /// Validates and normalizes a user-supplied address (`INVALID_ADDRESS` on failure).
    fn normalize_address(&self, input: &str) -> Result<String>;
    /// Checks a `sign_message` result really is `expected`'s signature over `message`.
    fn verify_message(&self, message: &[u8], signed: &Value, expected: &str) -> Result<()>;
    /// Decodes a `sign_tx` result from its raw bytes and recovers the signer.
    fn inspect_signed(&self, chain: &Chain, signed: &Value) -> Result<SignedTx>;
    /// Arbitrary contract reads and writes (`call` / `invoke`).
    fn supports_contract_calls(&self) -> bool {
        false
    }
    async fn native_balance(&self, net: &Net, owner: &str) -> Result<Balance>;
    async fn token_balance(&self, net: &Net, owner: &str, token: &str) -> Result<Balance>;
    /// The native coin and every registry token of `net`'s chain, in registry order, zero
    /// balances included (`balance` without `--chain`), all read by `limit` from the call.
    /// Symbols and decimals are the registry's; no contract is asked for them. A failed or late
    /// token read is that token's error; the call fails only when the endpoint, its chain id or
    /// the native balance does.
    async fn wallet_balances(
        &self,
        net: &Net,
        owner: &str,
        limit: Duration,
    ) -> Result<(Balance, Vec<TokenBalance>)>;
    /// Builds an unsigned transfer from live chain state. Outside dry-run this fails with
    /// `INSUFFICIENT_FUNDS` instead of returning a transaction that cannot pay for itself.
    async fn prepare_transfer(&self, net: &Net, req: &TransferRequest) -> Result<PreparedTx>;
    /// Hands `signed` to the node, calling `record.write()` right before the send request (a
    /// failure there sends nothing). A failure before the send is [`before_send_error`], and the
    /// node refusing the transaction itself is [`rejected_send_error`]; both say nothing was
    /// broadcast. A used nonce that is not this transaction's is [`nonce_taken_error`]
    /// (`explicit_nonce` = the send was a `--nonce` retry). Once the request has gone out, any
    /// other failure whose outcome the node cannot confirm is `BROADCAST_UNCERTAIN` (see
    /// [`after_send_error`]). Every error after the journal line was written names it
    /// (`journal`).
    async fn broadcast(
        &self,
        net: &Net,
        signed: &SignedTx,
        explicit_nonce: bool,
        record: &SendRecord,
    ) -> Result<Broadcast>;
    /// `Ok(None)` when no receipt appeared within `timeout`.
    async fn wait_for_receipt(
        &self,
        net: &Net,
        tx_hash: &str,
        timeout: Duration,
    ) -> Result<Option<Value>>;
    /// `zwallet tx`: the transaction `tx_hash` as the node reports it, `{ chain, txHash, found,
    /// status: "mined" | "pending" | "not found", from, to, nonce, value, valueRaw, symbol,
    /// explorerUrl, blockNumber?, gasUsed?, success?, hint? }`. An answer about another
    /// transaction is `RPC_ERROR`, never "not found".
    async fn lookup_tx(&self, net: &Net, tx_hash: &str) -> Result<Map<String, Value>>;
}

/// `CAPABILITY_UNSUPPORTED` unless the chain's family supports contract calls.
pub fn require_contract_calls(chain: &Chain) -> Result<()> {
    let supported = match &chain.family {
        Family::Evm(_) => evm::Evm.supports_contract_calls(),
    };
    if supported {
        Ok(())
    } else {
        Err(WalletError::new(
            ErrorCode::CapabilityUnsupported,
            format!(
                "{} ({} family) does not support contract calls",
                chain.key,
                chain.family.name()
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::seed::{Seed, TEST_MNEMONIC};
    use zeroize::Zeroizing;

    #[test]
    fn find_is_case_insensitive_and_rejects_unknown() {
        assert_eq!(find("Base").unwrap().chain_id_json(), Value::from(8453));
        assert_eq!(
            find("solana").unwrap_err().code,
            ErrorCode::ChainUnsupported
        );
    }

    #[test]
    fn sepolia_is_the_only_testnet() {
        let testnets: Vec<&str> = CHAINS
            .iter()
            .filter(|c| c.is_testnet())
            .map(|c| c.key)
            .collect();
        assert_eq!(testnets, ["sepolia"]);
        assert!(TESTNETS.iter().all(|k| find(k).is_ok()));
    }

    #[test]
    fn rpc_precedence_flag_env_default() {
        let c = find("ethereum").unwrap();
        assert_eq!(c.rpc_env_var(), "ZWALLET_RPC_ETHEREUM");
        assert_eq!(
            c.resolve_rpc(Some("http://flag"), Some("http://env")),
            "http://flag"
        );
        assert_eq!(c.resolve_rpc(None, Some("http://env")), "http://env");
        assert_eq!(c.resolve_rpc(Some(" "), None), c.default_rpc());
    }

    #[test]
    fn keys_and_chain_ids_are_unique() {
        let mut keys: Vec<&str> = CHAINS.iter().map(|c| c.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), CHAINS.len());
        let mut ids: Vec<String> = CHAINS
            .iter()
            .map(|c| c.chain_id_json().to_string())
            .collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), CHAINS.len());
    }

    #[test]
    fn rpc_urls_print_scheme_and_host_only() {
        for (url, shown) in [
            (
                "https://ethereum-rpc.publicnode.com",
                "https://ethereum-rpc.publicnode.com",
            ),
            ("http://127.0.0.1:8545", "http://127.0.0.1:8545"),
            ("https://host.example/", "https://host.example/"),
            (
                "https://eth-mainnet.g.alchemy.com/v2/KEY",
                "https://eth-mainnet.g.alchemy.com/<redacted>",
            ),
            (
                "https://host.example?apikey=KEY",
                "https://host.example/<redacted>",
            ),
            (
                "https://host.example/#KEY",
                "https://host.example/<redacted>",
            ),
            (
                "https://user:KEY@host.example:8545",
                "https://host.example:8545/<redacted>",
            ),
            ("not a url KEY", "<redacted>"),
            // The HTTP client reads a backslash as a path separator.
            (
                "https://h.example\\v2\\KEYKEYKEY",
                "https://h.example/<redacted>",
            ),
            (
                "https://u%40x:KEY@h.example",
                "https://h.example/<redacted>",
            ),
        ] {
            assert_eq!(redact_url(url), shown, "{url}");
        }
        let text = "error sending request for url (https://h.example/v3/KEY?x=KEY2): refused; \
                    retry http://127.0.0.1:1/KEY3 or \"https://a.example\"";
        let clean = redact_urls(text);
        assert!(!clean.contains("KEY"), "{clean}");
        assert_eq!(
            clean,
            "error sending request for url (https://h.example/<redacted>): refused; retry \
             http://127.0.0.1:1/<redacted> or \"https://a.example\""
        );
        assert_eq!(redact_urls("no urls here"), "no urls here");
    }

    #[test]
    fn key_like_host_labels_are_redacted_and_default_hosts_stay_readable() {
        // Every default endpoint prints as it is, including the 21- and 20-character labels
        // `avalanche-c-chain-rpc` and `ethereum-sepolia-rpc`.
        for chain in CHAINS {
            assert_eq!(redact_url(chain.default_rpc()), chain.default_rpc());
            assert_eq!(url_origin(chain.default_rpc()), chain.default_rpc());
            assert!(url_secrets(chain.default_rpc()).is_empty(), "{}", chain.key);
        }
        for (url, shown) in [
            // 20+ characters in one run.
            (
                "https://abcdefghijklmnopqrstu.rpc.example",
                "https://<redacted>.rpc.example",
            ),
            // 12+ characters with letters and digits (a hex key, a UUID).
            (
                "https://deadbeef0123.eth.example:8545/v2/x",
                "https://<redacted>.eth.example:8545/<redacted>",
            ),
            (
                "https://123e4567-e89b-12d3-a456-426614174000.rpc.example",
                "https://<redacted>.rpc.example",
            ),
            // Short or letters-only labels stay.
            ("https://mainnet.infura.io", "https://mainnet.infura.io"),
            (
                "https://eth-mainnet.g.alchemy.com",
                "https://eth-mainnet.g.alchemy.com",
            ),
            ("https://node1.example", "https://node1.example"),
            ("http://127.0.0.1:8545", "http://127.0.0.1:8545"),
        ] {
            assert_eq!(redact_url(url), shown, "{url}");
        }
        assert_eq!(
            url_origin("https://deadbeef0123.eth.example/v2/x"),
            "https://<redacted>.eth.example"
        );
        assert_eq!(
            redact_urls("refused by https://deadbeef0123.eth.example (x)"),
            "refused by https://<redacted>.eth.example (x)"
        );
        // The label alone, echoed by the endpoint, is scrubbed too.
        let net = Net {
            chain: find("ethereum").unwrap(),
            rpc_url: "https://deadbeef0123.eth.example".into(),
        };
        assert_eq!(
            net.redact("project deadbeef0123 is over quota"),
            "project <redacted> is over quota"
        );
    }

    #[test]
    fn net_debug_prints_the_redacted_url() {
        let net = Net {
            chain: find("base").unwrap(),
            rpc_url: "https://base.example.com/v2/SECRETKEY1?k=SECRETKEY2".into(),
        };
        let shown = format!("{net:?}");
        assert!(!shown.contains("SECRETKEY"), "{shown}");
        assert!(
            shown.contains("https://base.example.com/<redacted>"),
            "{shown}"
        );
    }

    #[test]
    fn url_secrets_cover_credentials_query_values_and_long_path_segments() {
        assert_eq!(
            url_secrets("https://u:PASSWORD1@h.example/v2/KEY12345?apikey=QKEY5678&x=1#frag"),
            vec![
                "apikey=QKEY5678&x=1",
                "/v2/KEY12345",
                "u:PASSWORD1",
                "PASSWORD1",
                "KEY12345",
                "QKEY5678",
                "1",
                "u",
            ]
        );
        // Nothing after the host: nothing to scrub. Short path segments only inside the whole
        // path; short credential slots (userinfo, query values) on their own.
        assert!(url_secrets("https://ethereum-rpc.publicnode.com").is_empty());
        assert!(url_secrets("http://127.0.0.1:8545/").is_empty());
        assert_eq!(url_secrets("https://h.example/v2/short"), vec!["/v2/short"]);
        assert_eq!(
            url_secrets("https://h.example/?apikey=abc123"),
            vec!["apikey=abc123", "abc123"]
        );
        assert_eq!(url_secrets(" not a url KEY "), vec!["not a url KEY"]);
        // Percent-encoded slots also count in decoded form ('+' in a query is a space too).
        let s = url_secrets("https://us%40r:p%3Ass@h.example/v3/abc%2Fdef01?k=a%26b+c");
        for want in [
            "us%40r",
            "us@r",
            "p%3Ass",
            "p:ss",
            "abc%2Fdef01",
            "abc/def01",
            "a&b c",
        ] {
            assert!(s.iter().any(|x| x == want), "{want} missing from {s:?}");
        }
    }

    #[test]
    fn net_redact_scrubs_decoded_and_short_credentials_only_as_whole_words() {
        let net = |url: &str| Net {
            chain: find("ethereum").unwrap(),
            rpc_url: url.into(),
        };
        // A percent-encoded key the endpoint echoes decoded.
        let n = net("https://h.example/v3/abc%2Bdef0123456");
        assert_eq!(
            n.redact("invalid key abc+def0123456 for project"),
            "invalid key <redacted> for project"
        );
        // A short query value echoed bare.
        let n = net("https://h.example/?apikey=abc123&chain=1");
        assert_eq!(
            n.redact("key abc123 is not valid at block 1234 (chain 1)"),
            "key <redacted> is not valid at block 1234 (chain <redacted>)"
        );
        // A userinfo password with a percent escape.
        let n = net("https://user:p%40ss@h.example");
        assert_eq!(n.redact("bad password p@ss"), "bad password <redacted>");
        // A short fragment is never sent, so it scrubs nothing.
        let n = net("https://h.example/#a");
        assert_eq!(n.redact("a bad answer"), "a bad answer");
    }

    #[test]
    fn net_redact_scrubs_an_echoed_key() {
        let net = Net {
            chain: find("ethereum").unwrap(),
            rpc_url: "https://eth.example.com/v2/abcdef0123456789".into(),
        };
        let text = "HTTP error 403 with body: key abcdef0123456789 is not allowed on \
                    https://eth.example.com/v2/abcdef0123456789 (path /v2/abcdef0123456789)";
        assert_eq!(
            net.redact(text),
            "HTTP error 403 with body: key <redacted> is not allowed on \
             https://eth.example.com/<redacted> (path <redacted>)"
        );
        let public = Net {
            chain: find("ethereum").unwrap(),
            rpc_url: "https://ethereum-rpc.publicnode.com".into(),
        };
        assert_eq!(public.redact("nonce too low"), "nonce too low");
    }

    #[test]
    fn rejected_send_error_says_nothing_was_broadcast_with_hash_nonce_from() {
        let signed = SignedTx {
            raw: "0x02".into(),
            tx_hash: "0xabc".into(),
            signer: "0xF00".into(),
            nonce: Some(4),
        };
        let e = rejected_send_error(&signed, "transaction underpriced", Some("Hint"));
        assert!(e.message.ends_with(". Hint"), "{}", e.message);
        let v = e.to_json();
        assert_eq!(v["errorCode"], "RPC_ERROR");
        assert_eq!(v["txHash"], "0xabc");
        assert_eq!(v["nonce"], 4);
        assert_eq!(v["from"], "0xF00");
        assert!(
            e.message
                .starts_with("the node rejected the transaction; nothing was broadcast"),
            "{}",
            e.message
        );
        assert!(e.message.contains("pass --nonce 4"), "{}", e.message);
        assert!(!e.message.contains("may have reached"), "{}", e.message);

        let before = before_send_error(&signed, WalletError::rpc("connect failed"));
        assert_eq!(before.to_json()["errorCode"], "RPC_ERROR");
        assert_eq!(before.to_json()["nonce"], 4);
        assert!(
            before
                .message
                .starts_with("connect failed; nothing was broadcast")
        );
        assert!(
            before.message.contains("pass --nonce 4"),
            "{}",
            before.message
        );
    }

    fn temp_home() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("zwt-")
            .tempdir_in("/tmp")
            .unwrap()
    }

    #[test]
    fn nonce_taken_names_the_refused_hash_and_sends_to_the_earlier_send() {
        let home = temp_home();
        let chain = find("ethereum").unwrap();
        let journal = SendJournal::new(home.path(), chain, "0xF00");
        // An earlier send at nonce 4, then the refused retry's own line.
        let earlier = format!("0x{:064x}", 0xea1);
        journal
            .append(
                &json!({ "nonce": 4, "txHash": earlier, "ts": "t0", "kind": "transfer",
                             "value": "1 ETH", "to": "0xB0B" }),
            )
            .unwrap();
        journal
            .append(&json!({ "nonce": 4, "txHash": "0xabc", "ts": "t1" }))
            .unwrap();
        let signed = SignedTx {
            raw: "0x02".into(),
            tx_hash: "0xabc".into(),
            signer: "0xF00".into(),
            nonce: Some(4),
        };
        let fresh = nonce_taken_error(&signed, "nonce too low", false, None, Some(&journal));
        assert_eq!(fresh.code, ErrorCode::RpcError);
        assert!(
            fresh.message.starts_with(
                "nonce 4 is taken by another transaction from this key, pending or mined"
            ),
            "{}",
            fresh.message
        );
        assert!(
            fresh.message.contains(
                "If you were retrying an earlier send, look up that send's txHash before \
                 sending again"
            ),
            "{}",
            fresh.message
        );
        let retry = nonce_taken_error(&signed, "nonce too low", true, Some("Hint"), Some(&journal));
        assert_eq!(retry.code, ErrorCode::BroadcastUncertain);
        assert!(retry.message.ends_with(". Hint"), "{}", retry.message);
        for part in [
            "This retry was refused and will not be found",
            "look up the txHash of your earlier send at nonce 4",
            "do not send this payment without --nonce",
            "--nonce 4 --replace <earlier txHash>",
        ] {
            assert!(retry.message.contains(part), "{part}: {}", retry.message);
        }
        assert!(!retry.message.contains("next nonce"), "{}", retry.message);
        for e in [&fresh, &retry] {
            let v = e.to_json();
            // The refused transaction is never `txHash`: it will not be found.
            assert!(v.get("txHash").is_none(), "{v}");
            assert_eq!(v["refusedTxHash"], "0xabc");
            assert_eq!(v["nonce"], 4);
            assert_eq!(v["journal"], journal.display());
            // The journal's earlier send is named; the refused one is not offered as it.
            assert!(e.message.contains(&earlier), "{}", e.message);
            assert!(
                !e.message.contains("0xabc ("),
                "refused hash listed: {}",
                e.message
            );
        }
    }

    #[test]
    fn send_journal_appends_fsynced_lines_in_a_0600_file() {
        let home = temp_home();
        let chain = find("base").unwrap();
        let journal = SendJournal::new(home.path(), chain, "0xAbC");
        assert_eq!(
            journal.path(),
            home.path().join("sends-8453-0xabc.jsonl").as_path()
        );
        assert!(journal.entries_at(0..=9, None).unwrap().is_empty());
        // Transaction hashes as alloy writes them: 32 bytes of hex.
        let (h11, h22) = (format!("0x{:064x}", 0x11), format!("0x{:064x}", 0x22));
        let signed = SignedTx {
            raw: "0x02".into(),
            tx_hash: h11.clone(),
            signer: "0xAbC".into(),
            nonce: Some(3),
        };
        let summary = SendSummary {
            kind: "transfer".into(),
            to: "0xB0B".into(),
            value: "0.5 ETH".into(),
            amount_raw: Some("500000000000000000".into()),
            token: None,
        };
        let record = SendRecord::new(journal.clone(), &signed, &summary);
        record.write().unwrap();
        journal
            .append(&json!({ "nonce": 5, "txHash": h22 }))
            .unwrap();
        let mode = std::fs::metadata(journal.path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        let text = std::fs::read_to_string(journal.path()).unwrap();
        assert_eq!(text.lines().count(), 2);
        let first: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        for (k, want) in [
            ("chain", json!("base")),
            ("from", json!("0xAbC")),
            ("nonce", json!(3)),
            ("txHash", json!(h11)),
            ("to", json!("0xB0B")),
            ("value", json!("0.5 ETH")),
            ("amountRaw", json!("500000000000000000")),
            ("kind", json!("transfer")),
        ] {
            assert_eq!(first[k], want, "{k}: {first}");
        }
        assert!(first["ts"].as_str().unwrap().ends_with('Z'), "{first}");
        assert_eq!(journal.entries_at(3..=3, None).unwrap().len(), 1);
        assert_eq!(journal.entries_at(3..=5, None).unwrap().len(), 2);
        assert!(
            journal
                .entries_at(3..=5, Some(&h11.to_uppercase()))
                .unwrap()
                .len()
                == 1
        );
        let (text, _) = journal.pointer(4..=4, None);
        assert!(text.contains("has no entry at nonce 4"), "{text}");
        assert!(
            text.contains("https://basescan.org/address/0xAbC"),
            "{text}"
        );
        let (text, entries) = journal.pointer(3..=5, None);
        assert_eq!(entries.len(), 2);
        assert!(
            text.contains(&format!("at nonces 3 to 5: {h11} (nonce 3")),
            "{text}"
        );
        // A symlink in its place is never followed.
        let other = SendJournal::new(home.path(), chain, "0xdef");
        std::os::unix::fs::symlink(home.path().join("target"), other.path()).unwrap();
        assert!(other.append(&json!({})).is_err());
        assert!(!home.path().join("target").exists());
    }

    #[test]
    fn recent_duplicates_match_the_send_inside_the_window_only() {
        use crate::core::time::rfc3339_utc;
        let home = temp_home();
        let chain = find("ethereum").unwrap();
        let journal = SendJournal::new(home.path(), chain, "0xF00");
        let now = 1_800_000_000;
        let usdc = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48";
        // A transaction hash, as the journal holds one: 32 bytes of hex.
        let h = |n: u8| format!("0x{n:064x}");
        let entry = |hash: &str, ago: i64, kind: &str, value: &str, token: Option<&str>| {
            let mut e = json!({
                "ts": rfc3339_utc((now as i64 - ago) as u64), "chain": "ethereum",
                "nonce": 1, "txHash": hash, "to": "0xb0b", "value": value, "kind": kind,
            });
            if let Some(t) = token {
                e["token"] = json!(t);
            }
            e
        };
        for e in [
            entry(&h(1), 600, "transfer", "1 ETH", None), // at the edge: in
            entry(&h(2), 601, "transfer", "1 ETH", None), // just past it: out
            entry(&h(3), -30, "transfer", "1 ETH", None), // clock set back: in
            entry(&h(4), 5, "transfer", "2 ETH", None),
            entry(&h(5), 5, "invoke", "1 ETH", None),
            entry(&h(6), 5, "token-transfer", "1 USDC", Some(usdc)),
            entry(&h(7), 5, "token-transfer", "1 USDC", Some("0x1111")),
            json!({ "ts": rfc3339_utc(now), "chain": "base", "to": "0xb0b", "value": "1 ETH",
                    "kind": "transfer", "txHash": h(8) }),
        ] {
            journal.append(&e).unwrap();
        }
        let hashes = |send: &SendSummary| -> Vec<String> {
            journal
                .recent_duplicates(send, now)
                .unwrap()
                .iter()
                .map(|e| e["txHash"].as_str().unwrap().to_string())
                .collect()
        };
        let native = SendSummary {
            kind: "transfer".into(),
            to: "0xB0B".into(),
            value: "1 ETH".into(),
            amount_raw: None,
            token: None,
        };
        assert_eq!(hashes(&native), [h(1), h(3)]);
        let token = SendSummary {
            kind: "token-transfer".into(),
            value: "1 USDC".into(),
            token: Some(usdc.to_lowercase()),
            ..native.clone()
        };
        assert_eq!(hashes(&token), [h(6)]);
        let e = journal.duplicate_error(journal.recent_duplicates(&native, now).unwrap());
        assert_eq!(e.code, ErrorCode::ValidationError);
        assert!(
            e.message.starts_with(
                "this looks like a repeat of a recent send; look those hashes up first. If this \
                 really is a second payment, pass --allow-duplicate."
            ),
            "{}",
            e.message
        );
        assert!(
            e.message.contains(&format!("{} (nonce 1, sent ", h(1))),
            "{}",
            e.message
        );
        let v = e.to_json();
        assert_eq!(v["duplicateSends"].as_array().unwrap().len(), 2);
        assert_eq!(v["journal"], journal.display());
    }

    #[test]
    fn journal_fields_show_only_in_the_shape_zwallet_writes() {
        // The journal is a file any process of this user can write. Five entries use every field
        // to try to draw or forge a line: spaces and points in the amount, a kind zwallet never
        // writes, a time that is not RFC 3339, a long run of digits, and terminal escapes. Each
        // such field shows as `<invalid>`, and the rest show as zwallet writes them.
        use crate::core::text::INVALID;
        let home = temp_home();
        let chain = find("ethereum").unwrap();
        let journal = SendJournal::new(home.path(), chain, "0xF00");
        let h = |n: u8| format!("0x{n:064x}");
        let to = "0x000000000000000000000000000000000000dEaD";
        let lines = [
            json!({ "nonce": 1, "txHash": h(1), "ts": "2026-10-20T04:05:06Z", "kind": "transfer",
                    "value": "## ## ## ETH", "to": to }),
            json!({ "nonce": 1, "txHash": h(2), "ts": "2026-10-20T04:05:06Z", "kind": "invoke",
                    "value": "1.2.3 ETH", "to": to }),
            json!({ "nonce": 1, "txHash": h(3), "ts": "2026-10-20T04:05:06Z",
                    "kind": "xfer\u{1b}[2J", "value": "0.5 USD\u{20ae}0", "to": to }),
            json!({ "nonce": 2, "txHash": h(4), "ts": "2026-10-20 04:05:06",
                    "kind": "token-transfer", "value": "1 USDC.e", "to": "0x## ##" }),
            json!({ "nonce": 2, "txHash": "\u{1b}[2J", "ts": "\r\nverified: true",
                    "kind": "transfer", "value": format!("{} ETH", "8".repeat(200)), "to": to }),
        ];
        for line in &lines {
            journal.append(line).unwrap();
        }
        let printable = |s: &str| {
            s.chars()
                .all(|c| (' '..='~').contains(&c) || c == '\u{20ae}')
        };
        // Three at nonce 1 and two at nonce 2, so each message shows all of its own.
        let (one, entries) = journal.pointer(1..=1, None);
        let (two, _) = journal.pointer(2..=2, None);
        let text = format!("{one}\n{two}");
        // Spaces in an amount, points in an amount, a bogus kind, a non-RFC 3339 time, an
        // address with spaces, a 200-digit amount and a hash that is none: never their own text.
        for bad in [
            "## ##",
            "1.2.3",
            "xfer",
            "2026-10-20 04",
            "0x## ##",
            "88888888888",
            "[2J",
        ] {
            assert!(!text.contains(bad), "{bad}: {text}");
        }
        assert_eq!(text.matches(INVALID).count(), 7, "{text}");
        // What zwallet writes shows as it is: a time, a kind, `USD₮0` and `USDC.e`.
        for good in [
            "sent 2026-10-20T04:05:06Z: transfer <invalid> to 0x000000000000000000000000000000000000dEaD",
            "0.5 USD\u{20ae}0",
            "token-transfer 1 USDC.e to <invalid>",
            "? (nonce 2, sent <invalid>: transfer <invalid> to 0x",
        ] {
            assert!(text.contains(good), "{good}: {text}");
        }
        for text in [&one, &two] {
            assert!(printable(text), "{text:?}");
            assert!(!text.contains("more there"), "{text}");
            // One lookup hint for all of them, with a placeholder, not the journal's hash.
            assert!(
                text.ends_with(
                    "; look each hash up with `zwallet tx --chain ethereum <txHash>` or \
                     https://etherscan.io/tx/<txHash>"
                ),
                "{text}"
            );
        }
        // `--json` keeps every entry's own data, escaped, not `<invalid>`.
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0]["value"], "## ## ## ETH");
        assert_eq!(entries[2]["kind"], "xfer\\u{1b}[2J");
    }

    #[test]
    fn journal_text_in_one_message_is_one_budget_of_whole_entries() {
        // Five entries at the most each field allows (a 20-digit nonce, an amount of 78 + 78
        // digits and a 32-character symbol) are 352 characters of journal text each: one fits
        // the 512 budget whole and the other four are counted, never cut.
        use crate::core::text::PROSE_MAX;
        let home = temp_home();
        let chain = find("ethereum").unwrap();
        let h = |n: u8| format!("0x{n:064x}");
        let to = "0x000000000000000000000000000000000000dEaD";
        let printable = |s: &str| {
            s.chars()
                .all(|c| (' '..='~').contains(&c) || c == '\u{20ae}')
        };
        let big = SendJournal::new(home.path(), chain, "0xB16");
        let amount = format!("{}.{} {}", "9".repeat(78), "9".repeat(78), "A".repeat(32));
        for n in 1..=5u8 {
            big.append(&json!({
                "nonce": u64::MAX, "txHash": h(n), "ts": "2026-10-20T04:05:06Z",
                "kind": "token-transfer", "value": amount, "to": to,
            }))
            .unwrap();
        }
        let (text, _) = big.pointer(u64::MAX..=u64::MAX, None);
        assert_eq!(text.matches(" (nonce ").count(), 1, "{text}");
        assert!(
            text.contains(&h(5)) && text.contains(" (and 4 more there)"),
            "{text}"
        );
        let one = 66 + 20 + 20 + "token-transfer".len() + amount.len() + 42;
        assert!(one <= PROSE_MAX && 2 * one > PROSE_MAX, "{one}");
        // The same five in the duplicate refusal: hash, nonce and time are 106 characters each,
        // so four fit (an ordinary nonce of a few digits lets all five fit).
        let e = big.duplicate_error(big.entries_at(0..=u64::MAX, None).unwrap());
        assert_eq!(e.message.matches(" (nonce ").count(), 4, "{}", e.message);
        assert!(e.message.contains(" (and 1 more there)"), "{}", e.message);
        assert!(printable(&e.message), "{:?}", e.message);
        assert!(
            !e.message.contains("zwallet tx --chain ethereum 0x"),
            "{}",
            e.message
        );
        let json = e.to_json().to_string();
        assert!(json.bytes().all(|b| (0x20..=0x7e).contains(&b)), "{json}");
    }

    #[test]
    fn recent_duplicates_key_on_base_units_not_on_the_display_value() {
        use crate::core::time::rfc3339_utc;
        let home = temp_home();
        let chain = find("ethereum").unwrap();
        let journal = SendJournal::new(home.path(), chain, "0xF00");
        let now = 1_800_000_000;
        let usdc = "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48";
        let line = |hash: &str, kind: &str, value: &str, raw: Option<&str>, token: Option<&str>| {
            let mut e = json!({
                "ts": rfc3339_utc(now - 5), "chain": "ethereum", "nonce": 1, "txHash": hash,
                "to": "0xb0b", "value": value, "kind": kind,
            });
            if let Some(r) = raw {
                e["amountRaw"] = json!(r);
            }
            if let Some(t) = token {
                e["token"] = json!(t);
            }
            e
        };
        for e in [
            // Written while symbol() answered.
            line(
                "0x01",
                "token-transfer",
                "0.5 USDC",
                Some("500000"),
                Some(usdc),
            ),
            // Same display text, other base units (a token with other decimals): not the same.
            line("0x02", "token-transfer", "0.5 USDC", Some("5"), Some(usdc)),
            // Native: the wei decide.
            line(
                "0x03",
                "transfer",
                "1 ETH",
                Some("1000000000000000000"),
                None,
            ),
            // A line from before amountRaw was journaled: its value text decides.
            line("0x04", "transfer", "2 ETH", None, None),
        ] {
            journal.append(&e).unwrap();
        }
        let hashes = |send: &SendSummary| -> Vec<String> {
            journal
                .recent_duplicates(send, now)
                .unwrap()
                .iter()
                .map(|e| e["txHash"].as_str().unwrap().to_string())
                .collect()
        };
        let send = |kind: &str, value: &str, raw: &str, token: Option<&str>| SendSummary {
            kind: kind.into(),
            to: "0xB0B".into(),
            value: value.into(),
            amount_raw: Some(raw.into()),
            token: token.map(str::to_string),
        };
        // symbol() failed this time: "0.5 ?" still matches on the token and base units.
        assert_eq!(
            hashes(&send("token-transfer", "0.5 ?", "500000", Some(usdc))),
            ["0x01"]
        );
        assert_eq!(
            hashes(&send("transfer", "1 ETH", "1000000000000000000", None)),
            ["0x03"]
        );
        assert!(hashes(&send("transfer", "1 ETH", "1000000000000000001", None)).is_empty());
        assert_eq!(
            hashes(&send("transfer", "2 ETH", "2000000000000000000", None)),
            ["0x04"]
        );
    }

    #[test]
    fn a_partial_trailing_record_does_not_swallow_the_next_send() {
        // A run killed mid-append left a record without its newline; the next send's line must
        // still be a line of its own, or the duplicate check never sees it.
        let home = temp_home();
        let chain = find("ethereum").unwrap();
        let journal = SendJournal::new(home.path(), chain, "0xF00");
        let fragment = r#"{"ts":"2026-10-20T00:00:00Z","chain":"ethe"#;
        std::fs::write(journal.path(), fragment).unwrap();
        let signed = SignedTx {
            raw: "0x02".into(),
            tx_hash: "0x77".into(),
            signer: "0xF00".into(),
            nonce: Some(9),
        };
        let summary = SendSummary {
            kind: "transfer".into(),
            to: "0xB0B".into(),
            value: "1 ETH".into(),
            amount_raw: None,
            token: None,
        };
        SendRecord::new(journal.clone(), &signed, &summary)
            .write()
            .unwrap();
        let now = crate::core::time::unix_secs(std::time::SystemTime::now());
        let found = journal.recent_duplicates(&summary, now).unwrap();
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0]["txHash"], "0x77");
        // The fragment stays as a line of its own (skipped as malformed), then the new record.
        let text = std::fs::read_to_string(journal.path()).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "{text:?}");
        assert_eq!(lines[0], fragment);
        assert!(serde_json::from_str::<Value>(lines[1]).is_ok(), "{text:?}");
        assert!(text.ends_with('\n'));
        // A journal that ends in a newline gets nothing extra.
        SendRecord::new(journal.clone(), &signed, &summary)
            .write()
            .unwrap();
        let text = std::fs::read_to_string(journal.path()).unwrap();
        assert_eq!(text.lines().count(), 3, "{text:?}");
        assert!(!text.contains("\n\n"), "{text:?}");
    }

    #[test]
    fn a_journal_that_is_not_a_regular_file_fails_closed_without_blocking() {
        let home = temp_home();
        let chain = find("ethereum").unwrap();
        let journal = SendJournal::new(home.path(), chain, "0xF00");
        // Read and append on another thread: a blocking open would never return.
        let check = |journal: &SendJournal| -> (Result<Vec<Value>>, Result<()>) {
            let j = journal.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send((j.entries(), j.append(&json!({ "nonce": 1 }))));
            });
            rx.recv_timeout(std::time::Duration::from_secs(10))
                .expect("the journal open blocked")
        };
        let c = std::ffi::CString::new(journal.path().as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: mkfifo with a valid NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        let (read, append) = check(&journal);
        let e = read.unwrap_err();
        assert!(e.message.contains("is not a regular file"), "{}", e.message);
        assert!(e.message.contains(&journal.display()), "{}", e.message);
        assert!(append.is_err());
        // The duplicate check refuses rather than seeing no earlier sends; the pointer in an
        // error message says it cannot read the journal.
        let summary = SendSummary {
            kind: "transfer".into(),
            to: "0xB0B".into(),
            value: "1 ETH".into(),
            amount_raw: None,
            token: None,
        };
        let e = journal.recent_duplicates(&summary, 0).unwrap_err();
        assert!(
            e.message.ends_with("so nothing was signed"),
            "{}",
            e.message
        );
        assert_eq!(e.to_json()["journal"], journal.display());
        let (text, entries) = journal.pointer(4..=4, None);
        assert!(text.contains("cannot read the send journal"), "{text}");
        assert!(
            text.contains("https://etherscan.io/address/0xF00"),
            "{text}"
        );
        assert!(entries.is_empty());
        // A directory, or a symlink (even to a good journal), is refused the same way.
        std::fs::remove_file(journal.path()).unwrap();
        std::fs::create_dir(journal.path()).unwrap();
        let (read, append) = check(&journal);
        assert!(read.is_err() && append.is_err());
        std::fs::remove_dir(journal.path()).unwrap();
        let real = home.path().join("real.jsonl");
        std::fs::write(&real, "{\"nonce\":1}\n").unwrap();
        std::os::unix::fs::symlink(&real, journal.path()).unwrap();
        let (read, append) = check(&journal);
        assert!(
            read.unwrap_err().message.contains("symbolic link"),
            "symlink followed"
        );
        assert!(append.is_err());
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "{\"nonce\":1}\n");
        // No journal yet: no entries, not an error.
        std::fs::remove_file(journal.path()).unwrap();
        assert!(journal.entries().unwrap().is_empty());
    }

    #[test]
    fn the_journal_is_read_from_its_last_mebibyte_skipping_malformed_lines() {
        let home = temp_home();
        let chain = find("ethereum").unwrap();
        let journal = SendJournal::new(home.path(), chain, "0xF00");
        let line = |n: u64| json!({ "nonce": n, "txHash": format!("0x{n}") }).to_string();
        // An entry, then more than the tail of filler, then malformed lines and an entry.
        let filler = format!("{}\n", "x".repeat(1023)).repeat(1100);
        let text = format!(
            "{}\n{filler}not json\n[1]\n\u{fffd}\n{}\n",
            line(1),
            line(2)
        );
        std::fs::write(journal.path(), text).unwrap();
        let found: Vec<u64> = journal
            .entries()
            .unwrap()
            .iter()
            .map(|e| e["nonce"].as_u64().unwrap())
            .collect();
        assert_eq!(found, [2]);
        // A short journal is read whole, its first line included.
        std::fs::write(journal.path(), format!("{}\n{}", line(1), line(2))).unwrap();
        assert_eq!(journal.entries().unwrap().len(), 2);
    }

    #[test]
    fn receipt_outcomes_never_fail_a_sent_transaction() {
        let sent = || {
            let mut m = Map::new();
            m.insert("txHash".into(), Value::from("0xab"));
            m.insert("nonce".into(), Value::from(7));
            m
        };
        let base = find("base").unwrap();
        let mut timed_out = sent();
        record_receipt(&mut timed_out, base, Ok(None));
        assert_eq!(timed_out["status"], "pending");
        assert_eq!(timed_out["nonce"], 7);
        assert_eq!(
            timed_out["hint"],
            "still pending; to speed up or cancel use --nonce 7 --replace 0xab; a run without \
             --nonce is a second payment; look it up with `zwallet tx --chain base 0xab` or \
             https://basescan.org/tx/0xab"
        );
        let mut failed = sent();
        record_receipt(&mut failed, base, Err(WalletError::rpc("boom")));
        assert_eq!(failed["status"], "unknown");
        assert_eq!(failed["receiptError"], "boom");
        assert_eq!(failed["nonce"], 7);
        let hint = failed["hint"].as_str().unwrap();
        assert!(
            hint.starts_with("receipt unknown, it may still be pending"),
            "{hint}"
        );
        assert!(
            hint.ends_with(
                "to speed up or cancel use --nonce 7 --replace 0xab; a run without --nonce is a \
                 second payment; look it up with `zwallet tx --chain base 0xab` or \
                 https://basescan.org/tx/0xab"
            ),
            "{hint}"
        );
        let mut mined = sent();
        let receipt = serde_json::json!({ "status": "success", "blockNumber": 9 });
        record_receipt(&mut mined, base, Ok(Some(receipt)));
        assert_eq!(mined["status"], "success");
        assert_eq!(mined["nonce"], 7);
        assert!(mined.get("hint").is_none());
    }

    #[test]
    fn after_send_error_carries_hash_nonce_and_from() {
        let chain = find("base").unwrap();
        let signed = SignedTx {
            raw: "0x02".into(),
            tx_hash: "0xabc".into(),
            signer: "0xF00".into(),
            nonce: Some(4),
        };
        let e = after_send_error(ErrorCode::BroadcastUncertain, chain, &signed, "timed out");
        let v = e.to_json();
        assert_eq!(v["errorCode"], "BROADCAST_UNCERTAIN");
        assert_eq!(v["txHash"], "0xabc");
        assert_eq!(v["nonce"], 4);
        assert_eq!(v["from"], "0xF00");
        assert!(e.message.starts_with("timed out."), "{}", e.message);
        assert!(
            e.message.contains("retry with --nonce 4 --replace 0xabc"),
            "{}",
            e.message
        );
        assert!(
            e.message.contains(
                "look it up first (`zwallet tx --chain base 0xabc` or \
                 https://basescan.org/tx/0xabc)"
            ),
            "{}",
            e.message
        );
    }

    #[test]
    fn bump_factors() {
        for (s, v) in [
            ("1.25", 12_500),
            ("1.1", 11_000),
            ("3", 30_000),
            ("2.5", 25_000),
        ] {
            assert_eq!(parse_bump(s).unwrap(), v, "{s}");
        }
        for s in ["1.09", "3.01", "1", "0", "x", "1.12345", "-2", ""] {
            assert!(parse_bump(s).is_err(), "{s}");
        }
        assert_eq!(bump_label(DEFAULT_BUMP), "1.25x");
        assert_eq!(bump_label(20_000), "2x");
    }

    #[test]
    fn every_evm_chain_shares_one_address() {
        let seed = Seed::from_phrase(Zeroizing::new(TEST_MNEMONIC.into())).unwrap();
        let addrs = derive_addresses(&seed, 0).unwrap();
        assert_eq!(addrs.len(), CHAINS.len());
        for a in addrs.values() {
            assert_eq!(a, "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
        }
        let second = derive_addresses(&seed, 1).unwrap();
        assert!(
            second
                .values()
                .all(|a| a == "0x70997970C51812dc3A010C7d01b50e0d17dc79C8")
        );
        assert!(require_contract_calls(find("ethereum").unwrap()).is_ok());
    }
}
