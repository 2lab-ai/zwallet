//! The TUI's terminal: the one write point for its cells ([`FilteredBackend`]) and the guard that
//! gives the terminal back ([`Guard`]), on drop and from the panic hook (03-tui.md §Display
//! integrity rule 1, §Password handling).

use crate::core::terminal::{terminal_ok, write_human};
use crossterm::cursor::{Hide, Show};
use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::{Backend, ClearType, WindowSize};
use ratatui::buffer::{Cell, CellDiffOption, CellWidth};
use ratatui::layout::{Position, Size};
use std::io::Write;
use std::os::fd::RawFd;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

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

/// The columns ratatui's diff counts `cell` as covering (it skips all but the first): its own
/// width function, `CellWidth` for a cell. That function counts a lone byte as one column, and in
/// a debug build asserts that it is no control character; here it is counted without the check.
fn covered(cell: &Cell) -> u16 {
    if cell.symbol().len() == 1 && !matches!(cell.diff_option, CellDiffOption::ForcedWidth(_)) {
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
        // The screen's width, read once per draw and only for a wide cell replaced.
        let mut width = None;
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
                let width = match width {
                    Some(w) => w,
                    None => *width.insert(self.inner.size()?.width),
                };
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

    fn append_lines(&mut self, n: u16) -> Result<(), Self::Error> {
        self.inner.append_lines(n)
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

/// How a process ends after a panic while the TUI held the terminal (Rust's own panic code).
const PANIC_EXIT: i32 = 101;

/// Who has the terminal, per process ([`STATE`]). A [`Guard`] takes it from `IDLE` only, and gives
/// it back through `RELEASING` (while it restores, nobody else takes it). A panic while it is
/// `HELD` or `RELEASING` makes it `FATAL` for good: the process is ending.
const IDLE: u8 = 0;
const HELD: u8 = 1;
const RELEASING: u8 = 2;
const FATAL: u8 = 3;

static STATE: AtomicU8 = AtomicU8::new(IDLE);

/// Gives the terminal back to the shell: leaves the alternate screen, shows the cursor, and ends
/// raw mode (which a terminal that never entered it ignores). Errors are ignored: there is no
/// better place left to report them. The guard's path; the panic hook has its own ([`on_panic`]).
fn restore(out: &mut impl Write) {
    let _ = execute!(out, LeaveAlternateScreen, Show);
    let _ = crossterm::terminal::disable_raw_mode();
}

/// What crossterm writes for `LeaveAlternateScreen` and `Show`, for the panic hook to write
/// without crossterm.
const RESTORE_SCREEN: &[u8] = b"\x1b[?1049l\x1b[?25h";

/// The terminal [`Guard::enter`] puts in raw mode: stdin, as crossterm picks it when stdin is a
/// terminal (which `enter` requires).
const TTY: RawFd = 0;

/// [`TTY`]'s modes before the first guard of the process took it, for the panic hook. A
/// `OnceLock` reads without a lock once set; it is set before raw mode is, so a hook that finds
/// it unset has no raw mode to undo.
static MODES: OnceLock<libc::termios> = OnceLock::new();

/// Sets [`TTY`]'s modes back to [`MODES`], with no lock (crossterm's `disable_raw_mode` takes a
/// mutex another thread may hold).
fn restore_modes() {
    if let Some(modes) = MODES.get() {
        // SAFETY: tcsetattr reads a termios this process filled with tcgetattr.
        unsafe {
            libc::tcsetattr(TTY, libc::TCSANOW, modes);
        }
    }
}

/// A file descriptor written with plain `write(2)`: no buffer and no lock, unlike std's stdout
/// and stderr, whose locks a thread that panicked later may hold forever ([`on_panic`]).
struct Fd(RawFd);

impl Write for Fd {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // SAFETY: write(2) reads `buf.len()` bytes from `buf`.
        let n = unsafe { libc::write(self.0, buf.as_ptr().cast(), buf.len()) };
        usize::try_from(n).map_err(|_| std::io::Error::last_os_error())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// What the panic hook decided ([`on_panic`]).
#[derive(Debug, PartialEq, Eq)]
enum Panicked {
    /// This panic took the terminal from its guard: restored, the fixed line written; the
    /// process must end.
    Fatal,
    /// Another panic took it first and is ending the process; this one says nothing.
    Suppressed,
    /// No guard holds the terminal: the hook that was there before ran.
    Delegated,
}

/// The panic hook's decision on `state`. The one panic that moves it from `HELD` or `RELEASING`
/// to `FATAL` gives the terminal back ([`RESTORE_SCREEN`] on `out`, [`restore_modes`]) and writes
/// [`PANIC_LINE`] on `err` through the write point, taking no lock besides what `out` and `err`
/// take. A panic that finds it `FATAL` writes nothing and never runs `before`, which would print
/// its payload. Only while `IDLE` does `before`, the hook that was there before, get `info`.
fn on_panic<I: ?Sized>(
    state: &AtomicU8,
    out: &mut impl Write,
    err: &mut impl Write,
    before: &dyn Fn(&I),
    info: &I,
) -> Panicked {
    loop {
        match state.load(Ordering::SeqCst) {
            FATAL => return Panicked::Suppressed,
            IDLE => {
                before(info);
                return Panicked::Delegated;
            }
            held => {
                let taken = state.compare_exchange(held, FATAL, Ordering::SeqCst, Ordering::SeqCst);
                if taken.is_ok() {
                    let _ = out.write_all(RESTORE_SCREEN);
                    restore_modes();
                    let _ = write_human(err, &format!("{PANIC_LINE}\n"));
                    return Panicked::Fatal;
                }
            }
        }
    }
}

/// [`on_panic`] on file descriptors `out` and `err` written with plain `write(2)` ([`Fd`]): the
/// panic hook's path, which must not wait on a lock. A thread that panics while another one is
/// ending the process parks inside the hook before it unwinds, so it keeps every lock it holds,
/// std's stdout and stderr locks included.
fn emergency<I: ?Sized>(
    state: &AtomicU8,
    out: RawFd,
    err: RawFd,
    before: &dyn Fn(&I),
    info: &I,
) -> Panicked {
    on_panic(state, &mut Fd(out), &mut Fd(err), before, info)
}

/// Installs the panic hook once per process. A hook runs for every panic, also one tokio catches
/// (a panicking `spawn_blocking` task is a `JoinError` to the TUI), so while a [`Guard`] holds
/// the terminal no panic is survivable: [`emergency`] gives the terminal back through fd 1 (a
/// guard of the TUI always holds stdout) and writes the fixed line on fd 2, and the process ends
/// at once with [`PANIC_EXIT`] (`_exit`: no destructor, no atexit, no flush that could wait on a
/// lock). A panic on another thread meanwhile waits here for that end. Without a guard the hook
/// that was there before runs. It is never removed: a hook cannot be changed from a panicking
/// thread.
fn install_panic_hook() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let before = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            match emergency(&STATE, 1, 2, &*before, info) {
                // SAFETY: _exit ends the process; nothing after it runs.
                Panicked::Fatal => unsafe { libc::_exit(PANIC_EXIT) },
                Panicked::Suppressed => loop {
                    std::thread::park();
                },
                Panicked::Delegated => {}
            }
        }));
    });
}

