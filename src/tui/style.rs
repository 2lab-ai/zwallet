//! Every style the TUI draws with (03-tui.md §Display integrity rule 2, amended by 10-tui-look.md
//! F2). Code picks one of these for each piece of text it draws, and nothing from outside chooses a
//! colour, a reverse video or a background. Every cell's style is one of [`ALL`]: a widget's patched
//! style included, as the frame checks prove.

use ratatui::buffer::Cell;
use ratatui::style::{Color, Modifier, Style};
use twolab_tui::{Motion, Palette, Theme};

/// Text and the empty screen.
pub const NORMAL: Style = Style::new();
/// The title line.
pub const TITLE: Style = Style::new().add_modifier(Modifier::BOLD);
/// The selected row.
pub const SELECTED: Style = Style::new().add_modifier(Modifier::REVERSED);
/// Key hints and notes.
pub const HINT: Style = Style::new().add_modifier(Modifier::DIM);
/// What needs attention: signing is open, a warning about the daemon socket.
pub const WARNING: Style = Style::new().fg(Color::Yellow);
/// An error.
pub const ERROR: Style = Style::new().fg(Color::Red);
/// zwallet's own QR code of a verified address, and nothing else: white half blocks (the light
/// modules) on black (the dark ones), as `crate::qr` draws a code without colour. Under
/// `NO_COLOR` crossterm writes no colour, and the same cells are that code on a dark terminal.
pub const QR: Style = Style::new().fg(Color::White).bg(Color::Black);

/// The palette the `twolab-tui` widgets draw with (10-tui-look.md F2): the 16 named colours of
/// `Theme::ansi()`, so the user's own terminal palette decides the hues, with no yellow, red or
/// green, which would equal [`WARNING`] or [`ERROR`] or read as "verified".
pub const THEME: Theme = Theme {
    palette: Palette::Ansi,
    text: Color::Reset,
    text_secondary: Color::Gray,
    muted: Color::DarkGray,
    border: Color::DarkGray,
    accent: Color::Cyan,
    surface_raised: Color::Reset,
    run: Color::LightBlue,
    wait: Color::LightYellow,
    done: Color::Reset,
    fail: Color::LightRed,
    ok: Color::Reset,
    warn: Color::LightYellow,
    crit: Color::LightRed,
    motion: Motion::Full,
};

/// Frame lines, the tab divider and the bar's separators: `THEME.border()` and `THEME.dim()`.
pub const BORDER: Style = Style::new().fg(Color::DarkGray);
/// A pane's title and the other tabs' labels: `THEME.accent()`.
pub const ACCENT: Style = Style::new().fg(Color::Cyan);
/// A key in the hint row and the chain in the bar: `THEME.accent()` in bold.
pub const ACCENT_BOLD: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
/// The current tab: `THEME.selected()` patched with `THEME.accent()`, in bold.
pub const TAB: Style = Style::new()
    .fg(Color::Cyan)
    .add_modifier(Modifier::REVERSED.union(Modifier::BOLD));
/// What a key in the hint row does: `THEME.secondary()`.
pub const SECONDARY: Style = Style::new().fg(Color::Gray);

/// The chrome styles (10-tui-look.md F1): only in these may a [`FRAME_GLYPHS`] glyph pass the write
/// point, and no outside value is ever drawn in one (`Row::outside` refuses them).
pub const FRAME: [Style; 5] = [BORDER, ACCENT, ACCENT_BOLD, TAB, SECONDARY];

/// The rounded light frame, the only glyphs besides ASCII, `₮` and the QR half blocks that reach the
/// terminal, and only in a [`FRAME`] style.
pub const FRAME_GLYPHS: [char; 6] = ['╭', '╮', '╰', '╯', '─', '│'];

/// Every style above.
pub const ALL: [Style; 12] = [
    NORMAL,
    TITLE,
    SELECTED,
    HINT,
    WARNING,
    ERROR,
    QR,
    BORDER,
    ACCENT,
    ACCENT_BOLD,
    TAB,
    SECONDARY,
];

