//! Chain registry and the chain-family boundary.
//!
//! A family (EVM today; Solana and TON later) owns key derivation from the shared seed, signing,
//! and its chain I/O. The split is deliberate: [`FamilyKeys`] is everything the unlock daemon may
//! do (no network), [`ChainFamily`] adds what the one-shot CLI does over RPC.

pub mod evm;

use crate::core::error::{ErrorCode, Result, WalletError};
use crate::core::seed::Seed;
use alloy::primitives::U256;
use alloy::transports::http::reqwest::Url;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fmt;
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
    /// Returns `{ address, signature, digest }` (family-defined encodings).
    fn sign_message(&self, seed: &Seed, index: u32, chain: &Chain, message: &[u8])
    -> Result<Value>;
    /// Signs a family-specific unsigned transaction; returns `{ rawTx, txHash }`.
    fn sign_tx(&self, seed: &Seed, index: u32, chain: &Chain, unsigned: &Value) -> Result<Value>;
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

/// The parts of an RPC URL that may carry an API key, longest first, each as written in the URL
/// and percent-decoded (`+` also as a space in the query), parsed the way the HTTP client parses
/// the URL:
/// - credential slots, at any length: the userinfo whole and each of its parts, the query string
///   whole and each parameter value;
/// - the path whole (unless empty or `/`), and each path segment of 8 or more characters;
/// - the fragment, at 8 or more characters (it is never sent, so a short one is noise).
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

/// `scheme://host[:port]` of an RPC URL as the HTTP client parses it, or `None` if it does not
/// parse or has no host.
fn origin(u: &Url) -> Option<String> {
    let host = u.host_str()?;
    Some(match u.port() {
        Some(port) => format!("{}://{host}:{port}", u.scheme()),
        None => format!("{}://{host}", u.scheme()),
    })
}

/// `scheme://host[:port]` of an RPC URL (as the HTTP client parses it), or `<redacted>` if it has
/// none: what an error about the endpoint itself may name.
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
/// `/<redacted>`. `https://host` and `https://host/` print as they are. Text that is not a URL
/// with a host is redacted whole.
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

