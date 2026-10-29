//! Tron's HTTP API (`/wallet/*`, `.prd/05-tron.md` §Network access): the chain identity check,
//! an account's TRX balance, activation and owner permission, and TRC20 reads through
//! `triggerconstantcontract`. Runs in the CLI process only; the daemon never reaches this module.
//!
//! Every reply is text from outside. A value is used only in the shape expected (a balance a JSON
//! integer below 2^63, a word of exactly 32 bytes), and any of its text an error or a warning
//! shows is escaped and cut where it comes in ([`crate::core::text`]): at most 512 characters per
//! source, as every RPC endpoint's error text.

use super::{address, config, registry};
use crate::chains::evm::tx::IERC20;
use crate::chains::{Balance, Net, TokenBalance, redact_url, url_origin};
use crate::core::error::{Result, WalletError};
use crate::core::text::{PROSE_MAX, ascii_inert};
use alloy::hex;
use alloy::primitives::{Address, U256};
use alloy::sol_types::SolCall;
use alloy::transports::http::reqwest::{self, Url};
use serde_json::{Map, Value, json};
use std::time::Duration;

const RPC_TIMEOUT: Duration = Duration::from_secs(30);
/// What every request says it is: the program and its version, nothing about who runs it.
pub const USER_AGENT: &str = concat!("zwallet/", env!("CARGO_PKG_VERSION"));
/// The most bytes of one reply read; a larger one is an error, never parsed.
const REPLY_MAX: usize = 1 << 20;
/// The most characters of a token's `symbol()` that output shows (as on EVM).
const SYMBOL_SHOWN: usize = 32;
/// The most characters of one value from a reply (an id, a key, a number) that a message shows.
const VALUE_SHOWN: usize = 64;
/// Sun (the smallest TRX unit) a balance may hold: java-tron keeps it in a signed 64-bit long.
const SUN_LIMIT: u64 = 1 << 63;

/// One Tron endpoint. Errors become text only through [`TronClient::post`], which scrubs the
/// endpoint's URL and key ([`Net::redact`]). No `Debug`: it holds the URL.
pub struct TronClient {
    http: reqwest::Client,
    base: Url,
    net: Net,
}

