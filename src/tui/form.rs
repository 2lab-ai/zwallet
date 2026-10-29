//! Bounded, non-secret single-line input. A focused field owns every input event: callers must
//! not send a consumed character on to the TUI's global shortcuts. Passwords use their own
//! zeroizing input-thread buffer, never this type. Endpoint credentials are never drawn.

use crate::chains;
use crate::core::time::parse_duration;
use alloy::primitives::U256;
use alloy::transports::http::reqwest::Url;
use std::fmt;
use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Filter,
    Nickname,
    KeyName,
    Amount,
    LimitAmount,
    LimitRaw,
    LimitBytes,
    Bump,
    Duration,
    EvmAddress,
    TxHash,
    TronTxHash,
    Selector,
    Calldata,
    Signature,
    Arguments,
    Message,
    WordCount,
    TronAddress,
    RpcUrl,
    Unsigned,
}

impl Kind {
    pub const fn max_len(self) -> usize {
        match self {
            Self::Filter => 64,
            Self::Nickname | Self::KeyName | Self::Duration => 32,
            Self::Amount | Self::LimitAmount | Self::Bump => 128,
            Self::LimitBytes => 10,
            Self::EvmAddress => 42,
            Self::TxHash | Self::TronTxHash => 66,
            Self::Selector => 10,
            Self::Calldata => 4096,
            Self::Signature => 256,
            Self::Arguments | Self::Message | Self::RpcUrl => 512,
            Self::WordCount => 2,
            Self::TronAddress => 34,
            Self::Unsigned | Self::LimitRaw => 78,
        }
    }

    fn prefix(self, value: &str) -> bool {
        if value.len() > self.max_len() || !value.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
            return false;
        }
        match self {
            Self::Filter | Self::Signature | Self::Arguments | Self::Message | Self::RpcUrl => true,
            Self::Nickname | Self::KeyName => value
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_-".contains(&b)),
            Self::Amount | Self::Bump => {
                value.bytes().all(|b| b.is_ascii_digit() || b == b'.')
                    && value.bytes().filter(|&b| b == b'.').count() <= 1
            }
            Self::LimitAmount => "none".starts_with(value) || Self::Amount.prefix(value),
            Self::LimitBytes | Self::LimitRaw => {
                "none".starts_with(value) || value.bytes().all(|b| b.is_ascii_digit())
            }
            Self::Duration => {
                let digits = value.bytes().take_while(u8::is_ascii_digit).count();
                let suffix = &value[digits..];
                suffix.is_empty()
                    || (digits > 0
                        && suffix.len() == 1
                        && b"smhdwy".contains(&suffix.as_bytes()[0]))
            }
            Self::EvmAddress | Self::TxHash | Self::Selector | Self::Calldata => {
                value.is_empty()
                    || value == "0"
                    || value
                        .strip_prefix("0x")
                        .is_some_and(|s| s.bytes().all(|b| b.is_ascii_hexdigit()))
            }
            Self::TronTxHash => {
                value.strip_prefix("0x").unwrap_or(value).len() <= 64
                    && value
                        .strip_prefix("0x")
                        .unwrap_or(value)
                        .bytes()
                        .all(|b| b.is_ascii_hexdigit())
            }
            Self::WordCount => ["12", "15", "18", "21", "24"]
                .iter()
                .any(|n| n.starts_with(value)),
            Self::TronAddress => {
                value.is_empty()
                    || value.starts_with('T')
                        && value.bytes().all(|b| {
                            b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
                                .contains(&b)
                        })
            }
            Self::Unsigned => value.bytes().all(|b| b.is_ascii_digit()),
        }
    }

    fn complete(self, value: &str) -> bool {
        if !self.prefix(value) {
            return false;
        }
        match self {
            Self::Filter | Self::Arguments | Self::Message => true,
            Self::EvmAddress | Self::TxHash | Self::Selector | Self::TronAddress => {
                value.len() == self.max_len()
            }
            Self::TronTxHash => chains::tron::lookup::parse_hash(value).is_ok(),
            Self::Calldata => value.len() >= 2 && value.len().is_multiple_of(2),
            Self::Duration => parse_duration(value).is_ok(),
            Self::WordCount => ["12", "15", "18", "21", "24"].contains(&value),
            Self::Amount | Self::Bump => {
                value.bytes().any(|b| b.is_ascii_digit()) && !value.ends_with('.')
            }
            Self::LimitAmount => value == "none" || Self::Amount.complete(value),
            Self::LimitBytes => value == "none" || value.parse::<u32>().is_ok(),
            Self::LimitRaw => value == "none" || value.parse::<U256>().is_ok(),
            Self::RpcUrl => Url::parse(value)
                .is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some()),
            _ => !value.is_empty(),
        }
    }
}

