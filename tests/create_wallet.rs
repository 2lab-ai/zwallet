//! The password-bearing create entry point shared by the CLI and TUI. Only temporary homes.

mod common;

use common::{Home, PW};
use std::time::Duration;
use zeroize::Zeroizing;
use zwallet::chains;
use zwallet::core::error::ErrorCode;
use zwallet::core::store::{self, KDF_MIN_LOG_N, KdfParams, MasterOptions};
use zwallet::wallet;

fn run<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

fn password() -> Zeroizing<String> {
    Zeroizing::new(PW.into())
}

fn existing(home: &Home) {
    store::create_master(
        home.path(),
        PW,
        "main",
        MasterOptions {
            words: 12,
            kdf: KdfParams {
                log_n: KDF_MIN_LOG_N,
                ..KdfParams::default()
            },
        },
        &chains::derive_addresses,
    )
    .unwrap();
}

#[test]
fn prepare_create_validates_before_password_and_writes_nothing() {
    let home = Home::new();
    for (name, words) in [("bad name", None), ("main", Some(13))] {
        assert!(wallet::prepare_create(home.path(), name, words, None, false).is_err());
        assert!(!home.wallet_json().exists());
    }
    let plan = wallet::prepare_create(home.path(), "main", Some(12), None, false).unwrap();
    assert!(plan.creates_master());
    assert!(!home.wallet_json().exists());
    let error = run(wallet::create(plan, Zeroizing::new("short".to_string()))).unwrap_err();
    assert_eq!(error.code, ErrorCode::ValidationError);
    assert!(!home.wallet_json().exists());
}

