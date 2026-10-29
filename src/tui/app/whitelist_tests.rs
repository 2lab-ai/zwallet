use super::*;
use crate::core::whitelist::{Entry, ListFiles, Whitelist};
use crate::tui::{actions, term::FilteredBackend, ui};
use crate::wallet::EditSession;
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;
use zeroize::Zeroizing;

fn new_app() -> App {
    let net = crate::wallet::net(crate::chains::find("base").unwrap());
    let mut app = App::new(&net, Duration::from_secs(600));
    app.status = Pane::Ready(Bar {
        configured: true,
        key: None,
        address: None,
        trust: None,
        lock: LockLine::Locked,
    });
    app.nav.tab = Tab::Whitelist;
    app.whitelist.lines = Pane::Ready(lines("not verified"));
    app
}
fn checked() -> CheckedList {
    let mut list = Whitelist::empty(&crate::chains::WHITELIST_RULES);
    list.generation = 3;
    list.add(Entry::new_address(
        "bob",
        "0x3333333333333333333333333333333333333333",
        &["base"],
    ))
    .unwrap();
    list.add(Entry::new_address(
        "zero",
        "0x0000000000000000000000000000000000000000",
        &["base"],
    ))
    .unwrap();
    CheckedList {
        list,
        source: "whitelist.json",
        fingerprint: ListFiles::read(Path::new("/tmp/zwt-tui6-absent-fixture")).fingerprint(),
    }
}
fn paste(app: &mut App, s: &str) -> Vec<Effect> {
    app.update(Msg::Paste(UserText(Zeroizing::new(s.to_string()))))
}
fn key(app: &mut App, k: Key) -> Vec<Effect> {
    app.update(Msg::Key(k))
}
fn field(app: &mut App, s: &str) -> Vec<Effect> {
    paste(app, s);
    key(app, Key::Enter)
}
fn frame(app: &App, name: &str) -> String {
    use crate::tui::{style, text::Source};
    use ratatui::buffer::Cell;
    let mut terminal = Terminal::new(FilteredBackend::new(TestBackend::new(80, 24))).unwrap();
    let mut placed = Vec::new();
    terminal.draw(|f| placed = ui::draw(f, app)).unwrap();
    assert_eq!(terminal.backend().replaced(), 0);
    let buf = terminal.backend().inner().buffer();
    let styles: Vec<_> = style::ALL
        .iter()
        .map(|style| {
            let mut cell = Cell::default();
            cell.set_style(*style);
            cell.style()
        })
        .collect();
    for cell in &buf.content {
        assert_eq!(cell.symbol().len(), 1, "not a single ASCII cell");
        assert!(cell.symbol().bytes().all(|b| (0x20..=0x7e).contains(&b)));
        assert!(styles.contains(&cell.style()), "style: {:?}", cell.style());
    }
    let rows: Vec<String> = (0..24)
        .map(|y| (0..80).map(|x| buf[(x, y)].symbol()).collect())
        .collect();
    for row in &rows {
        assert!(row.bytes().all(|b| (0x20..=0x7e).contains(&b)));
    }
    let mut locations = Vec::new();
    for p in placed {
        // These contract budgets are independent of the production Source::budget values.
        let max = match p.outside.source() {
            Source::WhitelistName => 32,
            Source::Quantity => 78,
            Source::TransactionHash => 66,
            Source::KeyName | Source::RpcHost => 32,
            Source::Detail => 72,
            Source::Home | Source::Expiry => 64,
            Source::Address => 42,
            Source::Input | Source::Error | Source::Warning => 512,
        };
        assert!(
            p.outside.text().len() <= max,
            "source: {:?}",
            p.outside.source()
        );
        assert!(p.area.x + p.area.width <= 80 && p.area.y + p.area.height <= 24);
        assert!(p.outside.text().bytes().all(|b| (0x20..=0x7e).contains(&b)));
        assert!(
            p.area.height == 1
                || (p.outside.source() == Source::Error && (1..=7).contains(&p.area.height)),
            "outside block: {:?}",
            p.area
        );
        if p.area.height == 1 {
            assert_eq!(usize::from(p.area.width), p.outside.text().len());
        }
        let shown = (p.area.y..p.area.bottom())
            .flat_map(|y| (p.area.x..p.area.right()).map(move |x| buf[(x, y)].symbol()))
            .collect::<String>();
        assert!(
            shown.starts_with(p.outside.text()),
            "outside placement: {:?}",
            p.area
        );
        assert!(shown[p.outside.text().len()..].bytes().all(|b| b == b' '));
        locations.push(format!(
            "{:?}: x={} y={} width={} height={} text={}",
            p.outside.source(),
            p.area.x,
            p.area.y,
            p.area.width,
            p.area.height,
            p.outside.text()
        ));
    }
    let text = rows.join("\n");
    let dir = Path::new("/tmp/zwallet-tui6-frames");
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(format!("{name}.txt")), &text).unwrap();
    std::fs::write(
        dir.join(format!("{name}.placements.txt")),
        locations.join("\n"),
    )
    .unwrap();
    text
}
fn prepare_add(app: &mut App, address: &str, contract: bool) -> Vec<Effect> {
    key(app, Key::Char(if contract { 'C' } else { 'a' }));
    field(app, "bob");
    field(app, address);
    field(app, "base");
    if contract {
        field(app, "0x12345678");
    }
    field(app, "")
}
fn review(app: &mut App, summary: &str) {
    let id = app.whitelist.request;
    app.update(Msg::WhitelistPrepared(id, Ok(summary.into())));
    assert!(matches!(app.whitelist.stage, Stage::Review(_)));
}
/// Pages with Enter until every summary row has been on screen; that Enter opens the field. End
/// no longer jumps there (it pages one screen at a time).
fn enter_password(app: &mut App) {
    for _ in 0..64 {
        key(app, Key::Enter);
        if matches!(app.password_state(), PasswordState::Input { .. }) {
            return;
        }
    }
    panic!("Enter never opened the password field");
}