impl TronClient {
    /// No I/O: the HTTP client connects on the first request.
    fn new(net: &Net) -> Result<Self> {
        let base: Url = net.rpc_url.trim().parse().map_err(|_| {
            WalletError::validation(format!("invalid RPC URL '{}'", redact_url(&net.rpc_url)))
        })?;
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| {
                WalletError::rpc(format!(
                    "cannot start the HTTP client: {}",
                    ascii_inert(&net.redact(&e.to_string()), PROSE_MAX)
                ))
            })?;
        Ok(Self {
            http,
            base,
            net: net.clone(),
        })
    }

    /// `<the endpoint's path>/wallet/<call>`, its query kept (a key may ride there).
    fn url(&self, call: &str) -> Url {
        let mut url = self.base.clone();
        let path = format!("{}/wallet/{call}", self.base.path().trim_end_matches('/'));
        url.set_path(&path);
        url
    }

    /// One node-supplied diagnostic value, with endpoint credentials removed before escaping.
    fn shown(&self, value: &Value) -> String {
        ascii_inert(&self.net.redact(&text_of(value)), VALUE_SHOWN)
    }

    /// `POST /wallet/<call>` with `body`; the reply must be a JSON object without java-tron's
    /// `Error` field. Every failure is `RPC_ERROR` naming the call.
    async fn post(&self, call: &str, body: &Value) -> Result<Map<String, Value>> {
        match tokio::time::timeout(RPC_TIMEOUT, self.exchange(call, body)).await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(why)) => Err(WalletError::rpc(format!(
                "/wallet/{call} failed: {}",
                ascii_inert(&why, PROSE_MAX)
            ))),
            Err(_) => Err(WalletError::rpc(format!(
                "/wallet/{call} timed out after {}s",
                RPC_TIMEOUT.as_secs()
            ))),
        }
    }

    /// One request and its reply, or why it failed as unescaped text: a transport error with the
    /// URL and key scrubbed, a non-2xx status with the endpoint's origin only (never the body, a
    /// gateway's text that can quote the key), or the parse error's kind and position (never the
    /// value it quotes).
    async fn exchange(
        &self,
        call: &str,
        body: &Value,
    ) -> std::result::Result<Map<String, Value>, String> {
        let redact = |e: reqwest::Error| self.net.redact(&e.to_string());
        let mut reply = self
            .http
            .post(self.url(call))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.to_string())
            .send()
            .await
            .map_err(redact)?;
        let status = reply.status();
        if !status.is_success() {
            return Err(format!(
                "HTTP {} from {}",
                status.as_u16(),
                url_origin(&self.net.rpc_url)
            ));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = reply.chunk().await.map_err(redact)? {
            if bytes.len() + chunk.len() > REPLY_MAX {
                return Err(format!(
                    "the endpoint's answer is larger than {REPLY_MAX} bytes"
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|e| deser_text(&e))?;
        let Value::Object(map) = value else {
            return Err("the endpoint's answer is not a JSON object".into());
        };
        if let Some(e) = map.get("Error") {
            return Err(self
                .net
                .redact(&format!("the node answered an error: {}", text_of(e))));
        }
        Ok(map)
    }
}

/// A serde error as printable text: its category and position only, never its message.
fn deser_text(err: &serde_json::Error) -> String {
    use serde_json::error::Category;
    let kind = match err.classify() {
        Category::Io => "an I/O error",
        Category::Syntax => "a syntax error",
        Category::Data => "a data type error",
        Category::Eof => "an unexpected end",
    };
    format!(
        "the endpoint's answer is not JSON ({kind} at line {} column {})",
        err.line(),
        err.column()
    )
}

/// A JSON value from a reply as text: a string as it is, anything else as its JSON.
fn text_of(v: &Value) -> String {
    v.as_str().map_or_else(|| v.to_string(), str::to_string)
}

/// One value from a reply for a message: escaped and cut.
fn shown(v: &Value) -> String {
    ascii_inert(&text_of(v), VALUE_SHOWN)
}

/// Opens the endpoint and checks it serves the Tron network `net` names: block 0's id must be
/// the chain's genesis id, the Tron form of the `eth_chainId` check. Nothing else is asked first.
pub async fn connect(net: &Net) -> Result<TronClient> {
    let client = TronClient::new(net)?;
    let want = config(net.chain)?.genesis;
    let block = client.post("getblockbynum", &json!({ "num": 0 })).await?;
    match block.get("blockID") {
        Some(Value::String(id)) if id == want => Ok(client),
        Some(other) => Err(WalletError::rpc(format!(
            "RPC {} serves a network whose block 0 is {}, but {}'s genesis block is {want}; not \
             reading from it",
            redact_url(&net.rpc_url),
            client.shown(other),
            net.chain.key
        ))),
        None => Err(WalletError::rpc(format!(
            "RPC {} answered /wallet/getblockbynum without a blockID, so it cannot be checked to \
             serve {}; not reading from it",
            redact_url(&net.rpc_url),
            net.chain.key
        ))),
    }
}

/// What `getaccount` says of one address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    /// The node knows the account (`{}` = it never received TRX).
    pub activated: bool,
    /// TRX in sun.
    pub balance: u64,
    /// Why this wallet's key may not control the account, when the owner permission is not this
    /// address alone at threshold 1 ([`owner_warning`]).
    pub warning: Option<String>,
}

/// `getaccount` of `owner` (a `T…` address), refused unless the reply is about that account and
/// its balance is a whole number of sun below 2^63.
pub async fn account(c: &TronClient, owner: &str) -> Result<Account> {
    let a = c
        .post("getaccount", &json!({ "address": owner, "visible": true }))
        .await?;
    if a.is_empty() {
        return Ok(Account {
            activated: false,
            balance: 0,
            warning: None,
        });
    }
    match a.get("address") {
        Some(Value::String(s)) if address::same(s, owner) => {}
        other => {
            return Err(WalletError::rpc(format!(
                "/wallet/getaccount answered about the account {}, not {owner}",
                other.map_or_else(|| "<none>".to_string(), |v| c.shown(v))
            )));
        }
    }
    let balance = match a.get("balance") {
        // A zero balance is left out of the reply, as protobuf leaves out every default.
        None => 0,
        Some(v) => v.as_u64().filter(|b| *b < SUN_LIMIT).ok_or_else(|| {
            WalletError::rpc(format!(
                "/wallet/getaccount answered a balance of {}, which is not a whole number of sun \
                 from 0 to 2^63 - 1",
                c.shown(v)
            ))
        })?,
    };
    Ok(Account {
        activated: true,
        balance,
        warning: owner_warning_with(a.get("owner_permission"), owner, |v| c.shown(v)),
    })
}

/// The warning for an account whose owner permission is not `{threshold: 1, keys: [this
/// address, weight 1]}`: another key can then move everything, and change the permission again
/// (05-tron.md §CLI surface). An account without `owner_permission` has the default one, this
/// address alone. The keys and the threshold come from the node: each escaped and cut, all of
/// them within one 512-character budget, then how many more.
pub fn owner_warning(permission: Option<&Value>, owner: &str) -> Option<String> {
    owner_warning_with(permission, owner, shown)
}

fn owner_warning_with(
    permission: Option<&Value>,
    owner: &str,
    shown: impl Fn(&Value) -> String,
) -> Option<String> {
    let p = permission?;
    let threshold = p.get("threshold");
    let keys = p.get("keys").and_then(Value::as_array);
    let alone = threshold.and_then(Value::as_u64) == Some(1)
        && keys.is_some_and(|k| {
            k.len() == 1
                && k[0]
                    .get("address")
                    .and_then(Value::as_str)
                    .is_some_and(|a| address::same(a, owner))
                && k[0].get("weight").and_then(Value::as_u64) == Some(1)
        });
    if alone {
        return None;
    }
    let mut shown_keys = Vec::new();
    let mut used = 0;
    let all = keys.map_or(&[][..], Vec::as_slice);
    for key in all {
        let one = format!(
            "{} weight {}",
            key.get("address")
                .map_or_else(|| "<none>".to_string(), &shown),
            key.get("weight")
                .map_or_else(|| "<none>".to_string(), &shown)
        );
        if used + one.len() > PROSE_MAX {
            break;
        }
        used += one.len();
        shown_keys.push(one);
    }
    let more = all.len() - shown_keys.len();
    let mut list = if keys.is_none() {
        "none readable".to_string()
    } else {
        shown_keys.join(", ")
    };
    if more > 0 {
        list.push_str(&format!(" and {more} more"));
    }
    Some(format!(
        "this Tron account's owner permission is not this wallet's key alone (threshold {}; \
         keys: {list}): funds here may not be controlled by this wallet's key; do not fund it",
        threshold.map_or_else(|| "<none>".to_string(), &shown)
    ))
}

/// One read-only call of `contract` (`triggerconstantcontract`, from `owner`): the bytes it
/// returned. A call the node reports as failed, or one whose `result.result` is not `true`, is
/// `RPC_ERROR` with the node's message (hex-decoded when it is hex), escaped and cut.
async fn constant_call(
    c: &TronClient,
    owner: &str,
    contract: &str,
    selector: &str,
    parameter: &[u8],
) -> Result<Vec<u8>> {
    let r = c
        .post(
            "triggerconstantcontract",
            &json!({
                "owner_address": owner,
                "contract_address": contract,
                "function_selector": selector,
                "parameter": hex::encode(parameter),
                "visible": true,
            }),
        )
        .await?;
    let ok = r.get("result").and_then(|x| x.get("result")) == Some(&Value::Bool(true));
    let failed = r
        .get("transaction")
        .and_then(|t| t.get("ret"))
        .is_some_and(|results| {
            results.as_array().is_none_or(|results| {
                results.iter().any(|result| {
                    // Request acceptance (result.result) is separate from VM execution. A
                    // failed contract must never turn its apparent return word into a balance.
                    !result.is_object()
                        || result.get("ret").is_some_and(|s| s != "SUCESS")
                        || result.get("contractRet").is_some_and(|s| s != "SUCCESS")
                })
            })
        });
    if !ok || failed {
        let why = r
            .get("result")
            .and_then(|x| x.get("message"))
            .map(|m| {
                let text = text_of(m);
                hex::decode(&text)
                    .ok()
                    .map_or(text, |b| String::from_utf8_lossy(&b).into_owned())
            })
            .unwrap_or_else(|| "no message".to_string());
        return Err(WalletError::rpc(format!(
            "{selector} on {contract} failed: {}",
            ascii_inert(&c.net.redact(&why), PROSE_MAX)
        )));
    }
    let word = r
        .get("constant_result")
        .and_then(|x| x.get(0))
        .and_then(Value::as_str)
        .and_then(|h| hex::decode(h).ok())
        .ok_or_else(|| {
            WalletError::rpc(format!(
                "{selector} on {contract} returned no hex constant_result"
            ))
        })?;
    Ok(word)
}

/// The 32-byte ABI word of a Tron address: 12 zero bytes, then its 20-byte body.
fn address_word(t: &str) -> Result<Vec<u8>> {
    let payload = address::parse(t)?;
    Ok(IERC20::balanceOfCall {
        owner: Address::from_slice(&payload[1..]),
    }
    .abi_encode()[4..]
        .to_vec())
}

/// `balanceOf(owner)` of the TRC20 `token`: exactly one 32-byte word, else `RPC_ERROR`.
pub async fn trc20_balance(c: &TronClient, token: &str, owner: &str) -> Result<U256> {
    let raw = constant_call(c, owner, token, "balanceOf(address)", &address_word(owner)?).await?;
    if raw.len() != 32 {
        return Err(WalletError::rpc(format!(
            "balanceOf(address) on {token} returned {} bytes, not one 32-byte word",
            raw.len()
        )));
    }
    Ok(U256::from_be_slice(&raw))
}

/// A token's symbol and decimals as shown, and whose the symbol is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenMeta {
    pub symbol: String,
    pub decimals: u8,
    /// The registry's symbol (a registry contract that answered its recorded `symbol()`), or the
    /// contract's own, escaped and cut.
    pub from_registry: bool,
}

