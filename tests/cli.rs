//! Offline integration tests through the built binary. No network: anything that would need
//! RPC is either rejected before it (validation, WALLET_LOCKED) or pointed at a closed port.

mod common;

use common::{Home, PW, pid_alive, run_in};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use zwallet::chains;
use zwallet::core::store;

fn mode(p: &std::path::Path) -> u32 {
    fs::metadata(p).unwrap().permissions().mode() & 0o777
}

fn words_on_disk(home: &Home) -> usize {
    // Decrypt inside the test process; the phrase itself is never printed.
    let (seed, _) = store::unlock(home.path(), PW, &chains::derive_addresses).unwrap();
    seed.word_count()
}

#[test]
fn create_then_fresh_processes_read_the_same_wallet() {
    let home = Home::new();
    let created = home.create("main", &[]);
    let address = created["address"].as_str().unwrap().to_string();
    assert!(address.starts_with("0x") && address.len() == 42);
    assert_eq!(created["name"], "main");
    assert_eq!(created["index"], 0);
    assert_eq!(created["master"], "created");
    assert_eq!(created["active"], "main");
    assert!(created["chains"].as_array().unwrap().len() >= 8);

    // Custody on disk: 0700 home, 0600 wallet.json, and no other file (no password file).
    assert_eq!(mode(home.path()), 0o700);
    assert_eq!(mode(&home.wallet_json()), 0o600);
    let names: Vec<_> = fs::read_dir(home.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, vec!["wallet.json".to_string()]);
    let on_disk = fs::read_to_string(home.wallet_json()).unwrap();
    assert!(!on_disk.contains(PW));

    // 24-word master by default (counted from the decrypted store, never printed).
    assert_eq!(words_on_disk(&home), 24);

    // A second process with only ZWALLET_HOME sees the same address, while locked.
    let again = home.run(&["address", "--json"]).ok();
    assert_eq!(again["address"], address.as_str());
    assert_eq!(again["chain"], "ethereum");
    let base = home.run(&["address", "--chain", "base", "--json"]).ok();
    assert_eq!(base["address"], address.as_str());
    let all = home.run(&["addresses", "--json"]).ok();
    let rows = all["addresses"].as_array().unwrap();
    assert!(rows.iter().all(|r| r["address"] == address.as_str()));
    assert!(rows.iter().any(|r| r["chainId"] == "polygon"));

    let status = home.run(&["status", "--json"]).ok();
    assert_eq!(status["wallet"], "configured");
    assert_eq!(status["locked"], true);
    assert_eq!(status["active"], "main");
    assert_eq!(status["address"], address.as_str());
    assert!(status.get("expiresAt").is_none());

    // Human output works too and also never contains the password.
    let human = home.run(&["list"]);
    assert_eq!(human.code, 0);
    assert!(human.stdout.contains("* main"));
    for out in [&human.stdout, &human.stderr] {
        assert!(!out.contains(PW));
    }
}

#[test]
fn word_count_option_and_empty_home() {
    let home = Home::new();
    home.create("a", &["--words", "12"]);
    assert_eq!(words_on_disk(&home), 12);

    let bad = Home::new();
    let r = bad.run_stdin(
        &["create", "a", "--words", "13", "--password-stdin", "--json"],
        &format!("{PW}\n"),
    );
    r.err("VALIDATION_ERROR");
    assert!(!bad.wallet_json().exists());

    let empty = Home::new();
    empty
        .run(&["address", "--chain", "ethereum", "--json"])
        .err("WALLET_NOT_FOUND");
    empty.run(&["list", "--json"]).err("WALLET_NOT_FOUND");
    let s = empty.run(&["status", "--json"]).ok();
    assert_eq!(s["wallet"], "not configured");
    assert_eq!(s["locked"], true);
}

