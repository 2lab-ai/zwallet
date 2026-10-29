//! Drawing: one frame from the [`App`], nothing else. Outside values are drawn only as the
//! [`Outside`] the state made of them, each on its own line, cut to the width (an error alone in a
//! pane is wrapped there, 03-tui.md §Display integrity rule 4); the chrome is ASCII; every style
//! is one of `style`'s. [`draw`] returns where it put each outside value, for the tests.

pub(crate) mod history;
mod transfer;
mod tron_transfer;

use crate::tui::actions::{Availability, Tab};
use crate::tui::app::{
    App, Bar, FieldPurpose, KeyList, LockLine, Pane, PasswordPurpose, PasswordState, Popup,
    TrustLine,
};
use crate::tui::style::{ERROR, HINT, NORMAL, SELECTED, TITLE, WARNING};
use crate::tui::text::{Outside, Source};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::symbols::border;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

/// The smallest terminal the TUI draws in.
pub const MIN_WIDTH: u16 = 80;
pub const MIN_HEIGHT: u16 = 24;

/// The one line drawn on a smaller terminal.
pub const TOO_SMALL: &str = "terminal too small (80x24 needed)";

/// Borders in ASCII: the TUI's own chrome keeps to the cell rule too.
const ASCII: border::Set = border::Set {
    top_left: "+",
    top_right: "+",
    bottom_left: "+",
    bottom_right: "+",
    vertical_left: "|",
    vertical_right: "|",
    horizontal_top: "-",
    horizontal_bottom: "-",
};

const SEP: &str = " | ";

/// The question after `make <key>` that `s` asks: the active key is what the agent signs with.
const MAKE_ACTIVE: &str = " the active key, the agent's default? y = yes, any other key = no";

/// The key list's note while the bar's key is not verified (03-tui.md §Address trust).
const NOT_VERIFIED: &str = "not verified: V checks addresses from the seed before funding";

/// The key list's note while a session of this build verifies the bar's key: the other rows are
/// not verified, and what checks them (TUI-2b makes it the TUI's own password check).
const OTHER_KEYS: &str = "not verified: the other keys' addresses; V verifies every key";

/// The widest key name the key list shows; a longer one is cut to it, ending in `...`. Narrower
/// still when the index is wider than three digits, so the address is always whole (marker 3,
/// name 27, index 6 and address 42 fill the pane's 78 columns).
const NAME_COLUMN: usize = 27;

/// The narrowest bar line 1 cuts the key name to, `...` included (5 characters of the name): the
/// trust words come first, and the name gives way down to this. At 80 columns, beside it, the
/// widest chain (9), the short address (13) and the separators (7): the 40 of
/// `not verified (no session check on Linux)` fit with 3 to spare; an impostor's 29 of
/// `NOT this build's key daemon: ` leave its warning 14 characters with a 9-character chain and
/// 15 with an 8-character one, so the warning gets its full [`WARNING_ROOM`] only with chains of
/// 7 characters or fewer.
const BAR_NAME_MIN: usize = 8;

/// How much of an impostor's warning line 1 keeps room for before the key name gets any more:
/// enough to read whose process it is (`the process an...`).
const WARNING_ROOM: usize = 16;

/// The widest expiry the status bar shows, `YYYY-MM-DD HH:MMZ`: with the words before it (26),
/// a separator and the RPC host (32), line 2 fills 78 columns.
const BAR_EXPIRY: usize = 17;

/// The widest status error line 2 shows before the RPC host (80 columns).
const BAR_ERROR: usize = 37;

/// Where a frame shows an outside value: one row, or for an error alone in a pane its block.
#[derive(Clone, Debug)]
pub struct Placed {
    pub outside: Outside,
    pub area: Rect,
}