#[test]
fn t44_zero_form_rejects_before_password_prepare_or_write_for_both_families() {
    for address in [
        "0x0000000000000000000000000000000000000000".to_string(),
        crate::chains::tron::address::from_body(&[0; 20]),
    ] {
        for contract in [false, true] {
            let mut app = new_app();
            let effects = prepare_add(&mut app, &address, contract);
            assert!(effects.is_empty());
            assert_eq!(app.password_state(), PasswordState::Idle);
            assert!(matches!(app.whitelist.stage, Stage::Form(_)));
            let shown = frame(
                &app,
                if contract {
                    "zero-contract"
                } else {
                    "zero-payee"
                },
            );
            assert!(shown.contains("zero address"), "{shown}");
            assert!(!shown.contains("Wallet password"));
        }
    }
}

#[test]
fn t7_exact_summary_is_reviewed_before_password_and_one_apply_runs_to_outcome() {
    let mut app = new_app();
    let effects = prepare_add(
        &mut app,
        "0x3333333333333333333333333333333333333333",
        false,
    );
    assert!(matches!(&effects[..], [Effect::PrepareWhitelist(_)]));
    assert_eq!(app.password_state(), PasswordState::Preparing);
    let summary = (0..28)
        .map(|n| format!("exact authorized line {n:02}: {}", "q?123[]Lr".repeat(3)))
        .collect::<Vec<_>>()
        .join("\n");
    review(&mut app, &summary);
    let Stage::Review(r) = &app.whitelist.stage else {
        panic!()
    };
    assert_eq!(r.summary, summary);
    let shown = frame(&app, "review-start");
    assert!(shown.contains("exact authorized line 00"));
    assert!(!shown.contains("Wallet password"));
    key(&mut app, Key::Enter);
    assert_eq!(
        app.password_state(),
        PasswordState::Idle,
        "a long summary pages first"
    );
    enter_password(&mut app);
    app.update(Msg::PasswordLength(17, false));
    let shown = frame(&app, "password");
    assert!(shown.contains("exact authorized line 27") && shown.contains("Wallet password"));
    assert!(shown.contains("*****************"));
    let id = app.whitelist.request;
    assert_eq!(
        app.update(Msg::PasswordSubmitted),
        vec![Effect::ApplyWhitelist(id)]
    );
    for msg in [
        Msg::Key(Key::Char('q')),
        Msg::Key(Key::Interrupt),
        Msg::Key(Key::Char('L')),
        Msg::Key(Key::Char('1')),
        Msg::Key(Key::Char('a')),
        Msg::PasswordSubmitted,
        Msg::PasswordCancelled,
    ] {
        assert!(app.update(msg).is_empty());
        assert!(app.whitelist_busy() && !app.quitting());
    }
    let shown = frame(&app, "applying");
    assert!(shown.contains("Saving/checking"));
    let checked = checked();
    let outcome = WhitelistOutcome {
        list: checked.list.clone(),
        stopped_daemons: vec![8],
        session: EditSession::Locked,
        warning: None,
        checked: Ok(checked.clone()),
        output: json!({"locked":true}),
        text: "added bob; gen 3; stopped daemon 8; wallet stays locked".into(),
    };
    let work = app.update(Msg::WhitelistApplied(id, Applied(Ok(outcome))));
    assert!(work.contains(&Effect::LockAll));
    assert!(!app.whitelist_busy());
    assert_eq!(app.checked_list().unwrap(), &checked);
    assert!(frame(&app, "outcome").contains("wallet stays locked"));
    key(&mut app, Key::Char('q'));
    assert!(!app.quitting());
    app.update(Msg::Locked(Ok(vec![])));
    assert!(app.quitting());
}