/// The terminal as the TUI holds it: raw mode (no echo), the alternate screen, the cursor hidden.
/// One at a time per process. Dropping it, on return or on unwinding, gives the terminal back
/// ([`restore`]) unless a panic took it, and ended the process.
pub struct Guard<W: Write> {
    out: W,
}

impl<W: Write> Guard<W> {
    /// Takes the terminal: raw mode on stdin, which must be a terminal, and the screen `out`
    /// writes to. Refused, before anything is touched, unless the terminal is free.
    pub fn enter(out: W) -> std::io::Result<Self> {
        claim()?;
        let taken = Self::take_modes();
        if let Err(e) = taken {
            let _ = STATE.compare_exchange(HELD, IDLE, Ordering::SeqCst, Ordering::SeqCst);
            return Err(e);
        }
        Self::begin(out)
    }

    /// Records stdin's modes for the panic hook ([`MODES`]), installs the hook, then sets raw mode.
    fn take_modes() -> std::io::Result<()> {
        // SAFETY: tcgetattr fills a zeroed termios for fd 0, or fails without touching it.
        let modes = unsafe {
            let mut modes: libc::termios = std::mem::zeroed();
            (libc::tcgetattr(TTY, &mut modes) == 0).then_some(modes)
        };
        let modes =
            modes.ok_or_else(|| std::io::Error::other("zwallet tui needs stdin on a terminal"))?;
        MODES.get_or_init(|| modes);
        install_panic_hook();
        crossterm::terminal::enable_raw_mode()
    }

