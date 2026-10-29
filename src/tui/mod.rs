//! `zwallet tui`: the terminal UI (`.prd/03-tui.md`). [`run`] holds the terminal and runs the
//! loop: one std thread reads the terminal's events and posts them as [`Msg`]s, the state
//! ([`app`]) answers each with the work it asks for, which runs as tokio tasks on the CLI's
//! runtime and posts its result back; every frame is drawn ([`ui`]) through the one write point
//! ([`term::FilteredBackend`]).

pub mod actions;
pub mod app;
mod create_runtime;
pub mod form;
pub mod password;
pub mod style;
pub mod term;
pub mod text;
pub mod ui;

use crate::chains::Net;
use crate::core::error::{Result, WalletError};
use crate::core::protocol;
use crate::wallet;
use app::{App, Effect, Key, Msg, PasswordState, UserText};
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
    app.configure(&home, allow_ptrace_scope_0);
    let mut terminal = Terminal::new(FilteredBackend::new(CrosstermBackend::new(
        std::io::stdout(),
    )))
    .map_err(terminal_error)?;
    let (tx, mut rx) = unbounded_channel();
    let stop = Arc::new(AtomicBool::new(false));
    let secret = Arc::new(Mutex::new(None));
    let plan = Arc::new(Mutex::new(None));
    let whitelist_plan = Arc::new(Mutex::new(None));
    let create_plan = Arc::new(Mutex::new(None));
    let (ack_tx, ack_rx) = sync::sync_channel(1);
    let input = read_input(tx.clone(), stop.clone(), ack_rx, secret.clone());
    let mut input_waiting = false;
    let mut tick = tokio::time::interval(REFRESH);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The first tick is now, and the state reads the status at the start already.
    tick.tick().await;
    let mut work = app.start();
    let mut terminal_failure = None;
    let result = loop {
        for effect in work.drain(..) {
            if let Effect::Replacement(id) = effect {
                if let Some(job) = app.take_replacement_job(id) {
                    let (home, tx) = (home.clone(), tx.clone());
                    tokio::spawn(async move {
                        job.run(id, &home, &tx).await;
                    });
                }
                continue;
            }
            if let Effect::Transfer(id) = effect {
                if let Some(job) = app.take_transfer_job(id) {
                    let (home, tx) = (home.clone(), tx.clone());
                    tokio::spawn(async move {
                        job.run(id, &home, &tx).await;
                    });
                }
                continue;
            }
            if let Effect::Create(effect) = effect {
                create_runtime::start(
                    effect,
                    &home,
                    &tx,
                    allow_ptrace_scope_0,
                    &create_plan,
                    &secret,
                );
                continue;
            }
            start(
                effect,
                &home,
                &tx,
                warm,
                allow_ptrace_scope_0,
                &plan,
                &whitelist_plan,
                &secret,
                app.net().clone(),
                app.nets(),
            );
        }
        if let Err(error) = terminal_frame(&app, &mut terminal_failure, || {
            terminal
                .draw(|f| {
                    ui::draw(f, &app);
                })
                .map(|_| ())
        }) {
            break Err(error);
        }
        if terminal_failure.is_some() {
            // A failed screen cannot authorize more work. The submitted worker already owns
            // its password; drop any unsubmitted owner and drain only completion messages.
            stop.store(true, Ordering::SeqCst);
            let _ = ack_tx.try_send(InputMode::Discard);
            secret.lock().expect("secret handoff lock").take();
            input_waiting = false;
        }
        if app.quitting() {
            break Ok(());
        }
        // The input thread never reads a following event before the App acknowledges the
        // preceding one. In particular Enter's preparation finishes before typeahead is read.
        if input_waiting && app.password_state() != PasswordState::Preparing {
            let mode = if matches!(app.password_state(), PasswordState::Input { .. }) {
                match app.creation().stage {
                    app::create::CreateStage::PasswordFirst => InputMode::NewPassword,
                    app::create::CreateStage::PasswordAgain => InputMode::ConfirmPassword,
                    _ => InputMode::Password,
                }
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
        input_waiting |= terminal_failure.is_none() && input_message(&msg);
        work = terminal_update(&mut app, &terminal_failure, msg);
    };
    stop.store(true, Ordering::SeqCst);
    drop(ack_tx);
    let _ = input.join();
    match app.take_quit_error() {
        Some(e) => result.and(Err(e)),
        None => result,
    }
}

fn terminal_frame(
    app: &App,
    failure: &mut Option<WalletError>,
    draw: impl FnOnce() -> std::io::Result<()>,
) -> Result<()> {
    if failure.is_none()
        && let Err(error) = draw()
    {
        *failure = Some(WalletError::validation(format!("terminal: {error}")));
    }
    // Already-submitted writes and optional session starts are not cancellable. Their
    // outcome may schedule a deferred lock, so wait for that mutation too, without redrawing.
    if app.mutation_pending() {
        return Ok(());
    }
    match failure.take() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn input_message(msg: &Msg) -> bool {
    matches!(
        msg,
        Msg::Key(_)
            | Msg::Paste(_)
            | Msg::Resize
            | Msg::PasswordLength(..)
            | Msg::PasswordSubmitted
            | Msg::PasswordCancelled
            | Msg::Create(
                app::create::CreateMsg::PasswordAgain | app::create::CreateMsg::PasswordMismatch
            )
    )
}

/// Stop queued user input from authorizing an action after its review can no longer be drawn.
/// Worker results still advance the state and may schedule a requested, deferred daemon stop.
fn terminal_update(app: &mut App, failure: &Option<WalletError>, msg: Msg) -> Vec<Effect> {
    if failure.is_some() && input_message(&msg) {
        Vec::new()
    } else {
        app.update(msg)
    }
}

/// Starts `effect` as a task that posts its result on `tx`.
#[allow(clippy::too_many_arguments)]
fn start(
    effect: Effect,
    home: &Path,
    tx: &UnboundedSender<Msg>,
    warm: Duration,
    allow_ptrace_scope_0: bool,
    plan: &Arc<Mutex<Option<wallet::UnlockPlan>>>,
    whitelist_plan: &Arc<Mutex<Option<(u64, wallet::WhitelistPlan)>>>,
    secret: &Arc<Mutex<Option<PasswordBuf>>>,
    net: Net,
    nets: Vec<Net>,
) {
    let (home, tx) = (home.to_path_buf(), tx.clone());
    let effect = match effect {
        Effect::PrepareUnlock => Effect::PrepareDuration(warm),
        other => other,
    };
    match effect {
        Effect::Transfer(_) | Effect::Replacement(_) => {
            unreachable!("send jobs are moved out of App by the loop")
        }
        Effect::HistoryOwner(target) => {
            tokio::spawn(async move {
                let result = match target.net() {
                    Ok(net) => wallet::address(&home, &target.key, net.chain).await,
                    Err(error) => Err(error),
                };
                let _ = tx.send(Msg::HistoryOwner(target, result));
            });
        }
        Effect::LoadHistory(scope) => {
            tokio::spawn(async move {
                let result = app::history::read_history(&home, &scope).await;
                let _ = tx.send(Msg::History(scope, result));
            });
        }
        Effect::LoadTransaction(request) => {
            tokio::spawn(async move {
                let result = app::history::read_transaction(&home, &request).await;
                let _ = tx.send(Msg::Transaction(request, result));
            });
        }
        Effect::CheckHistoryList(generation, fingerprint) => {
            tokio::task::spawn_blocking(move || {
                let result = crate::core::whitelist::check_unchanged(&home, &fingerprint);
                let _ = tx.send(Msg::HistoryList(generation, fingerprint, result));
            });
        }
        Effect::ReadWhitelist(id, checked) => {
            tokio::spawn(async move {
                let result =
                    tokio::task::spawn_blocking(move || app::whitelist::read(&home, checked))
                        .await
                        .unwrap_or_else(|_| Err(WalletError::validation("whitelist read stopped")));
                let _ = tx.send(Msg::WhitelistRead(id, result));
            });
        }
        Effect::PrepareWhitelist(request) => {
            let slot = whitelist_plan.clone();
            tokio::spawn(async move {
                let id = request.id;
                let result = tokio::task::spawn_blocking(move || {
                    if let Some(fingerprint) = request.checked_fingerprint {
                        crate::core::whitelist::check_unchanged(&home, &fingerprint)?;
                    }
                    let change = request.draft.change()?;
                    let unlock = (!request.unlock.is_empty()).then_some(wallet::WhitelistUnlock {
                        duration: request.unlock,
                        allow_ptrace_scope_0,
                    });
                    let plan = wallet::whitelist_plan(&home, change, unlock)?;
                    let summary = plan.summary().to_string();
                    *slot.lock().expect("whitelist plan lock") = Some((id, plan));
                    Ok(summary)
                })
                .await
                .unwrap_or_else(|_| Err(WalletError::validation("whitelist preparation stopped")));
                let _ = tx.send(Msg::WhitelistPrepared(id, result));
            });
        }
        Effect::DiscardWhitelist(id) => {
            let mut slot = whitelist_plan.lock().expect("whitelist plan lock");
            if slot.as_ref().is_some_and(|(pending, _)| *pending == id) {
                slot.take();
            }
        }
        Effect::ApplyWhitelist(id) => {
            let plan = whitelist_plan.lock().expect("whitelist plan lock").take();
            let password = secret.lock().expect("secret handoff lock").take();
            let runtime = tokio::runtime::Handle::current();
            tokio::spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    let (prepared, plan) = plan
                        .ok_or_else(|| WalletError::validation("whitelist preparation missing"))?;
                    if prepared != id {
                        return Err(WalletError::validation("whitelist preparation changed"));
                    }
                    let password = password
                        .ok_or_else(|| WalletError::validation("password input missing"))?;
                    runtime.block_on(wallet::whitelist_apply(plan, password))
                })
                .await
                .unwrap_or_else(|_| {
                    Err(WalletError::validation(
                        "whitelist worker stopped; check the list before retrying",
                    ))
                });
                let _ = tx.send(Msg::WhitelistApplied(id, app::whitelist::Applied(result)));
            });
        }
        Effect::Create(_) => unreachable!("creation worker handled first"),
        Effect::PrepareDuration(ttl) => {
            let plan = plan.clone();
            tokio::task::spawn_blocking(move || {
                let result =
                    wallet::prepare_unlock(&home, ttl, allow_ptrace_scope_0).map(|prepared| {
                        *plan.lock().expect("unlock plan lock") = Some(prepared);
                    });
                let _ = tx.send(Msg::Prepared(result));
            });
        }
        Effect::PrepareRead => {
            tokio::task::spawn_blocking(move || {
                let result = crate::core::store::read_wallet_file(&home).map(|_| ());
                let _ = tx.send(Msg::Prepared(result));
            });
        }
        Effect::CheckPassword { key, start_session } => {
            start_check(home, tx, key, start_session, false, false, plan, secret);
        }
        Effect::CheckReadPassword { key, all, repair } => {
            start_check(home, tx, key, false, all, repair, plan, secret);
        }
        Effect::LoadBalances(generation, key) => {
            tokio::spawn(async move {
                let result = wallet::balances(&home, &key, net).await;
                let _ = tx.send(Msg::Balances(generation, key, result));
            });
        }
        Effect::LoadEveryBalances(generation, key) => {
            tokio::spawn(async move {
                let result = wallet::every_balance_with(&home, &key, nets).await;
                let _ = tx.send(Msg::EveryBalances(generation, key, result));
            });
        }
        Effect::LoadAddresses(generation, key) => {
            tokio::spawn(async move {
                let result = wallet::addresses(&home, &key).await;
                let _ = tx.send(Msg::Addresses(generation, key, result));
            });
        }
        Effect::CheckRpc(request, choice) => {
            tokio::spawn(async move {
                let result = async {
                    let chain = crate::chains::find(choice.chain)?;
                    wallet::validate_rpc(&wallet::net_with(chain, &choice.url)).await?;
                    Ok(choice)
                }
                .await;
                let _ = tx.send(Msg::RpcChecked(request, result));
            });
        }
        Effect::PrepareUnlock => unreachable!("warm duration was resolved"),
        Effect::LoadAddress(generation, key) => {
            tokio::spawn(async move {
                let result = wallet::address(&home, &key, net.chain).await;
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

#[allow(clippy::too_many_arguments)]
fn start_check(
    home: PathBuf,
    tx: UnboundedSender<Msg>,
    key: String,
    start_session: bool,
    all: bool,
    repair: bool,
    plan: &Arc<Mutex<Option<wallet::UnlockPlan>>>,
    secret: &Arc<Mutex<Option<PasswordBuf>>>,
) {
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
                let key = if key.is_empty() { &file.active } else { &key };
                let mut names = vec![key.as_str()];
                if file.active != *key { names.push(&file.active); }
                let checked = if repair {
                    wallet::repair(&home, secret.as_str())?.checked
                } else {
                    wallet::check_password(&home, secret.as_str(), if all { None } else { Some(&names) })?
                };
                drop(secret);
                if !checked.opened.keys.iter().any(|k| k.name == checked.opened.active) {
                    return Err(WalletError::validation("active key changed during the password check; check again"));
                }
                if let Some(session) = session {
                    let name = session.ready["active"].as_str();
                    let address = session.ready["address"].as_str();
                    if !checked.opened.keys.iter().any(|k| Some(k.name.as_str()) == name && k.address.as_deref() == address) {
                        return Err(WalletError::validation("session readiness does not match the address derived from the seed; do not send funds; run zwallet address --verify"));
                    }
                }
                let list = wallet::check_list(&home, &checked.opened.anchor);
                Ok((checked, list))
            }).await.map_err(|_| WalletError::validation("password check stopped"))?
        }.await;
        let _ = tx.send(Msg::Verified(result));
    });
}