#[test]
fn t11_cancel_discards_plan_and_retry_never_retains_password_or_typeahead() {
    let mut app = new_app();
    prepare_add(
        &mut app,
        "0x3333333333333333333333333333333333333333",
        false,
    );
    review(
        &mut app,
        "The password authorizes this exact change\nafter the change: the wallet stays locked",
    );
    enter_password(&mut app);
    app.update(Msg::PasswordLength(9, false));
    let id = app.whitelist.request;
    assert_eq!(
        app.update(Msg::PasswordCancelled),
        vec![Effect::DiscardWhitelist(id)]
    );
    assert_eq!(app.password_state(), PasswordState::Idle);
    assert!(app.update(Msg::PasswordSubmitted).is_empty());
    assert!(!frame(&app, "cancel").contains("*********"));
    prepare_add(
        &mut app,
        "0x3333333333333333333333333333333333333333",
        false,
    );
    review(&mut app, "new summary");
    enter_password(&mut app);
    assert_eq!(
        app.password_state(),
        PasswordState::Input {
            characters: 0,
            rejected: false
        }
    );
}

#[test]
fn t10_outcome_keeps_the_global_lock_and_waits_before_quitting() {
    for failed in [false, true] {
        let mut app = new_app();
        app.whitelist.stage = Stage::Applying(Review::new("reviewed change".into()));
        app.password = PasswordState::Checking;
        let result = if failed {
            Err(WalletError::validation("session start failed after edit"))
        } else {
            let checked = checked();
            Ok(WhitelistOutcome {
                list: checked.list.clone(),
                stopped_daemons: vec![],
                session: EditSession::Unlocked {
                    duration: "10m".into(),
                    ready: json!({"pid": 91}),
                },
                warning: None,
                checked: Ok(checked),
                output: json!({"locked":false}),
                text: "added bob; session started for 10m".into(),
            })
        };
        app.update(Msg::WhitelistApplied(
            app.whitelist.request,
            Applied(result),
        ));
        assert!(matches!(app.whitelist.stage, Stage::Outcome { .. }));
        assert_eq!(key(&mut app, Key::Char('L')), vec![Effect::LockAll]);
        assert!(app.locking());
        assert!(key(&mut app, Key::Char('L')).is_empty());
        key(&mut app, Key::Char('q'));
        assert!(!app.quitting());
        app.update(Msg::Locked(Ok(vec![91])));
        assert!(app.quitting());
    }
}

#[test]
fn t10_review_lock_discards_the_plan_before_the_global_lock() {
    let mut app = new_app();
    prepare_add(
        &mut app,
        "0x3333333333333333333333333333333333333333",
        false,
    );
    review(&mut app, "the exact change before password");
    let id = app.whitelist.request;
    assert_eq!(
        key(&mut app, Key::Char('L')),
        vec![Effect::DiscardWhitelist(id), Effect::LockAll]
    );
    assert!(matches!(app.whitelist.stage, Stage::List));
    assert_eq!(app.password_state(), PasswordState::Idle);
    assert!(app.update(Msg::PasswordSubmitted).is_empty());
    assert!(app.locking());
    key(&mut app, Key::Char('q'));
    assert!(!app.quitting());
    app.update(Msg::Locked(Ok(vec![])));
    assert!(app.quitting());
}