/// Draws the frame for `app` and returns where each outside value went.
pub fn draw(frame: &mut Frame, app: &App) -> Vec<Placed> {
    let mut placed = Vec::new();
    let area = frame.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        Row::default()
            .text(TOO_SMALL, NORMAL)
            .draw(frame, line(area, 0), &mut placed);
        return placed;
    }
    let [title, pane, hint, bar1, bar2] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);
    let mut tabs = Row::default();
    for tab in Tab::ALL {
        tabs = tabs.text(
            &format!("{} {}  ", tab.number(), tab.name()),
            if tab == app.tab() { SELECTED } else { NORMAL },
        );
    }
    tabs.draw(frame, title, &mut placed);
    if app.password_state() != PasswordState::Idle {
        keys_pane(frame, pane, app, &mut placed);
    } else if let Some(popup) = &app.navigation().popup {
        popup_pane(frame, pane, app, popup, &mut placed);
    } else if app.tab() == Tab::Send {
        transfer::pane(frame, pane, app, &mut placed);
    } else if app.tab() == Tab::Keys {
        keys_pane(frame, pane, app, &mut placed);
    } else if app.tab() == Tab::Wallet && !app.navigation().every {
        history::draw(frame, pane, app, &mut placed);
    } else {
        data_pane(frame, pane, app, &mut placed);
    }
    let hint_row = if matches!(app.password_state(), PasswordState::Input { .. }) {
        Row::default().text("Enter check   Esc cancel   Ctrl-C quit", HINT)
    } else if app.password_state() == PasswordState::Failed {
        Row::default().text("Enter/Esc dismiss   Ctrl-C quit", HINT)
    } else if app.password_state() != PasswordState::Idle {
        Row::default().text("checking...   Ctrl-C quits when done", HINT)
    } else if app.navigation().popup.is_some() {
        Row::default().text(
            "arrows move/edit   Enter choose   Esc back   Ctrl-C quit",
            HINT,
        )
    } else if app.tab() == Tab::Send {
        if let Some(notice) = &app.transfer().notice {
            Row::default()
                .outside(
                    &notice.cut(usize::from(hint.width).saturating_sub(12)),
                    ERROR,
                )
                .text("   ? actions", HINT)
        } else {
            Row::default().text(app.transfer_hint(), HINT)
        }
    } else if let Some(key) = app.confirming() {
        let room = usize::from(hint.width).saturating_sub(5 + MAKE_ACTIVE.len());
        Row::default()
            .text("make ", WARNING)
            .outside(&key.name.cut(room.max(8)), WARNING)
            .text(MAKE_ACTIVE, WARNING)
    } else if let Some(error) = app.notice() {
        Row::default()
            .outside(
                &error.cut(usize::from(hint.width).saturating_sub(12)),
                ERROR,
            )
            .text("   ? actions", HINT)
    } else {
        let words = match app.tab() {
            Tab::Keys => "Enter open  V verify  A repair  s active  u unlock  ? actions",
            Tab::Wallet => {
                "Tab focus  j/k  Enter detail  / hash  a networks  c chain  r  ? actions"
            }
            Tab::Receive => "V verify  Q QR hint  c chain  r refresh  q quit  ? actions",
            Tab::Session => "u unlock  L lock all  r refresh  q quit  ? actions",
            _ => "1 keys  c chain  L lock all  q quit  ? actions",
        };
        Row::default().text(words, HINT)
    };
    hint_row.draw(frame, hint, &mut placed);
    status_bar(frame, [bar1, bar2], app, &mut placed);
    placed
}

/// Row `y` of `area`.
fn line(area: Rect, y: u16) -> Rect {
    Rect {
        y: area.y + y,
        height: 1,
        ..area
    }
}

fn data_pane(frame: &mut Frame, area: Rect, app: &App, placed: &mut Vec<Placed>) {
    let block = Block::bordered()
        .border_set(ASCII)
        .title(format!(" {} ", app.tab().name()));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    match &app.navigation().lines {
        Pane::Loading => {
            Row::default()
                .text("reading...", HINT)
                .draw(frame, line(inner, 0), placed)
        }
        Pane::Failed(error) => error_view(
            frame,
            inner,
            "the read could not be completed:",
            error,
            placed,
        ),
        Pane::Ready(lines) => draw_lines(frame, inner, lines, app.navigation().scroll, placed),
    }
}

