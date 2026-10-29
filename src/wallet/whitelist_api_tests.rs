use super::*;
use crate::core::error::ErrorCode;
use std::cell::Cell;

const PW: &str = "whitelist boundary test password";
const BOB: &str = "0x3333333333333333333333333333333333333333";
const ALICE: &str = "0x4444444444444444444444444444444444444444";
const ZERO: &str = "0x0000000000000000000000000000000000000000";

fn home() -> tempfile::TempDir {
    let home = tempfile::Builder::new()
        .prefix("zwt-whitelist-")
        .tempdir_in("/tmp")
        .unwrap();
    store::create_master(
        home.path(),
        PW,
        "main",
        store::MasterOptions {
            kdf: store::KdfParams {
                log_n: store::KDF_MIN_LOG_N,
                ..Default::default()
            },
            ..Default::default()
        },
        &chains::derive_addresses,
    )
    .unwrap();
    home
}

fn add(nick: &str, address: &str) -> Change {
    Change::add(nick, address, &["base"], EntryKind::Address, &[]).unwrap()
}

fn snapshot(home: &Path) -> (Vec<u8>, Option<Vec<u8>>, Option<Vec<u8>>) {
    (
        std::fs::read(home.join("wallet.json")).unwrap(),
        std::fs::read(home.join(WHITELIST_FILE)).ok(),
        std::fs::read(home.join(NEXT_FILE)).ok(),
    )
}

async fn apply(home: &Path, change: Change) -> WhitelistOutcome {
    let plan = whitelist_plan(home, change, None).unwrap();
    apply_with(
        plan,
        Zeroizing::new(PW.to_string()),
        || Ok(vec![901]),
        |_, _, _, _| async { panic!("locked edit must never start a session") },
    )
    .await
    .unwrap()
}

#[test]
fn t44_zero_add_refused_for_both_families_and_entry_kinds() {
    let tron_zero = chains::tron::address::from_body(&[0; 20]);
    for address in [ZERO, tron_zero.as_str()] {
        for kind in [EntryKind::Address, EntryKind::Contract] {
            let selectors: &[&str] = if kind == EntryKind::Contract {
                &["0x12345678"]
            } else {
                &[]
            };
            let error = match Change::add("zero", address, &[], kind, selectors) {
                Err(e) => e,
                Ok(_) => panic!("zero address was accepted"),
            };
            assert_eq!(error.code, ErrorCode::ValidationError);
            assert!(error.message.contains("zero address"), "{error}");
        }
    }
}

#[test]
fn t24_constructors_validate_selectors_family_registry_and_caps() {
    assert!(
        Change::add(
            "router",
            BOB,
            &["base"],
            EntryKind::Contract,
            &["0x12345678"]
        )
        .is_ok()
    );
    assert!(
        Change::add(
            "token",
            "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
            &["base"],
            EntryKind::Contract,
            &["0xa9059cbb"]
        )
        .is_err()
    );
    assert!(Change::add("router", BOB, &["*"], EntryKind::Contract, &["0x12345678"]).is_err());
    assert!(Change::add("bob", BOB, &["base"], EntryKind::Address, &["0xa9059cbb"]).is_err());
    assert!(Change::add("bob", BOB, &["tron"], EntryKind::Address, &[]).is_err());
    assert!(Change::fee_cap("base", "0").is_err());
    assert!(Change::native_cap("base", "-1").is_err());
    assert!(Change::token_cap("base", BOB, "1").is_err());
    assert!(Change::calldata_cap("base", "4294967296").is_err());
    assert!(Change::remove("bad name").is_err());
}

#[tokio::test(flavor = "current_thread")]
async fn t7_wrong_password_refuses_without_stop_start_or_write() {
    let home = home();
    let before = snapshot(home.path());
    let plan = whitelist_plan(home.path(), add("bob", BOB), None).unwrap();
    assert!(plan.summary().contains("bob") && plan.summary().contains(BOB));
    assert_eq!(before, snapshot(home.path()));
    let stopped = Cell::new(false);
    let result = apply_with(
        plan,
        Zeroizing::new("wrong password".to_string()),
        || {
            stopped.set(true);
            Ok(vec![])
        },
        |_, _, _, _| async { panic!("must not start") },
    )
    .await;
    assert!(result.is_err());
    assert!(!stopped.get());
    assert_eq!(before, snapshot(home.path()));
}