/// The input thread: terminal events as [`Msg`]s on `tx`, until `stop`. If the terminal can no
/// longer be read, nothing could end the TUI, so it posts [`Key::Interrupt`].
#[derive(Clone, Copy)]
enum InputMode {
    Normal,
    Password,
    NewPassword,
    ConfirmPassword,
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
        let mut first_password = password::PasswordPair::default();
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
                    InputMode::Password | InputMode::NewPassword | InputMode::ConfirmPassword => {
                        match k.code {
                            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                                password = PasswordBuf::default();
                                Some(Msg::Key(Key::Interrupt))
                            }
                            KeyCode::Esc => {
                                password = PasswordBuf::default();
                                Some(Msg::PasswordCancelled)
                            }
                            KeyCode::Enter if password.characters() == 0 => {
                                Some(password_edit_result(
                                    mode,
                                    false,
                                    &mut password,
                                    &mut first_password,
                                ))
                            }
                            KeyCode::Enter => match mode {
                                InputMode::NewPassword => {
                                    if password.characters() < 12 {
                                        password = PasswordBuf::default();
                                        Some(Msg::PasswordLength(0, true))
                                    } else {
                                        first_password.remember(std::mem::take(&mut password));
                                        Some(Msg::Create(app::create::CreateMsg::PasswordAgain))
                                    }
                                }
                                InputMode::ConfirmPassword => {
                                    match first_password.confirm(std::mem::take(&mut password)) {
                                        Some(first) => {
                                            *submitted.lock().expect("secret handoff lock") =
                                                Some(first);
                                            Some(Msg::PasswordSubmitted)
                                        }
                                        None => Some(Msg::Create(
                                            app::create::CreateMsg::PasswordMismatch,
                                        )),
                                    }
                                }
                                _ => {
                                    *submitted.lock().expect("secret handoff lock") =
                                        Some(std::mem::take(&mut password));
                                    Some(Msg::PasswordSubmitted)
                                }
                            },
                            KeyCode::Tab => {
                                let accepted = password.push('\t');
                                Some(password_edit_result(
                                    mode,
                                    accepted,
                                    &mut password,
                                    &mut first_password,
                                ))
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
                                Some(password_edit_result(
                                    mode,
                                    accepted,
                                    &mut password,
                                    &mut first_password,
                                ))
                            }
                            _ => None,
                        }
                    }
                },
                Ok(Some(Event::Paste(mut text))) => match mode {
                    InputMode::Normal => Some(Msg::Paste(UserText(zeroize::Zeroizing::new(text)))),
                    InputMode::Discard => {
                        use zeroize::Zeroize;
                        text.zeroize();
                        None
                    }
                    InputMode::Password | InputMode::NewPassword | InputMode::ConfirmPassword => {
                        let accepted = password.paste(&mut text);
                        Some(password_edit_result(
                            mode,
                            accepted,
                            &mut password,
                            &mut first_password,
                        ))
                    }
                },
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
                if !matches!(mode, InputMode::ConfirmPassword) {
                    first_password.clear();
                }
                if matches!(mode, InputMode::Normal | InputMode::Discard) {
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
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Delete => Key::Delete,
        KeyCode::Tab => Key::Tab,
        KeyCode::BackTab => Key::BackTab,
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc => Key::Esc,
        _ => return None,
    })
}

