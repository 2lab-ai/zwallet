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
/// better place left to report them.
fn restore(out: &mut impl Write) {
    let _ = execute!(out, LeaveAlternateScreen, Show);
    let _ = crossterm::terminal::disable_raw_mode();
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
/// to `FATAL` restores the terminal on `out` and writes [`PANIC_LINE`] on `err` through the write
/// point. A panic that finds it `FATAL` writes nothing and never runs `before`, which would print
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
                    restore(out);
                    let _ = write_human(err, &format!("{PANIC_LINE}\n"));
                    return Panicked::Fatal;
                }
            }
        }
    }
}

/// Installs the panic hook once per process. A hook runs for every panic, also one tokio catches
/// (a panicking `spawn_blocking` task is a `JoinError` to the TUI), so while a [`Guard`] holds
/// the terminal no panic is survivable: [`on_panic`] gives the terminal back through stdout (a
/// guard of the TUI always holds stdout) and the process ends with [`PANIC_EXIT`]. A panic on
/// another thread meanwhile waits here for that end. Without a guard the hook that was there
/// before runs. It is never removed: a hook cannot be changed from a panicking thread.
fn install_panic_hook() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let before = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let (mut out, mut err) = (std::io::stdout(), std::io::stderr());
            match on_panic(&STATE, &mut out, &mut err, &*before, info) {
                Panicked::Fatal => std::process::exit(PANIC_EXIT),
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
    /// Takes the terminal `out` writes to (raw mode is set on the controlling terminal). Refused,
    /// before anything is touched, unless the terminal is free.
    pub fn enter(out: W) -> std::io::Result<Self> {
        claim()?;
        install_panic_hook();
        if let Err(e) = crossterm::terminal::enable_raw_mode() {
            let _ = STATE.compare_exchange(HELD, IDLE, Ordering::SeqCst, Ordering::SeqCst);
            return Err(e);
        }
        Self::begin(out)
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
    use std::sync::{Arc, Barrier, Mutex};

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

    /// A writer whose first write after it is armed meets the test's thread at `barrier` twice:
    /// once when it is inside the write, once to go on. So a test can act while the code under
    /// test is in the middle of restoring the terminal, without sleeping.
    #[derive(Clone)]
    struct Gate {
        written: Shared,
        armed: Arc<AtomicBool>,
        barrier: Arc<Barrier>,
    }

    impl Gate {
        fn new(armed: bool) -> Self {
            Self {
                written: Shared::default(),
                armed: Arc::new(AtomicBool::new(armed)),
                barrier: Arc::new(Barrier::new(2)),
            }
        }
    }

    impl Write for Gate {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if self.armed.swap(false, Ordering::SeqCst) {
                self.barrier.wait();
                self.barrier.wait();
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
    fn the_hook_writes_the_fixed_line_while_held_and_leaves_any_other_panic_to_the_hook_before() {
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
    fn a_second_panic_while_the_first_restores_the_terminal_is_suppressed_never_delegated() {
        let state = Arc::new(AtomicU8::new(HELD));
        let out = Gate::new(true);
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
        out.barrier.wait();
        let (out_b, err_b) = (Shared::default(), Shared::default());
        let before_b = AtomicBool::new(false);
        let before = |_: &()| before_b.store(true, Ordering::SeqCst);
        let second = on_panic(&state, &mut out_b.clone(), &mut err_b.clone(), &before, &());
        assert_eq!(second, Panicked::Suppressed);
        assert!(!before_b.load(Ordering::SeqCst), "the payload hook ran");
        assert!(out_b.bytes().is_empty() && err_b.bytes().is_empty());
        out.barrier.wait();
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
        let out = Gate::new(false);
        let guard = Guard::on(out.clone()).unwrap();
        out.armed.store(true, Ordering::SeqCst);
        let dropping = std::thread::spawn(move || drop(guard));
        // The drop is inside its restore.
        out.barrier.wait();
        let refused = Guard::enter(Shared::default())
            .err()
            .expect("refused while releasing");
        assert!(
            refused.to_string().contains("already holds the terminal"),
            "{refused}"
        );
        out.barrier.wait();
        dropping.join().unwrap();
        assert!(holds(&out.written.bytes(), LEAVE_ALTERNATE_SCREEN));
        drop(Guard::on(Shared::default()).expect("free once restored"));
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
        let child_slave = slave.try_clone().unwrap();
        let out = Command::new(std::env::current_exe().unwrap())
            .args(["tui::term::tests::panic_child", "--exact", "--nocapture"])
            .env(PANIC_CHILD, "1")
            .stdin(Stdio::from(child_slave))
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
        // The terminal was given back: the screen and the cursor through stdout, canonical mode
        // with echo on the pty; and the payload went nowhere.
        assert!(holds(&out.stdout, LEAVE_ALTERNATE_SCREEN), "{stdout}");
        assert!(holds(&out.stdout, SHOW_CURSOR), "{stdout}");
        // SAFETY: tcgetattr fills a termios for an fd this test owns.
        let modes = unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            assert_eq!(libc::tcgetattr(slave.as_raw_fd(), &mut t), 0);
            t.c_lflag
        };
        assert_ne!(modes & libc::ICANON, 0, "canonical mode is off");
        assert_ne!(modes & libc::ECHO, 0, "echo is off");
        for text in [&stdout, &stderr] {
            assert!(
                !text.contains("secret") && !text.contains("panicked"),
                "{text}"
            );
        }
    }
}
