//! Independent state regression; no files, password, process, wallet or network.
use std::path::Path;
use std::time::Duration;
use zwallet::core::{
    error::WalletError,
    whitelist::{Entry, ListFiles, TransactionLimits, Whitelist},
};
use zwallet::tui::app::whitelist::Applied;
use zwallet::tui::app::whitelist::Read;
use zwallet::tui::app::{App, Effect, Key, Msg, UserText};
use zwallet::{chains, wallet};

fn press(app: &mut App, key: Key) -> Vec<Effect> {
    app.update(Msg::Key(key))
}
fn field(app: &mut App, value: &str) -> Vec<Effect> {
    app.update(Msg::Paste(UserText(zeroize::Zeroizing::new(value.into()))));
    press(app, Key::Enter)
}
#[test]
fn independent_lock_during_apply_is_not_lost_when_the_edit_starts_a_session() {
    let net = wallet::net(chains::find("base").unwrap());
    let mut app = App::new(&net, Duration::from_secs(600));
    app.update(Msg::Status(
        0,
        Ok(wallet::StatusView {
            configured: true,
            locked: true,
            daemon_pids: vec![],
            other_daemon_pids: vec![],
            serving: None,
            active: None,
            address: None,
        }),
    ));
    press(&mut app, Key::Char('5'));
    press(&mut app, Key::Char('a'));
    field(&mut app, "bob");
    field(&mut app, "0x3333333333333333333333333333333333333333");
    field(&mut app, "base");
    let work = field(&mut app, "10m");
    let id = work
        .iter()
        .find_map(|e| match e {
            Effect::PrepareWhitelist(r) => Some(r.id),
            _ => None,
        })
        .unwrap();
    app.update(Msg::WhitelistPrepared(
        id,
        Ok("add bob; then unlock for 10m".into()),
    ));
    press(&mut app, Key::End);
    press(&mut app, Key::Enter);
    assert_eq!(
        app.update(Msg::PasswordSubmitted),
        vec![Effect::ApplyWhitelist(id)]
    );
    assert!(
        press(&mut app, Key::Char('L')).is_empty(),
        "edit must complete before the queued lock"
    );
    press(&mut app, Key::Char('q'));
    assert!(!app.quitting());
    let work = app.update(Msg::WhitelistApplied(
        id,
        Applied(Ok(wallet::WhitelistOutcome {
            list: zwallet::core::whitelist::Whitelist::empty(&chains::WHITELIST_RULES),
            stopped_daemons: vec![],
            session: wallet::EditSession::Unlocked {
                duration: "10m".into(),
                ready: serde_json::json!({"pid":91}),
            },
            warning: None,
            checked: Err("fixture does not carry list authority".into()),
            output: serde_json::json!({"locked":false}),
            text: "added bob; session started for 10m".into(),
        })),
    ));
    assert!(
        work.iter().any(|e| matches!(e, Effect::LockAll)),
        "L during the edit was lost; the explicitly unlocked signing session remains open"
    );
    assert!(
        !app.quitting(),
        "quit must wait until the deferred lock completes"
    );
    app.update(Msg::Locked(Ok(vec![91])));
    assert!(app.quitting());
}

fn checked(home: &Path) -> wallet::CheckedList {
    let mut list = Whitelist::empty(&chains::WHITELIST_RULES);
    list.generation = 3;
    list.add(Entry::new_address(
        "bob",
        "0x3333333333333333333333333333333333333333",
        &["base"],
    ))
    .unwrap();
    wallet::CheckedList {
        list,
        source: "whitelist.json",
        fingerprint: ListFiles::read(home).fingerprint(),
    }
}