#[cfg(test)]
mod terminal_send_failure_tests {
    use super::*;
    use crate::wallet::Trust;
    use app::transfer::TransferMsg;

    const ADDRESS: &str = "0x1111111111111111111111111111111111111111";
    fn app() -> App {
        let net = Net {
            chain: crate::chains::find("ethereum").unwrap(),
            rpc_url: "http://127.0.0.1:1".into(),
        };
        let mut app = App::new(&net, Duration::from_secs(600));
        app.update(Msg::Keys(
            app.generation(),
            Ok(wallet::Keys {
                active: "main".into(),
                trust: Trust::Locked,
                rows: vec![wallet::KeyRow {
                    name: "main".into(),
                    index: 0,
                    address: Some(ADDRESS.into()),
                    active: true,
                }],
            }),
        ));
        app.update(Msg::Status(
            app.generation(),
            Ok(wallet::StatusView {
                configured: true,
                locked: true,
                daemon_pids: vec![],
                other_daemon_pids: vec![],
                serving: None,
                active: Some("main".into()),
                address: Some((Some(ADDRESS.into()), Trust::Locked)),
            }),
        ));
        app
    }
    fn failed_frame(app: &App, failure: &mut Option<WalletError>) -> Result<()> {
        terminal_frame(app, failure, || {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "original fixture terminal failure",
            ))
        })
    }
    fn drain_frame(app: &App, failure: &mut Option<WalletError>) -> Result<()> {
        terminal_frame(app, failure, || {
            panic!("must never redraw after terminal failure")
        })
    }
    fn assert_original(error: WalletError) {
        assert_eq!(error.message, "terminal: original fixture terminal failure");
    }

    #[test]
    fn broken_terminal_waits_for_send_progress_outcome_and_deferred_lock() {
        for deferred_lock in [false, true] {
            let mut app = app();
            let id = app.begin_external_send();
            if deferred_lock {
                assert!(app.update(Msg::Key(Key::Char('L'))).is_empty());
            }
            app.update(Msg::Key(Key::Char('q')));
            let mut failure = None;
            assert!(
                failed_frame(&app, &mut failure).is_ok(),
                "send must finish before terminal error escapes"
            );
            app.update(Msg::Transfer(
                id,
                TransferMsg::Progress {
                    hash: format!("0x{}", "1".repeat(64)),
                    nonce: 7,
                    journal: "/tmp/unused-journal".into(),
                },
            ));
            assert!(
                app.transfer().status.is_some(),
                "durable progress must still land"
            );
            assert!(drain_frame(&app, &mut failure).is_ok());
            let work = app.update(Msg::Transfer(
                id,
                TransferMsg::Sent(Err(WalletError::validation("send worker failure"))),
            ));
            assert!(!app.sending());
            if deferred_lock {
                assert!(work.contains(&Effect::LockAll));
                assert!(drain_frame(&app, &mut failure).is_ok());
                app.update(Msg::Locked(Err(WalletError::validation(
                    "deferred lock failure",
                ))));
                assert!(app.take_quit_error().is_some());
            }
            assert_original(drain_frame(&app, &mut failure).unwrap_err());
        }
    }

    #[test]
    fn broken_terminal_waits_for_unlock_outcome_and_deferred_lock() {
        let mut app = app();
        assert_eq!(
            app.update(Msg::Key(Key::Enter)),
            vec![Effect::PrepareUnlock]
        );
        app.update(Msg::Prepared(Ok(())));
        assert!(matches!(
            app.update(Msg::PasswordSubmitted).as_slice(),
            [Effect::CheckPassword {
                start_session: true,
                ..
            }]
        ));
        app.update(Msg::Key(Key::Char('L')));
        let mut failure = None;
        assert!(failed_frame(&app, &mut failure).is_ok());
        assert!(drain_frame(&app, &mut failure).is_ok());
        let work = app.update(Msg::Checked(Err(WalletError::validation(
            "unlock worker finished",
        ))));
        assert!(work.contains(&Effect::LockAll));
        assert!(drain_frame(&app, &mut failure).is_ok());
        app.update(Msg::Locked(Ok(vec![])));
        assert_original(drain_frame(&app, &mut failure).unwrap_err());
    }

    #[test]
    fn broken_terminal_waits_for_active_write_and_existing_lock() {
        for activate in [false, true] {
            let mut app = app();
            if activate {
                app.update(Msg::Key(Key::Char('s')));
                assert_eq!(
                    app.update(Msg::Key(Key::Char('y'))),
                    vec![Effect::SetActive("main".into())]
                );
            } else {
                assert_eq!(app.update(Msg::Key(Key::Char('L'))), vec![Effect::LockAll]);
            }
            let mut failure = None;
            assert!(failed_frame(&app, &mut failure).is_ok());
            assert!(drain_frame(&app, &mut failure).is_ok());
            if activate {
                app.update(Msg::Activated(Ok((ADDRESS.into(), Trust::Locked))));
            } else {
                app.update(Msg::Locked(Ok(vec![])));
            }
            assert_original(drain_frame(&app, &mut failure).unwrap_err());
        }
    }

    #[test]
    fn queued_input_cannot_authorize_work_but_worker_results_still_drain() {
        let failure = Some(WalletError::validation(
            "terminal: original fixture terminal failure",
        ));
        let mut active = app();
        active.update(Msg::Key(Key::Char('s')));
        assert!(terminal_update(&mut active, &failure, Msg::Key(Key::Char('y'))).is_empty());
        assert!(!active.activating());
        assert_eq!(
            terminal_update(&mut active, &None, Msg::Key(Key::Char('y'))),
            vec![Effect::SetActive("main".into())]
        );
        terminal_update(
            &mut active,
            &failure,
            Msg::Activated(Ok((ADDRESS.into(), Trust::Locked))),
        );
        assert!(!active.mutation_pending());

        let mut warm = app();
        warm.update(Msg::Key(Key::Enter));
        warm.update(Msg::Prepared(Ok(())));
        assert!(terminal_update(&mut warm, &failure, Msg::PasswordSubmitted).is_empty());
        assert!(matches!(warm.password_state(), PasswordState::Input { .. }));
        assert!(matches!(
            terminal_update(&mut warm, &None, Msg::PasswordSubmitted).as_slice(),
            [Effect::CheckPassword {
                start_session: true,
                ..
            }]
        ));
        terminal_update(
            &mut warm,
            &failure,
            Msg::Checked(Err(WalletError::validation("worker done"))),
        );
        assert!(!warm.mutation_pending());

        for msg in [
            Msg::Key(Key::Char('L')),
            Msg::Resize,
            Msg::PasswordCancelled,
            Msg::PasswordLength(7, false),
            Msg::Paste(UserText(zeroize::Zeroizing::new("queued".into()))),
        ] {
            assert!(terminal_update(&mut warm, &failure, msg).is_empty());
        }
        assert!(!warm.mutation_pending());
    }

    #[test]
    fn broken_terminal_does_not_wait_for_unsubmitted_password_or_read_preparation() {
        for prepared in [false, true] {
            let mut app = app();
            app.update(Msg::Key(Key::Enter));
            if prepared {
                app.update(Msg::Prepared(Ok(())));
            }
            assert_original(failed_frame(&app, &mut None).unwrap_err());
        }
    }
}

