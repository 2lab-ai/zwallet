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
