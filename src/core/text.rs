//! Text zwallet does not control (a file anyone of this user can edit, a process answering on
//! the daemon socket, an executable path) made inert before it goes into a message, so the same
//! safe text reaches the terminal and `--json`.

/// `value` as it may appear inside a message: as is when it is 1 to `max` characters of
/// printable ASCII (no spaces), else quoted with control and format characters escaped (`{:?}`,
/// which escapes ESC, CR, LF and the like) and cut after `max` characters. So a planted terminal
/// escape, a line break that fakes a second line, or an overlong value is shown inert.
pub fn shown(value: &str, max: usize) -> String {
    if !value.is_empty() && value.len() <= max && value.bytes().all(|b| b.is_ascii_graphic()) {
        return value.to_string();
    }
    match value.char_indices().nth(max) {
        Some((cut, _)) => format!("{:?}…", &value[..cut]),
        None => format!("{value:?}"),
    }
}

/// The most characters of free text from outside (an error relayed from the daemon socket or
/// an RPC endpoint) a message shows. A terminal soft-wraps a long line, so a long enough text of
/// block characters could draw a QR code of another address: the smallest code that holds an
/// address (version 2, 25 x 25 modules, 13 rows of half blocks) takes 13 rows of the terminal's
/// width, more than this on any terminal 40 or more columns wide; and block characters are
/// escaped anyway ([`draws_blocks`]).
pub const PROSE_MAX: usize = 512;

/// The most characters of one short value from outside (an error detail, a whitelist entry as
/// relayed) a message shows.
pub const VALUE_MAX: usize = 256;

/// The most items of an array or object from outside that is relayed ([`inert_json_capped`]).
pub const ITEMS_MAX: usize = 64;

/// Whether `c` must never reach a terminal raw: a control character (Cc: C0, DEL, C1), a format
/// character (Cf: bidi overrides and isolates, zero-width characters, the BOM, …), or a line or
/// paragraph separator (Zl, Zp). The same rule the tests assert, independent of how
/// [`printable`] escapes.
pub fn is_terminal_unsafe(c: char) -> bool {
    c.is_control()
        || matches!(
            c as u32,
            0x00AD
                | 0x0600..=0x0605
                | 0x061C
                | 0x06DD
                | 0x070F
                | 0x0890..=0x0891
                | 0x08E2
                | 0x180E
                | 0x200B..=0x200F
                | 0x2028..=0x202E
                | 0x2060..=0x2064
                | 0x2066..=0x206F
                | 0xFEFF
                | 0xFFF9..=0xFFFB
                | 0x110BD
                | 0x110CD
                | 0x13430..=0x1343F
                | 0x1BCA0..=0x1BCA3
                | 0x1D173..=0x1D17A
                | 0xE0001
                | 0xE0020..=0xE007F
        )
}

/// Whether `c` draws a solid shape on a terminal (Block Elements, Geometric Shapes, Braille):
/// the stuff of a QR code. zwallet's own messages never contain one.
pub fn draws_blocks(c: char) -> bool {
    matches!(c as u32, 0x2580..=0x25FF | 0x2800..=0x28FF)
}

/// Free text that may quote what zwallet does not control (a reason, an error relayed from the
/// daemon socket or an RPC endpoint), for a terminal and for JSON alike: every character that is
/// not printable as it is ([`is_terminal_unsafe`], and whatever else `{:?}` escapes) and every
/// block-drawing character ([`draws_blocks`]) escaped, quotes and backslashes left as they are.
/// The result has no line break of its own and moves no cursor, but a long text still wraps on
/// a terminal: cut what may be long ([`printable_cut`], [`PROSE_MAX`]).
pub fn printable(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '"' | '\\' | '\'') {
            out.push(c);
            continue;
        }
        let debug: String = c.escape_debug().collect();
        if debug.chars().count() > 1 {
            out.push_str(&debug);
        } else if is_terminal_unsafe(c) || draws_blocks(c) {
            out.push_str(&format!("\\u{{{:x}}}", c as u32));
        } else {
            out.push(c);
        }
    }
    out
}

/// [`printable`] `text`, cut after `max` characters (with `…`): free text from outside that has
/// no business being long, such as a token's `symbol()`.
pub fn printable_cut(text: &str, max: usize) -> String {
    let p = printable(text);
    match p.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", &p[..cut]),
        None => p,
    }
}