/// Whether `style` and `other` draw the same: ratatui gives a cell's style every colour field, so
/// an unset colour counts as `Reset`.
fn same(style: Style, other: Style) -> bool {
    let reset = |c: Option<Color>| c.unwrap_or(Color::Reset);
    reset(style.fg) == reset(other.fg)
        && reset(style.bg) == reset(other.bg)
        && style.add_modifier == other.add_modifier
        && style.sub_modifier.is_empty()
        && other.sub_modifier.is_empty()
}

/// Whether a cell's `style` is [`QR`].
pub fn is_qr(style: Style) -> bool {
    same(style, QR)
}

/// Whether a cell's `style` (or a style code is about to draw with) is one of the [`FRAME`] styles.
pub fn is_frame(style: Style) -> bool {
    FRAME.iter().any(|frame| same(style, *frame))
}

/// Whether `c` in a cell of `style` is a frame glyph the write point lets through.
pub fn frame_glyph(c: char, style: Style) -> bool {
    FRAME_GLYPHS.contains(&c) && is_frame(style)
}

/// What `cell` shows in the old ASCII chrome: a frame glyph in a frame style as `+`, `-` or `|`,
/// anything else as it is. The tests read screens through this, so a layout check reads the rows
/// it read before the rounded frame, one byte per column (10-tui-look.md L1.5).
pub fn ascii_chrome(cell: &Cell) -> &str {
    let mut chars = cell.symbol().chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if frame_glyph(c, cell.style()) => match c {
            '─' => "-",
            '│' => "|",
            _ => "+",
        },
        _ => cell.symbol(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_frame_styles_are_the_widgets_and_none_is_a_text_style() {
        use super::*;
        // 10-tui-look.md F2: no yellow or red (they are WARNING and ERROR) and no green (it would
        // read as "verified") anywhere in the widgets' palette.
        for colour in [
            THEME.text,
            THEME.text_secondary,
            THEME.muted,
            THEME.border,
            THEME.accent,
            THEME.surface_raised,
            THEME.run,
            THEME.wait,
            THEME.done,
            THEME.fail,
            THEME.ok,
            THEME.warn,
            THEME.crit,
        ] {
            assert!(
                !matches!(
                    colour,
                    Color::Yellow | Color::Red | Color::Green | Color::LightGreen
                ),
                "{colour:?}"
            );
        }
        // Each frame style is what the widgets draw with THEME, composed as ratatui composes it.
        assert_eq!(THEME.border(), BORDER);
        assert_eq!(THEME.dim(), BORDER);
        assert_eq!(THEME.accent(), ACCENT);
        assert_eq!(THEME.accent().add_modifier(Modifier::BOLD), ACCENT_BOLD);
        assert_eq!(
            THEME
                .selected()
                .patch(THEME.accent())
                .add_modifier(Modifier::BOLD),
            TAB
        );
        assert_eq!(THEME.secondary(), SECONDARY);
        // No frame style draws like a style that carries outside text.
        for frame in FRAME {
            for text in [NORMAL, TITLE, SELECTED, HINT, WARNING, ERROR, QR] {
                assert!(!same(frame, text), "{frame:?} draws like {text:?}");
            }
            assert!(is_frame(frame) && !is_qr(frame));
        }
        for text in [NORMAL, TITLE, SELECTED, HINT, WARNING, ERROR, QR] {
            assert!(!is_frame(text), "{text:?}");
        }
        assert_eq!(ALL.len(), 7 + FRAME.len());
    }

    #[test]
    fn the_old_chrome_reads_a_frame_glyph_only_in_a_frame_style() {
        use super::*;
        let cell = |symbol: &str, style: Style| {
            let mut cell = Cell::default();
            cell.set_symbol(symbol).set_style(style);
            cell
        };
        assert_eq!(ascii_chrome(&cell("╭", BORDER)), "+");
        assert_eq!(ascii_chrome(&cell("╯", BORDER)), "+");
        assert_eq!(ascii_chrome(&cell("─", BORDER)), "-");
        assert_eq!(ascii_chrome(&cell("│", BORDER)), "|");
        assert_eq!(ascii_chrome(&cell("│", NORMAL)), "│");
        assert_eq!(ascii_chrome(&cell("x", BORDER)), "x");
    }

    use std::path::{Path, PathBuf};

    /// Where `source` (Rust text) names [`super::QR`] through this module: a path ending in
    /// `style::QR`, or a `use` of this module (one statement, over any number of lines) that
    /// imports `QR` or everything (`*`), or renames a `style` module. Whole-line comments are
    /// skipped: a doc link draws nothing.
    fn qr_references(source: &str) -> Vec<String> {
        let ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
        let code: Vec<&str> = source
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect();
        let code = code.join("\n");
        let mut found = Vec::new();
        for (at, path) in code.match_indices("style::QR") {
            if !code[at + path.len()..].starts_with(ident) {
                found.push(code[at..].lines().next().unwrap_or_default().to_string());
            }
        }
        for (at, _) in code.match_indices("use ") {
            if code[..at].ends_with(ident) {
                continue;
            }
            let statement = &code[at..];
            let statement = &statement[..statement.find(';').unwrap_or(statement.len())];
            if statement.contains("style as ") {
                found.push(statement.to_string());
                continue;
            }
            let Some(list) = statement.find("style::").map(|i| &statement[i + 7..]) else {
                continue;
            };
            let mut names = list.split(|c: char| !ident(c) && c != '*');
            if names.any(|name| name == "QR" || name == "*") {
                found.push(statement.split_whitespace().collect::<Vec<_>>().join(" "));
            }
        }
        found
    }

    fn rust_files(dir: &Path, into: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                rust_files(&path, into);
            } else if path.extension().is_some_and(|e| e == "rs") {
                into.push(path);
            }
        }
    }

    #[test]
    fn the_qr_scan_catches_the_supported_spellings_and_nothing_else() {
        for named in [
            "frame.render_widget(p.style(crate::tui::style::QR), area);",
            "let s = style::QR;",
            "use crate::tui::style::{\n    ERROR, HINT,\n    QR,\n};",
            "use super::style::QR as Code;",
            "use crate::tui::style::*;",
            "pub use crate::tui::style::{NORMAL, QR};",
            "use crate::tui::{style::{QR}, text::Outside};",
            "use crate::tui::style as s; s.fg(x);",
        ] {
            assert!(!qr_references(named).is_empty(), "{named:?}");
        }
        for not in [
            "/// Only in [`QR`](crate::tui::style::QR).",
            "crate::tui::style::is_qr(cell.style())",
            "use crate::tui::style::{ERROR, HINT, NORMAL};",
            "const QR_HINT: &str = \"Q/Esc\"; let qr = QrCode::new(x);",
            "use crate::qr; let lines = qr::lines(address, false);",
        ] {
            assert!(qr_references(not).is_empty(), "{not:?}");
        }
    }

    /// 03-tui.md §Display integrity: only zwallet's own QR code of a verified address draws in
    /// the QR style, the one style whose half blocks pass the write point (`term.rs`). So no
    /// source under `src/` but this file and the QR view (`ui/receive.rs`) names it.
    #[test]
    fn only_the_qr_view_names_the_qr_style() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        rust_files(&src, &mut files);
        let (style, view) = (src.join("tui/style.rs"), src.join("tui/ui/receive.rs"));
        for file in &files {
            let found = qr_references(&std::fs::read_to_string(file).unwrap());
            if *file == view {
                // The scan sees what it looks for.
                assert!(!found.is_empty(), "{}", file.display());
            } else if *file != style {
                assert!(found.is_empty(), "{}: {found:?}", file.display());
            }
        }
        assert!(files.contains(&view) && files.contains(&style));
        assert!(files.len() > 50, "{} files scanned", files.len());
    }
}
