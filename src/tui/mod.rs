//! `zwallet tui`: the terminal UI (`.prd/03-tui.md`). [`run`] holds the terminal and runs the
//! loop: one std thread reads the terminal's events and posts them as [`Msg`]s, the state
//! ([`app`]) answers each with the work it asks for, which runs as tokio tasks on the CLI's
//! runtime and posts its result back; every frame is drawn ([`ui`]) through the one write point
//! ([`term::FilteredBackend`]).

pub mod app;
pub mod password;
pub mod style;
pub mod term;
pub mod text;
pub mod ui;

use crate::chains::Net;
use crate::core::error::{Result, WalletError};
use crate::core::protocol;
use crate::wallet;
use app::{App, Effect, Key, Msg, PasswordState};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use password::PasswordBuf;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc as sync};
use std::time::Duration;
use term::{FilteredBackend, Guard};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

/// How often the lock state is read again (03-tui.md §Status bar); `r` reads it at once.
pub const REFRESH: Duration = Duration::from_secs(5);

/// How long the input thread waits for an event before it looks whether to stop.
const INPUT_POLL: Duration = Duration::from_millis(100);

/// Runs the TUI for `home` on `net` until `q`, on the terminal stdin and stdout are on. The
/// terminal is given back on return and on a panic.
pub async fn run(
    home: PathBuf,
    net: Net,
    warm: Duration,
    allow_ptrace_scope_0: bool,
) -> Result<()> {
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
    let secret = Arc::new(Mutex::new(None));
    let plan = Arc::new(Mutex::new(None));
    let (ack_tx, ack_rx) = sync::sync_channel(1);
    let input = read_input(tx.clone(), stop.clone(), ack_rx, secret.clone());
    let mut input_waiting = false;
    let mut tick = tokio::time::interval(REFRESH);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The first tick is now, and the state reads the status at the start already.
    tick.tick().await;
    let mut work = app.start();
    let result = loop {
        for effect in work.drain(..) {
            start(
                effect,
                &home,
                &tx,
                warm,
                allow_ptrace_scope_0,
                &plan,
                &secret,
            );
        }
        if let Err(e) = terminal.draw(|f| {
            ui::draw(f, &app);
        }) {
            break Err(terminal_error(e));
        }
        if app.quitting() {
            break Ok(());
        }
        // The input thread never reads a following event before the App acknowledges the
        // preceding one. In particular Enter's preparation finishes before typeahead is read.
        if input_waiting && app.password_state() != PasswordState::Preparing {
            let mode = if matches!(app.password_state(), PasswordState::Input { .. }) {
                InputMode::Password
            } else if app.password_state() == PasswordState::Failed {
                InputMode::Discard
            } else {
                InputMode::Normal
            };
            let _ = ack_tx.send(mode);
            input_waiting = false;
        }
        let msg = tokio::select! {
            Some(msg) = rx.recv() => msg,
            _ = tick.tick() => Msg::Tick,
        };
        input_waiting |= matches!(
            &msg,
            Msg::Key(_)
                | Msg::Resize
                | Msg::PasswordLength(..)
                | Msg::PasswordSubmitted
                | Msg::PasswordCancelled
        );
        work = app.update(msg);
    };
    stop.store(true, Ordering::SeqCst);
    drop(ack_tx);
    let _ = input.join();
    match app.take_quit_error() {
        Some(e) => result.and(Err(e)),
        None => result,
    }
}

/// Starts `effect` as a task that posts its result on `tx`.
fn start(
    effect: Effect,
    home: &Path,
    tx: &UnboundedSender<Msg>,
    warm: Duration,
    allow_ptrace_scope_0: bool,
    plan: &Arc<Mutex<Option<wallet::UnlockPlan>>>,
    secret: &Arc<Mutex<Option<PasswordBuf>>>,
) {
    let (home, tx) = (home.to_path_buf(), tx.clone());
    match effect {
        Effect::PrepareUnlock => {
            let plan = plan.clone();
            tokio::task::spawn_blocking(move || {
                let result =
                    wallet::prepare_unlock(&home, warm, allow_ptrace_scope_0).map(|prepared| {
                        *plan.lock().expect("unlock plan lock") = Some(prepared);
                    });
                let _ = tx.send(Msg::Prepared(result));
            });
        }
        Effect::CheckPassword { key, start_session } => {
            let plan = plan.lock().expect("unlock plan lock").take();
            let secret = secret.lock().expect("secret handoff lock").take();
            tokio::spawn(async move {
                let result = async {
                    let secret = secret.ok_or_else(|| WalletError::validation("password input missing"))?;
                    let session = if start_session {
                        let plan = plan.ok_or_else(|| WalletError::validation("unlock preparation missing"))?;
                        Some(wallet::unlock(&plan, secret.as_str()).await?)
                    } else { None };
                    tokio::task::spawn_blocking(move || {
                        let file = crate::core::store::read_wallet_file(&home)?;
                        let mut names = vec![key.as_str()];
                        if file.active != key { names.push(&file.active); }
                        let opened = wallet::open_with_password(&home, secret.as_str(), Some(&names))?;
                        drop(secret);
                        if !opened.keys.iter().any(|k| k.name == opened.active) {
                            return Err(WalletError::validation("active key changed during the password check; check again"));
                        }
                        if let Some(session) = session {
                            let name = session.ready["active"].as_str();
                            let address = session.ready["address"].as_str();
                            if !opened.keys.iter().any(|k| Some(k.name.as_str()) == name && k.address.as_deref() == address) {
                                return Err(WalletError::validation("session readiness does not match the address derived from the seed; do not send funds; run zwallet address --verify"));
                            }
                        }
                        let list = wallet::check_list(&home, &opened.anchor);
                        Ok((opened, list))
                    }).await.map_err(|_| WalletError::validation("password check stopped"))?
                }.await;
                let _ = tx.send(Msg::Checked(result));
            });
        }
        Effect::LoadAddress(generation, key) => {
            tokio::spawn(async move {
                let primary =
                    crate::chains::find(crate::chains::PRIMARY_CHAIN).expect("primary chain");
                let result = wallet::address(&home, &key, primary).await;
                let _ = tx.send(Msg::Address(generation, key, result));
            });
        }
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
        Effect::SetActive(name) => {
            tokio::spawn(async move {
                let _ = tx.send(Msg::Activated(wallet::set_active(&home, &name).await));
            });
        }
    }
}

