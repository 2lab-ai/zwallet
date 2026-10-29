//! The shared helper that seeds a signed whitelist (`common::seed_whitelist`, `Home::allow`,
//! `Home::allow_contract`), checked against wallets the CLI created: enforcement and CLI tests
//! build on it.

mod common;

use common::{Home, PW};
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;
use zwallet::chains;
use zwallet::core::store;
use zwallet::core::whitelist::{self, BaseSource, Entry};

/// The EIP-55 example address.
const PAYEE: &str = "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed";
const ROUTER: &str = "0x2222222222222222222222222222222222222222";
/// USDC on Ethereum: a registry token.
const USDC: &str = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48";

fn sealed_anchor(home: &Home) -> store::WhitelistAnchor {
    let (unsealed, _) = store::unlock(home.path(), PW, &chains::derive_addresses).unwrap();
    unsealed
        .whitelist_anchor()
        .unwrap()
        .expect("a sealed whitelist")
}

#[test]
fn seeded_lists_verify_against_the_cli_wallet_seal() {
    let home = Home::new();
    home.create("main", &[]);
    let list = home.allow("alice", PAYEE, &["base"]);
    assert_eq!(list.generation, 1);
    let alice = list.by_nickname("alice").unwrap();
    assert_eq!(alice.address, PAYEE.to_ascii_lowercase());
    assert!(list.payee(PAYEE, "base", true).is_some());
    assert!(list.payee(PAYEE, "ethereum", true).is_none());
    let caps = whitelist::Policy::defaults(&chains::evm::registry::WHITELIST_RULES);
    assert_eq!(
        list.policy, caps,
        "the default fee caps, message signing off"
    );
    let anchor = sealed_anchor(&home);
    assert_eq!(anchor.generation, 1);
    assert_eq!(
        whitelist::load_verified(home.path(), &anchor).unwrap(),
        list
    );
    for f in [whitelist::WHITELIST_FILE, "wallet.json"] {
        let mode = std::fs::metadata(home.path().join(f))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "{f}");
    }
    assert!(!home.path().join(whitelist::NEXT_FILE).exists());
    // The key is derived from the password at every edit: no key file.
    assert!(!home.path().join("whitelist.key").exists());

    let list = home.allow_contract("router", ROUTER, &["base"], &["0x12345678"]);
    assert_eq!(list.generation, 2);
    assert!(
        list.contract(ROUTER, "base")
            .unwrap()
            .allows_selector("0x12345678")
    );
    let list = home.edit_whitelist(|l| l.policy.allow_message_signing = true);
    assert_eq!(list.generation, 3);
    assert!(list.policy.allow_message_signing);
    assert_eq!(
        whitelist::load_verified(home.path(), &sealed_anchor(&home)).unwrap(),
        list
    );
    let base = whitelist::pick_base(
        home.path(),
        Some(&sealed_anchor(&home)),
        &chains::evm::registry::WHITELIST_RULES,
    );
    assert_eq!(base.source, BaseSource::Current);

    // The metadata commands still read the re-sealed wallet.
    let listed = home.run(&["list", "--json"]).ok();
    assert_eq!(listed["keys"][0]["name"], "main", "{listed}");
}

#[test]
fn seeding_after_unlock_ends_the_session_and_unlock_works_again() {
    let home = Home::new();
    home.create("main", &[]);
    home.allow("alice", PAYEE, &["*"]);
    home.unlock("10m");
    assert_eq!(home.run(&["status", "--json"]).ok()["locked"], false);
    // The re-seal changes wallet.json's ciphertext; the daemon ends its session on its next
    // tick.
    home.allow("router", ROUTER, &["base"]);
    assert!(
        home.wait_locked(Duration::from_secs(15)),
        "the session outlived the re-seal"
    );
    home.unlock("10m");
    assert_eq!(home.run(&["status", "--json"]).ok()["locked"], false);
}

#[test]
fn a_refused_seed_leaves_the_sealed_list_as_it_was() {
    let home = Home::new();
    home.create("main", &[]);
    let list = home.allow("alice", PAYEE, &["base"]);
    let anchor = sealed_anchor(&home);
    let files = || {
        ["wallet.json", whitelist::WHITELIST_FILE]
            .map(|f| std::fs::read(home.path().join(f)).unwrap())
    };
    let before = files();
    // A registry token as a contract entry is refused before anything is written.
    let err = common::try_seed_whitelist(home.path(), |l| {
        l.add(Entry::new_contract(
            "usdc",
            USDC,
            &["ethereum"],
            &["0xa9059cbb"],
        ))
    })
    .unwrap_err();
    assert!(err.message.contains("registry token"), "{}", err.message);
    assert_eq!(files(), before, "wallet.json and whitelist.json unchanged");
    assert!(!home.path().join(whitelist::NEXT_FILE).exists());
    let after = sealed_anchor(&home);
    assert_eq!(
        (after.generation, after.digest),
        (anchor.generation, anchor.digest)
    );
    assert_eq!(whitelist::load_verified(home.path(), &after).unwrap(), list);
}