fn draw_lines(
    frame: &mut Frame,
    area: Rect,
    lines: &[Outside],
    scroll: usize,
    placed: &mut Vec<Placed>,
) {
    for (y, value) in lines
        .iter()
        .skip(scroll)
        .take(usize::from(area.height))
        .enumerate()
    {
        Row::default()
            .outside(&value.cut(usize::from(area.width)), NORMAL)
            .draw(frame, line(area, y as u16), placed);
    }
}

fn popup_pane(frame: &mut Frame, area: Rect, app: &App, popup: &Popup, placed: &mut Vec<Placed>) {
    let title = match popup {
        Popup::Menu(menu) => menu.title(),
        Popup::Chains { .. } => "chains".to_string(),
        Popup::Field {
            purpose: FieldPurpose::Duration,
            ..
        } => "unlock duration".to_string(),
        Popup::Field {
            purpose: FieldPurpose::Rpc(_),
            ..
        } => "session RPC endpoint".to_string(),
        Popup::Field {
            purpose: FieldPurpose::TxHash,
            ..
        } => "transaction hash".to_string(),
        Popup::CheckingRpc => "checking endpoint".to_string(),
        Popup::Error(_) => "refused".to_string(),
        Popup::Lines { title, .. } => title.to_string(),
    };
    let block = Block::bordered()
        .border_set(ASCII)
        .title(format!(" {title} "));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    match popup {
        Popup::Menu(menu) => {
            Row::default()
                .text("filter: ", HINT)
                .outside(&Outside::input(menu.filter().drawn().as_str()), NORMAL)
                .draw(frame, line(inner, 0), placed);
            let rows = menu.rows();
            let count = usize::from(inner.height.saturating_sub(6));
            let first = (menu.selected_index() + 1).saturating_sub(count);
            for (y, (index, action)) in rows.iter().enumerate().skip(first).take(count).enumerate()
            {
                let style = if index == menu.selected_index() {
                    SELECTED
                } else {
                    NORMAL
                };
                Row::default()
                    .text(&format!("{:>2} {:<15} ", action.row, action.key), style)
                    .outside(&Outside::input(&format!("[{}]", action.alias)), style)
                    .text(
                        &format!(
                            "{} tab {}",
                            " ".repeat(34usize.saturating_sub(action.alias.len() + 2)),
                            action.tab.number()
                        ),
                        style,
                    )
                    .draw(frame, line(inner, y as u16 + 1), placed);
            }
            if let Some(action) = rows.get(menu.selected_index()) {
                Row::default().text(action.label, TITLE).draw(
                    frame,
                    line(inner, inner.height - 4),
                    placed,
                );
                let reason = match action.availability(&app.menu_context()) {
                    Availability::Ready => "Enter opens this action",
                    Availability::Disabled(reason) => reason,
                };
                Row::default().text(reason, HINT).draw(
                    frame,
                    line(inner, inner.height - 3),
                    placed,
                );
            }
            Row::default()
                .text(
                    "Outside menus: 1-7 tabs  [ ] cycle  j/k move  r refresh  q quit",
                    HINT,
                )
                .draw(frame, line(inner, inner.height - 2), placed);
            Row::default().text(&crate::version_line(), HINT).draw(
                frame,
                line(inner, inner.height - 1),
                placed,
            );
        }
        Popup::Chains { selected } => {
            for (y, chain) in crate::chains::CHAINS
                .iter()
                .enumerate()
                .take(usize::from(inner.height - 2))
            {
                let style = if y == *selected { SELECTED } else { NORMAL };
                let host = app
                    .net_for(chain.key)
                    .map(|n| Outside::rpc_host(&n.rpc_url))
                    .unwrap_or_else(|| Outside::rpc_host(chain.default_rpc()));
                Row::default()
                    .text(
                        &format!(
                            "{:<10} {:<9} {:<5} id {:<7} ",
                            chain.key,
                            chain.name,
                            chain.symbol,
                            chain.chain_id_json()
                        ),
                        style,
                    )
                    .outside(&host, style)
                    .draw(frame, line(inner, y as u16), placed);
            }
            Row::default()
                .text(
                    "Enter selects   e edits this chain's RPC for this TUI session",
                    HINT,
                )
                .draw(frame, line(inner, inner.height - 1), placed);
        }
        Popup::Field { purpose, field } => {
            let label = match purpose {
                FieldPurpose::TxHash => "Transaction hash: 0x followed by 64 hexadecimal digits",
                FieldPurpose::Duration => "Duration, e.g. 10m or 1h:",
                FieldPurpose::Rpc(_) => "HTTP(S) endpoint; credentials and suffix are hidden:",
            };
            Row::default()
                .text(label, TITLE)
                .draw(frame, line(inner, 0), placed);
            let shown = Outside::input(&field.drawn()).cut(usize::from(inner.width));
            Row::default()
                .outside(&shown, NORMAL)
                .draw(frame, line(inner, 2), placed);
            Row::default()
                .text(
                    &format!(
                        "{} of {} bytes",
                        field.value().len(),
                        field.kind().max_len()
                    ),
                    HINT,
                )
                .draw(frame, line(inner, 4), placed);
            if let FieldPurpose::Rpc(chain) = purpose {
                Row::default()
                    .text(
                        &format!("chain: {chain}; checked before use; never saved to a file"),
                        HINT,
                    )
                    .draw(frame, line(inner, 5), placed);
            }
            frame.set_cursor_position((
                inner.x + (field.cursor() as u16).min(inner.width.saturating_sub(1)),
                inner.y + 2,
            ));
        }
        Popup::CheckingRpc => Row::default()
            .text("checking chain identity... Esc cancels", HINT)
            .draw(frame, line(inner, 0), placed),
        Popup::Error(error) => {
            error_view(frame, inner, "the operation was refused:", error, placed)
        }
        Popup::Lines { lines, .. } => {
            draw_lines(frame, inner, lines, app.navigation().popup_scroll, placed)
        }
    }
    if let Some(notice) = &app.navigation().popup_notice {
        let at = line(inner, inner.height - 1);
        frame.render_widget(ratatui::widgets::Clear, at);
        placed.retain(|p| p.area.y != at.y);
        Row::default()
            .outside(&notice.cut(usize::from(inner.width)), ERROR)
            .draw(frame, line(inner, inner.height - 1), placed);
    }
}