#[test]
fn weak_or_missing_password_is_refused() {
    let home = Home::new();
    home.run_stdin(&["create", "a", "--password-stdin", "--json"], "short\n")
        .err("VALIDATION_ERROR");
    home.run_stdin(&["create", "a", "--password-stdin", "--json"], "")
        .err("VALIDATION_ERROR");
    // No terminal and no --password-stdin: refuses instead of hanging.
    home.run(&["create", "a", "--json"]).err("VALIDATION_ERROR");
    assert!(!home.wallet_json().exists());
}

#[test]
fn locked_matrix() {
    let home = Home::new();
    let created = home.create("main", &[]);
    let me = created["address"].as_str().unwrap();

    home.run(&["sign-message", "--message", "hi", "--json"])
        .err("WALLET_LOCKED");
    let msg = home
        .run(&[
            "transfer",
            "--chain",
            "ethereum",
            "--to",
            me,
            "--amount",
            "0",
            "--dry-run",
            "--json",
        ])
        .err("WALLET_LOCKED");
    assert!(msg.contains("zwallet unlock 24h"), "{msg}");
    home.run(&[
        "invoke",
        "--chain",
        "ethereum",
        "--to",
        me,
        "--data",
        "0x",
        "--dry-run",
        "--json",
    ])
    .err("WALLET_LOCKED");

    // Human mode: message on stderr, nothing on stdout, exit 1.
    let human = home.run(&["sign-message", "--message", "hi"]);
    assert_eq!(human.code, 1);
    assert!(human.stdout.is_empty());
    assert!(human.stderr.contains("WALLET_LOCKED"));
}

#[test]
fn backup_refuses_without_a_terminal() {
    let home = Home::new();
    home.create("main", &[]);
    let r = home.run_stdin(&["backup", "--json"], &format!("{PW}\n"));
    let msg = r.err("VALIDATION_ERROR");
    assert!(msg.contains("terminal"));
    // The whole output is the error object and nothing else.
    let v = r.json();
    assert_eq!(v.as_object().unwrap().len(), 2);
    assert!(r.stderr.is_empty());
    let (seed, _) = store::unlock(home.path(), PW, &chains::derive_addresses).unwrap();
    let first_three: Vec<&str> = seed.phrase().split(' ').take(3).collect();
    assert!(!r.stdout.contains(&first_three.join(" ")));
    let human = home.run_stdin(&["backup"], &format!("{PW}\n"));
    assert_eq!(human.code, 1);
    assert!(human.stdout.is_empty());
    assert!(!human.stderr.contains(&first_three.join(" ")));
}