#[test]
fn t10_apply_defers_lock_and_quit_until_optional_start_and_lock_finish() {
    for failed in [false, true] {
        let mut app = new_app();
        app.whitelist.stage =
            Stage::Applying(Review::new("after this change: start a 10m session".into()));
        app.password = PasswordState::Checking;
        for key_event in [
            Key::Char('L'),
            Key::Char('L'),
            Key::Char('q'),
            Key::Interrupt,
        ] {
            assert!(key(&mut app, key_event).is_empty());
            assert!(app.whitelist_busy() && !app.quitting());
        }
        assert!(
            app.locking(),
            "the panic lock must be remembered during the write"
        );
        assert!(app.quit_pending());
        assert!(app.update(Msg::PasswordCancelled).is_empty());
        assert!(app.update(Msg::PasswordSubmitted).is_empty());
        let outcome = if failed {
            Err(WalletError::validation(
                "optional start failed after the edit",
            ))
        } else {
            let checked = checked();
            Ok(WhitelistOutcome {
                list: checked.list.clone(),
                stopped_daemons: vec![],
                session: EditSession::Unlocked {
                    duration: "10m".into(),
                    ready: json!({"pid":91}),
                },
                warning: None,
                checked: Ok(checked),
                output: json!({"locked":false}),
                text: "edited and started a 10m session".into(),
            })
        };
        let id = app.whitelist.request;
        let effects = app.update(Msg::WhitelistApplied(id, Applied(outcome)));
        assert_eq!(
            effects
                .iter()
                .filter(|e| matches!(e, Effect::LockAll))
                .count(),
            1
        );
        assert!(!app.whitelist_busy() && !app.quitting());
        assert!(
            app.update(Msg::WhitelistApplied(
                id,
                Applied(Err(WalletError::validation("stale")))
            ))
            .is_empty()
        );
        app.update(Msg::Locked(Ok(vec![91])));
        assert!(app.quitting());
        assert_eq!(app.take_quit_error().is_some(), failed);
    }
}

#[test]
fn t10_apply_quit_waits_for_outcome_without_implicitly_requesting_lock() {
    for exit_key in [Key::Char('q'), Key::Interrupt] {
        let mut app = new_app();
        app.whitelist.stage = Stage::Applying(Review::new("reviewed change".into()));
        app.password = PasswordState::Checking;
        key(&mut app, exit_key);
        assert!(app.quit_pending() && !app.quitting());
        assert!(!app.locking());
        let effects = app.update(Msg::WhitelistApplied(
            app.whitelist.request,
            Applied(Err(WalletError::validation("edit refused"))),
        ));
        assert!(!effects.contains(&Effect::LockAll));
        assert!(app.quitting());
        assert!(app.take_quit_error().is_some());
    }
}

#[test]
fn t23_t25_each_menu_action_has_a_real_form_and_removal_takes_only_checked_selection() {
    let mut app = new_app();
    app.checked_list = Some(checked());
    for action in ACTIONS {
        assert!(app.menu_context().implemented.contains(action));
        assert!(matches!(
            actions::by_id(*action).availability(&app.menu_context()),
            actions::Availability::Ready
        ));
    }
    for action in [
        ActionId::FeeCap,
        ActionId::NativeCap,
        ActionId::TokenCap,
        ActionId::CalldataCap,
        ActionId::WhitelistContract,
        ActionId::MessagePolicy,
        ActionId::Reset,
        ActionId::WhitelistUnlock,
    ] {
        app.whitelist.stage = Stage::List;
        app.whitelist_action(action).unwrap();
        assert!(matches!(app.whitelist.stage, Stage::Form(_)), "{action:?}");
        frame(&app, &format!("form-{action:?}"));
    }
    app.whitelist.stage = Stage::List;
    app.whitelist.selected = 1;
    key(&mut app, Key::Char('d'));
    let effects = field(&mut app, "");
    let [Effect::PrepareWhitelist(request)] = &effects[..] else {
        panic!("{effects:?}")
    };
    assert_eq!(request.draft, Draft::Remove("zero".into()));
    assert_eq!(request.checked_fingerprint, Some(checked().fingerprint));
    app.whitelist.stage = Stage::List;
    app.password = PasswordState::Idle;
    app.checked_list = None;
    key(&mut app, Key::Char('d'));
    assert!(matches!(app.whitelist.stage, Stage::List));
}