/// Transport-neutral input; the crossterm reader translates events before calling the field.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Input<'a> {
    Char(char),
    Paste(&'a str),
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    Up,
    Down,
    Tab,
    BackTab,
    Enter,
    Esc,
    Interrupt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    Changed,
    Moved,
    Submit,
    Cancel,
    Next,
    Previous,
    Interrupt,
    Ignored,
    /// No part of the event was inserted, including a paste that exceeds the bound.
    Rejected(&'static str),
}

/// A field's value is readable for validation/submission, but cannot be modified outside its
/// alphabet. It is not cloned or displayed through `Debug`; an RPC URL may carry credentials.
pub struct Field {
    kind: Kind,
    value: Zeroizing<String>,
    cursor: usize,
}

impl fmt::Debug for Field {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Field")
            .field("kind", &self.kind)
            .field("length", &self.value.len())
            .field("cursor", &self.cursor)
            .finish()
    }
}

impl Field {
    pub fn new(kind: Kind) -> Self {
        Self {
            kind,
            value: Zeroizing::new(String::new()),
            cursor: 0,
        }
    }

    pub fn with_value(kind: Kind, value: &str) -> Result<Self, &'static str> {
        let mut field = Self::new(kind);
        match field.edit(Input::Paste(value)) {
            Edit::Rejected(reason) => Err(reason),
            _ => Ok(field),
        }
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }
    pub fn value(&self) -> &str {
        &self.value
    }
    pub fn cursor(&self) -> usize {
        self.cursor
    }
    pub fn is_complete(&self) -> bool {
        self.kind.complete(&self.value)
    }

    pub fn edit(&mut self, input: Input<'_>) -> Edit {
        match input {
            Input::Char(c) => {
                if !c.is_ascii() {
                    return Edit::Rejected("printable ASCII only");
                }
                let mut bytes = [0; 4];
                self.insert(c.encode_utf8(&mut bytes))
            }
            Input::Paste(text) => self.insert(text),
            Input::Backspace if self.cursor > 0 => {
                self.value.remove(self.cursor - 1);
                self.cursor -= 1;
                Edit::Changed
            }
            Input::Delete if self.cursor < self.value.len() => {
                self.value.remove(self.cursor);
                Edit::Changed
            }
            Input::Left => {
                self.cursor = self.cursor.saturating_sub(1);
                Edit::Moved
            }
            Input::Right => {
                self.cursor = (self.cursor + 1).min(self.value.len());
                Edit::Moved
            }
            Input::Home => {
                self.cursor = 0;
                Edit::Moved
            }
            Input::End => {
                self.cursor = self.value.len();
                Edit::Moved
            }
            Input::Enter if self.is_complete() => Edit::Submit,
            Input::Enter => Edit::Rejected("complete this field before continuing"),
            Input::Esc => Edit::Cancel,
            Input::Tab | Input::Down => Edit::Next,
            Input::BackTab | Input::Up => Edit::Previous,
            Input::Interrupt => Edit::Interrupt,
            _ => Edit::Ignored,
        }
    }

    fn insert(&mut self, text: &str) -> Edit {
        if text.is_empty() {
            return Edit::Ignored;
        }
        if text.len() > self.kind.max_len().saturating_sub(self.value.len()) {
            return Edit::Rejected("field is too long; nothing was inserted");
        }
        if !text.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
            return Edit::Rejected("printable ASCII only; nothing was inserted");
        }
        let mut next = Zeroizing::new(self.value.to_string());
        next.insert_str(self.cursor, text);
        if !self.kind.prefix(&next) {
            return Edit::Rejected("character is outside this field's alphabet");
        }
        self.value = next;
        self.cursor += text.len();
        Edit::Changed
    }

    /// Inert text for the renderer. Until a URL's authority is terminated, it is masked:
    /// an incomplete `https://user:password` might become userinfo after the next `@`.
    /// With an authority terminator, only the parsed, redacted origin is shown; all suffix
    /// characters are stars. The actual endpoint is read only by the submitting operation.
    pub fn drawn(&self) -> String {
        if self.kind != Kind::RpcUrl {
            return self.value.to_string();
        }
        let raw = self.value.as_str();
        let Some((scheme, rest)) = raw.split_once("://") else {
            return "*".repeat(raw.len());
        };
        if !matches!(scheme, "http" | "https") {
            return "*".repeat(raw.len());
        }
        let Some(at) = rest.find(['/', '?', '#']) else {
            return format!("{scheme}://{}", "*".repeat(rest.len()));
        };
        let origin = chains::url_origin(raw);
        format!("{origin}{}", "*".repeat(rest.len() - at))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focused_text_takes_global_shortcuts_as_characters() {
        let mut f = Field::new(Kind::Filter);
        for c in "q?123[]Lr".chars() {
            assert_eq!(f.edit(Input::Char(c)), Edit::Changed);
        }
        assert_eq!(f.value(), "q?123[]Lr");
        assert_eq!(f.edit(Input::Esc), Edit::Cancel);
        assert_eq!(f.edit(Input::Interrupt), Edit::Interrupt);
    }

    #[test]
    fn pastes_are_atomic_and_never_sanitize_the_bytes_to_sign() {
        let mut f = Field::with_value(Kind::Message, "  keep spaces ").unwrap();
        for bad in ["evil\nsecond line", "\u{1b}[2J", "solid █", "a\tmessage"] {
            assert!(matches!(f.edit(Input::Paste(bad)), Edit::Rejected(_)));
            assert_eq!(f.value(), "  keep spaces ");
        }
        let mut f = Field::new(Kind::Message);
        assert!(matches!(
            f.edit(Input::Paste(&"a".repeat(513))),
            Edit::Rejected(_)
        ));
        assert_eq!(f.value(), "");
        assert_eq!(f.edit(Input::Paste(&"a".repeat(512))), Edit::Changed);
        assert_eq!(f.edit(Input::Enter), Edit::Submit);
        assert!(matches!(f.edit(Input::Char('b')), Edit::Rejected(_)));
    }

    #[test]
    fn field_alphabets_and_complete_lengths_are_independent_of_submitters() {
        let mut amount = Field::with_value(Kind::Amount, "1.25").unwrap();
        assert!(matches!(amount.edit(Input::Char('.')), Edit::Rejected(_)));
        assert!(matches!(amount.edit(Input::Char('e')), Edit::Rejected(_)));
        assert!(Field::with_value(Kind::Nickname, "Bob").is_err());
        assert!(Field::with_value(Kind::Duration, "1hm").is_err());
        assert!(
            Field::with_value(Kind::Duration, "0s")
                .unwrap()
                .edit(Input::Enter)
                != Edit::Submit
        );
        for (kind, digits) in [
            (Kind::EvmAddress, 40),
            (Kind::TxHash, 64),
            (Kind::Selector, 8),
        ] {
            let mut f = Field::with_value(kind, &format!("0x{}", "a".repeat(digits - 1))).unwrap();
            assert!(matches!(f.edit(Input::Enter), Edit::Rejected(_)));
            assert_eq!(f.edit(Input::Char('F')), Edit::Changed);
            assert_eq!(f.edit(Input::Enter), Edit::Submit);
            assert!(matches!(f.edit(Input::Char('0')), Edit::Rejected(_)));
        }
        assert!(Field::with_value(Kind::TronAddress, "T0OlI").is_err());
        assert!(Field::with_value(Kind::WordCount, "13").is_err());
    }

    #[test]
    fn editing_in_the_middle_preserves_the_alphabet_and_ascii_cursor() {
        let mut f = Field::with_value(Kind::Filter, "ac").unwrap();
        f.edit(Input::Left);
        f.edit(Input::Char('b'));
        assert_eq!(f.value(), "abc");
        assert_eq!(f.cursor(), 2);
        f.edit(Input::Delete);
        f.edit(Input::Home);
        f.edit(Input::Delete);
        assert_eq!(f.value(), "b");
        assert_eq!(f.cursor(), 0);
        f.edit(Input::End);
        f.edit(Input::Backspace);
        assert_eq!(f.value(), "");
    }

    #[test]
    fn signed_limits_distinguish_zero_none_raw_units_and_overflow() {
        for kind in [Kind::LimitAmount, Kind::LimitRaw, Kind::LimitBytes] {
            for value in ["0", "none"] {
                assert_eq!(
                    Field::with_value(kind, value).unwrap().edit(Input::Enter),
                    Edit::Submit
                );
            }
            assert!(matches!(
                Field::with_value(kind, "non").unwrap().edit(Input::Enter),
                Edit::Rejected(_)
            ));
        }
        assert!(Field::with_value(Kind::LimitRaw, "1.5").is_err());
        assert_eq!(
            Field::with_value(Kind::LimitBytes, "4294967295")
                .unwrap()
                .edit(Input::Enter),
            Edit::Submit
        );
        assert!(matches!(
            Field::with_value(Kind::LimitBytes, "4294967296")
                .unwrap()
                .edit(Input::Enter),
            Edit::Rejected(_)
        ));
        assert!(matches!(
            Field::with_value(Kind::LimitRaw, &"9".repeat(78))
                .unwrap()
                .edit(Input::Enter),
            Edit::Rejected(_)
        ));
    }

    #[test]
    fn rpc_input_hides_credentials_before_and_after_the_at_sign() {
        let mut f = Field::new(Kind::RpcUrl);
        for c in "https://alice:secret@base.example/rpc?key=private".chars() {
            f.edit(Input::Char(c));
            let drawn = f.drawn();
            assert!(!drawn.contains("alice") && !drawn.contains("secret"));
            assert!(!drawn.contains("private") && !drawn.contains("?key="));
            assert!(!format!("{f:?}").contains("secret"));
        }
        assert!(f.drawn().starts_with("https://base.example"));
        assert!(f.drawn().ends_with(&"*".repeat("/rpc?key=private".len())));
        assert_eq!(f.edit(Input::Enter), Edit::Submit);
        assert!(
            Field::with_value(Kind::RpcUrl, "https://deadbeef0123.eth.example/a")
                .unwrap()
                .drawn()
                .starts_with("https://<redacted>.eth.example")
        );
        assert!(
            Field::with_value(Kind::RpcUrl, "file:///tmp/wallet")
                .unwrap()
                .edit(Input::Enter)
                != Edit::Submit
        );
    }
}