#[test]
fn daemon_unlock_sign_lock_expire_wrong_password() {
    let home = Home::new();
    let address = home.create("main", &[])["address"]
        .as_str()
        .unwrap()
        .to_string();

    let before = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let unlocked = home.unlock("30s");
    assert_eq!(unlocked["locked"], false);
    assert_eq!(unlocked["duration"], "30s");
    let status = home.run(&["status", "--json"]).ok();
    assert_eq!(status["locked"], false);
    let expires = status["expiresAt"].as_str().unwrap().to_string();
    let after = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let lo = zwallet::core::time::rfc3339_utc(before + 30);
    let hi = zwallet::core::time::rfc3339_utc(after + 31);
    assert!(lo <= expires && expires <= hi, "{lo} <= {expires} <= {hi}");
    let pid = status["daemonPid"].as_i64().unwrap() as i32;
    assert!(pid_alive(pid));

    // Sign in one process, verify the signature independently here.
    let signed = home
        .run(&["sign-message", "--message", "hello zwallet", "--json"])
        .ok();
    assert_eq!(signed["address"], address.as_str());
    let sig: alloy::primitives::Signature = signed["signature"].as_str().unwrap().parse().unwrap();
    let digest: alloy::primitives::B256 = signed["digest"].as_str().unwrap().parse().unwrap();
    assert_eq!(
        digest,
        alloy::primitives::eip191_hash_message(b"hello zwallet")
    );
    assert_eq!(
        sig.recover_address_from_prehash(&digest)
            .unwrap()
            .to_checksum(None),
        address
    );
    // A separate process sees the same address.
    assert_eq!(
        home.run(&["address", "--json"]).ok()["address"],
        address.as_str()
    );

    // lock -> signing is WALLET_LOCKED and the daemon is gone.
    let locked = home.run(&["lock", "--json"]).ok();
    assert_eq!(locked["wasUnlocked"], true);
    home.run(&["sign-message", "--message", "x", "--json"])
        .err("WALLET_LOCKED");
    std::thread::sleep(Duration::from_millis(200));
    assert!(!pid_alive(pid), "daemon survived lock");

    // Expiry: unlock 1s, wait past it.
    home.unlock("1s");
    let pid = home.daemon_pid().expect("daemon running");
    std::thread::sleep(Duration::from_millis(2500));
    home.run(&["sign-message", "--message", "x", "--json"])
        .err("WALLET_LOCKED");
    assert!(home.wait_locked(Duration::from_secs(5)));
    assert!(!pid_alive(pid), "daemon survived expiry");

    // Wrong password: WRONG_PASSWORD, and no daemon left behind.
    home.run_stdin(
        &["unlock", "1h", "--password-stdin", "--json"],
        "not the password\n",
    )
    .err("WRONG_PASSWORD");
    assert!(home.wait_locked(Duration::from_secs(5)));
    assert_eq!(home.run(&["status", "--json"]).ok()["locked"], true);
}

#[test]
fn named_keys_create_list_change_sign() {
    let home = Home::new();
    let a = home.create("a", &["--unlock", "30s"]);
    assert_eq!(a["locked"], false);
    let a_addr = a["address"].as_str().unwrap().to_string();

    // While unlocked the agent can add keys without the password (stdin is empty).
    let b = home.run(&["create", "b", "--json"]).ok();
    assert_eq!(b["index"], 1);
    assert_eq!(b["master"], "existing");
    let b_addr = b["address"].as_str().unwrap().to_string();
    assert_ne!(a_addr, b_addr);
    let (seed, _) = store::unlock(home.path(), PW, &chains::derive_addresses).unwrap();
    assert_eq!(
        chains::derive_addresses(&seed, 1).unwrap()["ethereum"],
        b_addr
    );

    let list = home.run(&["list", "--json"]).ok();
    assert_eq!(list["active"], "b");
    let keys = list["keys"].as_array().unwrap();
    assert_eq!(keys.len(), 2);
    assert_eq!(keys[1]["name"], "b");
    assert_eq!(keys[1]["index"], 1);
    assert_eq!(keys[1]["active"], true);
    assert_eq!(keys[0]["active"], false);

    let changed = home.run(&["change", "a", "--json"]).ok();
    assert_eq!(changed["active"], "a");
    assert_eq!(changed["address"], a_addr.as_str());
    assert_eq!(
        home.run(&["address", "--json"]).ok()["address"],
        a_addr.as_str()
    );

    let signed = home
        .run(&[
            "sign-message",
            "--key",
            "b",
            "--message",
            "from b",
            "--json",
        ])
        .ok();
    let sig: alloy::primitives::Signature = signed["signature"].as_str().unwrap().parse().unwrap();
    assert_eq!(
        sig.recover_address_from_msg(b"from b")
            .unwrap()
            .to_checksum(None),
        b_addr
    );

    home.run(&["create", "a", "--json"]).err("KEY_EXISTS");
    home.run(&["create", "Bad Name", "--json"])
        .err("VALIDATION_ERROR");
    home.run(&["change", "nope", "--json"]).err("KEY_NOT_FOUND");
    home.run(&["address", "--key", "nope", "--json"])
        .err("KEY_NOT_FOUND");

    // Locked: creating a key takes the password for that one operation and starts no daemon.
    home.run(&["lock", "--json"]).ok();
    home.run(&["create", "c", "--json"]).err("VALIDATION_ERROR"); // no terminal, no stdin flag
    home.run_stdin(
        &["create", "c", "--password-stdin", "--json"],
        "wrong password\n",
    )
    .err("WRONG_PASSWORD");
    let c = home.create("c", &[]);
    assert_eq!(c["index"], 2);
    assert_eq!(c["active"], "c");
    assert!(
        home.daemon_pid().is_none(),
        "locked create started a daemon"
    );
    assert!(!home.path().join("d.sock").exists());
    store::unlock(home.path(), PW, &chains::derive_addresses).unwrap();
}

