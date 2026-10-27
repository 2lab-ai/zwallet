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

/// Every style above.
pub const ALL: [Style; 6] = [NORMAL, TITLE, SELECTED, HINT, WARNING, ERROR];