#[test]
fn late_list_read_cannot_restore_trust_after_a_failed_key_read() {
    reject_late_read(false);
}
#[test]
fn late_list_read_cannot_restore_trust_after_a_failed_status_read() {
    reject_late_read(true);
}
fn reject_late_read(status: bool) {
    let home = std::env::temp_dir().join(format!("zwt-h14-review-{}", std::process::id()));
    let list = checked(&home);
    let net = wallet::net(chains::find("base").unwrap());
    let mut app = App::new(&net, Duration::from_secs(600));
    app.update(Msg::Checked(Ok((
        wallet::Opened {
            keys: vec![wallet::KeyRow {
                name: "main".into(),
                index: 0,
                address: Some("0x1111111111111111111111111111111111111111".into()),
                active: true,
            }],
            active: "main".into(),
            warning: None,
            anchor: Ok(None),
            salted: true,
        },
        Ok(list.clone()),
    ))));
    let work = app.update(Msg::Key(Key::Char('5')));
    let id = work
        .iter()
        .find_map(|e| match e {
            Effect::ReadWhitelist(id, Some(_)) => Some(*id),
            _ => None,
        })
        .expect("read of the previously checked list");
    assert!(app.checked_list().is_none());
    let error = WalletError::validation("key metadata changed");
    app.update(if status {
        Msg::Status(app.generation(), Err(error))
    } else {
        Msg::Keys(app.generation(), Err(error))
    });
    assert!(app.checked_list().is_none());
    app.update(Msg::WhitelistRead(
        id,
        Ok(Read {
            checked: Some(list),
            text: "verified gen 3".into(),
            reason: None,
        }),
    ));
    assert!(
        app.checked_list().is_none(),
        "old read reinstated checked authority after fail_pane"
    );
}

#[test]
fn tron_policy_view_labels_native_limit_as_sun() {
    let home = std::env::temp_dir().join(format!("zwt-h14-review-{}", std::process::id()));
    let mut list = checked(&home);
    list.list.policy.transaction_limits.insert(
        "tron".into(),
        TransactionLimits {
            max_native_wei: Some("1000000".into()),
            ..Default::default()
        },
    );
    let view = wallet::whitelist::checked_view(&home, &list).unwrap();
    let line = view
        .text
        .lines()
        .find(|s| s.contains("tron:"))
        .expect("Tron native limit");
    assert!(
        line.contains("sun") && !line.contains("native=wei,"),
        "Tron 1000000 sun displayed as wei: {line}"
    );
}

#[test]
fn q_on_policy_review_discards_the_unsubmitted_plan_and_quits() {
    let net = wallet::net(chains::find("base").unwrap());
    let mut app = App::new(&net, Duration::from_secs(600));
    app.update(Msg::Status(
        0,
        Ok(wallet::StatusView {
            configured: true,
            locked: true,
            daemon_pids: vec![],
            other_daemon_pids: vec![],
            serving: None,
            active: None,
            address: None,
        }),
    ));
    app.update(Msg::Key(Key::Char('5')));
    app.update(Msg::Key(Key::Char('a')));
    for c in "bob".chars() {
        app.update(Msg::Key(Key::Char(c)));
    }
    app.update(Msg::Key(Key::Enter));
    for c in "0x3333333333333333333333333333333333333333".chars() {
        app.update(Msg::Key(Key::Char(c)));
    }
    app.update(Msg::Key(Key::Enter));
    for c in "base".chars() {
        app.update(Msg::Key(Key::Char(c)));
    }
    app.update(Msg::Key(Key::Enter));
    let work = app.update(Msg::Key(Key::Enter));
    let id = work
        .iter()
        .find_map(|e| match e {
            Effect::PrepareWhitelist(r) => Some(r.id),
            _ => None,
        })
        .expect("prepare whitelist");
    app.update(Msg::WhitelistPrepared(
        id,
        Ok("reviewed change; wallet stays locked".into()),
    ));
    let work = app.update(Msg::Key(Key::Char('q')));
    assert!(
        app.quitting(),
        "q was swallowed on the non-input summary review"
    );
    assert!(work.contains(&Effect::DiscardWhitelist(id)));
}
