//! Pure public-App regression. No workers, sockets, daemon, CLI, or real wallet.
use std::{path::PathBuf, time::Duration};
use zwallet::{
    chains::{self, Net},
    core::error::{ErrorCode, WalletError},
    tui::app::{App, Effect, Key, Msg, transfer::TransferMsg},
};

fn app() -> App {
    App::new(
        &Net {
            chain: chains::find("ethereum").unwrap(),
            rpc_url: "http://127.0.0.1:1".into(),
        },
        Duration::from_secs(60),
    )
}
fn original() -> WalletError {
    WalletError::new(
        ErrorCode::BroadcastUncertain,
        "original submitted broadcast uncertain",
    )
    .with_detail("txHash", format!("0x{}", "ab".repeat(32)))
    .with_detail("nonce", 7)
    .with_detail("journal", "/tmp/zwt-h7-review/sends.jsonl")
}
fn finish_submitted_send(app: &mut App, deferred_lock: bool) {
    // The public hook used by a confirmed invoke: owns send/status until its result.
    let send_id = app.begin_external_send();
    app.update(Msg::Transfer(
        send_id,
        TransferMsg::Progress {
            hash: format!("0x{}", "ab".repeat(32)),
            nonce: 7,
            journal: PathBuf::from("/tmp/zwt-h7-review/sends.jsonl"),
        },
    ));
    if deferred_lock {
        assert!(app.update(Msg::Key(Key::Char('L'))).is_empty());
    }
    assert!(app.update(Msg::Key(Key::Char('q'))).is_empty());
    assert!(!app.quitting());
    let work = app.update(Msg::Transfer(send_id, TransferMsg::Sent(Err(original()))));
    assert_eq!(
        work.iter().any(|w| matches!(w, Effect::LockAll)),
        deferred_lock
    );
    assert_eq!(app.quitting(), !deferred_lock);
}
fn late_read_after_send_preserves_original(status: bool) {
    let mut app = app();
    let reads = app.start();
    let generation = reads
        .into_iter()
        .find_map(|work| match (status, work) {
            (true, Effect::LoadStatus(g)) | (false, Effect::LoadKeys(g)) => Some(g),
            _ => None,
        })
        .expect("a genuine in-flight read effect");
    finish_submitted_send(&mut app, true);
    assert_eq!(
        app.generation(),
        generation,
        "send failure and queued L did not invalidate the read"
    );
    assert!(app.quit_pending());
    let error = WalletError::rpc("late unrelated read failure");
    let msg = if status {
        Msg::Status(generation, Err(error))
    } else {
        Msg::Keys(generation, Err(error))
    };
    app.update(msg);
    assert!(
        app.generation() > generation,
        "accepted failure reached fail_pane"
    );
    assert!(
        !app.quitting(),
        "event loop continues while LockAll is pending"
    );
    app.update(Msg::Locked(Ok(vec![])));
    assert!(app.quitting());
    assert_eq!(
        app.take_quit_error(),
        Some(original()),
        "late reads must not replace the submitted send's recovery error"
    );
}
#[test]
fn late_keys_failure_during_deferred_lock_preserves_submitted_send_error() {
    late_read_after_send_preserves_original(false);
}
#[test]
fn late_status_failure_during_deferred_lock_preserves_submitted_send_error() {
    late_read_after_send_preserves_original(true);
}
#[test]
fn obsolete_generation_is_ignored_and_no_lock_exits_before_a_late_read() {
    let mut app = app();
    let old = app
        .start()
        .into_iter()
        .find_map(|w| match w {
            Effect::LoadKeys(g) => Some(g),
            _ => None,
        })
        .unwrap();
    app.update(Msg::Key(Key::Char('r')));
    assert!(app.generation() > old);
    finish_submitted_send(&mut app, true);
    app.update(Msg::Keys(
        old,
        Err(WalletError::rpc("obsolete read failure")),
    ));
    app.update(Msg::Locked(Ok(vec![])));
    assert!(app.quitting());
    assert_eq!(app.take_quit_error(), Some(original()));
    let mut app = self::app();
    app.start();
    finish_submitted_send(&mut app, false);
    assert!(
        app.quitting(),
        "event loop stops before dequeuing more reads without a pending mutation"
    );
    assert_eq!(app.take_quit_error(), Some(original()));
}
