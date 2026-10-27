//! The TUI's terminal: the one write point for its cells ([`FilteredBackend`]) and the guard that
//! gives the terminal back ([`Guard`]), on drop and from the panic hook (03-tui.md §Display
//! integrity rule 1, §Password handling).

use crate::core::terminal::{terminal_ok, write_human};
use crossterm::cursor::{Hide, Show};
use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::{Backend, ClearType, WindowSize};
use ratatui::buffer::{Cell, CellWidth};
use ratatui::layout::{Position, Size};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

/// The one way the TUI writes cells to a terminal, as `write_human` is the CLI's: a cell whose
/// symbol is not exactly one [`terminal_ok`] character other than the newline reaches the inner
/// backend as `?`, and is counted. A replaced glyph two columns wide becomes one `?`, and this
/// writes a space into the column it covered, whether or not ratatui's diff sends that column.
/// So no byte of a cell's content (an ESC, a CSI or OSC sequence, a BEL, a block glyph) reaches
/// the terminal; the inner backend's own cursor and style sequences do.
pub struct FilteredBackend<B> {
    inner: B,
    replaced: usize,
}

impl<B> FilteredBackend<B> {
    pub fn new(inner: B) -> Self {
        Self { inner, replaced: 0 }
    }

    /// How many cells were replaced since this was made. Every frame the tests draw keeps it 0:
    /// text from outside is made inert where it comes in, and this is the last filter.
    pub fn replaced(&self) -> usize {
        self.replaced
    }

    pub fn inner(&self) -> &B {
        &self.inner
    }
}

/// Whether a cell's symbol may reach the terminal as it is.
fn cell_ok(symbol: &str) -> bool {
    let mut chars = symbol.chars();
    matches!((chars.next(), chars.next()), (Some(c), None) if c != '\n' && terminal_ok(c))
}

/// The columns ratatui counts `cell` as covering. A symbol holding a control character counts
/// as one: ratatui's text never puts one in a cell, and its width function refuses a lone one.
fn covered(cell: &Cell) -> u16 {
    if cell.symbol().chars().any(char::is_control) {
        1
    } else {
        cell.cell_width()
    }
}

impl<B: Backend> Backend for FilteredBackend<B> {
    type Error = B::Error;

    fn draw<'a, I>(&mut self, content: I) -> Result<(), Self::Error>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        let mut cells: Vec<(u16, u16, Cell)> = Vec::new();
        for (x, y, cell) in content {
            if cell_ok(cell.symbol()) {
                cells.push((x, y, cell.clone()));
                continue;
            }
            self.replaced += 1;
            let mut shown = cell.clone();
            shown.set_symbol("?");
            let covered = covered(cell);
            cells.push((x, y, shown.clone()));
            if covered > 1 {
                let width = self.inner.size()?.width;
                shown.set_symbol(" ");
                for column in
                    (x.saturating_add(1)..x.saturating_add(covered)).filter(|c| *c < width)
                {
                    cells.push((column, y, shown.clone()));
                }
            }
        }
        self.inner.draw(cells.iter().map(|(x, y, c)| (*x, *y, c)))
    }

    fn hide_cursor(&mut self) -> Result<(), Self::Error> {
        self.inner.hide_cursor()
    }

    fn show_cursor(&mut self) -> Result<(), Self::Error> {
        self.inner.show_cursor()
    }

    fn get_cursor_position(&mut self) -> Result<Position, Self::Error> {
        self.inner.get_cursor_position()
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> Result<(), Self::Error> {
        self.inner.set_cursor_position(position)
    }

    fn clear(&mut self) -> Result<(), Self::Error> {
        self.inner.clear()
    }

    fn clear_region(&mut self, clear_type: ClearType) -> Result<(), Self::Error> {
        self.inner.clear_region(clear_type)
    }

    fn size(&self) -> Result<Size, Self::Error> {
        self.inner.size()
    }

    fn window_size(&mut self) -> Result<WindowSize, Self::Error> {
        self.inner.window_size()
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.inner.flush()
    }
}

