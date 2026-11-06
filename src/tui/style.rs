//! Every style the TUI draws with (03-tui.md §Display integrity rule 2). Code picks one of these
//! for each piece of text it draws, and nothing from outside chooses a colour, a reverse video or
//! a background. Each cell carries exactly one of them: the UI never layers a style over another.

use ratatui::style::{Color, Modifier, Style};

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

/// Whether a cell's `style` is [`QR`]. Ratatui gives a cell's style every colour field, so the
/// fields [`QR`] sets are compared.
pub fn is_qr(style: Style) -> bool {
    style.fg == QR.fg
        && style.bg == QR.bg
        && style.add_modifier.is_empty()
        && style.sub_modifier.is_empty()
}

/// Every style above.
pub const ALL: [Style; 7] = [NORMAL, TITLE, SELECTED, HINT, WARNING, ERROR, QR];

#[cfg(test)]
mod tests {
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
    fn the_qr_scan_finds_every_way_to_name_the_style_and_nothing_else() {
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
