//! Text zwallet does not control (a file anyone of this user can edit, a process answering on
//! the daemon socket or an RPC endpoint, an executable path, the send journal) made inert where it
//! comes in, by one rule: only printable ASCII (0x20..=0x7E) other than the backslash passes;
//! every other character, `\n`, `\t`, `\` and every non-ASCII char included, becomes a `\u{…}`
//! escape. Each source also cuts what it takes in to a total, here or where it is shown. The write
//! point ([`crate::core::terminal`]) filters again whatever a source missed, but cannot cut.

/// How many characters of free text from outside (an error relayed from the daemon socket or an
/// RPC endpoint, a whitelist reason, a whole relayed value) one message shows, cut marker included.
pub const PROSE_MAX: usize = 512;

/// How many characters of one short value from outside (an error detail, a cached address, an
/// executable path, a whitelist entry field) a message shows.
pub const VALUE_MAX: usize = 256;

/// The marker a cut leaves, in ASCII so it is inert itself.
const CUT: &str = "...(cut)";

/// Whether `b` is a printable ASCII byte (space through `~`).
fn ok(b: u32) -> bool {
    (0x20..=0x7e).contains(&b)
}

/// `c` as a `\u{hex}` escape.
fn u_escape(c: char) -> String {
    format!("\\u{{{:x}}}", c as u32)
}

/// `text` with every character that is not printable ASCII (0x20..=0x7E), and the backslash,
/// turned into a `\u{hex}` escape: control characters, `\n`, `\t`, all non-ASCII (blocks, emoji,
/// bidi controls, letters with accents). The result is all printable ASCII, so it has no line of
/// its own and draws nothing. The backslash is escaped too (`\u{5c}`), so the encoding is
/// one-to-one: a real ESC and the six typed characters `\u{1b}` never look the same.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if ok(c as u32) && c != '\\' {
            out.push(c);
        } else {
            out.push_str(&u_escape(c));
        }
    }
    out
}

/// `ascii` (printable ASCII only) cut so that the whole result, [`CUT`] marker included, is at
/// most `max` characters. Below the marker's own length the cut has no marker.
fn cut_to(ascii: String, max: usize) -> String {
    if ascii.len() <= max {
        return ascii;
    }
    // All ASCII: a byte index is a char index.
    match max.checked_sub(CUT.len()) {
        Some(keep) => format!("{}{CUT}", &ascii[..keep]),
        None => ascii[..max].to_string(),
    }
}

/// [`escape`] `text`, then cut so that the whole result, the marker included, is at most `max`
/// characters. For any one value from outside a message shows.
pub fn ascii_inert(text: &str, max: usize) -> String {
    cut_to(escape(text), max)
}

/// A value from outside, as a short identifier in a message: [`ascii_inert`] to `max`.
pub fn shown(value: &str, max: usize) -> String {
    ascii_inert(value, max)
}

/// `text` for one line of human output: printable ASCII passes, the backslash included (so a
/// string a source already [`escape`]d keeps its one escape), and everything else becomes a
/// `\u{hex}` escape.
fn one_line(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if ok(c as u32) {
            out.push(c);
        } else {
            out.push_str(&u_escape(c));
        }
    }
    out
}

/// `value` as compact JSON-like text for a human: strings quoted and [`one_line`], with no JSON
/// escaping of their own (so an escape a source made is seen once, `\u{2b1b}`, never `\\u{2b1b}`).
fn compact(value: &serde_json::Value, out: &mut String) {
    use serde_json::Value;
    match value {
        Value::String(s) => {
            out.push('"');
            out.push_str(&one_line(s));
            out.push('"');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, v) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                compact(v, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            out.push('{');
            for (i, (k, v)) in map.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push('"');
                out.push_str(&one_line(k));
                out.push_str("\":");
                compact(v, out);
            }
            out.push('}');
        }
        other => out.push_str(&other.to_string()),
    }
}

