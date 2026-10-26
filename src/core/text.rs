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
}