/// The input thread: terminal events as [`Msg`]s on `tx`, until `stop`. If the terminal can no
/// longer be read, nothing could end the TUI, so it posts [`Key::Interrupt`].
#[derive(Clone, Copy)]
enum InputMode {
    Normal,
    Password,
    Discard,
}

fn read_input(
    tx: UnboundedSender<Msg>,
    stop: Arc<AtomicBool>,
    ack: sync::Receiver<InputMode>,
    submitted: Arc<Mutex<Option<PasswordBuf>>>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut mode = InputMode::Normal;
        let mut password = PasswordBuf::default();
        while !stop.load(Ordering::SeqCst) {
            let event = event::poll(INPUT_POLL).and_then(|ready| {
                if ready {
                    event::read().map(Some)
                } else {
                    Ok(None)
                }
            });
            let msg = match event {
                Ok(Some(Event::Key(k))) if k.kind == KeyEventKind::Press => match mode {
                    InputMode::Normal => key(k).map(Msg::Key),
                    InputMode::Discard => match k.code {
                        KeyCode::Enter | KeyCode::Esc => Some(Msg::PasswordCancelled),
                        KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                            Some(Msg::Key(Key::Interrupt))
                        }
                        _ => None,
                    },
                    InputMode::Password => match k.code {
                        KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                            password = PasswordBuf::default();
                            Some(Msg::Key(Key::Interrupt))
                        }
                        KeyCode::Esc => {
                            password = PasswordBuf::default();
                            Some(Msg::PasswordCancelled)
                        }
                        KeyCode::Enter if password.characters() == 0 => {
                            Some(Msg::PasswordLength(0, true))
                        }
                        KeyCode::Enter => {
                            *submitted.lock().expect("secret handoff lock") =
                                Some(std::mem::take(&mut password));
                            Some(Msg::PasswordSubmitted)
                        }
                        KeyCode::Tab => {
                            let accepted = password.push('\t');
                            if !accepted {
                                password = PasswordBuf::default();
                            }
                            Some(Msg::PasswordLength(password.characters(), !accepted))
                        }
                        KeyCode::Backspace => {
                            password.backspace();
                            Some(Msg::PasswordLength(password.characters(), false))
                        }
                        KeyCode::Char(c)
                            if !k
                                .modifiers
                                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                        {
                            let accepted = password.push(c);
                            if !accepted {
                                password = PasswordBuf::default();
                            }
                            Some(Msg::PasswordLength(password.characters(), !accepted))
                        }
                        _ => None,
                    },
                },
                Ok(Some(Event::Paste(mut text))) => {
                    let accepted = if matches!(mode, InputMode::Password) {
                        password.paste(&mut text)
                    } else {
                        use zeroize::Zeroize;
                        text.zeroize();
                        true
                    };
                    if !accepted {
                        password = PasswordBuf::default();
                    }
                    matches!(mode, InputMode::Password)
                        .then(|| Msg::PasswordLength(password.characters(), !accepted))
                }
                Ok(Some(Event::Resize(..))) => Some(Msg::Resize),
                Ok(_) => None,
                Err(_) => {
                    let _ = tx.send(Msg::Key(Key::Interrupt));
                    return;
                }
            };
            if let Some(msg) = msg {
                if tx.send(msg).is_err() {
                    return;
                }
                // This handshake is also the password-mode boundary, before the next read.
                match ack.recv() {
                    Ok(next) => mode = next,
                    Err(_) => return,
                }
                if matches!(mode, InputMode::Normal) {
                    password = PasswordBuf::default();
                }
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