#[test]
fn created_contains_only_public_results_and_the_backup_reminder() {
    let home = Home::new();
    let plan = wallet::prepare_create(home.path(), "main", Some(12), None, false).unwrap();
    let created = run(wallet::create(plan, password())).unwrap();
    assert!(created.master_created && created.verified);
    assert_eq!(
        (
            created.name.as_str(),
            created.index,
            created.active.as_str()
        ),
        ("main", 0, "main")
    );
    assert!(created.backup_reminder.unwrap().contains("zwallet backup"));
    assert!(created.session.is_none());
    let (opened, file) = store::unlock(home.path(), PW, &chains::derive_addresses).unwrap();
    assert_eq!(created.addresses, file.keys[0].addresses);
    let debug = format!("{created:?}");
    assert!(!debug.contains(PW));
    let words: Vec<_> = opened.seed.phrase().split_whitespace().collect();
    let shown: Vec<_> = debug
        .split(|c: char| !c.is_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    for sequence in words.windows(3) {
        assert!(
            !shown.windows(3).any(|window| window == sequence),
            "create result must never carry recovery words"
        );
    }
}

#[test]
fn subkey_create_keeps_the_master_and_verifies_the_new_active_key() {
    let home = Home::new();
    existing(&home);
    let before = store::read_wallet_file(home.path()).unwrap();
    let plan = wallet::prepare_create(home.path(), "trading", None, None, false).unwrap();
    assert!(!plan.creates_master());
    let created = run(wallet::create(plan, password())).unwrap();
    assert!(created.verified && !created.master_created);
    assert_eq!(created.index, 1);
    assert_eq!(created.active, "trading");
    assert!(created.backup_reminder.is_none());
    let after = store::read_wallet_file(home.path()).unwrap();
    assert_eq!(
        serde_json::to_value(before.crypto).unwrap(),
        serde_json::to_value(after.crypto).unwrap()
    );
    let (opened, _) = store::unlock(home.path(), PW, &chains::derive_addresses).unwrap();
    assert_eq!(
        created.addresses,
        chains::derive_addresses(&opened.seed, 1).unwrap()
    );
}

#[test]
fn wrong_password_duplicate_name_and_words_on_existing_leave_the_file_untouched() {
    let home = Home::new();
    existing(&home);
    let before = std::fs::read(home.wallet_json()).unwrap();
    let plan = wallet::prepare_create(home.path(), "other", None, None, false).unwrap();
    assert_eq!(
        run(wallet::create(
            plan,
            Zeroizing::new("wrong password".to_string())
        ))
        .unwrap_err()
        .code,
        ErrorCode::WrongPassword
    );
    assert!(wallet::prepare_create(home.path(), "main", None, None, false).is_err());
    assert!(wallet::prepare_create(home.path(), "other", Some(12), None, false).is_err());
    assert_eq!(std::fs::read(home.wallet_json()).unwrap(), before);
}

#[test]
fn prepared_create_does_not_silently_switch_to_another_master() {
    let home = Home::new();
    let other = Home::new();
    existing(&home);
    existing(&other);
    let plan = wallet::prepare_create(home.path(), "trading", None, None, false).unwrap();
    let replacement = std::fs::read(other.wallet_json()).unwrap();
    std::fs::write(home.wallet_json(), &replacement).unwrap();
    let error = run(wallet::create(plan, password())).unwrap_err();
    assert_eq!(error.code, ErrorCode::ValidationError);
    assert!(error.message.contains("master changed"));
    assert_eq!(std::fs::read(home.wallet_json()).unwrap(), replacement);
}

#[test]
fn concurrent_plans_for_one_master_get_distinct_indices() {
    // The TUI spawns this future: it must be Send while retaining ownership of the password.
    fn send<F: std::future::Future + Send>(future: F) -> F {
        future
    }
    let home = Home::new();
    existing(&home);
    let a = wallet::prepare_create(home.path(), "alpha", None, None, false).unwrap();
    let b = wallet::prepare_create(home.path(), "beta", None, None, false).unwrap();
    let (a, b) = run(async {
        tokio::join!(
            send(wallet::create(a, password())),
            send(wallet::create(b, password()))
        )
    });
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_ne!(a.index, b.index);
    assert_ne!(a.addresses, b.addresses);
    let file = store::read_wallet_file(home.path()).unwrap();
    assert_eq!(file.keys.len(), 3);
    store::unlock(home.path(), PW, &chains::derive_addresses).unwrap();
}

#[test]
fn a_first_wallet_plan_never_overwrites_a_wallet_created_while_the_human_types() {
    let home = Home::new();
    let plan = wallet::prepare_create(home.path(), "trading", Some(12), None, false).unwrap();
    existing(&home);
    let before = std::fs::read(home.wallet_json()).unwrap();
    let error = run(wallet::create(plan, password())).unwrap_err();
    assert_eq!(error.code, ErrorCode::WalletExists);
    assert_eq!(std::fs::read(home.wallet_json()).unwrap(), before);
}

#[test]
fn create_accepts_a_non_clone_password_owner_and_drops_it_before_returning() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    // Deliberately no Debug or Clone: the library must accept the TUI's owning buffer shape.
    struct Secret {
        value: Zeroizing<String>,
        dropped: Arc<AtomicBool>,
    }
    impl zeroize::ZeroizeOnDrop for Secret {}
    impl AsRef<str> for Secret {
        fn as_ref(&self) -> &str {
            &self.value
        }
    }
    impl Drop for Secret {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::SeqCst);
        }
    }
    let home = Home::new();
    existing(&home);
    for (name, value, succeeds) in [
        ("wrong", "incorrect password", false),
        ("trading", PW, true),
    ] {
        let dropped = Arc::new(AtomicBool::new(false));
        let plan = wallet::prepare_create(home.path(), name, None, None, false).unwrap();
        let result = run(wallet::create(
            plan,
            Secret {
                value: Zeroizing::new(value.to_string()),
                dropped: dropped.clone(),
            },
        ));
        assert_eq!(result.is_ok(), succeeds);
        assert!(dropped.load(Ordering::SeqCst));
    }
}

#[test]
fn creation_waiting_for_a_writer_does_not_block_the_event_loop() {
    let home = Home::new();
    existing(&home);
    let plan = wallet::prepare_create(home.path(), "trading", None, None, false).unwrap();
    let lock = store::MetaLock::acquire(home.path()).unwrap();
    let (release, receiver) = std::sync::mpsc::channel();
    let owner = std::thread::spawn(move || {
        let _ = receiver.recv_timeout(Duration::from_secs(3));
        drop(lock);
    });
    run(async {
        let started = std::time::Instant::now();
        let work = wallet::create(plan, password());
        tokio::pin!(work);
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(30)) => {},
            _ = &mut work => panic!("the creation finished while another writer held its lock"),
        }
        let elapsed = started.elapsed();
        let _ = release.send(());
        work.await.unwrap();
        assert!(
            elapsed < Duration::from_secs(2),
            "event loop blocked for {elapsed:?}"
        );
    });
    owner.join().unwrap();
}

#[test]
fn cli_create_with_unlock_still_returns_the_verified_session_and_expiry() {
    let home = Home::new();
    let value = home.create("main", &["--words", "12", "--unlock", "10m"]);
    assert_eq!(value["verified"], true);
    assert_eq!(value["verifiedBy"], "password");
    assert_eq!(value["locked"], false);
    assert_eq!(value["master"], "created");
    assert!(value["expiresAt"].is_string());
    home.stop_daemons();
    assert!(home.wait_locked(Duration::from_secs(5)));
}