    /// [`Guard::enter`] without raw mode or the panic hook, for a test.
    #[cfg(test)]
    fn on(out: W) -> std::io::Result<Self> {
        claim()?;
        Self::begin(out)
    }

    /// The alternate screen and a hidden cursor on `out`, for a guard that holds the terminal.
    fn begin(out: W) -> std::io::Result<Self> {
        let mut guard = Self { out };
        execute!(guard.out, EnterAlternateScreen, Hide)?;
        Ok(guard)
    }
}

/// Takes [`STATE`] from `IDLE` to `HELD`, or refuses: another guard holds the terminal, is giving
/// it back, or a panic took it.
fn claim() -> std::io::Result<()> {
    STATE
        .compare_exchange(IDLE, HELD, Ordering::SeqCst, Ordering::SeqCst)
        .map(|_| ())
        .map_err(|_| std::io::Error::other("another zwallet tui guard already holds the terminal"))
}

impl<W: Write> Drop for Guard<W> {
    /// Gives the terminal back while still holding it (`RELEASING`), and frees it only after; a
    /// panic meanwhile takes it from there ([`on_panic`]), and then it stays `FATAL`.
    fn drop(&mut self) {
        if STATE
            .compare_exchange(HELD, RELEASING, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            restore(&mut self.out);
            let _ = STATE.compare_exchange(RELEASING, IDLE, Ordering::SeqCst, Ordering::SeqCst);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::{CrosstermBackend, TestBackend};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::process::{Command, Stdio};
    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc::{Receiver, Sender, channel};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    /// The longest a test waits for the code under test: past it, the test fails instead of
    /// hanging.
    const WAIT: Duration = Duration::from_secs(10);

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

    /// A writer whose first write after it is armed tells the test it is inside the write, then
    /// waits for the test to let it go on (at most [`WAIT`]). So a test acts while the code under
    /// test is in the middle of restoring the terminal, without sleeping.
    #[derive(Clone)]
    struct Gate {
        written: Shared,
        armed: Arc<AtomicBool>,
        inside: Sender<()>,
        go_on: Arc<Mutex<Receiver<()>>>,
    }

    /// The test's side of a [`Gate`].
    struct GateControl {
        inside: Receiver<()>,
        go_on: Sender<()>,
    }

    impl GateControl {
        /// Waits until the gated write has begun; fails the test after [`WAIT`].
        fn entered(&self) {
            self.inside
                .recv_timeout(WAIT)
                .expect("the code under test never wrote");
        }

        fn release(&self) {
            let _ = self.go_on.send(());
        }
    }

    fn gate(armed: bool) -> (Gate, GateControl) {
        let (inside, inside_rx) = channel();
        let (go_on_tx, go_on) = channel();
        let gate = Gate {
            written: Shared::default(),
            armed: Arc::new(AtomicBool::new(armed)),
            inside,
            go_on: Arc::new(Mutex::new(go_on)),
        };
        let control = GateControl {
            inside: inside_rx,
            go_on: go_on_tx,
        };
        (gate, control)
    }

    impl Write for Gate {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if self.armed.swap(false, Ordering::SeqCst) {
                let _ = self.inside.send(());
                let _ = self.go_on.lock().unwrap().recv_timeout(WAIT);
            }
            self.written.write(buf)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
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

    /// Held by every test here that takes [`STATE`], which is one per process.
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
        // A guard whose terminal a panic took restores nothing again.
        let out = Shared::default();
        let guard = Guard::on(out.clone()).unwrap();
        assert_eq!(STATE.swap(FATAL, Ordering::SeqCst), HELD);
        drop(guard);
        assert!(!holds(&out.bytes(), LEAVE_ALTERNATE_SCREEN));
        assert_eq!(STATE.swap(IDLE, Ordering::SeqCst), FATAL);
    }

    #[test]
    fn the_hook_ends_a_held_terminal_says_nothing_once_fatal_and_delegates_only_when_idle() {
        for (state, want) in [
            (HELD, Panicked::Fatal),
            (RELEASING, Panicked::Fatal),
            (FATAL, Panicked::Suppressed),
            (IDLE, Panicked::Delegated),
        ] {
            let state = AtomicU8::new(state);
            let (out, err) = (Shared::default(), Shared::default());
            let before_ran = AtomicBool::new(false);
            let before = |payload: &&str| {
                assert_eq!(*payload, "secret pw-XYZ-0123456789");
                before_ran.store(true, Ordering::SeqCst);
            };
            let got = on_panic(
                &state,
                &mut out.clone(),
                &mut err.clone(),
                &before,
                &"secret pw-XYZ-0123456789",
            );
            assert_eq!(got, want);
            let (written, said) = (out.bytes(), String::from_utf8(err.bytes()).unwrap());
            assert_eq!(
                before_ran.load(Ordering::SeqCst),
                want == Panicked::Delegated
            );
            if want == Panicked::Fatal {
                assert_eq!(state.load(Ordering::SeqCst), FATAL);
                assert!(holds(&written, LEAVE_ALTERNATE_SCREEN), "{written:?}");
                assert!(holds(&written, SHOW_CURSOR), "{written:?}");
                assert_eq!(said, format!("{PANIC_LINE}\n"));
            } else {
                assert!(written.is_empty() && said.is_empty());
            }
            assert!(!String::from_utf8_lossy(&written).contains("secret"));
            assert!(!said.contains("secret"));
        }
    }

    #[test]
    fn the_fixed_line_is_in_the_terminal_set_and_needs_no_filter() {
        assert_eq!(PANIC_OUTPUT, format!("{PANIC_LINE}\n").as_bytes());
        let line = std::str::from_utf8(PANIC_OUTPUT).unwrap();
        assert!(line.chars().all(terminal_ok), "{line:?}");
        assert!(RESTORE_SCREEN.len() < 512 && PANIC_OUTPUT.len() < 512);
    }

    #[test]
    fn a_second_panic_while_the_first_restores_the_terminal_is_suppressed_never_delegated() {
        let state = Arc::new(AtomicU8::new(HELD));
        let (out, control) = gate(true);
        let err = Shared::default();
        let before_a = Arc::new(AtomicBool::new(false));
        let first = {
            let (state, out, err, before_a) =
                (state.clone(), out.clone(), err.clone(), before_a.clone());
            std::thread::spawn(move || {
                let before = |_: &()| before_a.store(true, Ordering::SeqCst);
                on_panic(&state, &mut out.clone(), &mut err.clone(), &before, &())
            })
        };
        // The first panic's hook is inside its restore.
        control.entered();
        let (out_b, err_b) = (Shared::default(), Shared::default());
        let before_b = AtomicBool::new(false);
        let before = |_: &()| before_b.store(true, Ordering::SeqCst);
        let second = on_panic(&state, &mut out_b.clone(), &mut err_b.clone(), &before, &());
        control.release();
        assert_eq!(second, Panicked::Suppressed);
        assert!(!before_b.load(Ordering::SeqCst), "the payload hook ran");
        assert!(out_b.bytes().is_empty() && err_b.bytes().is_empty());
        assert_eq!(first.join().unwrap(), Panicked::Fatal);
        assert!(!before_a.load(Ordering::SeqCst));
        assert_eq!(
            String::from_utf8(err.bytes()).unwrap(),
            format!("{PANIC_LINE}\n")
        );
        assert!(holds(&out.written.bytes(), LEAVE_ALTERNATE_SCREEN));
    }

    #[test]
    fn a_guard_being_dropped_holds_the_terminal_until_it_has_restored_it() {
        let _serial = HOLDING_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let (out, control) = gate(false);
        let guard = Guard::on(out.clone()).unwrap();
        out.armed.store(true, Ordering::SeqCst);
        let dropping = std::thread::spawn(move || drop(guard));
        // The drop is inside its restore.
        control.entered();
        let refused = Guard::enter(Shared::default()).err();
        control.release();
        let refused = refused.expect("refused while releasing");
        assert!(
            refused.to_string().contains("already holds the terminal"),
            "{refused}"
        );
        dropping.join().unwrap();
        assert!(holds(&out.written.bytes(), LEAVE_ALTERNATE_SCREEN));
        drop(Guard::on(Shared::default()).expect("free once restored"));
    }

    /// Set on the child process [`panic_in_a_child`] starts, to run [`panic_child`] in a mode:
    /// - `plain`;
    /// - `locks`: another thread holds both stdio locks when the panic comes;
    /// - `stdout-full`, `stderr-full`: that pipe is full and its reader never reads;
    /// - `two`: stdout is full, and a second thread panics while the first panic's hook is
    ///   giving the terminal back.
    const PANIC_CHILD: &str = "ZWALLET_TEST_TUI_PANIC_CHILD";

    /// Fills the pipe on `fd` to its last byte. `O_NONBLOCK` is set on it meanwhile, then taken
    /// off again: this is a test's own pipe, never a terminal.
    fn fill(fd: RawFd) {
        // SAFETY: fcntl and write on an fd this process owns, from a buffer that outlives them.
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFL);
            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
            let block = [b'.'; 512];
            while libc::write(fd, block.as_ptr().cast(), block.len()) > 0 {}
            while libc::write(fd, block.as_ptr().cast(), 1) > 0 {}
            libc::fcntl(fd, libc::F_SETFL, flags);
        }
    }

