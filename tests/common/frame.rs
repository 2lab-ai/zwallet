/// The sequence a full-screen program writes when it gives the terminal back (crossterm's
/// `LeaveAlternateScreen`); `zwallet tui`'s guard writes it once, last.
pub const LEAVE_ALTERNATE_SCREEN: &[u8] = b"\x1b[?1049l";

/// What a terminal shows after a full-screen program's output, as ratatui's crossterm backend
/// writes it: `CSI row;col H` moves the cursor, `CSI 2J` and `CSI ?1049h` (the alternate screen)
/// clear it, any other CSI (styles, the cursor's visibility, leaving the alternate screen) changes
/// no cell; CR, LF (raw mode: no CR added) and BS move the cursor. Every other ESC and every
/// other control character is kept in `stray`: none may come from a cell's content (03-tui.md
/// §Display integrity rule 1). A sequence the bytes end inside (a lone ESC, or a CSI before its
/// final byte) is `pending`, not stray: a read of a live terminal ends anywhere.
pub struct Frame {
    /// One string per row, every column (trailing spaces included).
    pub rows: Vec<String>,
    pub stray: Vec<String>,
    /// The unfinished sequence the bytes end with, if any.
    pub pending: String,
}

impl Frame {
    pub fn parse(bytes: &[u8], (columns, rows): (u16, u16)) -> Self {
        let blank = || vec![vec![' '; usize::from(columns)]; usize::from(rows)];
        let (mut cells, mut stray, mut pending) = (blank(), Vec::new(), String::new());
        let (mut x, mut y) = (0usize, 0usize);
        let text = String::from_utf8_lossy(bytes);
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\u{1b}' if chars.peek() == Some(&'[') => {
                    chars.next();
                    let mut params = String::new();
                    let end = loop {
                        match chars.next() {
                            Some(c) if ('\u{20}'..='\u{3f}').contains(&c) => params.push(c),
                            other => break other,
                        }
                    };
                    match (end, params.as_str()) {
                        (None, _) => pending = format!("\u{1b}[{params}"),
                        (Some('H'), at) => {
                            let mut at = at.split(';').map(|n| n.parse().unwrap_or(1).max(1));
                            y = at.next().unwrap_or(1) - 1;
                            x = at.next().unwrap_or(1) - 1;
                        }
                        (Some('J'), "2") | (Some('h'), "?1049") => cells = blank(),
                        (Some('@'..='~'), _) => {}
                        _ => stray.push(format!("CSI {params:?} {end:?}")),
                    }
                }
                '\u{1b}' if chars.peek().is_none() => pending.push(c),
                '\r' => x = 0,
                '\n' => y += 1,
                '\u{8}' => x = x.saturating_sub(1),
                c if c.is_control() => stray.push(format!("{c:?}")),
                c => {
                    if let Some(cell) = cells.get_mut(y).and_then(|r| r.get_mut(x)) {
                        *cell = c;
                    }
                    x += 1;
                }
            }
        }
        Self {
            rows: cells.into_iter().map(String::from_iter).collect(),
            stray,
            pending,
        }
    }

    /// The last frame of a full-screen program, from its whole output: what the terminal showed
    /// when the program gave it back (the bytes before [`LEAVE_ALTERNATE_SCREEN`]); `None` before
    /// it has. Every draw is done by then, so the frame is finished. Its `stray` covers only
    /// the pre-restore output; callers must also check [`restored_tail`] for later writes.
    pub fn last(bytes: &[u8], size: (u16, u16)) -> Option<Self> {
        let end = bytes
            .windows(LEAVE_ALTERNATE_SCREEN.len())
            .position(|w| w == LEAVE_ALTERNATE_SCREEN)?;
        Some(Self::parse(&bytes[..end], size))
    }
}

impl std::fmt::Display for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for row in &self.rows {
            writeln!(f, "{}", row.trim_end())?;
        }
        write!(f, "stray: {:?}, pending: {:?}", self.stray, self.pending)
    }
}

/// Only the final bytes after exactly one alternate-screen restore. A duplicate restore is
/// rejected so an unexpected second terminal owner cannot be hidden by suffix parsing.
pub fn restored_tail(bytes: &[u8]) -> Option<&[u8]> {
    let mut restores = bytes
        .windows(LEAVE_ALTERNATE_SCREEN.len())
        .enumerate()
        .filter(|(_, w)| *w == LEAVE_ALTERNATE_SCREEN);
    let (at, _) = restores.next()?;
    if restores.next().is_some() {
        return None;
    }
    Some(&bytes[at + LEAVE_ALTERNATE_SCREEN.len()..])
}