/// The one line the panic hook writes once it has restored the terminal; never the payload,
/// which may quote anything the TUI held.
pub const PANIC_LINE: &str = "zwallet tui stopped unexpectedly; the terminal was restored";

/// Set while a [`Guard`] holds the terminal: the panic hook restores it only then.
static HOLDING: AtomicBool = AtomicBool::new(false);

/// Gives the terminal back to the shell: leaves the alternate screen, shows the cursor, and ends
/// raw mode (which a terminal that never entered it ignores). Errors are ignored: there is no
/// better place left to report them.
fn restore(out: &mut impl Write) {
    let _ = execute!(out, LeaveAlternateScreen, Show);
    let _ = crossterm::terminal::disable_raw_mode();
}

/// What the panic hook does while the TUI holds the terminal: [`restore`] on `out`, then
/// [`PANIC_LINE`] on `err` through the write point.
fn on_panic(out: &mut impl Write, err: &mut impl Write) {
    restore(out);
    let _ = write_human(err, &format!("{PANIC_LINE}\n"));
}

/// Installs the panic hook once per process: while a [`Guard`] holds the terminal, [`on_panic`]
/// on stdout and stderr; otherwise the hook that was there before. It is never removed: a hook
/// cannot be changed from a thread that is panicking, which is when a guard drops on unwinding.
fn install_panic_hook() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let before = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if HOLDING.swap(false, Ordering::SeqCst) {
                on_panic(&mut std::io::stdout(), &mut std::io::stderr());
            } else {
                before(info);
            }
        }));
    });
}

/// The terminal as the TUI holds it: raw mode (no echo), the alternate screen, the cursor hidden.
/// Dropping it, on return or on unwinding, gives the terminal back ([`restore`]); a panic gives
/// it back first, from the hook.
pub struct Guard<W: Write> {
    out: W,
}

impl<W: Write> Guard<W> {
    /// Takes the terminal `out` writes to (raw mode is set on the controlling terminal).
    pub fn enter(out: W) -> std::io::Result<Self> {
        install_panic_hook();
        crossterm::terminal::enable_raw_mode()?;
        Self::on(out)
    }

    /// The alternate screen and a hidden cursor on `out`, restored on drop. Raw mode is the
    /// caller's ([`Guard::enter`]); a test calls this alone.
    fn on(out: W) -> std::io::Result<Self> {
        let mut guard = Self { out };
        HOLDING.store(true, Ordering::SeqCst);
        execute!(guard.out, EnterAlternateScreen, Hide)?;
        Ok(guard)
    }
}