fn keys_pane(frame: &mut Frame, area: Rect, app: &App, placed: &mut Vec<Placed>) {
    let block = Block::bordered()
        .border_set(ASCII)
        .title(" keys (* active, > open) ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    match app.password_state() {
        PasswordState::Preparing => {
            Row::default()
                .text("preparing...", HINT)
                .draw(frame, line(inner, 0), placed);
            return;
        }
        PasswordState::Checking => {
            Row::default()
                .text("checking...", HINT)
                .draw(frame, line(inner, 0), placed);
            return;
        }
        PasswordState::Input {
            characters,
            rejected,
        } => {
            let purpose = match app.password_purpose() {
                PasswordPurpose::Warm => "Warm-up: unlock signing and verify addresses",
                PasswordPurpose::Unlock(_) => "Unlock signing for the chosen duration",
                PasswordPurpose::Verify { all: true } => {
                    "Verify every key and the whitelist (no session change)"
                }
                PasswordPurpose::Verify { all: false } => {
                    "Verify the open and active key (no session change)"
                }
                PasswordPurpose::Repair => "Repair every cached address from the seed",
            };
            Row::default()
                .text(purpose, TITLE)
                .draw(frame, line(inner, 0), placed);
            Row::default()
                .text("Wallet password:", TITLE)
                .draw(frame, line(inner, 1), placed);
            if app.navigation().allow_ptrace_scope_0 {
                Row::default()
                    .text("ptrace_scope 0 accepted", WARNING)
                    .draw(frame, line(inner, 2), placed);
            }
            // The complete mask can span the pane; it never contains any input byte.
            let mask = "*".repeat(characters);
            for (y, chunk) in mask.as_bytes().chunks(usize::from(inner.width)).enumerate() {
                if y + 4 >= usize::from(inner.height) {
                    break;
                }
                Row::default()
                    .text(std::str::from_utf8(chunk).expect("ASCII mask"), NORMAL)
                    .draw(frame, line(inner, 3 + y as u16), placed);
            }
            if rejected {
                Row::default()
                    .text("input refused: empty, too long, newline or NUL", ERROR)
                    .draw(frame, line(inner, inner.height.saturating_sub(1)), placed);
            }
            return;
        }
        PasswordState::Idle | PasswordState::Failed => {}
    }
    match app.keys() {
        Pane::Loading => {
            Row::default()
                .text("reading wallet.json...", HINT)
                .draw(frame, line(inner, 0), placed)
        }
        Pane::Failed(error) => {
            error_view(frame, inner, "the key list cannot be shown:", error, placed)
        }
        Pane::Ready(list) => {
            let note = key_note(app, list);
            key_rows(frame, inner, app, list, note, placed)
        }
    }
}