#[tokio::test(flavor = "current_thread")]
async fn t7_either_file_swap_after_summary_refuses_before_stop() {
    let home = home();
    apply(home.path(), add("bob", BOB)).await;
    for filename in [WHITELIST_FILE, NEXT_FILE] {
        let plan = whitelist_plan(home.path(), Change::message_signing(true), None).unwrap();
        let path = home.path().join(filename);
        let old = std::fs::read(&path).ok();
        let mut changed = std::fs::read(home.path().join(WHITELIST_FILE)).unwrap();
        changed.push(b' ');
        std::fs::write(&path, changed).unwrap();
        let before = snapshot(home.path());
        let stopped = Cell::new(false);
        let result = apply_with(
            plan,
            Zeroizing::new(PW.to_string()),
            || {
                stopped.set(true);
                Ok(vec![])
            },
            |_, _, _, _| async { panic!("must not start") },
        )
        .await;
        let e = match result {
            Err(e) => e,
            Ok(_) => panic!("swapped files were accepted"),
        };
        assert!(e.message.contains("changed after it was shown"), "{e}");
        assert!(!stopped.get());
        assert_eq!(before, snapshot(home.path()));
        if let Some(bytes) = old {
            std::fs::write(path, bytes).unwrap();
        } else {
            std::fs::remove_file(path).unwrap();
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn t7_refused_authenticated_mutation_stops_nothing() {
    let home = home();
    apply(home.path(), add("bob", BOB)).await;
    for change in [add("bob", ALICE), Change::remove("unknown").unwrap()] {
        let plan = whitelist_plan(home.path(), change, None).unwrap();
        let before = snapshot(home.path());
        let result = apply_with(
            plan,
            Zeroizing::new(PW.to_string()),
            || panic!("must not stop"),
            |_, _, _, _| async { panic!("must not start") },
        )
        .await;
        assert!(result.is_err());
        assert_eq!(before, snapshot(home.path()));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn t7_stop_precedes_write_and_landed_output_is_locally_verified() {
    let home = home();
    let before = snapshot(home.path());
    let plan = whitelist_plan(home.path(), add("bob", BOB), None).unwrap();
    let outcome = apply_with(
        plan,
        Zeroizing::new(PW.to_string()),
        || {
            assert_eq!(before, snapshot(home.path()));
            Ok(vec![91, 92])
        },
        |_, _, _, _| async { panic!("must not start") },
    )
    .await
    .unwrap();
    assert_eq!(outcome.list.generation, 1);
    assert_eq!(outcome.stopped_daemons, vec![91, 92]);
    assert!(matches!(outcome.session, EditSession::Locked));
    assert_eq!(outcome.output["locked"], true);
    assert_eq!(outcome.output["verified"], true);
    let checked = outcome.checked.unwrap();
    assert_eq!(checked.list, outcome.list);
    assert_eq!(
        checked_payees(home.path(), &checked, "base").unwrap().len(),
        1
    );
    assert!(!outcome.text.contains(PW));
    assert!(!outcome.output.to_string().contains(PW));
}

#[tokio::test(flavor = "current_thread")]
async fn t7_stop_failure_prevents_write() {
    let home = home();
    let before = snapshot(home.path());
    let plan = whitelist_plan(home.path(), add("bob", BOB), None).unwrap();
    let result = apply_with(
        plan,
        Zeroizing::new(PW.to_string()),
        || Err(WalletError::validation("test stop refused")),
        |_, _, _, _| async { panic!("must not start") },
    )
    .await;
    assert!(result.is_err());
    assert_eq!(before, snapshot(home.path()));
}

#[tokio::test(flavor = "current_thread")]
async fn t7_after_stop_swap_refuses_under_writer_lock() {
    let home = home();
    apply(home.path(), add("bob", BOB)).await;
    let wallet_before = std::fs::read(home.path().join("wallet.json")).unwrap();
    let plan = whitelist_plan(home.path(), Change::message_signing(true), None).unwrap();
    let result = apply_with(
        plan,
        Zeroizing::new(PW.to_string()),
        || {
            let path = home.path().join(WHITELIST_FILE);
            let mut bytes = std::fs::read(&path).unwrap();
            bytes.push(b' ');
            std::fs::write(path, bytes).unwrap();
            Ok(vec![1])
        },
        |_, _, _, _| async { panic!("must not start") },
    )
    .await;
    let e = match result {
        Err(e) => e,
        Ok(_) => panic!("swapped files were accepted"),
    };
    assert!(e.message.contains("already stopped") && e.message.contains("locked"));
    assert_eq!(
        wallet_before,
        std::fs::read(home.path().join("wallet.json")).unwrap()
    );
}

#[tokio::test(flavor = "current_thread")]
async fn t23_t25_caps_message_contract_remove_reset_share_the_verified_base() {
    let home = home();
    apply(home.path(), add("bob", BOB)).await;
    let changes = [
        Change::fee_cap("base", "0.001").unwrap(),
        Change::native_cap("tron", "2").unwrap(),
        Change::token_cap("tron", "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t", "1000000").unwrap(),
        Change::calldata_cap("tron", "68").unwrap(),
        Change::message_signing(true),
        Change::add(
            "router",
            ALICE,
            &["base"],
            EntryKind::Contract,
            &["0x12345678"],
        )
        .unwrap(),
    ];
    for (i, change) in changes.into_iter().enumerate() {
        let outcome = apply(home.path(), change).await;
        assert_eq!(outcome.list.generation, i as u64 + 2);
        assert_eq!(outcome.checked.unwrap().list, outcome.list);
        if i == 5 {
            assert_eq!(outcome.warning.as_deref(), Some(RESIDUAL));
        }
    }
    let opened = super::super::open_with_password(home.path(), PW, Some(&[])).unwrap();
    let view = whitelist_view(home.path(), Some(&opened.anchor)).unwrap();
    let list = &view.checked.as_ref().unwrap().list;
    assert_eq!(
        list.policy.transaction_limits["tron"]
            .max_native_wei
            .as_deref(),
        Some("2000000")
    );
    assert_eq!(
        list.policy.transaction_limits["tron"].max_calldata_bytes,
        Some(68)
    );
    assert_eq!(
        list.policy.transaction_limits["tron"]
            .max_token_base_units
            .values()
            .next()
            .unwrap(),
        "1000000"
    );
    assert!(list.policy.allow_message_signing);
    let plan = whitelist_plan(home.path(), Change::message_signing(false), None).unwrap();
    assert!(plan.summary().contains("on") && plan.summary().contains("off"));
    let removed = apply(home.path(), Change::remove("bob").unwrap()).await;
    assert_eq!(removed.output["entry"]["nickname"], "bob");
    let plan = whitelist_plan(home.path(), Change::reset(), None).unwrap();
    assert!(plan.summary().contains("router") && plan.summary().contains("reset"));
    let reset = apply(home.path(), Change::reset()).await;
    assert!(reset.list.entries.is_empty());
    assert!(reset.list.policy.transaction_limits.is_empty());
    assert!(!reset.list.policy.allow_message_signing);
}

#[tokio::test(flavor = "current_thread")]
async fn t44_legacy_zero_stays_verifiable_removable_and_is_never_offered_as_payee() {
    for zero_chain in ["base", "tron"] {
        let home = home();
        // This is a pre-existing signed list. Core validation deliberately permits the old entry.
        whitelist::edit(home.path(), PW, &WHITELIST_RULES, |list| {
            list.add(Entry::new_address("zero", ZERO, &[zero_chain]))?;
            list.add(Entry::new_address("any_evm", BOB, &["*"]))?;
            list.add(Entry::new_address("tron_payee", ALICE, &["tron"]))
        })
        .unwrap();
        let before = snapshot(home.path());
        let opened = super::super::open_with_password(home.path(), PW, Some(&[])).unwrap();
        let view = whitelist_view(home.path(), Some(&opened.anchor)).unwrap();
        assert_eq!(view.output["verified"], true);
        assert!(view.text.contains("zero address (unavailable as a payee)"));
        let checked = view.checked.unwrap();
        assert_eq!(checked.list.entries.len(), 3);
        assert_eq!(
            checked_payees(home.path(), &checked, "base").unwrap()[0].nickname,
            "any_evm"
        );
        let tron = checked_payees(home.path(), &checked, "tron").unwrap();
        assert_eq!(tron.len(), 1);
        assert_eq!(tron[0].nickname, "tron_payee");
        assert_eq!(before, snapshot(home.path()));
        assert!(
            whitelist_plan(home.path(), Change::remove("zero").unwrap(), None)
                .unwrap()
                .summary()
                .contains("zero")
        );
        let outcome = apply(home.path(), Change::remove("zero").unwrap()).await;
        assert!(outcome.list.by_nickname("zero").is_none());
    }
}

#[tokio::test(flavor = "current_thread")]
async fn t7_verified_view_requires_password_anchor_and_invalidates_both_file_fingerprints() {
    let home = home();
    apply(home.path(), add("bob", BOB)).await;
    let unverified = whitelist_view(home.path(), None).unwrap();
    assert!(unverified.checked.is_none());
    assert_eq!(unverified.output["verified"], false);
    let opened = super::super::open_with_password(home.path(), PW, Some(&[])).unwrap();
    let checked = whitelist_view(home.path(), Some(&opened.anchor))
        .unwrap()
        .checked
        .unwrap();
    let wallet_before = std::fs::read(home.path().join("wallet.json")).unwrap();
    // A forged report has no role in the API. A wrong local anchor never verifies the list.
    let mut wrong = opened.anchor.clone();
    wrong.as_mut().unwrap().as_mut().unwrap().generation += 1;
    let view = whitelist_view(home.path(), Some(&wrong)).unwrap();
    assert!(view.checked.is_none());
    assert_eq!(view.output["verified"], false);
    for filename in [WHITELIST_FILE, NEXT_FILE] {
        let path = home.path().join(filename);
        let old = std::fs::read(&path).ok();
        std::fs::write(&path, b"{} ").unwrap();
        assert!(checked_payees(home.path(), &checked, "base").is_err());
        if let Some(bytes) = old {
            std::fs::write(path, bytes).unwrap();
        } else {
            std::fs::remove_file(path).unwrap();
        }
    }
    assert_eq!(
        wallet_before,
        std::fs::read(home.path().join("wallet.json")).unwrap()
    );
}

#[tokio::test(flavor = "current_thread")]
async fn t23_limit_removal_is_explicit_and_clears_empty_chain_bucket() {
    let home = home();
    apply(home.path(), add("bob", BOB)).await;
    let usdc = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913";
    for change in [
        Change::native_cap("base", "0").unwrap(),
        Change::token_cap("base", usdc, "0").unwrap(),
        Change::calldata_cap("base", "0").unwrap(),
    ] {
        apply(home.path(), change).await;
    }
    for change in [
        Change::native_cap("base", "none").unwrap(),
        Change::token_cap("base", usdc, "none").unwrap(),
        Change::calldata_cap("base", "none").unwrap(),
    ] {
        let plan = whitelist_plan(home.path(), change, None).unwrap();
        assert!(plan.summary().contains("none (unbounded)"));
        let outcome = apply_with(
            plan,
            Zeroizing::new(PW.to_string()),
            || Ok(vec![]),
            |_, _, _, _| async { panic!("must not start") },
        )
        .await
        .unwrap();
        assert!(outcome.checked.is_ok());
    }
    let opened = super::super::open_with_password(home.path(), PW, Some(&[])).unwrap();
    let checked = super::super::check_list(home.path(), &opened.anchor).unwrap();
    assert!(checked.list.policy.transaction_limits.is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn t7_failed_optional_unlock_reports_locked_after_landed_change() {
    let mut list = Whitelist::empty(&WHITELIST_RULES);
    list.generation = 3;
    let edited = Edited {
        list,
        previous_generation: Some(2),
        base: BaseSource::Current,
        discarded: None,
        warning: None,
    };
    let (edited, session, _) = land_then_unlock(
        || Ok(edited),
        Some(("10m".into(), Duration::from_secs(600), "pinned-exe")),
        (),
        |exe, pw, ttl| async move {
            assert_eq!(exe, "pinned-exe");
            assert_eq!(ttl, Duration::from_secs(600));
            (pw, Err(WalletError::validation("test start refused")))
        },
    )
    .await
    .unwrap();
    assert_eq!(edited.list.generation, 3);
    assert!(matches!(&session, EditSession::Failed(_)));
    let mut report = json!({});
    session.report(&mut report);
    assert_eq!(report["locked"], true);
    assert_eq!(report["unlockError"], "test start refused");
}

#[tokio::test(flavor = "current_thread")]
async fn t11_nonclone_password_owner_survives_start_and_is_dropped_after_local_check() {
    struct OwnedPassword {
        value: Zeroizing<String>,
        dropped: std::rc::Rc<Cell<usize>>,
    }
    impl AsRef<str> for OwnedPassword {
        fn as_ref(&self) -> &str {
            assert_eq!(self.dropped.get(), 0);
            self.value.as_str()
        }
    }
    impl ZeroizeOnDrop for OwnedPassword {}
    impl Drop for OwnedPassword {
        fn drop(&mut self) {
            self.dropped.set(self.dropped.get() + 1);
        }
    }

    let dropped = std::rc::Rc::new(Cell::new(0));
    let password = OwnedPassword {
        value: Zeroizing::new(PW.to_string()),
        dropped: dropped.clone(),
    };
    let allocation = password.value.as_ptr();
    let edited = Edited {
        list: Whitelist::empty(&WHITELIST_RULES),
        previous_generation: None,
        base: BaseSource::Empty,
        discarded: None,
        warning: None,
    };
    let (_, session, password) = land_then_unlock(
        || Ok(edited),
        Some(("10m".into(), Duration::from_secs(600), ())),
        password,
        |_, password: OwnedPassword, _| async move {
            assert_eq!(password.as_ref(), PW);
            assert_eq!(password.value.as_ptr(), allocation);
            (password, Ok(json!({"pid": 91})))
        },
    )
    .await
    .unwrap();
    assert!(matches!(session, EditSession::Unlocked { .. }));
    assert_eq!(password.value.as_ptr(), allocation);
    assert_eq!(dropped.get(), 0);

    // The public apply boundary accepts the same non-Clone owner and retains it through
    // the final password-anchored reread. Its process callbacks cannot copy the owner.
    let home = home();
    let plan = whitelist_plan(home.path(), add("bob", BOB), None).unwrap();
    let outcome = apply_with(
        plan,
        password,
        || {
            assert_eq!(dropped.get(), 0);
            Ok(vec![])
        },
        |_, _, _, _| async { panic!("locked edit must not start") },
    )
    .await
    .unwrap();
    assert!(outcome.checked.is_ok());
    assert_eq!(dropped.get(), 1);
}

#[test]
fn t46_token_summary_discloses_registry_symbol_decimals_and_exact_base_units() {
    let home = home();
    for (chain, token, symbol, decimals) in [
        (
            "base",
            "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
            "USDC",
            6,
        ),
        (
            "ethereum",
            "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2",
            "WETH",
            18,
        ),
        ("tron", "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t", "USDT", 6),
    ] {
        let change = Change::token_cap(chain, token, "1000001").unwrap();
        let plan = whitelist_plan(home.path(), change, None).unwrap();
        for expected in [
            symbol.to_string(),
            format!("decimals {decimals}"),
            "smallest base units".into(),
            "1000001".into(),
        ] {
            assert!(
                plan.summary().contains(&expected),
                "missing {expected}: {}",
                plan.summary()
            );
        }
    }
}