    /// Writes `text` on fd 2 with plain `write(2)`.
    fn say(text: &[u8]) {
        // SAFETY: write(2) reads `text.len()` bytes from `text`.
        unsafe {
            libc::write(2, text.as_ptr().cast(), text.len());
        }
    }

    /// In the child process only: takes the terminal (its stdin is a pty) with the real
    /// `Guard::enter` and hook, sets up its [`PANIC_CHILD`] mode, then panics inside
    /// `spawn_blocking`, where tokio catches the panic. Says `CONTINUED` if it gets past it.
    #[test]
    fn panic_child() {
        let Some(mode) = std::env::var_os(PANIC_CHILD) else {
            return;
        };
        let mode = mode.to_string_lossy().into_owned();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let _guard = Guard::enter(std::io::stdout()).expect("stdin is a pty");
            match mode.as_str() {
                "locks" => {
                    // Another thread holds both stdio locks, and never lets go.
                    let (held, ready) = channel();
                    std::thread::spawn(move || {
                        let _out = std::io::stdout().lock();
                        let _err = std::io::stderr().lock();
                        held.send(()).unwrap();
                        loop {
                            std::thread::park();
                        }
                    });
                    ready.recv().unwrap();
                }
                "stdout-full" => fill(1),
                "stderr-full" => fill(2),
                "two" => {
                    fill(1);
                    // The second panic comes once the first one's hook has taken the terminal
                    // (it waits on that very state, no timer), while that hook waits on the
                    // full stdout.
                    std::thread::spawn(|| {
                        while STATE.load(Ordering::SeqCst) != FATAL {
                            std::thread::yield_now();
                        }
                        say(b"second: panicking\n");
                        panic!("secret second pw-XYZ-0123456789");
                    });
                }
                _ => {}
            }
            let joined = tokio::task::spawn_blocking(|| panic!("secret pw-XYZ-0123456789")).await;
            println!("CONTINUED after a caught panic: {}", joined.is_err());
        });
    }

    /// A child process killed and reaped when this drops, so a failing assertion never leaves
    /// it running.
    struct Reaped(std::process::Child);

    impl Drop for Reaped {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    /// Runs [`panic_child`] in `mode` with its stdin on a pty, killing it after [`WAIT`] (and
    /// failing then), and asserts how it ended: exit 101; canonical mode with echo back on the
    /// pty; the screen given back on stdout and the fixed line on stderr, where that pipe is
    /// not full; and the payload nowhere. A full pipe is read only once the child is gone.
    /// Returns what stdout and stderr held.
    fn panic_in_a_child(mode: &str) -> (String, String) {
        use std::io::Read;
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
        let mut child = Reaped(
            Command::new(std::env::current_exe().unwrap())
                .args(["tui::term::tests::panic_child", "--exact", "--nocapture"])
                .env(PANIC_CHILD, mode)
                .stdin(Stdio::from(slave.try_clone().unwrap()))
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let (stdout_full, stderr_full) = (
            mode == "stdout-full" || mode == "two",
            mode == "stderr-full",
        );
        let read = |mut r: Box<dyn Read + Send>| {
            let mut v = Vec::new();
            let _ = r.read_to_end(&mut v);
            v
        };
        let mut out: Box<dyn Read + Send> = Box::new(child.0.stdout.take().unwrap());
        let mut err: Box<dyn Read + Send> = Box::new(child.0.stderr.take().unwrap());
        let reading_out = (!stdout_full).then(|| {
            let r = std::mem::replace(&mut out, Box::new(std::io::empty()));
            std::thread::spawn(move || read(r))
        });
        let reading_err = (!stderr_full).then(|| {
            let r = std::mem::replace(&mut err, Box::new(std::io::empty()));
            std::thread::spawn(move || read(r))
        });
        let start = Instant::now();
        let status = loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                break status;
            }
            assert!(
                start.elapsed() < WAIT,
                "{mode}: the child still ran {WAIT:?} after its panic"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        let out = reading_out.map_or_else(|| read(out), |t| t.join().unwrap());
        let err = reading_err.map_or_else(|| read(err), |t| t.join().unwrap());
        let (stdout, stderr) = (
            String::from_utf8_lossy(&out).into_owned(),
            String::from_utf8_lossy(&err).into_owned(),
        );
        assert_eq!(status.code(), Some(101), "{mode}: {stdout}\n{stderr}");
        // SAFETY: tcgetattr fills a termios for an fd this test owns.
        let modes = unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            assert_eq!(libc::tcgetattr(slave.as_raw_fd(), &mut t), 0);
            t.c_lflag
        };
        assert_ne!(modes & libc::ICANON, 0, "{mode}: canonical mode is off");
        assert_ne!(modes & libc::ECHO, 0, "{mode}: echo is off");
        if !stdout_full {
            assert!(holds(&out, LEAVE_ALTERNATE_SCREEN), "{mode}: {stdout}");
            assert!(holds(&out, SHOW_CURSOR), "{mode}: {stdout}");
        }
        if !stderr_full {
            assert!(
                stderr.contains(&format!("{PANIC_LINE}\n")),
                "{mode}: {stderr}"
            );
        }
        assert!(!stdout.contains("CONTINUED"), "{mode}: {stdout}");
        for text in [&stdout, &stderr] {
            assert!(
                !text.contains("secret") && !text.contains("panicked"),
                "{mode}: {text}"
            );
        }
        (stdout, stderr)
    }

    #[test]
    fn a_panic_while_a_guard_holds_the_terminal_ends_the_process() {
        panic_in_a_child("plain");
    }

    #[test]
    fn a_panic_ends_the_process_even_while_another_thread_holds_both_stdio_locks() {
        panic_in_a_child("locks");
    }

    #[test]
    fn a_panic_ends_the_process_and_restores_the_modes_with_stdout_full() {
        panic_in_a_child("stdout-full");
    }

    #[test]
    fn a_panic_ends_the_process_and_restores_the_modes_with_stderr_full() {
        panic_in_a_child("stderr-full");
    }

    #[test]
    fn a_second_panic_while_the_first_ends_the_process_parks_and_says_nothing() {
        let (_, stderr) = panic_in_a_child("two");
        // The second thread got to its panic before the process ended, and its hook printed
        // nothing (no payload, no `panicked at`, checked above).
        assert!(stderr.contains("second: panicking\n"), "{stderr}");
    }

    /// A pipe: `(read end, write end)`.
    fn pipe() -> (std::fs::File, OwnedFd) {
        let mut fds = [-1; 2];
        // SAFETY: pipe writes two fds into the array.
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        // SAFETY: both fds were just returned by pipe and are owned by nobody else.
        unsafe {
            (
                std::fs::File::from_raw_fd(fds[0]),
                OwnedFd::from_raw_fd(fds[1]),
            )
        }
    }

    #[test]
    fn the_emergency_path_takes_no_stdio_lock() {
        use std::io::Read;
        let ((mut out_r, out_w), (mut err_r, err_w)) = (pipe(), pipe());
        let (held, ready) = channel();
        let (release, done) = channel::<()>();
        let holder = std::thread::spawn(move || {
            let _out = std::io::stdout().lock();
            let _err = std::io::stderr().lock();
            held.send(()).unwrap();
            let _ = done.recv();
        });
        ready.recv().unwrap();
        let (sent, result) = channel();
        std::thread::spawn(move || {
            let state = AtomicU8::new(HELD);
            let before = |_: &()| panic!("not idle");
            let got = emergency(&state, out_w.as_raw_fd(), err_w.as_raw_fd(), &before, &());
            drop((out_w, err_w));
            let _ = sent.send(got);
        });
        let got = result.recv_timeout(WAIT);
        release.send(()).unwrap();
        holder.join().unwrap();
        assert_eq!(
            got.expect("the emergency path waited on a stdio lock"),
            Panicked::Fatal
        );
        let (mut out, mut err) = (Vec::new(), String::new());
        out_r.read_to_end(&mut out).unwrap();
        err_r.read_to_string(&mut err).unwrap();
        assert!(holds(&out, LEAVE_ALTERNATE_SCREEN), "{out:?}");
        assert!(holds(&out, SHOW_CURSOR), "{out:?}");
        assert_eq!(err, format!("{PANIC_LINE}\n"));
    }
}
