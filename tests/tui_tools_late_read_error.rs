//! Tools-only public-App regression. No signing worker, daemon, CLI or network is run.
use std::{os::unix::fs::PermissionsExt, time::Duration};
use zwallet::{
    chains::{self, Net, evm},
    core::{
        error::{ErrorCode, WalletError},
        whitelist::{WHITELIST_FILE, Whitelist, WhitelistKey},
    },
    tui::app::{
        App, Effect, Key, Msg,
        tools::{Stage, ToolMsg},
    },
    wallet::{self, CheckedList, KeyRow, Keys, Opened, StatusView, Trust},
};

fn fixture() -> (tempfile::TempDir, CheckedList, KeyRow) {
    let dir = tempfile::Builder::new()
        .prefix("zwt-h7-review-")
        .tempdir_in("/tmp")
        .unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut list = Whitelist::empty(&evm::registry::WHITELIST_RULES);
    list.generation = 1;
    list.policy.allow_message_signing = true;
    let seal = WhitelistKey::from_secret(&[7; 32]);
    let signed = seal.sign(&list);
    std::fs::write(
        dir.path().join(WHITELIST_FILE),
        signed.to_file_bytes().unwrap(),
    )
    .unwrap();
    let checked = wallet::check_list(dir.path(), &Ok(Some(seal.anchor_for(&signed)))).unwrap();
    let key = KeyRow {
        name: "main".into(),
        index: 0,
        address: Some("0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf".into()),
        active: true,
    };
    (dir, checked, key)
}
fn type_text(app: &mut App, s: &str) {
    for c in s.chars() {
        assert!(app.update(Msg::Key(Key::Char(c))).is_empty());
    }
}
fn tools_id(work: Vec<Effect>) -> u64 {
    work.into_iter()
        .find_map(|e| match e {
            Effect::Tools(id) => Some(id),
            _ => None,
        })
        .expect("submitted Tools job")
}
fn preserve_signature_error(status: bool) {
    let (dir, checked, key) = fixture();
    let chain = chains::find("ethereum").unwrap();
    let mut app = App::new(
        &Net {
            chain,
            rpc_url: "http://127.0.0.1:1".into(),
        },
        Duration::from_secs(60),
    );
    app.configure(dir.path(), false);
    let work = app.update(Msg::Checked(Ok((
        Opened {
            keys: vec![key.clone()],
            active: "main".into(),
            warning: None,
            anchor: Ok(None),
            salted: false,
        },
        Ok(checked.clone()),
    ))));
    for effect in work {
        match effect {
            Effect::LoadKeys(g) => {
                app.update(Msg::Keys(
                    g,
                    Ok(Keys {
                        active: "main".into(),
                        rows: vec![key.clone()],
                        trust: Trust::Session,
                    }),
                ));
            }
            Effect::LoadStatus(g) => {
                app.update(Msg::Status(
                    g,
                    Ok(StatusView {
                        configured: true,
                        locked: false,
                        daemon_pids: vec![1],
                        other_daemon_pids: vec![],
                        serving: None,
                        active: Some("main".into()),
                        address: Some((key.address.clone(), Trust::Session)),
                    }),
                ));
            }
            _ => {}
        }
    }
    let reads = app.update(Msg::Key(Key::Char('6')));
    let generation = reads
        .into_iter()
        .find_map(|e| match (status, e) {
            (true, Effect::LoadStatus(g)) | (false, Effect::LoadKeys(g)) => Some(g),
            _ => None,
        })
        .expect("real in-flight read from opening Tools");
    app.update(Msg::Key(Key::Char('m')));
    type_text(&mut app, "held exact message");
    let review_id = tools_id(app.update(Msg::Key(Key::Enter)));
    assert!(app.take_tool_job(review_id).is_some());
    // Complete the pure MessageReview job through its public message; this calls its same API.
    let review =
        wallet::review_message(dir.path(), chain, &checked, &key, "held exact message").unwrap();
    app.update(Msg::Tools(review_id, ToolMsg::MessageReview(Ok(review))));
    assert_eq!(app.tools().stage, Stage::MessageReview);
    app.update(Msg::Key(Key::Enter));
    type_text(&mut app, "sign");
    let sign_id = tools_id(app.update(Msg::Key(Key::Enter)));
    assert!(app.take_tool_job(sign_id).is_some());
    assert_eq!(app.tools().stage, Stage::Signing);
    app.update(Msg::Key(Key::Char('L')));
    app.update(Msg::Key(Key::Char('q')));
    let original = WalletError::new(
        ErrorCode::WhitelistDenied,
        "original submitted signature refusal",
    );
    assert_eq!(
        app.update(Msg::Tools(
            sign_id,
            ToolMsg::Signature(Err(original.clone()))
        )),
        vec![Effect::LockAll]
    );
    assert!(app.quit_pending());
    assert_eq!(app.generation(), generation);
    let error = WalletError::rpc("late unrelated read failure");
    app.update(if status {
        Msg::Status(generation, Err(error))
    } else {
        Msg::Keys(generation, Err(error))
    });
    assert!(!app.quitting());
    app.update(Msg::Locked(Ok(vec![])));
    assert!(app.quitting());
    assert_eq!(app.take_quit_error(), Some(original));
}
#[test]
fn late_keys_failure_during_deferred_lock_preserves_submitted_signature_error() {
    preserve_signature_error(false);
}
#[test]
fn late_status_failure_during_deferred_lock_preserves_submitted_signature_error() {
    preserve_signature_error(true);
}