#[test]
fn json_error_codes_offline() {
    let home = Home::new();
    let me = home.create("main", &[])["address"]
        .as_str()
        .unwrap()
        .to_string();
    home.run(&["address", "--chain", "solana", "--json"])
        .err("CHAIN_UNSUPPORTED");
    let ens = home
        .run(&[
            "transfer",
            "--chain",
            "ethereum",
            "--to",
            "vitalik.eth",
            "--amount",
            "1",
            "--json",
        ])
        .err("INVALID_ADDRESS");
    assert!(ens.contains("ENS"));
    home.run(&[
        "transfer",
        "--chain",
        "ethereum",
        "--to",
        &me,
        "--amount",
        "0.0000000000000000001",
        "--json",
    ])
    .err("VALIDATION_ERROR");
    for bad in ["0s", "10", "1.5h", "-1h", "3x"] {
        home.run(&["unlock", bad, "--password-stdin", "--json"])
            .err("VALIDATION_ERROR");
    }
    home.run(&[
        "call", "--chain", "ethereum", "--to", &me, "--sig", "broken(", "--json",
    ])
    .err("VALIDATION_ERROR");
    home.run(&[
        "call", "--chain", "ethereum", "--to", &me, "--sig", "f(uint8)", "--json",
    ])
    .err("VALIDATION_ERROR"); // arity
    // clap-level errors are JSON too with --json.
    home.run(&["transfer", "--json"]).err("VALIDATION_ERROR");
    home.run(&["no-such-command", "--json"])
        .err("VALIDATION_ERROR");
    // An unreachable RPC is RPC_ERROR (closed local port; no network needed).
    home.run(&[
        "balance",
        "--chain",
        "ethereum",
        "--rpc",
        "http://127.0.0.1:1",
        "--json",
    ])
    .err("RPC_ERROR");
}

#[test]
fn rpc_env_override_shows_in_chains() {
    let home = Home::new();
    let v = home
        .run_env(
            &["chains", "--json"],
            &[("ZWALLET_RPC_BASE", "http://127.0.0.1:8545")],
        )
        .ok();
    let rows = v.as_array().unwrap();
    let base = rows.iter().find(|r| r["key"] == "base").unwrap();
    assert_eq!(base["rpc"], "http://127.0.0.1:8545");
    assert_eq!(base["chainId"], 8453);
    let eth = rows.iter().find(|r| r["key"] == "ethereum").unwrap();
    assert_eq!(eth["rpc"], "https://ethereum-rpc.publicnode.com");
    assert_eq!(eth["family"], "evm");
    assert_eq!(rows.len(), 8);
    assert!(rows.iter().all(|r| r["symbol"].is_string()));
}

#[test]
fn socket_path_over_sun_path_limit_is_validation_error() {
    let tmp = tempfile::Builder::new()
        .prefix("zwt-")
        .tempdir_in("/tmp")
        .unwrap();
    let long = tmp.path().join("x".repeat(110));
    let r = run_in(
        &long,
        &["unlock", "1h", "--password-stdin", "--json"],
        Some(PW),
        &[],
    );
    let msg = r.err("VALIDATION_ERROR");
    assert!(msg.contains("sun_path"), "{msg}");
}