impl<W: Write> Drop for Guard<W> {
    fn drop(&mut self) {
        HOLDING.store(false, Ordering::SeqCst);
        restore(&mut self.out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::{CrosstermBackend, TestBackend};
    use std::sync::{Arc, Mutex};

    /// A writer the test reads back after the code under test wrote to it.
    #[derive(Clone, Default)]
    struct Shared(Arc<Mutex<Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Shared {
        fn bytes(&self) -> Vec<u8> {
            self.0.lock().unwrap().clone()
        }
    }

    fn holds(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    /// `cells` (column, row, symbol) drawn through the `Backend` trait, as ratatui's `Terminal`
    /// hands it what its diff found changed. (The diff itself refuses a cell holding a control
    /// character in a debug build, so the tests hand the cells over themselves.)
    fn draw(backend: &mut impl Backend, cells: &[(u16, u16, &str)]) {
        let cells: Vec<(u16, u16, Cell)> = cells
            .iter()
            .map(|(x, y, symbol)| {
                let mut cell = Cell::default();
                cell.set_symbol(symbol);
                (*x, *y, cell)
            })
            .collect();
        let _ = backend.draw(cells.iter().map(|(x, y, c)| (*x, *y, c)));
        let _ = backend.flush();
    }

    /// One cell per character of `text`, from column `x` of row `y`.
    fn text(x: u16, y: u16, text: &str) -> Vec<(u16, u16, String)> {
        (x..)
            .zip(text.chars())
            .map(|(x, c)| (x, y, c.to_string()))
            .collect()
    }

    #[test]
    fn the_filter_replaces_every_cell_outside_the_terminal_set_where_the_backend_receives_it() {
        let mut backend = FilteredBackend::new(TestBackend::new(12, 2));
        // Frame 1, all ASCII: nothing replaced. The `X` at column 9 stays unless the filter
        // writes a space there in frame 2 (ratatui's diff does not send the column a wide glyph
        // covers).
        draw(&mut backend, &[(0, 0, "a"), (1, 0, "b"), (9, 0, "X")]);
        assert_eq!(backend.replaced(), 0);
        // Frame 2: five bad cells among ASCII ones; `\u{6f22}` at column 8 covers column 9.
        let mut cells = vec![(0, 0, "a"), (1, 0, "b")];
        cells.extend([
            (2, 0, "\u{2588}"),
            (3, 0, "\u{1b}"),
            (4, 0, "\u{250c}"),
            (5, 0, "e\u{301}"),
            (6, 0, "c"),
            (7, 0, "d"),
            (8, 0, "\u{6f22}"),
        ]);
        let ascii = text(0, 1, "all ASCII ~");
        cells.extend(ascii.iter().map(|(x, y, s)| (*x, *y, s.as_str())));
        draw(&mut backend, &cells);
        assert_eq!(backend.replaced(), 5);
        let got = backend.inner().buffer();
        let row = |y: u16| -> String { (0..12).map(|x| got[(x, y)].symbol()).collect() };
        assert_eq!(row(0), "ab????cd?   ");
        assert_eq!(row(1), "all ASCII ~ ");
    }

    #[test]
    fn terminal_control_from_a_cell_never_reaches_a_crossterm_writer() {
        let out = Shared::default();
        let mut backend = FilteredBackend::new(CrosstermBackend::new(out.clone()));
        draw(
            &mut backend,
            &[(0, 0, "\u{1b}[31m"), (4, 0, "k"), (8, 0, "\u{1b}]0;x\u{7}")],
        );
        assert_eq!(backend.replaced(), 2);
        let bytes = out.bytes();
        assert!(!holds(&bytes, b"\x1b[31m"), "{bytes:?}");
        assert!(!holds(&bytes, b"\x1b]0;x"), "{bytes:?}");
        assert!(!bytes.contains(&0x07), "{bytes:?}");
        // A `?` where each of them was: the cursor moved to column 1, then to column 9.
        assert!(holds(&bytes, b"\x1b[1;1H?"), "{bytes:?}");
        assert!(holds(&bytes, b"\x1b[1;9H?"), "{bytes:?}");
    }

    /// What crossterm writes to leave the alternate screen and to show the cursor.
    const LEAVE_ALTERNATE_SCREEN: &[u8] = b"\x1b[?1049l";
    const SHOW_CURSOR: &[u8] = b"\x1b[?25h";

    #[test]
    fn the_guard_restores_the_terminal_on_drop_and_from_the_panic_hook() {
        let out = Shared::default();
        drop(Guard::on(out.clone()).unwrap());
        let written = out.bytes();
        assert!(holds(&written, LEAVE_ALTERNATE_SCREEN), "{written:?}");
        assert!(holds(&written, SHOW_CURSOR), "{written:?}");

        // A panic while the TUI holds the terminal: the hook restores it, then writes its one
        // fixed line, never the payload.
        let (out, err) = (Shared::default(), Shared::default());
        let (hook_out, hook_err) = (out.clone(), err.clone());
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |_| {
            on_panic(&mut hook_out.clone(), &mut hook_err.clone())
        }));
        let caught = std::panic::catch_unwind(|| panic!("payload pw-XYZ-0123456789 \u{1b}[2J"));
        std::panic::set_hook(previous);
        assert!(caught.is_err());
        let written = out.bytes();
        assert!(holds(&written, LEAVE_ALTERNATE_SCREEN), "{written:?}");
        assert!(holds(&written, SHOW_CURSOR), "{written:?}");
        assert_eq!(
            String::from_utf8(err.bytes()).unwrap(),
            "zwallet tui stopped unexpectedly; the terminal was restored\n"
        );
    }
}
