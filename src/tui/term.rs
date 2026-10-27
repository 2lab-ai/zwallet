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
    use ratatui::Terminal;
    use ratatui::backend::{CrosstermBackend, TestBackend};
    use std::os::fd::{FromRawFd, OwnedFd};
    use std::panic::PanicHookInfo;
    use std::process::{Command, Stdio};
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

    fn count(haystack: &[u8], needle: &[u8]) -> usize {
        haystack
            .windows(needle.len())
            .filter(|w| *w == needle)
            .count()
    }

    fn holds(haystack: &[u8], needle: &[u8]) -> bool {
        count(haystack, needle) > 0
    }

    /// `cells` (column, row, symbol) handed to the `Backend` trait directly, as ratatui's
    /// `Terminal` hands it what its diff found changed. Only for a cell the diff refuses in a
    /// debug build: a lone control character (ratatui-core's `cell_width` asserts on it).
    fn hand_over(backend: &mut impl Backend, cells: &[(u16, u16, &str)]) {
        let cells: Vec<(u16, u16, Cell)> = cells
            .iter()
            .map(|(x, y, symbol)| {
                let mut cell = Cell::default();
                cell.set_symbol(symbol);
                (*x, *y, cell)
            })
            .collect();
        assert!(
            backend
                .draw(cells.iter().map(|(x, y, c)| (*x, *y, c)))
                .is_ok()
        );
        assert!(backend.flush().is_ok());
    }

    /// One frame through ratatui's `Terminal` (its diff, then the filter, then the inner
    /// backend): `cells` (column, row, symbol) on an empty buffer.
    fn frame<B: Backend>(terminal: &mut Terminal<FilteredBackend<B>>, cells: &[(u16, u16, &str)]) {
        let drawn = terminal.draw(|f| {
            for (x, y, symbol) in cells {
                f.buffer_mut()[(*x, *y)].set_symbol(symbol);
            }
        });
        assert!(drawn.is_ok());
    }

    fn row(buffer: &ratatui::buffer::Buffer, y: u16) -> String {
        (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect()
    }

    #[test]
    fn the_filter_replaces_every_cell_outside_the_terminal_set_where_the_backend_receives_it() {
        let mut terminal = Terminal::new(FilteredBackend::new(TestBackend::new(12, 3))).unwrap();
        // Frame 1: ASCII and the registry's `\u{20ae}`, nothing replaced. The `X` at column 9 stays
        // unless the filter writes a space there in frame 2: ratatui's diff does not send the
        // column a wide glyph covers.
        frame(
            &mut terminal,
            &[
                (0, 0, "a"),
                (1, 0, "b"),
                (9, 0, "X"),
                (0, 2, "U"),
                (1, 2, "\u{20ae}"),
            ],
        );
        assert_eq!(terminal.backend().replaced(), 0);
        assert_eq!(
            row(terminal.backend().inner().buffer(), 2),
            "U\u{20ae}          "
        );
        // Frame 2: four bad cells among ASCII ones, `\u{6f22}` at column 8 covering column 9, and
        // one more at the right edge, whose covered column is off the screen.
        frame(
            &mut terminal,
            &[
                (0, 0, "a"),
                (1, 0, "b"),
                (2, 0, "\u{2588}"),
                (4, 0, "\u{250c}"),
                (5, 0, "e\u{301}"),
                (6, 0, "c"),
                (7, 0, "d"),
                (8, 0, "\u{6f22}"),
                (11, 1, "\u{6f22}"),
            ],
        );
        assert_eq!(terminal.backend().replaced(), 5);
        let got = terminal.backend().inner().buffer();
        assert_eq!(row(got, 0), "ab? ??cd?   ");
        assert_eq!(row(got, 1), "           ?");
        // Frame 3: narrow text where the wide glyph was.
        frame(&mut terminal, &[(8, 0, "y"), (9, 0, "z")]);
        assert_eq!(terminal.backend().replaced(), 5);
        assert_eq!(row(terminal.backend().inner().buffer(), 0), "        yz  ");
        // A lone ESC (handed over directly: ratatui's diff asserts on it in a debug build).
        hand_over(terminal.backend_mut(), &[(3, 0, "\u{1b}")]);
        assert_eq!(terminal.backend().replaced(), 6);
        assert_eq!(row(terminal.backend().inner().buffer(), 0), "   ?    yz  ");
    }

    #[test]
    fn a_replaced_cell_leaves_no_stale_column_where_ratatui_counts_it_wide() {
        // ratatui's diff skips the columns it counts a symbol as covering; whatever width it
        // gives a symbol holding a control sequence, the filter blanks those columns.
        let mut terminal = Terminal::new(FilteredBackend::new(TestBackend::new(12, 1))).unwrap();
        frame(
            &mut terminal,
            &(0..12).map(|x| (x, 0, "X")).collect::<Vec<_>>(),
        );
        frame(&mut terminal, &[(0, 0, "\u{1b}[31m"), (6, 0, "k")]);
        assert_eq!(terminal.backend().replaced(), 1);
        let shown = row(terminal.backend().inner().buffer(), 0);
        assert!(shown.starts_with('?') && !shown.contains('X'), "{shown:?}");
    }

    #[test]
    fn terminal_control_from_a_cell_never_reaches_a_crossterm_writer() {
        let out = Shared::default();
        let mut backend = FilteredBackend::new(CrosstermBackend::new(out.clone()));
        hand_over(
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

    /// Held by every test here that takes [`HOLDING`], which is one per process.
    static HOLDING_TESTS: Mutex<()> = Mutex::new(());

    #[test]
    fn one_guard_holds_the_terminal_and_restores_it_once() {
        let _serial = HOLDING_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let out = Shared::default();
        let guard = Guard::on(out.clone()).unwrap();
        // A second guard is refused before it touches the terminal (no raw mode, no hook).
        let refused = Guard::enter(Shared::default()).err().expect("refused");
        assert!(
            refused.to_string().contains("already holds the terminal"),
            "{refused}"
        );
        drop(guard);
        let written = out.bytes();
        assert_eq!(count(&written, LEAVE_ALTERNATE_SCREEN), 1, "{written:?}");
        assert_eq!(count(&written, SHOW_CURSOR), 1, "{written:?}");
        // A guard whose hold the panic hook took back restores nothing again.
        let out = Shared::default();
        let guard = Guard::on(out.clone()).unwrap();
        assert!(HOLDING.swap(false, Ordering::SeqCst));
        drop(guard);
        assert!(!holds(&out.bytes(), LEAVE_ALTERNATE_SCREEN));
    }

    #[test]
    fn the_hook_writes_the_fixed_line_while_held_and_leaves_any_other_panic_to_the_hook_before() {
        for held in [true, false] {
            let holding = Arc::new(AtomicBool::new(held));
            let (out, err) = (Shared::default(), Shared::default());
            let before_ran = Arc::new(AtomicBool::new(false));
            let handled = Arc::new(AtomicBool::new(false));
            let me = std::thread::current().id();
            let previous = Arc::new(std::panic::take_hook());
            {
                let (holding, out, err) = (holding.clone(), out.clone(), err.clone());
                let (before_ran, handled, previous) =
                    (before_ran.clone(), handled.clone(), previous.clone());
                std::panic::set_hook(Box::new(move |info| {
                    if std::thread::current().id() != me {
                        return previous(info);
                    }
                    let before = |_: &PanicHookInfo<'_>| before_ran.store(true, Ordering::SeqCst);
                    let done =
                        on_panic(&holding, &mut out.clone(), &mut err.clone(), &before, info);
                    handled.store(done, Ordering::SeqCst);
                }));
            }
            let caught = std::panic::catch_unwind(|| panic!("secret pw-XYZ-0123456789"));
            let _ = std::panic::take_hook();
            let restore_previous = previous.clone();
            std::panic::set_hook(Box::new(move |info| restore_previous(info)));
            assert!(caught.is_err());
            let (written, said) = (out.bytes(), String::from_utf8(err.bytes()).unwrap());
            if held {
                assert!(handled.load(Ordering::SeqCst));
                assert!(!before_ran.load(Ordering::SeqCst));
                assert!(holds(&written, LEAVE_ALTERNATE_SCREEN), "{written:?}");
                assert!(holds(&written, SHOW_CURSOR), "{written:?}");
                assert_eq!(said, format!("{PANIC_LINE}\n"));
            } else {
                assert!(!handled.load(Ordering::SeqCst));
                assert!(before_ran.load(Ordering::SeqCst));
                assert!(written.is_empty() && said.is_empty());
            }
            assert!(!String::from_utf8_lossy(&written).contains("secret"));
            assert!(!said.contains("secret"));
        }
    }

    /// Set on the child process [`a_panic_while_a_guard_holds_the_terminal_ends_the_process`]
    /// starts, to run [`panic_child`].
    const PANIC_CHILD: &str = "ZWALLET_TEST_TUI_PANIC_CHILD";

    /// In the child process only: takes the terminal (its stdin is a pty) with the real
    /// `Guard::enter` and hook, then panics inside `spawn_blocking`, where tokio catches the
    /// panic. Says `CONTINUED` if it gets past it.
    #[test]
    fn panic_child() {
        if std::env::var_os(PANIC_CHILD).is_none() {
            return;
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let _guard = Guard::enter(std::io::stdout()).expect("stdin is a pty");
            let joined = tokio::task::spawn_blocking(|| panic!("secret pw-XYZ-0123456789")).await;
            println!("CONTINUED after a caught panic: {}", joined.is_err());
        });
    }

    #[test]
    fn a_panic_while_a_guard_holds_the_terminal_ends_the_process() {
        let (mut master, mut slave) = (-1, -1);
        // SAFETY: openpty writes two fds into the out-params; null name/termios/winsize are
        // allowed.
        let rc = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(rc, 0);
        // SAFETY: both fds were just returned by openpty and are owned by nobody else.
        let (_master, slave) =
            unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
        let out = Command::new(std::env::current_exe().unwrap())
            .args(["tui::term::tests::panic_child", "--exact", "--nocapture"])
            .env(PANIC_CHILD, "1")
            .stdin(Stdio::from(slave))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .unwrap();
        let (stdout, stderr) = (
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
        assert_eq!(out.status.code(), Some(101), "{stdout}\n{stderr}");
        assert!(stderr.contains(&format!("{PANIC_LINE}\n")), "{stderr}");
        assert!(!stdout.contains("CONTINUED"), "{stdout}");
        // The terminal was given back through stdout, and the payload went nowhere.
        assert!(holds(&out.stdout, LEAVE_ALTERNATE_SCREEN), "{stdout}");
        for text in [&stdout, &stderr] {
            assert!(
                !text.contains("secret") && !text.contains("panicked"),
                "{text}"
            );
        }
    }
}