fn password_edit_result(
    mode: InputMode,
    accepted: bool,
    password: &mut PasswordBuf,
    first: &mut password::PasswordPair,
) -> Msg {
    if !accepted {
        password.wipe();
        if matches!(mode, InputMode::ConfirmPassword) {
            first.clear();
            return Msg::Create(app::create::CreateMsg::PasswordMismatch);
        }
    }
    Msg::PasswordLength(password.characters(), !accepted)
}

#[cfg(test)]
mod creation_input_tests {
    use super::*;
    #[test]
    fn rejected_confirmation_clears_both_owners_before_the_next_event() {
        let mut first = password::PasswordPair::default();
        let mut original = PasswordBuf::default();
        assert!(original.append("fixture-password"));
        first.remember(original);
        let mut second = PasswordBuf::default();
        assert!(second.append("wrong-input"));
        assert!(matches!(
            password_edit_result(InputMode::ConfirmPassword, false, &mut second, &mut first),
            Msg::Create(app::create::CreateMsg::PasswordMismatch)
        ));
        assert_eq!(second.characters(), 0);
        let mut retry = PasswordBuf::default();
        assert!(retry.append("fixture-password"));
        assert!(first.confirm(retry).is_none());
    }
}

#[cfg(test)]
mod terminal_failure_tests {
    use super::*;
    use app::create::{CreateEffect, CreateMsg, CreateStage};