#[derive(Debug, Clone)]
pub struct TransferRequest {
    pub from: String,
    pub to: String,
    pub amount: String,
    pub token: Option<String>,
    pub dry_run: bool,
    /// Use this account sequence number instead of the next free one (to replace a pending tx).
    pub nonce: Option<u64>,
    /// Explicit ceiling on the price per gas unit, in base units, replacing the automatic one.
    pub max_fee_per_gas: Option<u128>,
    /// Allow recipients that are almost certainly mistakes (zero address, the token itself).
    pub force: bool,
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

/// The error for a transaction that may have reached the network: it carries `txHash`, `from`
/// and `nonce` so the caller can look the transaction up, or retry with the same nonce so that
/// the retry replaces it instead of becoming a second send.
pub fn after_send_error(
    code: ErrorCode,
    chain: &Chain,
    signed: &SignedTx,
    what: &str,
) -> WalletError {
    let mut message = format!(
        "{what}. The transaction {} may have reached the network ({}); do not resend blindly: \
         look it up first",
        signed.tx_hash,
        chain.explorer_tx_url(&signed.tx_hash)
    );
    if let Some(n) = signed.nonce {
        message.push_str(&format!(
            ", or retry with --nonce {n} so that the retry replaces it instead of sending twice"
        ));
    }
    with_tx_details(WalletError::new(code, message), signed)
}

/// What to do about a retry once a transaction is signed: reuse its nonce, so that the retry can
/// only replace this transaction, never add a second one.
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
pub fn rejected_send_error(signed: &SignedTx, why: &str) -> WalletError {
    with_tx_details(
        WalletError::rpc(format!(
            "the node rejected the transaction; nothing was broadcast ({why}). {}",
            retry_hint(signed)
        )),
        signed,
    )
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
/// report this transaction by its hash: another transaction, pending or mined, holds the nonce.
/// That one may be an earlier send of this same payment, so the message never invites a plain
/// resend. `BROADCAST_UNCERTAIN` when this send was a `--nonce` retry (`explicit_nonce`), else
/// `RPC_ERROR`. `hint` is a family-specific addition.
pub fn nonce_taken_error(
    signed: &SignedTx,
    why: &str,
    explicit_nonce: bool,
    hint: Option<&str>,
) -> WalletError {
    let nonce = signed
        .nonce
        .map_or_else(|| "its nonce".to_string(), |n| format!("nonce {n}"));
    let code = if explicit_nonce {
        ErrorCode::BroadcastUncertain
    } else {
        ErrorCode::RpcError
    };
    let mut message = format!(
        "{nonce} is taken by another transaction (pending or mined), so the node did not take \
         this one ({why}); if this was a retry of an earlier send, look that txHash up before \
         sending again; otherwise re-run to take the next nonce"
    );
    if let Some(h) = hint {
        message.push_str(". ");
        message.push_str(h);
    }
    with_tx_details(WalletError::new(code, message), signed)
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

/// Adds a `--wait` result to a broadcast's output. The transaction is already sent, so a receipt
/// problem never becomes a failure (a retry would send again): no receipt in time is
/// `status: "pending"`, an RPC error is `status: "unknown"` with `receiptError`.
pub fn record_receipt(out: &mut Map<String, Value>, receipt: Result<Option<Value>>) {
    match receipt {
        Ok(Some(Value::Object(r))) => out.extend(r),
        Ok(Some(_)) | Ok(None) => {
            out.insert("status".into(), Value::from("pending"));
        }
        Err(e) => {
            out.insert("status".into(), Value::from("unknown"));
            out.insert("receiptError".into(), Value::from(e.message));
        }
    }
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
    /// Builds an unsigned transfer from live chain state. Outside dry-run this fails with
    /// `INSUFFICIENT_FUNDS` instead of returning a transaction that cannot pay for itself.
    async fn prepare_transfer(&self, net: &Net, req: &TransferRequest) -> Result<PreparedTx>;
    /// Hands `signed` to the node. A failure before the send is [`before_send_error`], and the
    /// node refusing the transaction itself is [`rejected_send_error`]; both say nothing was
    /// broadcast. A used nonce that is not this transaction's is [`nonce_taken_error`]
    /// (`explicit_nonce` = the send was a `--nonce` retry). Once the request has gone out, any
    /// other failure whose outcome the node cannot confirm is `BROADCAST_UNCERTAIN` (see
    /// [`after_send_error`]).
    async fn broadcast(
        &self,
        net: &Net,
        signed: &SignedTx,
        explicit_nonce: bool,
    ) -> Result<Broadcast>;
    /// `Ok(None)` when no receipt appeared within `timeout`.
    async fn wait_for_receipt(
        &self,
        net: &Net,
        tx_hash: &str,
        timeout: Duration,
    ) -> Result<Option<Value>>;
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
        let e = rejected_send_error(&signed, "replacement transaction underpriced");
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

    #[test]
    fn nonce_taken_is_uncertain_only_for_a_nonce_retry() {
        let signed = SignedTx {
            raw: "0x02".into(),
            tx_hash: "0xabc".into(),
            signer: "0xF00".into(),
            nonce: Some(4),
        };
        let fresh = nonce_taken_error(&signed, "nonce too low", false, None);
        assert_eq!(fresh.code, ErrorCode::RpcError);
        assert!(
            fresh
                .message
                .starts_with("nonce 4 is taken by another transaction (pending or mined)"),
            "{}",
            fresh.message
        );
        assert!(
            fresh
                .message
                .contains("look that txHash up before sending again")
        );
        let retry = nonce_taken_error(&signed, "nonce too low", true, Some("Hint"));
        assert_eq!(retry.code, ErrorCode::BroadcastUncertain);
        assert!(retry.message.ends_with(". Hint"), "{}", retry.message);
        assert_eq!(retry.to_json()["txHash"], "0xabc");
    }

    #[test]
    fn receipt_outcomes_never_fail_a_sent_transaction() {
        let sent = || {
            let mut m = Map::new();
            m.insert("txHash".into(), Value::from("0xab"));
            m.insert("nonce".into(), Value::from(7));
            m
        };
        let mut timed_out = sent();
        record_receipt(&mut timed_out, Ok(None));
        assert_eq!(timed_out["status"], "pending");
        assert_eq!(timed_out["nonce"], 7);
        let mut failed = sent();
        record_receipt(&mut failed, Err(WalletError::rpc("boom")));
        assert_eq!(failed["status"], "unknown");
        assert_eq!(failed["receiptError"], "boom");
        assert_eq!(failed["nonce"], 7);
        let mut mined = sent();
        let receipt = serde_json::json!({ "status": "success", "blockNumber": 9 });
        record_receipt(&mut mined, Ok(Some(receipt)));
        assert_eq!(mined["status"], "success");
        assert_eq!(mined["nonce"], 7);
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
        assert!(e.message.contains("--nonce 4"), "{}", e.message);
        assert!(e.message.contains("https://basescan.org/tx/0xabc"));
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
