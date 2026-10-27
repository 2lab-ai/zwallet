//! `zwallet tui`: the terminal UI (`.prd/03-tui.md`). [`run`] holds the terminal and runs the
//! loop: one std thread reads the terminal's events and posts them as [`Msg`]s, the state
//! ([`app`]) answers each with the work it asks for, which runs as tokio tasks on the CLI's
//! runtime and posts its result back; every frame is drawn ([`ui`]) through the one write point
//! ([`term::FilteredBackend`]).

pub mod app;
pub mod style;
pub mod term;
pub mod text;
pub mod ui;

use crate::chains::Net;
use crate::core::error::{Result, WalletError};
use crate::core::protocol;
use crate::wallet;
use app::{App, Effect, Key, Msg};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use term::{FilteredBackend, Guard};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

/// How often the lock state is read again (03-tui.md §Status bar); `r` reads it at once.
pub const REFRESH: Duration = Duration::from_secs(5);

/// How long the input thread waits for an event before it looks whether to stop.
const INPUT_POLL: Duration = Duration::from_millis(100);

/// Runs the TUI for `home` on `net` until `q`, on the terminal stdin and stdout are on. The
/// terminal is given back on return and on a panic.
pub async fn run(home: PathBuf, net: Net, warm: Duration) -> Result<()> {
    if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
        return Err(WalletError::validation(
            "zwallet tui needs a terminal: run it with stdin (which it sets to raw mode) and \
             stdout on one",
        ));
    }
    let terminal_error = |e: std::io::Error| WalletError::validation(format!("terminal: {e}"));
    // First, before any thread or task starts (the input thread, the reads, the timer), as
    // `Guard::enter` requires: a panic while it takes the terminal then comes on this thread.
    // The guard and the backend write through the unlocked `Stdout` handle, which takes its lock
    // per write: no `StdoutLock` is ever held across an `.await`.
    let _guard = Guard::enter(std::io::stdout()).map_err(terminal_error)?;
    let mut app = App::new(&net, warm);
    let mut terminal = Terminal::new(FilteredBackend::new(CrosstermBackend::new(
        std::io::stdout(),
    )))
    .map_err(terminal_error)?;
    let (tx, mut rx) = unbounded_channel();
    let stop = Arc::new(AtomicBool::new(false));
    let input = read_input(tx.clone(), stop.clone());
    let mut tick = tokio::time::interval(REFRESH);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The first tick is now, and the state reads the status at the start already.
    tick.tick().await;
    let mut work = app.start();
    let result = loop {
        for effect in work.drain(..) {
            start(effect, &home, &tx);
        }
        if let Err(e) = terminal.draw(|f| {
            ui::draw(f, &app);
        }) {
            break Err(terminal_error(e));
        }
        if app.quitting() {
            break Ok(());
        }
        let msg = tokio::select! {
            Some(msg) = rx.recv() => msg,
            _ = tick.tick() => Msg::Tick,
        };
        work = app.update(msg);
    };
    stop.store(true, Ordering::SeqCst);
    let _ = input.join();
    match app.take_quit_error() {
        Some(e) => result.and(Err(e)),
        None => result,
    }
}

/// Starts `effect` as a task that posts its result on `tx`.
fn start(effect: Effect, home: &Path, tx: &UnboundedSender<Msg>) {
    let (home, tx) = (home.to_path_buf(), tx.clone());
    match effect {
        Effect::LoadKeys(generation) => {
            tokio::spawn(async move {
                let _ = tx.send(Msg::Keys(generation, wallet::keys(&home).await));
            });
        }
        Effect::LoadStatus(generation) => {
            tokio::spawn(async move {
                let _ = tx.send(Msg::Status(generation, wallet::status(&home).await));
            });
        }
        Effect::LockAll => {
            tokio::task::spawn_blocking(move || {
                let _ = tx.send(Msg::Locked(protocol::stop_all_daemons()));
            });
        }
    }
}

/// The input thread: terminal events as [`Msg`]s on `tx`, until `stop`. If the terminal can no
/// longer be read, nothing could end the TUI, so it posts [`Key::Interrupt`].
fn read_input(tx: UnboundedSender<Msg>, stop: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            let event = event::poll(INPUT_POLL).and_then(|ready| {
                if ready {
                    event::read().map(Some)
                } else {
                    Ok(None)
                }
            });
            let msg = match event {
                Ok(Some(Event::Key(k))) if k.kind == KeyEventKind::Press => key(k).map(Msg::Key),
                Ok(Some(Event::Resize(..))) => Some(Msg::Resize),
                Ok(_) => None,
                Err(_) => {
                    let _ = tx.send(Msg::Key(Key::Interrupt));
                    return;
                }
            };
            if let Some(msg) = msg
                && tx.send(msg).is_err()
            {
                return;
            }
        }
    })
}

fn key(k: KeyEvent) -> Option<Key> {
    Some(match k.code {
        KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => Key::Interrupt,
        KeyCode::Char(c) => Key::Char(c),
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc => Key::Esc,
        _ => return None,
    })
}