    fn submitted_create() -> (App, u64) {
        let net = Net {
            chain: crate::chains::find("ethereum").unwrap(),
            rpc_url: "http://127.0.0.1:1".into(),
        };
        let mut app = App::new(&net, Duration::from_secs(600));
        app.update(Msg::Key(Key::Char('n')));
        for text in ["main", "", "10m", "main"] {
            app.update(Msg::Paste(UserText(zeroize::Zeroizing::new(text.into()))));
            let effects = app.update(Msg::Key(Key::Enter));
            if let Some(Effect::Create(CreateEffect::Prepare { id, .. })) = effects.first() {
                let id = *id;
                app.update(Msg::Create(CreateMsg::Prepared(id, Ok(true))));
                app.update(Msg::Create(CreateMsg::PasswordAgain));
                assert_eq!(
                    app.update(Msg::PasswordSubmitted),
                    vec![Effect::Create(CreateEffect::Submit { id })]
                );
                return (app, id);
            }
        }
        panic!("fixture must reach submission without secret bytes");
    }

    fn created() -> wallet::Created {
        wallet::Created {
            name: "main".into(),
            index: 0,
            // One valid address per family: the TUI validates each family's format.
            addresses: crate::chains::CHAINS
                .iter()
                .map(|chain| {
                    let address = if chain.family.name() == "tron" {
                        "TPjjvMwjPoDC32V2dGDYTkLH4E5LAtBZ6C"
                    } else {
                        "0x1111111111111111111111111111111111111111"
                    };
                    (chain.key.into(), address.into())
                })
                .collect(),
            active: "main".into(),
            master_created: true,
            verified: true,
            session: None,
            backup_reminder: Some("Back up before funding."),
        }
    }

