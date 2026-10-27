//! Drawing: one frame from the [`App`], nothing else. Outside values are drawn only as the
//! [`Outside`] the state made of them, each on its own line, cut to the width (an error alone in a
//! pane is wrapped there, 03-tui.md §Display integrity rule 4); the chrome is ASCII; every style
//! is one of `style`'s. [`draw`] returns where it put each outside value, for the tests.

use crate::tui::app::{App, Bar, KeyList, LockLine, Pane, TrustLine};
use crate::tui::style::{ERROR, HINT, NORMAL, SELECTED, TITLE, WARNING};
use crate::tui::text::Outside;
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
    Row::default()
        .text("zwallet tui", TITLE)
        .text(SEP, NORMAL)
        .text("keys", NORMAL)
        .draw(frame, title, &mut placed);
    keys_pane(frame, pane, app, &mut placed);
    match app.notice() {
        Some(error) => Row::default().outside(error, ERROR),
        None => Row::default().text(
            "j/k move   r refresh   L lock every zwallet daemon   q quit",
            HINT,
        ),
    }
    .draw(frame, hint, &mut placed);
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

fn keys_pane(frame: &mut Frame, area: Rect, app: &App, placed: &mut Vec<Placed>) {
    let block = Block::bordered()
        .border_set(ASCII)
        .title(" keys (* = active) ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    match app.keys() {
        Pane::Loading => {
            Row::default()
                .text("reading wallet.json...", HINT)
                .draw(frame, line(inner, 0), placed)
        }
        Pane::Failed(error) => {
            error_view(frame, inner, "the key list cannot be shown:", error, placed)
        }
        Pane::Ready(list) => key_rows(frame, inner, app.selected(), list, placed),
    }
}

/// The rows (scrolled to keep the selected one in view), then the warning and the note.
fn key_rows(
    frame: &mut Frame,
    area: Rect,
    selected: usize,
    list: &KeyList,
    placed: &mut Vec<Placed>,
) {
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
        let row = Row::default()
            .text(if i == selected { ">" } else { " " }, style)
            .text(if key.active { "* " } else { "  " }, style)
            .outside(&key.name, style)
            .text(
                &" ".repeat(16usize.saturating_sub(key.name.text().len())),
                style,
            )
            .text(&format!(" {:>3}  ", key.index), style);
        match &key.address {
            Some(address) => row.outside(address, style),
            None => row.text("-", style),
        }
        .draw(frame, line(area, y as u16), placed);
    }
    let bottom = area.height.saturating_sub(1);
    if let Some(warning) = &list.warning {
        Row::default()
            .text("warning: ", WARNING)
            .outside(warning, WARNING)
            .draw(frame, line(area, bottom.saturating_sub(1)), placed);
    }
    Row::default()
        .text(
            "not verified: run `zwallet address --verify` before funding",
            HINT,
        )
        .draw(frame, line(area, bottom), placed);
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
            Row::default().text("status: ", ERROR).outside(error, ERROR)
        }
        Pane::Ready(bar) => {
            key_line(first, bar).draw(frame, one, placed);
            lock_line(app, bar)
        }
    };
    second
        .text(SEP, NORMAL)
        .text("rpc ", NORMAL)
        .outside(app.rpc_host(), NORMAL)
        .draw(frame, two, placed);
}

fn key_line(row: Row, bar: &Bar) -> Row {
    if !bar.configured {
        return row.text("no wallet in this home", HINT);
    }
    let mut row = match &bar.key {
        Some(key) => row.outside(key, NORMAL),
        None => row,
    };
    row = match &bar.address {
        Some(address) => row.text(" ", NORMAL).outside(address, NORMAL),
        None => row,
    };
    match &bar.trust {
        Some(TrustLine::Session) => row.text(SEP, NORMAL).text("verified by session", NORMAL),
        Some(TrustLine::NotVerified) if cfg!(target_os = "linux") => row
            .text(SEP, NORMAL)
            .text("not verified (no session check on Linux)", HINT),
        Some(TrustLine::NotVerified | TrustLine::Locked) => {
            row.text(SEP, NORMAL).text("not verified", HINT)
        }
        Some(TrustLine::Impostor(warning)) => row
            .text(SEP, NORMAL)
            .text("NOT this build's key daemon: ", WARNING)
            .outside(warning, WARNING),
        None => row,
    }
}

fn lock_line(app: &App, bar: &Bar) -> Row {
    if app.locking() {
        return Row::default().text("locking every zwallet daemon...", HINT);
    }
    match &bar.lock {
        LockLine::Locked => Row::default().text("locked", NORMAL),
        LockLine::Open(Some(until)) => Row::default()
            .text("signing open (agent too) until ", WARNING)
            .outside(until, WARNING),
        LockLine::Open(None) => Row::default().text("signing open (agent too)", WARNING),
    }
}

/// One line of text being built: its spans, and where each outside value in it starts.
#[derive(Default)]
struct Row {
    spans: Vec<Span<'static>>,
    width: usize,
    outside: Vec<(usize, Outside)>,
}

impl Row {
    /// zwallet's own text (ASCII).
    fn text(mut self, text: &str, style: Style) -> Self {
        self.width += text.len();
        self.spans.push(Span::styled(text.to_string(), style));
        self
    }

    fn outside(mut self, value: &Outside, style: Style) -> Self {
        self.outside.push((self.width, value.clone()));
        self.text(value.text(), style)
    }

    /// Draws the line into `area` (one row), cut at its width, never wrapped.
    fn draw(self, frame: &mut Frame, area: Rect, placed: &mut Vec<Placed>) {
        frame.render_widget(Paragraph::new(Line::from(self.spans)), area);
        for (at, outside) in self.outside {
            let x = area
                .x
                .saturating_add(u16::try_from(at).unwrap_or(u16::MAX))
                .min(area.right());
            let width = u16::try_from(outside.text().len())
                .unwrap_or(u16::MAX)
                .min(area.right() - x);
            placed.push(Placed {
                outside,
                area: Rect::new(x, area.y, width, 1),
            });
        }
    }
}