/// The note under the key list. The rows are never marked verified, so it flags them, but never
/// a key the bar shows verified by session: then it names the other keys and what checks them,
/// and with no other key it says nothing.
fn key_note(app: &App, list: &KeyList) -> Option<&'static str> {
    if list
        .rows
        .iter()
        .all(|k| app.family_pinned(&k.key, app.chain()))
    {
        return None;
    }
    if list
        .rows
        .iter()
        .any(|k| app.family_pinned(&k.key, app.chain()))
    {
        return Some("not verified: other keys; `zwallet list --verify` checks them");
    }
    let sessions = list
        .rows
        .iter()
        .filter(|key| {
            app.displayed_address(&key.key)
                .is_some_and(|(_, trust)| trust == TrustLine::Session)
        })
        .count();
    if sessions > 0 {
        (sessions < list.rows.len()).then_some(OTHER_KEYS)
    } else {
        Some(NOT_VERIFIED)
    }
}

/// The rows (scrolled to keep the selected one in view), then the warning and the note.
fn key_rows(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    list: &KeyList,
    note: Option<&str>,
    placed: &mut Vec<Placed>,
) {
    let selected = app.selected();
    let shown = area.height.saturating_sub(2) as usize;
    let first = (selected + 1).saturating_sub(shown);
    for (y, (i, key)) in list
        .rows
        .iter()
        .enumerate()
        .skip(first)
        .take(shown)
        .enumerate()
    {
        let style = if i == selected { SELECTED } else { NORMAL };
        let index = format!(" {:>3}  ", key.index);
        let family_address = app.displayed_address(&key.key).map(|(address, _)| address);
        let address = family_address.as_ref().map_or(1, |a| a.text().len());
        let verified = if app.family_pinned(&key.key, app.chain()) {
            " verified"
        } else {
            ""
        };
        let room =
            usize::from(area.width).saturating_sub(3 + index.len() + address + verified.len());
        let name = key.name.cut(NAME_COLUMN.min(room));
        let row = Row::default()
            .text(
                if app.open_key() == Some(key.key.as_str()) {
                    ">"
                } else {
                    " "
                },
                style,
            )
            .text(if key.active { "* " } else { "  " }, style)
            .outside(&name, style)
            .text(
                &" ".repeat(16usize.min(room).saturating_sub(name.text().len())),
                style,
            )
            .text(&index, style);
        match &family_address {
            Some(address) => row.outside(address, style),
            None => row.text("-", style),
        }
        .text(verified, NORMAL)
        .draw(frame, line(area, y as u16), placed);
    }
    let bottom = area.height.saturating_sub(1);
    if let Some(warning) = &list.warning {
        Row::default()
            .text("warning: ", WARNING)
            .outside(warning, WARNING)
            .draw(frame, line(area, bottom.saturating_sub(1)), placed);
    }
    if let Some(note) = note {
        Row::default()
            .text(note, HINT)
            .draw(frame, line(area, bottom), placed);
    }
}