    #[test]
    fn broken_terminal_waits_for_create_outcome_and_any_requested_lock() {
        for lock in [false, true] {
            let (mut app, id) = submitted_create();
            let mut failure = None;
            assert!(
                terminal_frame(&app, &mut failure, || Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "fixture terminal closed"
                )))
                .is_ok(),
                "submitted creation must finish before returning terminal failure"
            );
            assert!(failure.is_some());
            if lock {
                app.update(Msg::Key(Key::Char('L')));
            }
            assert!(
                terminal_frame(&app, &mut failure, || panic!(
                    "must not redraw broken terminal"
                ))
                .is_ok()
            );
            let work = app.update(Msg::Create(CreateMsg::Finished(id, Ok(created()))));
            assert!(matches!(app.creation().stage, CreateStage::Done(_)));
            assert!(app.family_pinned("main", crate::chains::find("ethereum").unwrap()));
            if lock {
                assert!(work.contains(&Effect::LockAll));
                assert!(terminal_frame(&app, &mut failure, || panic!("no redraw")).is_ok());
                app.update(Msg::Locked(Ok(vec![])));
            }
            let error = terminal_frame(&app, &mut failure, || panic!("no redraw")).unwrap_err();
            assert!(error.message.contains("fixture terminal closed"));
        }
    }

    #[test]
    fn broken_terminal_releases_after_create_failure_and_refresh() {
        let (mut app, id) = submitted_create();
        let mut failure = None;
        assert!(
            terminal_frame(&app, &mut failure, || Err(std::io::Error::from(
                std::io::ErrorKind::BrokenPipe
            )))
            .is_ok()
        );
        let work = app.update(Msg::Create(CreateMsg::Finished(
            id,
            Err(WalletError::validation(
                "optional unlock failed after creation",
            )),
        )));
        assert!(
            work.iter()
                .any(|effect| matches!(effect, Effect::LoadKeys(_)))
        );
        assert!(app.creation().mutation_failed);
        assert!(terminal_frame(&app, &mut failure, || panic!("no redraw")).is_err());
    }
}