#[test]
fn t23_all_mutation_forms_dispatch_typed_amounts_and_explicit_unlock() {
    let cases: &[(ActionId, &[&str])] = &[
        (ActionId::FeeCap, &["0.001"]),
        (ActionId::NativeCap, &["none"]),
        (
            ActionId::TokenCap,
            &["0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913", "1000000"],
        ),
        (ActionId::CalldataCap, &["68"]),
    ];
    for (action, values) in cases {
        let mut app = new_app();
        app.whitelist_action(*action).unwrap();
        key(&mut app, Key::Enter); // default chain
        for value in *values {
            field(&mut app, value);
        }
        let effects = field(&mut app, "10m");
        let [Effect::PrepareWhitelist(request)] = &effects[..] else {
            panic!("{action:?}: {effects:?}")
        };
        assert_eq!(request.unlock, "10m");
        assert!(request.draft.change().is_ok());
    }
    let mut app = new_app();
    app.checked_list = Some(checked());
    key(&mut app, Key::Char('m'));
    key(&mut app, Key::Enter);
    let effects = field(&mut app, "");
    assert!(matches!(
        &effects[..],
        [Effect::PrepareWhitelist(EditRequest {
            draft: Draft::Message(true),
            ..
        })]
    ));
    let mut app = new_app();
    key(&mut app, Key::Char('R'));
    let effects = field(&mut app, "");
    assert!(matches!(
        &effects[..],
        [Effect::PrepareWhitelist(EditRequest {
            draft: Draft::Reset,
            ..
        })]
    ));
}

#[test]
fn t7_forged_daemon_report_never_supplies_names_or_verified_edit_state() {
    let mut app = new_app();
    app.nav.session = Some(StatusView {
        configured: true,
        locked: false,
        daemon_pids: vec![1],
        other_daemon_pids: vec![],
        active: None,
        address: None,
        serving: Some(crate::wallet::Serving {
            pid: 1,
            expires_at: None,
            whitelist: json!({"gen":999,"verified":true,"entries":[{"nickname":"MALICIOUS_PAYEE","address":"0x1111111111111111111111111111111111111111"}]}),
        }),
    });
    let shown = frame(&app, "untrusted-session");
    assert!(shown.contains("reports gen 999") && shown.contains("not verified"));
    assert!(!shown.contains("MALICIOUS_PAYEE") && !shown.contains("verified by password"));
    key(&mut app, Key::Char('m'));
    assert!(matches!(app.whitelist.stage, Stage::List));
    assert!(app.checked_list().is_none());
}

#[test]
fn t7_refresh_drops_the_old_checked_cache_until_files_are_rechecked() {
    let mut app = new_app();
    app.checked_list = Some(checked());
    let effects = key(&mut app, Key::Char('r'));
    let id = effects
        .iter()
        .find_map(|e| {
            if let Effect::ReadWhitelist(id, Some(_)) = e {
                Some(*id)
            } else {
                None
            }
        })
        .unwrap();
    assert!(app.checked_list().is_none());
    app.update(Msg::WhitelistRead(
        id,
        Ok(Read {
            checked: None,
            text: "not verified: whitelist.json changed".into(),
            reason: Some("changed".into()),
        }),
    ));
    assert!(app.checked_list().is_none());
    assert!(!frame(&app, "invalidated").contains("verified by password"));
}