/// An error alone in `area`: `heading`, then the error wrapped at the area's width, as many rows
/// as it needs (at most 7 for 512 characters at 78 columns), never with another source.
fn error_view(
    frame: &mut Frame,
    area: Rect,
    heading: &str,
    error: &Outside,
    placed: &mut Vec<Placed>,
) {
    Row::default()
        .text(heading, ERROR)
        .draw(frame, line(area, 0), placed);
    let width = usize::from(area.width.max(1));
    let rows: Vec<Line> = error
        .text()
        .as_bytes()
        .chunks(width)
        .map(|chunk| {
            Line::from(Span::styled(
                String::from_utf8_lossy(chunk).into_owned(),
                ERROR,
            ))
        })
        .collect();
    let block = Rect {
        y: area.y + 1,
        width: if rows.len() == 1 {
            area.width.min(error.text().len() as u16)
        } else {
            area.width
        },
        height: (rows.len() as u16).min(area.height.saturating_sub(1)),
        ..area
    };
    frame.render_widget(Paragraph::new(rows), block);
    placed.push(Placed {
        outside: error.clone(),
        area: block,
    });
}

/// Line 1: chain, key, short address, address trust. Line 2: the lock state, the RPC host.
fn status_bar(frame: &mut Frame, [one, two]: [Rect; 2], app: &App, placed: &mut Vec<Placed>) {
    let first = Row::default()
        .text(app.chain().key, NORMAL)
        .text(SEP, NORMAL);
    let second = match app.status() {
        Pane::Loading => {
            first.text("checking...", HINT).draw(frame, one, placed);
            Row::default().text("checking...", HINT)
        }
        Pane::Failed(error) => {
            first.text("status unknown", ERROR).draw(frame, one, placed);
            Row::default()
                .text("status: ", ERROR)
                .outside(&error.cut(BAR_ERROR), ERROR)
        }
        Pane::Ready(bar) => {
            key_line(
                first,
                app.chain().key,
                &app.display_bar(bar),
                usize::from(one.width),
            )
            .draw(frame, one, placed);
            lock_line(app, bar)
        }
    };
    second
        .text(SEP, NORMAL)
        .outside(app.rpc_host(), NORMAL)
        .draw(frame, two, placed);
}

/// Line 1 after `<chain> | `, `width` columns in all: the key name, the short address, then the
/// trust words of this state. The words stay whole and the name gives way, cut with `...` down
/// to [`BAR_NAME_MIN`]; only when the words cannot fit beside that are they cut, with `...` too.
fn key_line(row: Row, chain: &str, bar: &Bar, width: usize) -> Row {
    if !bar.configured {
        return row.text("no wallet in this home", HINT);
    }
    let (words, style, warning) = match &bar.trust {
        Some(TrustLine::Password) => ("verified by password", NORMAL, None),
        Some(TrustLine::Session) => ("verified by session", NORMAL, None),
        Some(TrustLine::NotVerified) if cfg!(target_os = "linux") => {
            ("not verified (no session check on Linux)", HINT, None)
        }
        Some(TrustLine::NotVerified | TrustLine::Locked) => ("not verified", HINT, None),
        Some(TrustLine::Impostor(w)) => ("NOT this build's key daemon: ", WARNING, Some(w)),
        None => ("", NORMAL, None),
    };
    let address = bar.address.as_ref().map_or(0, |a| 1 + a.text().len());
    let trust = if words.is_empty() {
        0
    } else {
        SEP.len() + words.len() + warning.map_or(0, |w| w.text().len().min(WARNING_ROOM))
    };
    let fixed = chain.len() + SEP.len() + address;
    let name = width
        .saturating_sub(fixed + trust)
        .clamp(BAR_NAME_MIN, Source::KeyName.budget());
    let mut row = match &bar.key {
        Some(key) => row.outside(&key.cut(name), NORMAL),
        None => row,
    };
    let name_shown = bar.key.as_ref().map_or(0, |k| k.cut(name).text().len());
    row = match &bar.address {
        Some(address) => row.text(" ", NORMAL).outside(address, NORMAL),
        None => row,
    };
    if words.is_empty() {
        return row;
    }
    let room = width.saturating_sub(fixed + name_shown + SEP.len());
    let row = row.text(SEP, NORMAL).text(&cut_words(words, room), style);
    match warning {
        Some(w) if words.len() <= room => row.outside(w, style),
        _ => row,
    }
}

