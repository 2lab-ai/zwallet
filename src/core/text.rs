//! Text zwallet does not control (a file anyone of this user can edit, a process answering on
//! the daemon socket or an RPC endpoint, an executable path) made inert before it goes into a
//! message, by one rule: only printable ASCII (0x20..=0x7E) passes; every other character,
//! `\n` and `\t` and every non-ASCII char included, becomes a `\u{…}` escape. So nothing from
//! outside can move a terminal's cursor, start a new line, or draw a shape (a QR code). The same
//! escaped text reaches the terminal and `--json`, and totals are bounded where they are shown.

/// How many characters of free text from outside (an error relayed from the daemon socket or an
/// RPC endpoint, a whitelist reason) one message shows.
pub const PROSE_MAX: usize = 512;

/// How many characters of one short value from outside (an error detail, a cached address, an
/// executable path, a whitelist entry field) a message shows.
pub const VALUE_MAX: usize = 256;

/// The marker a cut leaves, in ASCII so it is inert itself.
const CUT: &str = "...(cut)";

/// Whether `b` is a printable ASCII byte (space through `~`): the only bytes that pass [`escape`].
fn ok(b: u32) -> bool {
    (0x20..=0x7e).contains(&b)
}

/// `text` with every character that is not printable ASCII (0x20..=0x7E) turned into a `\u{hex}`
/// escape: control characters, `\n`, `\t`, all non-ASCII (blocks, emoji, bidi controls, letters
/// with accents). The result is all printable ASCII, so it has no line of its own and draws
/// nothing; backslash, quote and the rest of ASCII pass through.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if ok(c as u32) {
            out.push(c);
        } else {
            out.push_str(&format!("\\u{{{:x}}}", c as u32));
        }
    }
    out
}

/// [`escape`] `text`, then cut to at most `max` characters with an ASCII [`CUT`] marker. For any
/// one value from outside a message shows.
pub fn ascii_inert(text: &str, max: usize) -> String {
    let escaped = escape(text);
    match escaped.char_indices().nth(max) {
        // `escaped` is all ASCII, so the nth char boundary is byte `cut`.
        Some((cut, _)) => format!("{}{CUT}", &escaped[..cut]),
        None => escaped,
    }
}

/// A value from outside, as a short identifier in a message: [`ascii_inert`] to `max`.
pub fn shown(value: &str, max: usize) -> String {
    ascii_inert(value, max)
}

/// A whole JSON value from outside rendered for a human and bounded: compact JSON, then the ASCII
/// rule, then cut to `budget`. A string value is shown bare (no quotes). So a nested object or a
/// long array from the daemon socket becomes one inert, bounded line, never a block to draw with.
pub fn human_capped(value: &serde_json::Value, budget: usize) -> String {
    match value {
        serde_json::Value::String(s) => ascii_inert(s, budget),
        other => ascii_inert(&other.to_string(), budget),
    }
}

/// `value` with every string and object key made [`escape`]d; its structure and counts kept, so
/// `--json` consumers still see every entry. For a JSON value relayed from outside into `--json`.
/// Strings are not cut here (cap prose fields such as `reason` on their own); `--json` is machine
/// output, and [`human_capped`] bounds the human side.
pub fn inert_json(value: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Value::String(s) => Value::String(escape(s)),
        Value::Array(a) => Value::Array(a.iter().map(inert_json).collect()),
        Value::Object(o) => {
            Value::Object(o.iter().map(|(k, v)| (escape(k), inert_json(v))).collect())
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_output_is_always_printable_ascii() {
        // Every scalar Unicode code point: escaped, it is nothing but printable ASCII.
        for u in 0..=0x10FFFF_u32 {
            if let Some(c) = char::from_u32(u) {
                let e = escape(&c.to_string());
                assert!(e.bytes().all(|b| ok(b as u32)), "U+{u:04X} -> {e:?}");
            }
        }
        // And a cut result is still all printable ASCII (the marker included).
        let cut = ascii_inert(&"\u{1f7e5}".repeat(1000), 8);
        assert!(cut.bytes().all(|b| ok(b as u32)), "{cut:?}");
        assert!(cut.ends_with(CUT), "{cut:?}");
    }

    #[test]
    fn escape_turns_every_non_ascii_or_control_into_a_u_escape() {
        for c in [
            '\u{1b}',
            '\r',
            '\n',
            '\t',
            '\u{9b}',
            '\u{202e}',
            '\u{2066}',
            '\u{2028}',
            '\u{2029}',
            '\u{200b}',
            '\u{feff}',
            '\u{2580}',
            '\u{2588}',
            '\u{25a0}',
            '\u{2800}',
            '\u{2b1b}',
            '\u{2b1c}',
            '\u{1f7e5}',
            '\u{1fb00}',
            '\u{1cd00}',
            '₮',
            'é',
            '…',
            '—',
        ] {
            let e = escape(&format!("a{c}b"));
            assert_eq!(e, format!("a\\u{{{:x}}}b", c as u32), "{c:?}");
        }
        // Printable ASCII, backslash and quotes pass through unchanged.
        assert_eq!(escape("USDC a-b \"q\" 'x' \\ /"), "USDC a-b \"q\" 'x' \\ /");
    }

    #[test]
    fn inert_json_escapes_strings_and_keys_but_keeps_structure() {
        let v = inert_json(&serde_json::json!({
            "c\u{1b}": ["a\nb", 1, true, { "k": "\u{202e}" }],
        }));
        assert_eq!(
            v,
            serde_json::json!({ "c\\u{1b}": ["a\\u{a}b", 1, true, { "k": "\\u{202e}" }] })
        );
        // Counts are kept: a long array is not trimmed.
        let big = serde_json::json!(vec!["x"; 100]);
        assert_eq!(inert_json(&big).as_array().unwrap().len(), 100);
    }

    #[test]
    fn human_capped_bounds_a_nested_value_to_one_inert_line() {
        let v = serde_json::json!({ "entries": vec!["\u{2588}".repeat(50); 64] });
        let line = human_capped(&v, PROSE_MAX);
        assert!(line.bytes().all(|b| ok(b as u32)), "{line:?}");
        assert!(
            line.chars().count() <= PROSE_MAX + CUT.len(),
            "{}",
            line.len()
        );
        // A bare string loses no boundary and keeps its content inert.
        assert_eq!(
            human_capped(&serde_json::json!("0xAbC"), PROSE_MAX),
            "0xAbC"
        );
    }
}