/// A whole JSON value from outside rendered for a human and bounded: one printable-ASCII line of
/// at most `budget` characters, the cut marker included. A string is shown bare (no quotes); an
/// array or object as compact JSON-like text. Escaping happens once: a value already
/// [`inert_json`]-ed shows `\u{2b1b}` as it is, and a raw one gets that escape here.
pub fn human_capped(value: &serde_json::Value, budget: usize) -> String {
    let text = match value {
        serde_json::Value::String(s) => one_line(s),
        other => {
            let mut out = String::new();
            compact(other, &mut out);
            out
        }
    };
    cut_to(text, budget)
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

    /// The inverse of [`escape`]: `\u{hex}` back to its character, everything else as it is.
    fn unescape(text: &str) -> String {
        let mut out = String::new();
        let mut rest = text;
        while let Some(i) = rest.find("\\u{") {
            out.push_str(&rest[..i]);
            let end = rest[i..].find('}').expect("closed escape") + i;
            let code = u32::from_str_radix(&rest[i + 3..end], 16).unwrap();
            out.push(char::from_u32(code).unwrap());
            rest = &rest[end + 1..];
        }
        out.push_str(rest);
        out
    }

    #[test]
    fn escape_output_is_always_printable_ascii_and_one_to_one() {
        // Every scalar Unicode code point: escaped, it is printable ASCII, a backslash only as the
        // start of an escape, and it reads back as the same character.
        for u in 0..=0x10FFFF_u32 {
            if let Some(c) = char::from_u32(u) {
                let e = escape(&c.to_string());
                assert!(e.bytes().all(|b| ok(b as u32)), "U+{u:04X} -> {e:?}");
                assert!(
                    !e.contains('\\') || e.starts_with("\\u{"),
                    "U+{u:04X} -> {e:?}"
                );
                assert_eq!(unescape(&e), c.to_string(), "U+{u:04X}");
            }
        }
        // A real ESC and the six characters `\u{1b}` stay different.
        assert_ne!(escape("\u{1b}"), escape("\\u{1b}"));
        assert_eq!(escape("\\u{1b}"), "\\u{5c}u{1b}");
        // Mixed text reads back whole.
        let mixed = "a\\b\u{1b}[2J\n\u{2b1b}\"q\" \\u{1b}";
        assert_eq!(unescape(&escape(mixed)), mixed);
    }

    #[test]
    fn escape_turns_every_non_ascii_or_control_into_a_u_escape() {
        for c in [
            '\u{1b}',
            '\r',
            '\n',
            '\t',
            '\u{7f}',
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
            '\\',
        ] {
            let e = escape(&format!("a{c}b"));
            assert_eq!(e, format!("a\\u{{{:x}}}b", c as u32), "{c:?}");
        }
        // Printable ASCII other than the backslash, quotes included, passes through unchanged.
        assert_eq!(escape("USDC a-b \"q\" 'x' / ~"), "USDC a-b \"q\" 'x' / ~");
    }

    #[test]
    fn a_cut_result_is_never_longer_than_its_max() {
        let long = "\u{1f7e5}x".repeat(1000);
        for max in (0..40).chain([64, 200, 256, PROSE_MAX]) {
            let cut = ascii_inert(&long, max);
            assert!(cut.len() <= max, "max {max}: {} chars", cut.len());
            assert!(cut.bytes().all(|b| ok(b as u32)), "{cut:?}");
            if max >= CUT.len() {
                assert!(cut.ends_with(CUT), "{cut:?}");
            }
        }
        // Short enough: as it is, no marker.
        assert_eq!(ascii_inert("0xAbC", 5), "0xAbC");
        assert_eq!(ascii_inert("0xAbCd", 5), "0xAbC");
        assert_eq!(ascii_inert(&"y".repeat(20), 12), "yyyy...(cut)");
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
    fn human_capped_bounds_a_nested_value_to_one_inert_line_escaped_once() {
        let v = serde_json::json!({ "entries": vec!["\u{2588}".repeat(50); 64] });
        let line = human_capped(&v, PROSE_MAX);
        assert!(line.bytes().all(|b| ok(b as u32)), "{line:?}");
        assert!(line.len() <= PROSE_MAX, "{}", line.len());
        // A bare string loses no boundary and keeps its content inert.
        assert_eq!(
            human_capped(&serde_json::json!("0xAbC"), PROSE_MAX),
            "0xAbC"
        );
        // One escape whether the value was escaped at its source or not.
        let raw = serde_json::json!({ "k": ["\u{2b1b}\u{1b}"] });
        let once = "{\"k\":[\"\\u{2b1b}\\u{1b}\"]}";
        assert_eq!(human_capped(&raw, PROSE_MAX), once);
        assert_eq!(human_capped(&inert_json(&raw), PROSE_MAX), once);
    }
}