/// zwallet's own `words` cut to `room` with a visible `...` when they do not fit.
fn cut_words(words: &str, room: usize) -> String {
    if words.len() <= room {
        words.to_string()
    } else if room >= 4 {
        format!("{}...", &words[..room - 3])
    } else {
        ".".repeat(room.min(3))
    }
}

fn lock_line(app: &App, bar: &Bar) -> Row {
    if app.sending() && app.quit_pending() {
        return Row::default().text("sending... (quits when done)", WARNING);
    }
    if app.activating() {
        return Row::default().text(
            if app.quit_pending() {
                "setting active key... (quits when done)"
            } else {
                "setting active key..."
            },
            HINT,
        );
    }
    if app.quit_pending() {
        return Row::default().text(
            if app.locking() {
                "locking... (quits when done)"
            } else {
                "checking... (quits when done)"
            },
            HINT,
        );
    }
    if app.locking() {
        return Row::default().text("locking every zwallet daemon...", HINT);
    }
    match &bar.lock {
        LockLine::Locked => Row::default().text("locked", NORMAL),
        LockLine::Unknown(error) => Row::default()
            .text("status unknown: ", ERROR)
            .outside(&error.cut(27), ERROR),
        LockLine::Open(Some(until)) => Row::default()
            .text("signing (agent too) until ", WARNING)
            .outside(&until.cut(BAR_EXPIRY), WARNING),
        LockLine::Open(None) => Row::default().text("signing (agent too)", WARNING),
    }
}

/// A piece of a [`Row`]: zwallet's own text, or a value from outside.
enum Part {
    Text(String),
    Outside(Outside),
}

/// One line of text being built.
#[derive(Default)]
struct Row {
    parts: Vec<(Part, Style)>,
}

impl Row {
    /// zwallet's own text (ASCII).
    fn text(mut self, text: &str, style: Style) -> Self {
        self.parts.push((Part::Text(text.to_string()), style));
        self
    }

    fn outside(mut self, value: &Outside, style: Style) -> Self {
        self.parts.push((Part::Outside(value.clone()), style));
        self
    }

    /// Draws the line into `area` (one row), never wrapped. zwallet's text is cut at the edge;
    /// an outside value is drawn whole: cut with `...` to the room left when longer, or left out
    /// when not even that fits.
    fn draw(self, frame: &mut Frame, area: Rect, placed: &mut Vec<Placed>) {
        let width = usize::from(area.width);
        let mut spans = Vec::new();
        let mut x = 0;
        for (part, style) in self.parts {
            let text = match part {
                Part::Text(text) => text,
                Part::Outside(value) => {
                    let room = width.saturating_sub(x);
                    if value.text().len() > room && room < 4 {
                        continue;
                    }
                    let value = value.cut(room);
                    let at = area.x + u16::try_from(x).unwrap_or(u16::MAX);
                    let shown = u16::try_from(value.text().len()).unwrap_or(u16::MAX);
                    let text = value.text().to_string();
                    placed.push(Placed {
                        outside: value,
                        area: Rect::new(at, area.y, shown, 1),
                    });
                    text
                }
            };
            x += text.len();
            spans.push(Span::styled(text, style));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }
}
