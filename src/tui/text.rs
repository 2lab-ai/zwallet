//! Text from outside zwallet, made inert and bounded where it comes in (03-tui.md §Display
//! integrity rule 3). Every such value reaches a widget as an [`Outside`], built only by the
//! function of its [`Source`], which escapes it (the CLI's [`ascii_inert`]) and cuts it to the
//! source's budget. So no control character, no glyph and no long block from outside ever reaches
//! a cell, and the tests can check each value's budget and placement on its own.

use crate::chains;
use crate::core::error::WalletError;
use crate::core::text::{PROSE_MAX, ascii_inert, cut_to};
use crate::core::time::{parse_rfc3339_utc, rfc3339_utc};

/// Where a value shown in the TUI comes from: one row of rule 3's table each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// A key name in wallet.json (checked by `validate_key_name` when parsed).
    KeyName,
    /// An address: wallet.json's cache or a reply on `d.sock`, address-shaped by the CLI's checks.
    Address,
    /// The session's expiry, as a reply on `d.sock` wrote it.
    Expiry,
    /// The RPC endpoint (`ZWALLET_RPC_<CHAIN>` or the default): its host.
    RpcHost,
    /// An error, built by the CLI's code from values it already escaped and cut.
    Error,
    /// What `PeerCheck::warning` says about a process answering on `d.sock`.
    Warning,
}

impl Source {
    /// The most characters a value of this source shows.
    pub const fn budget(self) -> usize {
        match self {
            Self::KeyName | Self::RpcHost => 32,
            Self::Address => 42,
            Self::Expiry => 64,
            Self::Error | Self::Warning => PROSE_MAX,
        }
    }
}

/// A value from outside, inert (printable ASCII only) and within its source's budget.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outside {
    source: Source,
    text: String,
}

/// How far ahead an expiry may be and still be drawn as a time of day alone.
const ONE_DAY: u64 = 86_400;

impl Outside {
    /// `text` from `source`: kept when it is printable ASCII already (as the CLI's escaping
    /// leaves it, so an escape is not escaped twice), escaped otherwise; then cut to the budget.
    fn inert(source: Source, text: &str) -> Self {
        let budget = source.budget();
        let text = if text.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
            cut_to(text.to_string(), budget)
        } else {
            ascii_inert(text, budget)
        };
        Self { source, text }
    }

    pub fn key_name(name: &str) -> Self {
        Self::inert(Source::KeyName, name)
    }

    pub fn address(address: &str) -> Self {
        Self::inert(Source::Address, address)
    }

    /// An address as the status bar shortens it: `0x1234...abcd`.
    pub fn short_address(address: &str) -> Self {
        let full = Self::address(address).text;
        let text = match (full.get(..6), full.get(38..)) {
            (Some(head), Some(tail)) if full.len() == 42 => format!("{head}...{tail}"),
            _ => full,
        };
        Self {
            source: Source::Address,
            text,
        }
    }

    /// The session's expiry as `wallet::status` relays it (escaped, at most 64). One zwallet can
    /// read (RFC 3339 UTC) is drawn by zwallet: `HH:MMZ`, with the date in front when it is a day
    /// or more after `now` (UNIX seconds). Anything else is shown as it came.
    pub fn expiry(expires_at: &str, now: u64) -> Self {
        match parse_rfc3339_utc(expires_at) {
            Some(secs) => {
                let at = rfc3339_utc(secs);
                let text = if secs.saturating_sub(now) < ONE_DAY {
                    format!("{}Z", &at[11..16])
                } else {
                    format!("{} {}Z", &at[..10], &at[11..16])
                };
                Self::inert(Source::Expiry, &text)
            }
            None => Self::inert(Source::Expiry, expires_at),
        }
    }

    /// The host (and port) of an RPC URL's origin, as the HTTP client parses it with key-like
    /// labels redacted ([`chains::url_origin`]): never its path, query or userinfo, where keys
    /// go. Longer than the budget, it is cut with `...`.
    pub fn rpc_host(url: &str) -> Self {
        let origin = chains::url_origin(url);
        let host = origin.split_once("://").map_or(origin.as_str(), |(_, h)| h);
        let budget = Source::RpcHost.budget();
        if host.len() > budget && host.is_ascii() {
            let cut = format!("{}...", &host[..budget - 3]);
            return Self::inert(Source::RpcHost, &cut);
        }
        Self::inert(Source::RpcHost, host)
    }

    /// An error as the CLI prints it, `<message> [CODE]`.
    pub fn error(e: &WalletError) -> Self {
        Self::inert(Source::Error, &format!("{} [{}]", e.message, e.code))
    }

    pub fn warning(warning: &str) -> Self {
        Self::inert(Source::Warning, warning)
    }

    pub fn source(&self) -> Source {
        self.source
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}
