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
    WhitelistName,
    Quantity,
    TransactionHash,
    /// A bounded read-only pane line, built from independently checked values.
    Detail,
    /// One row of a list pane, from zwallet's own list text (`whitelist list`), whose fields
    /// that text already shape-checked and escaped. Drawn on one line, cut to the pane's width
    /// (rule 4); its budget is the CLI's prose budget for the whole list.
    ListRow,
    /// Public wallet-home path.
    Home,
    /// A field's already alphabet-checked, redacted display value.
    Input,
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
            Self::WhitelistName => 32,
            Self::Quantity => 78,
            Self::TransactionHash => 66,
            Self::Detail => 72,
            Self::ListRow => PROSE_MAX,
            Self::Home => 64,
            Self::Input => 512,
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
    pub fn nickname(text: &str) -> Self {
        Self::inert(Source::WhitelistName, text)
    }
    pub fn quantity(text: &str) -> Self {
        Self::inert(
            Source::Quantity,
            if text.len() <= 78 && !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) {
                text
            } else {
                "<invalid>"
            },
        )
    }
    pub fn transaction_hash(text: &str) -> Self {
        Self::inert(
            Source::TransactionHash,
            if text.parse::<alloy::primitives::B256>().is_ok() {
                text
            } else {
                "<invalid>"
            },
        )
    }
    pub fn detail(text: &str) -> Self {
        Self::inert(Source::Detail, text)
    }
    /// `text` escaped and bounded as an error is, then broken at spaces into [`Outside::detail`]
    /// lines within their budget: a message whose end matters (the command it names) is shown
    /// whole, line by line, never cut. A word longer than a line is broken inside it.
    pub fn details(text: &str) -> Vec<Self> {
        let whole = Self::inert(Source::Error, text).text;
        let width = Source::Detail.budget();
        let mut lines = Vec::new();
        let mut line = String::new();
        for word in whole.split(' ') {
            if !line.is_empty() && line.len() + 1 + word.len() <= width {
                line.push(' ');
            } else if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            // Only an empty line gets here with a word that does not fit.
            let mut rest = word;
            while line.len() + rest.len() > width {
                let (head, tail) = rest.split_at(width - line.len());
                line.push_str(head);
                lines.push(std::mem::take(&mut line));
                rest = tail;
            }
            line.push_str(rest);
        }
        if !line.is_empty() {
            lines.push(line);
        }
        lines.iter().map(|line| Self::detail(line)).collect()
    }
    /// [`Outside::details`] in at most `rows` lines (one at least): when the text needs more, the
    /// last line holds the rest (its lines joined), cut with `...` as [`Outside::cut`] cuts a
    /// value to fit.
    pub fn details_within(text: &str, rows: usize) -> Vec<Self> {
        let mut lines = Self::details(text);
        let keep = rows.max(1) - 1;
        if lines.len() > keep + 1 {
            let rest: Vec<&str> = lines[keep..].iter().map(Self::text).collect();
            let rest = Self {
                source: Source::Detail,
                text: rest.join(" "),
            };
            lines.truncate(keep);
            lines.push(rest.cut(Source::Detail.budget()));
        }
        lines
    }
    pub fn list_row(text: &str) -> Self {
        Self::inert(Source::ListRow, text)
    }
    pub fn home(text: &str) -> Self {
        Self::inert(Source::Home, text)
    }
    pub fn input(text: &str) -> Self {
        Self::inert(Source::Input, text)
    }
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

    /// Only the status bar shortens addresses: six leading and four trailing characters.
    pub fn short_address(address: &str) -> Self {
        let full = Self::address(address).text;
        let text = if full.len() == 42 || full.len() == 34 && full.starts_with('T') {
            format!("{}...{}", &full[..6], &full[full.len() - 4..])
        } else {
            full
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

    /// This value cut to `width` characters, ending in `...` when it is longer; still inert and
    /// from the same source.
    pub fn cut(&self, width: usize) -> Self {
        if self.text.len() <= width {
            return self.clone();
        }
        let keep = width.saturating_sub(3);
        let text = format!("{}...", &self.text[..keep]);
        Self {
            source: self.source,
            text: text[..width.min(text.len())].to_string(),
        }
    }

    pub fn source(&self) -> Source {
        self.source
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn details_show_a_long_message_whole_in_lines_within_the_detail_budget() {
        let message = "key 'a' has no cached tron address yet; unlock once (`zwallet unlock 24h`) \
                       to derive it, or run `zwallet address --chain tron --verify` (with the \
                       password) to add it";
        let lines = Outside::details(message);
        assert_eq!(lines.len(), 3, "{lines:#?}");
        for line in &lines {
            assert_eq!(line.source(), Source::Detail);
            assert!(line.text().len() <= Source::Detail.budget(), "{line:?}");
        }
        let texts: Vec<&str> = lines.iter().map(Outside::text).collect();
        assert_eq!(texts.join(" "), message);
        // A word longer than a line is broken inside it; a control character is escaped.
        let long = "x".repeat(100);
        let lines = Outside::details(&format!("a {long} \u{1b}b"));
        let texts: Vec<&str> = lines.iter().map(Outside::text).collect();
        let escaped = format!("{} {}", "x".repeat(28), ascii_inert("\u{1b}b", 64));
        assert_eq!(texts, ["a", "x".repeat(72).as_str(), escaped.as_str()]);
        // Bounded as an error is: cut once, at its end.
        let lines = Outside::details(&"word ".repeat(200));
        let all: Vec<&str> = lines.iter().map(Outside::text).collect();
        let all = all.join(" ");
        assert!(all.len() <= PROSE_MAX && all.ends_with("...(cut)"), "{all}");
        assert_eq!(all.matches("(cut)").count(), 1);
    }

    #[test]
    fn details_within_rows_cut_only_the_last_line_and_mark_it() {
        let message = (0..60)
            .map(|i| format!("w{i:02}xq"))
            .collect::<Vec<_>>()
            .join(" ");
        // Twelve words of six characters to a line.
        let all = Outside::details(&message);
        assert_eq!(all.len(), 5);
        // What fits is the same lines.
        assert_eq!(Outside::details_within(&message, 5), all);
        assert_eq!(Outside::details_within(&message, 9), all);
        // Fewer rows: the lines before the last whole, the last the rest cut with `...`.
        let four = Outside::details_within(&message, 4);
        assert_eq!(four.len(), 4);
        assert_eq!(four[..3], all[..3]);
        let rest = all[3..]
            .iter()
            .map(Outside::text)
            .collect::<Vec<_>>()
            .join(" ");
        let last = four[3].text();
        assert_eq!(last.len(), Source::Detail.budget());
        assert_eq!(last, format!("{}...", &rest[..69]));
        assert_eq!(four[3].source(), Source::Detail);
        // No row at all still shows the start.
        assert_eq!(Outside::details_within(&message, 0).len(), 1);
    }
}
