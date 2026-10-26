//! The write point for everything zwallet prints, as the last filter: whatever a source forgot to
//! make inert cannot move a cursor or draw a shape once it gets here.
//!
//! Human output reaches a terminal only through [`write_human`] (and its [`print_line`] and
//! [`eprint_line`]): every character outside [`TERMINAL_CHARS`] becomes a `\u{hex}` escape, the
//! same shape as [`crate::core::text::escape`]. The one exception is zwallet's own QR code
//! (`crate::qr`), printed after the filtered text by the CLI from a verified address only.
//! `--json` output goes through [`json_ascii`]: every character outside printable ASCII becomes a
//! JSON `\uXXXX` escape, which a JSON parser reads back as the same string.
//!
//! The filter does not know which text came from outside, so it bounds nothing: a newline from
//! outside and a 1 MiB string pass it as they are. Sources still escape and cut what they take in.

use std::io::Write;
use std::ops::RangeInclusive;

/// The characters human output may carry to a terminal: printable ASCII (space through `~`),
/// the newline, and `₮` (U+20AE, in the registry's recorded symbol `USD₮0`). Not `\r`, `\t`, ESC,
/// DEL, C1, or any other non-ASCII character.
pub const TERMINAL_CHARS: [RangeInclusive<char>; 3] =
    [' '..='~', '\n'..='\n', '\u{20ae}'..='\u{20ae}'];

/// Whether `c` is in [`TERMINAL_CHARS`].
pub fn terminal_ok(c: char) -> bool {
    TERMINAL_CHARS.iter().any(|r| r.contains(&c))
}

/// `text` with every character outside [`TERMINAL_CHARS`] as a `\u{hex}` escape. A backslash
/// passes, so text a source already escaped is not escaped again.
pub fn for_terminal(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if terminal_ok(c) {
            out.push(c);
        } else {
            out.push_str(&format!("\\u{{{:x}}}", c as u32));
        }
    }
    out
}

/// Writes `text` for a human to `w`, through [`for_terminal`]. Every human write goes through
/// here: stdout, stderr and the password prompt on `/dev/tty`.
pub fn write_human(w: &mut dyn Write, text: &str) -> std::io::Result<()> {
    w.write_all(for_terminal(text).as_bytes())?;
    w.flush()
}

/// `text` and a newline on stdout, through [`write_human`]. A closed stdout is not an error the
/// command can do anything about (the work is done), so it is ignored rather than a panic.
pub fn print_line(text: &str) {
    let _ = write_human(&mut std::io::stdout().lock(), &format!("{text}\n"));
}

/// `text` and a newline on stderr, through [`write_human`]. A closed stderr must not stop a
/// command (a send in flight least of all), so the error is ignored.
pub fn eprint_line(text: &str) {
    let _ = write_human(&mut std::io::stderr().lock(), &format!("{text}\n"));
}

/// Serialized JSON with every character outside printable ASCII (and the newline of pretty
/// printing) as a JSON `\uXXXX` escape, two for a character above U+FFFF. serde writes such
/// characters only inside string literals (it escapes C0 itself, but not DEL, C1 or any other
/// non-ASCII), so the result is the same JSON value, all of it printable ASCII on the wire.
pub fn json_ascii(json: &str) -> String {
    let mut out = String::with_capacity(json.len());
    for c in json.chars() {
        if c == '\n' || (' '..='~').contains(&c) {
            out.push(c);
        } else {
            let mut units = [0u16; 2];
            for unit in c.encode_utf16(&mut units) {
                out.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    out
}

/// `json` (serialized JSON) and a newline on stdout, through [`json_ascii`]; a closed stdout is
/// ignored as in [`print_line`].
pub fn print_json(json: &str) {
    let mut out = std::io::stdout().lock();
    let _ = out
        .write_all(format!("{}\n", json_ascii(json)).as_bytes())
        .and_then(|()| out.flush());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_terminal_set_is_printable_ascii_the_newline_and_the_tugrik_sign() {
        let allowed: Vec<char> = (0..=0x10FFFF_u32)
            .filter_map(char::from_u32)
            .filter(|&c| terminal_ok(c))
            .collect();
        let mut want: Vec<char> = (0x20..=0x7e_u8).map(char::from).collect();
        want.push('\n');
        want.push('\u{20ae}');
        want.sort();
        assert_eq!(allowed, want);
        assert_eq!(allowed.len(), 97);
    }

    #[test]
    fn for_terminal_escapes_controls_c1_and_glyphs_but_not_backslash() {
        for c in [
            '\u{1b}',
            '\r',
            '\t',
            '\u{7f}',
            '\u{9b}',
            '\u{202e}',
            '\u{2028}',
            '\u{2588}',
            '\u{2b1b}',
            '\u{1fb00}',
            '\u{1cd00}',
            '\u{1f7e5}',
            '\u{2265}',
            'é',
        ] {
            assert_eq!(
                for_terminal(&format!("a{c}b")),
                format!("a\\u{{{:x}}}b", c as u32),
                "{c:?}"
            );
        }
        // Kept: printable ASCII (backslash too, so a source's escape stays one escape), the
        // newline, and the registry's `₮`.
        let kept = "verified: false\nUSD\u{20ae}0 \\u{1b} \"q\" ~";
        assert_eq!(for_terminal(kept), kept);
        // Whatever comes in, what goes out is in the set.
        let mut every = String::new();
        for u in (0..=0x10FFFF_u32).step_by(7) {
            every.extend(char::from_u32(u));
        }
        assert!(for_terminal(&every).chars().all(terminal_ok));
    }

    #[test]
    fn write_human_filters_what_it_writes() {
        let mut buf = Vec::new();
        write_human(&mut buf, "x\u{1b}[2J\u{9b}\u{2b1b}\ny").unwrap();
        assert_eq!(buf, b"x\\u{1b}[2J\\u{9b}\\u{2b1b}\ny");
    }

    #[test]
    fn json_ascii_is_ascii_and_parses_back_to_the_same_value() {
        let original = serde_json::json!({
            "s": "\u{2b1b}\u{9b}\u{7f}\u{1cd00}\u{1b}[2J\\ \"q\" USD\u{20ae}0",
            "\u{2b1b}k": [1, "é"],
        });
        for text in [
            serde_json::to_string_pretty(&original).unwrap(),
            serde_json::to_string(&original).unwrap(),
        ] {
            let ascii = json_ascii(&text);
            assert!(
                ascii
                    .bytes()
                    .all(|b| b == b'\n' || (0x20..=0x7e).contains(&b)),
                "{ascii}"
            );
            let back: serde_json::Value = serde_json::from_str(&ascii).unwrap();
            assert_eq!(back, original);
        }
        // A character above U+FFFF is a surrogate pair.
        assert_eq!(json_ascii("\"\u{1cd00}\""), "\"\\ud833\\udd00\"");
    }
}
