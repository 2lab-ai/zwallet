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

/// Free text that may quote what zwallet does not control (a reason, an error relayed from the
/// daemon socket), for a terminal and for JSON alike: every character that is not printable as
/// it is (control and format characters such as ESC, CR, LF, C1 controls, U+202E) escaped as
/// `{:?}` would, quotes and backslashes left as they are. Prose stays readable and on one line.
pub fn printable(text: &str) -> String {
    text.chars()
        .flat_map(|c| {
            let escaped: Vec<char> = if matches!(c, '"' | '\\' | '\'') {
                vec![c]
            } else {
                c.escape_debug().collect()
            };
            escaped
        })
        .collect()
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