#[test]
fn t7_fields_reject_escape_glyphs_overflow_and_consume_global_shortcuts() {
    let mut app = new_app();
    key(&mut app, Key::Char('a'));
    for text in ["\u{1b}[2J", "▀▄█", &"x".repeat(600)] {
        paste(&mut app, text);
        let Stage::Form(form) = &app.whitelist.stage else {
            panic!()
        };
        assert_eq!(form.rows[0].field.value(), "");
        frame(&app, "rejected-input");
    }
    paste(&mut app, "q123r");
    assert!(!app.quitting());
    assert_eq!(app.tab(), Tab::Whitelist);
    let Stage::Form(form) = &app.whitelist.stage else {
        panic!()
    };
    assert_eq!(form.rows[0].field.value(), "q123r");
}

#[test]
fn t7_malformed_files_are_bounded_unreadable_views_with_no_replaced_cells() {
    use crate::core::store::{self, KDF_MIN_LOG_N, KdfParams, MasterOptions};
    use crate::core::whitelist::WhitelistKey;
    let home = tempfile::Builder::new()
        .prefix("zwt-tui6-files-")
        .tempdir_in("/tmp")
        .unwrap();
    store::create_master(
        home.path(),
        "test whitelist view password",
        "main",
        MasterOptions {
            kdf: KdfParams {
                log_n: KDF_MIN_LOG_N,
                ..Default::default()
            },
            ..Default::default()
        },
        &crate::chains::derive_addresses,
    )
    .unwrap();
    let bytes = WhitelistKey::from_secret(&[1; 32])
        .sign(&checked().list)
        .to_file_bytes()
        .unwrap();
    let original: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    for (name, bad) in [
        ("long", "x".repeat(600)),
        ("glyph", "▀▄█".into()),
        ("escape", "bad\u{1b}[2J".into()),
    ] {
        let mut value = original.clone();
        value["entries"][0]["nickname"] = json!(bad);
        std::fs::write(
            home.path().join("whitelist.json"),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
        let result = read(home.path(), None).unwrap();
        assert!(result.checked.is_none());
        assert!(result.text.contains("cannot be read"), "{}", result.text);
        assert!(result.text.len() <= 125);
        let mut app = new_app();
        app.load_whitelist();
        let id = app.whitelist.request;
        app.update(Msg::WhitelistRead(id, Ok(result)));
        let shown = frame(&app, &format!("malformed-{name}"));
        assert!(shown.contains("cannot be read"));
        assert!(!shown.contains("verified by password"));
    }
}

#[test]
fn t7_stale_completion_cannot_replace_a_new_plan_or_a_finished_outcome() {
    let mut app = new_app();
    prepare_add(
        &mut app,
        "0x3333333333333333333333333333333333333333",
        false,
    );
    let id = app.whitelist.request;
    app.update(Msg::WhitelistPrepared(
        id + 1,
        Ok("forged stale summary".into()),
    ));
    assert!(matches!(app.whitelist.stage, Stage::Preparing));
    review(&mut app, "genuine summary");
    enter_password(&mut app);
    app.update(Msg::PasswordSubmitted);
    app.update(Msg::WhitelistPrepared(id, Ok("late prepare".into())));
    assert!(app.whitelist_busy());
    app.update(Msg::WhitelistApplied(
        id,
        Applied(Err(WalletError::validation("edit refused before writing"))),
    ));
    assert!(matches!(app.whitelist.stage, Stage::Outcome { .. }));
    app.update(Msg::WhitelistApplied(
        id,
        Applied(Err(WalletError::validation("replayed completion"))),
    ));
    assert!(!frame(&app, "failed-edit").contains("replayed completion"));
    assert!(app.checked_list().is_none());
}

/// The prepare worker checked the checked list's fingerprint, then the plan read the files again.
/// A list rewritten in between must not become the plan the password authorizes.
#[test]
fn prepare_refuses_a_list_rewritten_between_the_check_and_the_plan_read() {
    use crate::core::store;
    use crate::core::whitelist::{WHITELIST_FILE, WhitelistKey};
    let home = tempfile::Builder::new()
        .prefix("zwt-wlprep-")
        .tempdir_in("/tmp")
        .unwrap();
    store::create_master(
        home.path(),
        "whitelist prepare test password",
        "main",
        store::MasterOptions {
            kdf: store::KdfParams {
                log_n: store::KDF_MIN_LOG_N,
                ..Default::default()
            },
            ..Default::default()
        },
        &crate::chains::derive_addresses,
    )
    .unwrap();
    let write = |generation: u64, entries: &[(&str, &str)]| {
        let mut list = Whitelist::empty(&crate::chains::WHITELIST_RULES);
        list.generation = generation;
        for (nickname, address) in entries {
            list.add(Entry::new_address(nickname, address, &["base"]))
                .unwrap();
        }
        let signed = WhitelistKey::from_secret(&[9; 32]).sign(&list);
        std::fs::write(
            home.path().join(WHITELIST_FILE),
            signed.to_file_bytes().unwrap(),
        )
        .unwrap();
    };
    write(1, &[("bob", "0x3333333333333333333333333333333333333333")]);
    let checked = ListFiles::read(home.path()).fingerprint();
    let request = EditRequest {
        id: 1,
        draft: Draft::Message(true),
        unlock: String::new(),
        checked_fingerprint: Some(checked),
    };
    let result = prepare_with(home.path(), request, false, || {
        write(
            2,
            &[
                ("bob", "0x3333333333333333333333333333333333333333"),
                ("mallory", "0x4444444444444444444444444444444444444444"),
            ],
        )
    });
    let error = match result {
        Ok(plan) => panic!(
            "a plan of the rewritten list was prepared:\n{}",
            plan.summary()
        ),
        Err(error) => error,
    };
    assert!(error.message.contains("verify it again"), "{error}");
    // Unchanged files still prepare, and an unchecked edit (no verified list) compares nothing.
    for checked_fingerprint in [Some(ListFiles::read(home.path()).fingerprint()), None] {
        let request = EditRequest {
            id: 2,
            draft: Draft::Message(true),
            unlock: String::new(),
            checked_fingerprint,
        };
        assert!(prepare(home.path(), request, false).is_ok());
    }
}

/// Enter opens the password field only after every summary row has been drawn at least once.
/// End must not count the rows it jumps over.
#[test]
fn the_password_opens_only_after_every_summary_row_was_on_screen() {
    const ROWS: usize = 40;
    let summary = (0..ROWS)
        .map(|n| format!("summary row {n:02}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut app = new_app();
    prepare_add(
        &mut app,
        "0x3333333333333333333333333333333333333333",
        false,
    );
    review(&mut app, &summary);
    let mut drawn = std::collections::BTreeSet::new();
    let look = |app: &App, drawn: &mut std::collections::BTreeSet<usize>| {
        let shown = frame(app, "summary-paging");
        drawn.extend((0..ROWS).filter(|n| shown.contains(&format!("summary row {n:02}"))));
    };
    look(&app, &mut drawn);
    // The app counts a page as 10 rows; at 80x24 the frame draws at least that many.
    assert!(drawn.len() >= 10, "{drawn:?}");
    key(&mut app, Key::End);
    look(&app, &mut drawn);
    key(&mut app, Key::Enter);
    assert!(
        !matches!(app.password_state(), PasswordState::Input { .. }),
        "End then Enter opened the password with rows never drawn: {drawn:?}"
    );
    look(&app, &mut drawn);
    let mut presses = 0;
    loop {
        let before = drawn.len();
        key(&mut app, Key::Enter);
        if matches!(app.password_state(), PasswordState::Input { .. }) {
            assert_eq!(before, ROWS, "the password opened with rows never drawn");
            break;
        }
        look(&app, &mut drawn);
        presses += 1;
        assert!(presses < ROWS, "Enter never reached the password");
    }
}

/// D1: the summary review and the outcome are not text fields; the global tab keys act there,
/// and the review is still there on return.
#[test]
fn summary_review_and_outcome_pass_the_global_tab_keys() {
    let mut app = new_app();
    prepare_add(
        &mut app,
        "0x3333333333333333333333333333333333333333",
        false,
    );
    review(&mut app, "the change before the password");
    key(&mut app, Key::Char('2'));
    assert_eq!(app.nav.tab, Tab::Wallet);
    app.nav.tab = Tab::Whitelist;
    assert!(matches!(app.whitelist.stage, Stage::Review(_)));
    key(&mut app, Key::Char('['));
    assert_eq!(app.nav.tab, Tab::Receive);
    app.nav.tab = Tab::Whitelist;
    app.whitelist.stage = Stage::Outcome {
        lines: lines("added bob"),
        scroll: 0,
    };
    key(&mut app, Key::Char(']'));
    assert_eq!(app.nav.tab, Tab::Tools);
}

/// Fix 11 (real-terminal acceptance): the list pane never wraps. One row per entry and one per
/// fee cap, cut with `...` at the pane width; the selected entry shows its full fields.
#[test]
fn the_list_pane_never_wraps_and_the_selection_shows_full_fields() {
    use crate::core::whitelist::{TransactionLimits, WHITELIST_FILE, WhitelistKey};
    use alloy::primitives::U256;
    let home = tempfile::Builder::new()
        .prefix("zwt-wl-rows-")
        .tempdir_in("/tmp")
        .unwrap();
    let mut list = Whitelist::empty(&crate::chains::WHITELIST_RULES);
    list.generation = 8;
    list.add(Entry::new_address(
        "bob",
        "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC",
        &["base", "arbitrum"],
    ))
    .unwrap();
    list.add(Entry::new_contract(
        "qa",
        "0x5FbDB2315678afecb367f032d93F642f64180aa3",
        &["base"],
        &["0x55241077"],
    ))
    .unwrap();
    for chain in crate::chains::CHAINS {
        list.policy
            .set_max_fee_wei(chain.key, U256::from(1_000_000u64));
    }
    list.policy.transaction_limits.insert(
        "base".into(),
        TransactionLimits {
            max_native_wei: Some("500000000000000000".into()),
            max_calldata_bytes: Some(256),
            max_token_base_units: [(
                "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913".to_string(),
                "1000000".to_string(),
            )]
            .into(),
        },
    );
    let key = WhitelistKey::from_secret(&[7; 32]);
    let signed = key.sign(&list);
    std::fs::write(
        home.path().join(WHITELIST_FILE),
        signed.to_file_bytes().unwrap(),
    )
    .unwrap();
    let checked =
        crate::wallet::check_list(home.path(), &Ok(Some(key.anchor_for(&signed)))).unwrap();
    let view = crate::wallet::whitelist::checked_view(home.path(), &checked).unwrap();
    let mut app = new_app();
    app.whitelist.request = 1;
    app.whitelist.read_generation = Some(app.generation);
    app.update(Msg::WhitelistRead(
        1,
        Ok(Read {
            checked: Some(checked),
            text: view.text,
            reason: None,
        }),
    ));
    let draw = |app: &App, width: u16, height: u16| -> Vec<String> {
        let mut terminal =
            Terminal::new(FilteredBackend::new(TestBackend::new(width, height))).unwrap();
        terminal.draw(|f| drop(ui::draw(f, app))).unwrap();
        assert_eq!(terminal.backend().replaced(), 0);
        let b = terminal.backend().inner().buffer();
        (0..height)
            .map(|y| (0..width).map(|x| b[(x, y)].symbol()).collect::<String>())
            .collect()
    };
    for (width, height) in [(120, 40), (80, 24)] {
        let rows = draw(&app, width, height);
        let text = rows.join("\n");
        let bob = rows
            .iter()
            .find(|r| r.contains("bob") && r.contains("address"))
            .unwrap_or_else(|| panic!("{text}"));
        if width == 120 {
            assert!(bob.contains("base, arbitrum"), "{text}");
        }
        // A row never starts with the rest of the previous one.
        for fragment in ["|ase", "|ia ", "|taBytes", "|x833589"] {
            assert!(!text.contains(fragment), "{width}x{height} wraps:\n{text}");
        }
        // One fee cap per chain, each on its own row.
        for chain in crate::chains::CHAINS {
            let with: Vec<_> = rows
                .iter()
                .filter(|r| r.contains(&format!("fee cap {} ", chain.key)))
                .collect();
            assert_eq!(with.len(), 1, "{}:\n{text}", chain.key);
        }
    }
    // The selected entry's full fields, whatever the width.
    let text = draw(&app, 80, 24).join("\n");
    assert!(text.contains("selected for d: bob"), "{text}");
    assert!(text.contains("chains base, arbitrum"), "{text}");
    assert!(
        text.contains("address 0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC"),
        "{text}"
    );
}