/// `decimals()` and `symbol()` of `token`, read from the chain, as EVM's `erc20_meta` does: the
/// symbol is whatever the contract returns, escaped and cut, unless `token` is a registry row
/// whose `symbol()` is exactly the one the registry records.
pub async fn trc20_meta(c: &TronClient, token: &str, owner: &str) -> Result<TokenMeta> {
    let not_token = || {
        WalletError::invalid_address(format!(
            "{token} is not a TRC20 contract on tron (decimals() returned no usable data)"
        ))
    };
    let raw = constant_call(c, owner, token, "decimals()", &[])
        .await
        .map_err(|_| not_token())?;
    let decimals = (raw.len() == 32)
        .then(|| U256::from_be_slice(&raw))
        .and_then(|d| u8::try_from(d).ok())
        .ok_or_else(not_token)?;
    let raw = constant_call(c, owner, token, "symbol()", &[])
        .await
        .unwrap_or_default();
    let symbol = IERC20::symbolCall::abi_decode_returns(&raw)
        .ok()
        .or_else(|| {
            (raw.len() == 32).then(|| {
                String::from_utf8_lossy(&raw)
                    .trim_end_matches('\0')
                    .to_string()
            })
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "?".to_string());
    let registry = address::parse(token)
        .ok()
        .and_then(|p| registry::lookup(&p))
        .filter(|t| t.onchain_symbol == symbol);
    Ok(match registry {
        Some(t) => TokenMeta {
            symbol: t.onchain_symbol.to_string(),
            decimals,
            from_registry: true,
        },
        None => TokenMeta {
            symbol: ascii_inert(&c.net.redact(&symbol), SYMBOL_SHOWN),
            decimals,
            from_registry: false,
        },
    })
}

/// Everything `balance` shows for one Tron account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Read {
    pub native: Balance,
    pub tokens: Vec<TokenBalance>,
    pub activated: bool,
    pub warning: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWER: &str = "TWer2Ygk5TEheHp3TPuYeqxmB6SsGZmaL6";
    const OTHER: &str = "TDRqzjYye3bGaBtg1bAPw5F4duDchNSbtk";

    #[test]
    fn the_owner_permission_warns_unless_it_is_this_address_alone_at_threshold_1() {
        let alone = json!({ "threshold": 1, "keys": [{ "address": TWER, "weight": 1 }] });
        assert_eq!(owner_warning(Some(&alone), TWER), None);
        // No owner permission in the reply: the default one, this address alone.
        assert_eq!(owner_warning(None, TWER), None);
        for (permission, names) in [
            // M7: another key holds it.
            (
                json!({ "threshold": 1, "keys": [{ "address": OTHER, "weight": 1 }] }),
                OTHER,
            ),
            // This address, but it alone cannot reach the threshold.
            (
                json!({ "threshold": 2, "keys": [{ "address": TWER, "weight": 1 }] }),
                "threshold 2",
            ),
            // Two keys: either signs alone.
            (
                json!({ "threshold": 1, "keys": [
                    { "address": TWER, "weight": 1 }, { "address": OTHER, "weight": 1 },
                ] }),
                OTHER,
            ),
            // A case variant of this address is another key.
            (
                json!({ "threshold": 1, "keys": [{ "address": TWER.to_ascii_lowercase(), "weight": 1 }] }),
                "twer2",
            ),
            (json!("not an object"), "none readable"),
        ] {
            let w = owner_warning(Some(&permission), TWER)
                .unwrap_or_else(|| panic!("no warning for {permission}"));
            assert!(w.contains(names), "{permission}: {w}");
            assert!(w.ends_with("do not fund it"), "{w}");
        }
    }

    #[test]
    fn the_keys_a_warning_names_are_escaped_and_share_one_budget() {
        let evil = "T\u{1b}[2J\nverified: true";
        let keys: Vec<Value> = (0..400)
            .map(|_| json!({ "address": evil, "weight": "\u{1b}" }))
            .collect();
        let w = owner_warning(Some(&json!({ "threshold": 1, "keys": keys })), TWER).unwrap();
        assert!(w.bytes().all(|b| (0x20..=0x7e).contains(&b)), "{w:?}");
        assert!(w.contains("more"), "{w}");
        // The node's text in it: at most 512 characters of keys, and the threshold's 64.
        assert!(w.len() < 512 + 64 + 200, "{} characters", w.len());
    }
}