/// [`inert_json`], and bounded: every string cut after `max_str` characters, object keys after
/// 64, and arrays and objects after [`ITEMS_MAX`] items (an array then ends with a string
/// saying how many were left out). For a JSON value relayed from outside.
pub fn inert_json_capped(value: &serde_json::Value, max_str: usize) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Value::String(s) => Value::String(printable_cut(s, max_str)),
        Value::Array(a) => {
            let mut out: Vec<Value> = a
                .iter()
                .take(ITEMS_MAX)
                .map(|v| inert_json_capped(v, max_str))
                .collect();
            if a.len() > ITEMS_MAX {
                out.push(Value::String(format!("… {} more", a.len() - ITEMS_MAX)));
            }
            Value::Array(out)
        }
        Value::Object(o) => Value::Object(
            o.iter()
                .take(ITEMS_MAX)
                .map(|(k, v)| (printable_cut(k, 64), inert_json_capped(v, max_str)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// `value` with every string in it, object keys included, made [`printable`]: a JSON value from
/// the daemon socket, relayed as it is otherwise.
pub fn inert_json(value: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Value::String(s) => Value::String(printable(s)),
        Value::Array(a) => Value::Array(a.iter().map(inert_json).collect()),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, v)| (printable(k), inert_json(v)))
                .collect(),
        ),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shown_leaves_plain_values_and_escapes_the_rest() {
        assert_eq!(shown("0xAbC", 64), "0xAbC");
        assert_eq!(shown("/usr/bin/zwallet", 64), "/usr/bin/zwallet");
        let evil = shown("/tmp/x\u{1b}[2J\nPWNED", 64);
        assert!(!evil.contains('\u{1b}') && !evil.contains('\n'), "{evil}");
        assert!(evil.contains("\\u{1b}") && evil.contains("\\n"), "{evil}");
        // Right-to-left override (a format character) is escaped too.
        assert!(!shown("a\u{202e}b", 64).contains('\u{202e}'));
        // A space or an empty value is quoted, so where it starts and ends is visible.
        assert_eq!(shown("a b", 64), "\"a b\"");
        assert_eq!(shown("", 64), "\"\"");
        // Cut after `max` characters.
        assert_eq!(shown(&"x".repeat(70), 64), format!("{:?}…", "x".repeat(64)));
    }

    #[test]
    fn printable_escapes_every_unsafe_and_block_character() {
        for c in [
            '\u{1b}', '\r', '\n', '\u{9b}', '\u{202e}', '\u{2066}', '\u{2028}', '\u{2029}',
            '\u{200b}', '\u{feff}', '\u{2580}', '\u{2588}', '\u{25a0}', '\u{2800}',
        ] {
            let p = printable(&format!("a{c}b"));
            assert!(!p.contains(c), "{c:?} -> {p:?}");
            assert!(
                !p.chars().any(|x| is_terminal_unsafe(x) || draws_blocks(x)),
                "{p:?}"
            );
        }
        // Text a human reads is left alone.
        assert_eq!(
            printable("USD₮0 a-b \"q\" 'x' \\ é"),
            "USD₮0 a-b \"q\" 'x' \\ é"
        );
        // Bounded.
        let long = printable_cut(&"\u{2588}".repeat(5000), PROSE_MAX);
        assert!(long.chars().count() <= PROSE_MAX + 1, "{}", long.len());
        let v = inert_json_capped(&serde_json::json!(vec!["x".repeat(900); 100]), VALUE_MAX);
        let a = v.as_array().unwrap();
        assert_eq!(a.len(), ITEMS_MAX + 1);
        assert!(a[0].as_str().unwrap().chars().count() <= VALUE_MAX + 1);
        assert_eq!(a[ITEMS_MAX], "… 36 more");
    }

    #[test]
    fn printable_keeps_prose_on_one_inert_line() {
        let said =
            printable("refusing \"x\" \\ ok\r\u{1b}[2Kaddress: 0xA\nverified: true\u{9b}\u{202e}");
        assert!(
            !said.chars().any(|c| c.is_control() || c == '\u{202e}'),
            "{said}"
        );
        assert!(
            said.starts_with("refusing \"x\" \\ ok\\r\\u{1b}[2Kaddress"),
            "{said}"
        );
        assert!(said.contains("\\nverified: true\\u{9b}\\u{202e}"), "{said}");
        let v = inert_json(&serde_json::json!({ "c\u{1b}": ["a\nb", 1, { "k": "\r" }] }));
        assert_eq!(
            v,
            serde_json::json!({ "c\\u{1b}": ["a\\nb", 1, { "k": "\\r" }] })
        );
    }
}
