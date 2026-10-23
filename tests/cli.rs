//! Offline integration tests through the built binary. No network: anything that would need
//! RPC is either rejected before it (validation, WALLET_LOCKED) or pointed at a closed port.

mod common;

use common::{DaemonChild, FakeDaemon, Home, PW, pid_alive, run_in, wait_dead};
use serde_json::json;
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
    let seed = store::unlock(home.path(), PW, &chains::derive_addresses)
        .unwrap()
        .0
        .seed;
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

    // Custody on disk: 0700 home, 0600 wallet.json, and no other file (no password file) but
    // the empty 0600 writer lock.
    assert_eq!(mode(home.path()), 0o700);
    assert_eq!(mode(&home.wallet_json()), 0o600);
    let mut names: Vec<_> = fs::read_dir(home.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, vec!["w.lock".to_string(), "wallet.json".to_string()]);
    let lock = home.path().join("w.lock");
    assert_eq!(mode(&lock), 0o600);
    assert_eq!(fs::metadata(&lock).unwrap().len(), 0);
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
    let seed = store::unlock(home.path(), PW, &chains::derive_addresses)
        .unwrap()
        .0
        .seed;
    let first_three: Vec<&str> = seed.phrase().split(' ').take(3).collect();
    assert!(!r.stdout.contains(&first_three.join(" ")));
    let human = home.run_stdin(&["backup"], &format!("{PW}\n"));
    assert_eq!(human.code, 1);
    assert!(human.stdout.is_empty());
    assert!(!human.stderr.contains(&first_three.join(" ")));
}

#[test]
fn daemon_unlock_sign_lock_expire_wrong_password() {
    let _alone = common::exclusive();
    let home = Home::new();
    let address = home.create("main", &[])["address"]
        .as_str()
        .unwrap()
        .to_string();
    // `sign-message` signs only under a signed whitelist that allows it (`.prd/04-whitelist.md`).
    home.edit_whitelist(|l| l.policy.allow_message_signing = true);

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
    assert_eq!(locked["stopped"], json!([pid]), "{locked}");
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
    // `sign-message` signs only under a signed whitelist that allows it; the edit re-seals
    // wallet.json, which ends the session, so unlock again.
    home.edit_whitelist(|l| l.policy.allow_message_signing = true);
    assert!(home.wait_locked(Duration::from_secs(15)));
    home.unlock("30s");

    // While unlocked the agent can add keys without the password (stdin is empty).
    let b = home.run(&["create", "b", "--json"]).ok();
    assert_eq!(b["index"], 1);
    assert_eq!(b["master"], "existing");
    let b_addr = b["address"].as_str().unwrap().to_string();
    assert_ne!(a_addr, b_addr);
    let seed = store::unlock(home.path(), PW, &chains::derive_addresses)
        .unwrap()
        .0
        .seed;
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
    home.stop_daemons();
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
    // clap-level errors are JSON too with --json, with clap's whole message (not its usage and
    // help hint): a missing-argument error names every missing argument.
    let msg = home.run(&["transfer", "--json"]).err("VALIDATION_ERROR");
    for arg in ["--chain <CHAIN>", "--to <TO>", "--amount <AMOUNT>"] {
        assert!(msg.contains(arg), "{arg}: {msg}");
    }
    assert!(!msg.contains("Usage:") && !msg.contains("--help"), "{msg}");
    let msg = home
        .run(&["unlock", "--password-stdin", "--json"])
        .err("VALIDATION_ERROR");
    assert!(msg.contains("<DURATION>"), "{msg}");
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

// ---- Custody and daemon hardening (review round 1: C1-C14) ----

/// A valid address that is not this wallet's (index 1 of the well-known test mnemonic).
const POISON: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";

fn poison_cache(home: &Home) {
    home.edit_wallet(|v| {
        for (_, a) in v["keys"][0]["addresses"].as_object_mut().unwrap() {
            *a = serde_json::json!(POISON);
        }
    });
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[test]
fn c1_fake_daemon_on_the_socket_never_receives_the_password() {
    let home = Home::new();
    let real = home.create("main", &[])["address"]
        .as_str()
        .unwrap()
        .to_string();
    // `sign-message` signs only under a signed whitelist that allows it (`.prd/04-whitelist.md`).
    home.edit_whitelist(|l| l.policy.allow_message_signing = true);
    let fake = FakeDaemon::bind(&home.sock());
    let r = home.run_stdin(
        &["unlock", "30s", "--password-stdin", "--json"],
        &format!("{PW}\n"),
    );
    let got = String::from_utf8_lossy(&fake.bytes()).into_owned();
    assert!(!got.contains(PW), "the fake daemon received the password");
    // Everything it received is a request with no payload at all.
    for msg in fake.received.lock().unwrap().iter() {
        let v: serde_json::Value = serde_json::from_slice(msg).unwrap();
        assert!(v["op"] == "lock" || v["op"] == "status", "{v}");
        assert_eq!(v.as_object().unwrap().len(), 1, "{v}");
    }
    let out = r.ok();
    assert_eq!(out["address"], real.as_str());
    // The real daemon took the socket over and serves.
    let status = home.run(&["status", "--json"]).ok();
    assert_eq!(status["locked"], false, "{status}");
    let pid = status["daemonPid"].as_i64().unwrap() as i32;
    assert_eq!(home.daemon_pids(), vec![pid]);
    let signed = home.run(&["sign-message", "--message", "m", "--json"]).ok();
    assert_eq!(signed["address"], real.as_str());
}

#[test]
fn c3_concurrent_unlocks_leave_exactly_one_daemon() {
    let home = Home::new();
    let real = home.create("main", &[])["address"]
        .as_str()
        .unwrap()
        .to_string();
    // `sign-message` signs only under a signed whitelist that allows it (`.prd/04-whitelist.md`).
    home.edit_whitelist(|l| l.policy.allow_message_signing = true);
    let runs: Vec<common::Run> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..3)
            .map(|_| {
                s.spawn(|| {
                    home.run_stdin(
                        &["unlock", "30s", "--password-stdin", "--json"],
                        &format!("{PW}\n"),
                    )
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert!(
        runs.iter().any(|r| r.code == 0),
        "no unlock succeeded: {:?}",
        runs.iter().map(|r| &r.stdout).collect::<Vec<_>>()
    );
    let outputs: Vec<_> = runs.iter().map(|r| (&r.stdout, &r.stderr)).collect();
    let status = home.run(&["status", "--json"]).ok();
    assert_eq!(status["locked"], false, "{status}; runs {outputs:?}");
    let pid = status["daemonPid"].as_i64().unwrap() as i32;
    // Losers exit once their CLI gives up; then the serving daemon is the only process bound to
    // the socket or holding the lock.
    let start = std::time::Instant::now();
    let mut pids = home.daemon_pids();
    while pids != vec![pid] && start.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(100));
        pids = home.daemon_pids();
    }
    assert_eq!(pids, vec![pid], "daemons for one home; runs {outputs:?}");
    let signed = home.run(&["sign-message", "--message", "m", "--json"]).ok();
    assert_eq!(signed["address"], real.as_str());
}

#[test]
fn c4_silent_connection_does_not_keep_the_seed_past_expiry() {
    let home = Home::new();
    home.create("main", &[]);
    home.unlock("1s");
    let pid = home.daemon_pid().expect("daemon running");
    // Connect and never send a byte.
    let silent = std::os::unix::net::UnixStream::connect(home.sock()).ok();
    assert!(
        wait_dead(pid, Duration::from_millis(2500)),
        "the daemon outlived its 1s expiry while a client stayed silent"
    );
    drop(silent);
}

#[test]
fn c5_replaced_ciphertext_is_noticed_by_a_bare_status() {
    let home = Home::new();
    home.create("main", &[]);
    home.unlock("30s");
    let pid = home.daemon_pid().expect("daemon running");
    home.edit_wallet(|v| {
        let ct = v["crypto"]["ciphertext"].as_str().unwrap().to_string();
        let first = if ct.starts_with('0') { "1" } else { "0" };
        v["crypto"]["ciphertext"] = serde_json::json!(format!("{first}{}", &ct[1..]));
    });
    let status = home.run(&["status", "--json"]).ok();
    assert_eq!(status["locked"], true, "{status}");
    assert!(
        wait_dead(pid, Duration::from_secs(3)),
        "daemon kept running"
    );
}

#[test]
fn c5_removed_socket_stops_the_daemon_within_a_tick() {
    let home = Home::new();
    home.create("main", &[]);
    home.unlock("30s");
    let pid = home.daemon_pid().expect("daemon running");
    fs::remove_file(home.sock()).unwrap();
    assert!(
        wait_dead(pid, Duration::from_millis(2500)),
        "daemon kept the seed after its socket was removed"
    );
}

#[test]
fn c6_squatter_on_the_daemon_lock_fails_unlock_and_leaves_no_daemon() {
    let home = Home::new();
    home.create("main", &[]);
    let squat = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(home.path().join("d.lock"))
        .unwrap();
    squat.try_lock().unwrap();
    let r = home.run_stdin(
        &["unlock", "30s", "--password-stdin", "--json"],
        &format!("{PW}\n"),
    );
    let msg = r.err("VALIDATION_ERROR");
    assert!(msg.contains("d.lock"), "{msg}");
    assert!(home.daemon_pids().is_empty(), "{:?}", home.daemon_pids());
    assert_eq!(home.run(&["status", "--json"]).ok()["locked"], true);
    drop(squat);
    assert_eq!(home.unlock("30s")["locked"], false);
}

#[test]
fn c7_poisoned_cache_while_unlocked_is_a_validation_error() {
    let home = Home::new();
    home.create("main", &["--unlock", "30s"]);
    poison_cache(&home);
    for args in [
        &["address", "--json"][..],
        &["addresses", "--json"],
        &["list", "--json"],
        &["status", "--json"],
        &["change", "main", "--json"],
    ] {
        let msg = home.run(args).err("VALIDATION_ERROR");
        assert!(msg.contains("does not match"), "{args:?}: {msg}");
    }
}

#[test]
fn c7_poisoned_cache_while_locked_is_reported_unverified() {
    let home = Home::new();
    home.create("main", &[]);
    poison_cache(&home);
    for args in [
        &["address", "--json"][..],
        &["addresses", "--json"],
        &["list", "--json"],
        &["status", "--json"],
        &["change", "main", "--json"],
    ] {
        let v = home.run(args).ok();
        assert_eq!(v["verified"], false, "{args:?}: {v}");
    }
    let human = home.run(&["address"]);
    assert_eq!(human.code, 0);
    assert!(human.stdout.contains("not verified"), "{}", human.stdout);
}

#[test]
fn c7_cache_swapped_during_the_unlock_prompt_does_not_reach_the_output() {
    let home = Home::new();
    let real = home.create("main", &[])["address"]
        .as_str()
        .unwrap()
        .to_string();
    let original = fs::read(home.wallet_json()).unwrap();
    poison_cache(&home);
    let mut pty = common::PtyChild::spawn(home.command(&["unlock", "30s", "--json"]));
    // The CLI has read wallet.json and now waits for the password at the prompt; restore the
    // real cache so the daemon accepts the file.
    pty.wait_for("Wallet password: ", Duration::from_secs(10));
    fs::write(home.wallet_json(), &original).unwrap();
    pty.type_line(PW);
    let out = pty.finish();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["address"], real.as_str(), "{v}");
    assert_eq!(v["verified"], true, "{v}");
}

#[test]
fn c9_new_password_needs_12_characters() {
    let home = Home::new();
    home.run_stdin(
        &["create", "a", "--password-stdin", "--json"],
        "elevenchars\n",
    )
    .err("VALIDATION_ERROR");
    assert!(!home.wallet_json().exists());
    home.run_stdin(
        &["create", "a", "--password-stdin", "--json"],
        "twelve chars\n",
    )
    .ok();
}

#[test]
fn c11_concurrent_subkey_creates_get_distinct_indices() {
    let home = Home::new();
    home.create("main", &["--unlock", "60s"]);
    let names = ["k1", "k2", "k3", "k4"];
    let runs: Vec<common::Run> = std::thread::scope(|s| {
        let handles: Vec<_> = names
            .iter()
            .map(|n| {
                let home = &home;
                s.spawn(move || home.run(&["create", n, "--json"]))
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut indices: Vec<i64> = runs
        .iter()
        .map(|r| r.ok()["index"].as_i64().unwrap())
        .collect();
    indices.sort_unstable();
    assert_eq!(indices, vec![1, 2, 3, 4]);
    let list = home.run(&["list", "--json"]).ok();
    assert_eq!(list["keys"].as_array().unwrap().len(), 5);
    store::unlock(home.path(), PW, &chains::derive_addresses).unwrap();
}

#[test]
fn c13_first_create_on_a_terminal_nudges_backup() {
    let home = Home::new();
    let (master, slave) = common::openpty();
    // Keep a slave open here: on macOS the pty drops unread output once every slave is closed.
    let _keep = slave.try_clone().unwrap();
    let mut cmd = home.command(&["create", "main", "--password-stdin", "--json"]);
    cmd.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::from(slave));
    let mut child = cmd.spawn().unwrap();
    drop(cmd);
    use std::io::Write as _;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{PW}\n").as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.contains("backup"), "{stdout}");
    let tty = common::drain_pty(&master);
    assert!(tty.contains("zwallet backup"), "{tty:?}");
    // Piped stderr (an agent): no nudge.
    let other = Home::new();
    let r = other.run_stdin(
        &["create", "main", "--password-stdin", "--json"],
        &format!("{PW}\n"),
    );
    r.ok();
    assert!(!r.stderr.contains("backup"), "{}", r.stderr);
}

#[test]
fn c14_backup_refuses_a_terminal_on_only_one_side() {
    use std::process::Stdio;
    let home = Home::new();
    home.create("main", &[]);
    let seed = store::unlock(home.path(), PW, &chains::derive_addresses)
        .unwrap()
        .0
        .seed;
    let first_three: Vec<&str> = seed.phrase().split(' ').take(3).collect();
    let first_three = first_three.join(" ");

    // stdin on a terminal, stdout piped.
    let (_master, slave) = common::openpty();
    let out = home
        .command(&["backup"])
        .stdin(Stdio::from(slave))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("terminal") && !err.contains(&first_three),
        "{err}"
    );

    // stdout on a terminal, stdin piped.
    let (master, slave) = common::openpty();
    let _keep = slave.try_clone().unwrap();
    let out = home
        .command(&["backup"])
        .stdin(Stdio::piped())
        .stdout(Stdio::from(slave))
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("terminal") && !err.contains(&first_three),
        "{err}"
    );
    let tty = common::drain_pty(&master);
    assert!(!tty.contains(&first_three), "{tty}");
}

#[test]
fn c14_unlock_durations_set_expires_at() {
    let home = Home::new();
    home.create("main", &[]);
    for (duration, secs) in [
        ("24h", 86_400),
        ("365d", 365 * 86_400),
        ("1y", 365 * 86_400),
    ] {
        let before = now_secs();
        let v = home.unlock(duration);
        let after = now_secs();
        let expires = v["expiresAt"].as_str().unwrap().to_string();
        let lo = zwallet::core::time::rfc3339_utc(before + secs);
        let hi = zwallet::core::time::rfc3339_utc(after + secs + 1);
        assert!(
            lo <= expires && expires <= hi,
            "{duration}: {lo} <= {expires} <= {hi}"
        );
        home.stop_daemons();
    }
}

#[test]
fn c14_stale_socket_without_a_daemon_is_wallet_locked() {
    // Since C1 a daemon exists only while unlocked, so "running but not unlocked" became "killed
    // and left its socket behind".
    let home = Home::new();
    home.create("main", &[]);
    home.unlock("30s");
    let pid = home.daemon_pid().expect("daemon running");
    // SAFETY: kill(2) on the daemon this test started.
    unsafe {
        libc::kill(pid, libc::SIGKILL);
    }
    assert!(wait_dead(pid, Duration::from_secs(3)));
    assert!(home.sock().exists(), "SIGKILL leaves the socket file");
    home.run(&["sign-message", "--message", "x", "--json"])
        .err("WALLET_LOCKED");
    assert_eq!(home.run(&["status", "--json"]).ok()["locked"], true);
    assert_eq!(home.unlock("30s")["locked"], false);
}

// ---- Review round 2 (K1-K12) ----

/// Freezes a daemon (SIGSTOP), the stand-in for one that won the race between binding its socket
/// and recording that socket's identity: it cannot notice anything done to its files meanwhile.
fn freeze(pid: i32) {
    // SAFETY: kill(2) on a daemon this test started.
    assert_eq!(unsafe { libc::kill(pid, libc::SIGSTOP) }, 0);
}

fn only_daemon(home: &Home) -> i32 {
    let pids = home.daemon_pids();
    assert_eq!(pids.len(), 1, "{pids:?}");
    pids[0]
}

#[test]
fn k2_lock_stops_a_daemon_whose_socket_was_swapped_for_a_fake() {
    let _alone = common::exclusive();
    let home = Home::new();
    home.create("main", &[]);
    home.unlock("30s");
    let pid = only_daemon(&home);
    freeze(pid);
    // The real socket moves aside (still served by the daemon) and a fake that answers "locked"
    // takes d.sock.
    fs::rename(home.sock(), home.path().join("moved.sock")).unwrap();
    let fake = FakeDaemon::bind(&home.sock());
    home.run(&["lock", "--json"]).ok();
    assert!(
        home.daemon_pids().is_empty(),
        "a zwallet daemon for this home survived lock: {:?}",
        home.daemon_pids()
    );
    assert!(wait_dead(pid, Duration::from_secs(3)));
    drop(fake);
}

#[test]
fn k2_replaced_daemon_lock_still_leaves_exactly_one_daemon_after_unlock() {
    let home = Home::new();
    home.create("main", &[]);
    home.unlock("30s");
    let old = only_daemon(&home);
    // The old daemon cannot answer the polite lock, and d.lock is now a different file, so its
    // flock no longer keeps a second daemon out.
    freeze(old);
    let lock = home.path().join("d.lock");
    fs::remove_file(&lock).unwrap();
    fs::write(&lock, b"").unwrap();
    let new = home.unlock("30s")["daemonPid"].as_i64().unwrap() as i32;
    assert_ne!(new, old);
    assert_eq!(
        home.daemon_pids(),
        vec![new],
        "exactly one daemon after unlock"
    );
    assert!(wait_dead(old, Duration::from_secs(3)));
}

#[test]
fn k6_wrong_password_on_re_unlock_leaves_the_session_running() {
    let home = Home::new();
    let real = home.create("main", &[])["address"]
        .as_str()
        .unwrap()
        .to_string();
    // `sign-message` signs only under a signed whitelist that allows it (`.prd/04-whitelist.md`).
    home.edit_whitelist(|l| l.policy.allow_message_signing = true);
    home.unlock("30s");
    let pid = only_daemon(&home);
    home.run_stdin(
        &["unlock", "30s", "--password-stdin", "--json"],
        "not the password\n",
    )
    .err("WRONG_PASSWORD");
    let status = home.run(&["status", "--json"]).ok();
    assert_eq!(status["locked"], false, "{status}");
    assert_eq!(status["daemonPid"], pid, "{status}");
    assert_eq!(home.daemon_pids(), vec![pid]);
    let signed = home.run(&["sign-message", "--message", "m", "--json"]).ok();
    assert_eq!(signed["address"], real.as_str());
}

#[test]
fn k12_daemon_inherits_no_stray_descriptor() {
    use std::io::Read as _;
    use std::os::fd::AsRawFd;
    use std::os::unix::process::CommandExt;
    let home = Home::new();
    home.create("main", &[]);
    // A pipe whose write end the unlocking CLI inherits as fd 5, not close-on-exec there: a
    // descriptor a parent leaked into zwallet.
    let (mut reader, writer) = std::io::pipe().unwrap();
    let w = writer.as_raw_fd();
    let mut cmd = home.command(&["unlock", "30s", "--password-stdin", "--json"]);
    cmd.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    // SAFETY: dup2/fcntl are async-signal-safe.
    unsafe {
        cmd.pre_exec(move || {
            let rc = if w == 5 {
                libc::fcntl(5, libc::F_SETFD, 0)
            } else {
                libc::dup2(w, 5)
            };
            if rc == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = cmd.spawn().unwrap();
    drop(cmd);
    drop(writer);
    use std::io::Write as _;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{PW}\n").as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(home.daemon_pids().len(), 1);
    // Every holder of the write end is gone (this test, the CLI) unless the daemon kept it.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut sink = Vec::new();
        let _ = tx.send(reader.read_to_end(&mut sink).map(|_| ()));
    });
    let eof = rx.recv_timeout(Duration::from_secs(3));
    assert!(
        eof.is_ok(),
        "the key daemon still holds a descriptor it inherited from the unlocking CLI"
    );
}

#[test]
fn k5_binary_swapped_during_the_prompt_never_gets_the_password() {
    let home = Home::new();
    home.create("main", &[]);
    let exe = home.path().join("zwallet");
    fs::copy(env!("CARGO_BIN_EXE_zwallet"), &exe).unwrap();
    let captured = home.path().join("captured");
    let mut cmd = std::process::Command::new(&exe);
    cmd.env_clear()
        .env("ZWALLET_HOME", home.path())
        .args(["unlock", "30s", "--json"]);
    if common::ptrace_scope_is_open() {
        cmd.arg("--allow-ptrace-scope-0");
    }
    let mut pty = common::PtyChild::spawn(cmd);
    pty.wait_for("Wallet password: ", Duration::from_secs(10));
    // While the human types, the binary at that path is replaced by one that plays the daemon's
    // first step (asks for its input, as the real one does once core dumps are off) and keeps
    // whatever it is given: an unlock that started it would hand it the password.
    let fake = home.path().join("fake");
    fs::write(
        &fake,
        format!(
            "#!/bin/sh\nprintf '%s\\n' '{{\"v\":2,\"ok\":true,\"result\":{{\"stage\":\"input\"}}}}'\n\
             exec /bin/cat > '{}'\n",
            captured.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    fs::rename(&fake, &exe).unwrap();
    pty.type_line(PW);
    let out = pty.finish();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let got = fs::read(&captured).unwrap_or_default();
    assert!(
        !String::from_utf8_lossy(&got).contains(PW),
        "the replaced binary received the password; unlock said {stdout}"
    );
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    if cfg!(target_os = "linux") {
        // /proc/self/exe: the image this CLI runs, whatever the path now holds.
        assert_eq!(v["locked"], false, "{v}");
    } else {
        assert_eq!(v["errorCode"], "VALIDATION_ERROR", "{v}");
        assert!(v["error"].as_str().unwrap().contains("binary"), "{v}");
        assert!(home.daemon_pids().is_empty());
    }
}

#[test]
fn k4_daemon_asks_for_the_password_before_anything_is_written() {
    let home = Home::new();
    home.create("main", &[]);
    // Started the way `unlock` starts it; nothing is written to its channel yet.
    let mut daemon = DaemonChild::spawn(home.path());
    let v = daemon.next();
    assert_eq!(v["result"]["stage"], "input", "{v}");
}

const NOT_VERIFIED: &str = "not verified: run `zwallet address --verify` before funding";

/// The commands that show an address without the human's password.
const ADDRESS_VIEWS: [&[&str]; 4] = [
    &["address", "--json"],
    &["addresses", "--json"],
    &["list", "--json"],
    &["status", "--json"],
];

#[test]
fn k1_swapped_wallet_with_a_genuine_attacker_daemon_is_never_verified() {
    const ATTACKER_PW: &str = "attacker chosen password";
    let home = Home::new();
    let human = home.create("main", &[])["address"]
        .as_str()
        .unwrap()
        .to_string();
    let attacker = Home::new();
    let evil = attacker
        .run_stdin(
            &["create", "main", "--password-stdin", "--json"],
            &format!("{ATTACKER_PW}\n"),
        )
        .ok()["address"]
        .as_str()
        .unwrap()
        .to_string();
    // The attacker's wallet.json replaces the human's, and a genuine zwallet daemon serves it,
    // unlocked with the attacker's own password.
    fs::copy(attacker.wallet_json(), home.wallet_json()).unwrap();
    home.run_stdin(
        &["unlock", "30s", "--password-stdin", "--json"],
        &format!("{ATTACKER_PW}\n"),
    )
    .ok();
    for args in ADDRESS_VIEWS
        .iter()
        .copied()
        .chain([&["change", "main", "--json"][..]])
    {
        let v = home.run(args).ok();
        assert_eq!(v["verified"], false, "{args:?}: {v}");
    }
    assert_eq!(home.run(&["address", "--json"]).ok()["address"], evil);
    let human_out = home.run(&["address"]);
    assert!(
        human_out.stdout.contains(NOT_VERIFIED),
        "{}",
        human_out.stdout
    );
    // The human's password does not open the attacker's wallet: --verify fails.
    home.run_stdin(
        &["address", "--verify", "--password-stdin", "--json"],
        &format!("{PW}\n"),
    )
    .err("WRONG_PASSWORD");
    assert_ne!(human, evil);
}

#[test]
fn k1_squatter_answering_the_cached_address_is_never_verified() {
    let home = Home::new();
    let addr = home.create("main", &[])["address"]
        .as_str()
        .unwrap()
        .to_string();
    // It read the cache and repeats it, so the daemon cross-check finds no mismatch.
    let fake = FakeDaemon::answering(
        &home.sock(),
        serde_json::json!({
            "locked": false, "pid": 1, "expiresAt": "2099-01-01T00:00:00Z",
            "chain": "ethereum", "index": 0, "active": "main", "address": addr,
        }),
    );
    for args in ADDRESS_VIEWS.iter().copied().chain([
        &["change", "main", "--json"][..],
        &["create", "b", "--json"],
    ]) {
        let v = home.run(args).ok();
        assert_ne!(v["verified"], true, "{args:?}: {v}");
    }
    drop(fake);
}

#[test]
fn k1_verify_derives_from_the_password_and_checks_the_cache() {
    let home = Home::new();
    let created = home.create("main", &[]);
    assert_eq!(created["verified"], true, "first create derives in-process");
    let addr = created["address"].as_str().unwrap().to_string();
    let pw = format!("{PW}\n");

    let v = home
        .run_stdin(&["address", "--verify", "--password-stdin", "--json"], &pw)
        .ok();
    assert_eq!(
        (&v["verified"], &v["address"]),
        (&json!(true), &json!(addr))
    );
    let human = home.run_stdin(&["address", "--verify", "--password-stdin"], &pw);
    assert!(!human.stdout.contains("not verified"), "{}", human.stdout);
    let l = home
        .run_stdin(&["list", "--verify", "--password-stdin", "--json"], &pw)
        .ok();
    assert_eq!(l["verified"], true);
    assert_eq!(l["keys"][0]["address"], addr.as_str());

    // unlock reports what its own child derived after the password: verified.
    assert_eq!(home.unlock("30s")["verified"], true);
    // Without --verify, even unlocked: cross-checked against the daemon, not verified.
    let v = home.run(&["address", "--json"]).ok();
    assert_eq!(v["verified"], false);
    // balance shows the cached address: not verified either.
    let rpc = common::tiny_rpc(|method| match method {
        "eth_chainId" => json!("0x1"),
        _ => json!("0x0"),
    });
    let b = home
        .run(&["balance", "--chain", "ethereum", "--rpc", &rpc, "--json"])
        .ok();
    assert_eq!(
        (&b["verified"], &b["address"]),
        (&json!(false), &json!(addr))
    );
    home.stop_daemons();

    // A poisoned cache fails --verify; so do a wrong password and a missing terminal.
    poison_cache(&home);
    let msg = home
        .run_stdin(&["address", "--verify", "--password-stdin", "--json"], &pw)
        .err("VALIDATION_ERROR");
    assert!(msg.contains("does not match"), "{msg}");
    home.run_stdin(&["list", "--verify", "--password-stdin", "--json"], &pw)
        .err("VALIDATION_ERROR");
    home.run_stdin(
        &["address", "--verify", "--password-stdin", "--json"],
        "not the password\n",
    )
    .err("WRONG_PASSWORD");
    home.run(&["address", "--verify", "--json"])
        .err("VALIDATION_ERROR");
    // --password-stdin means nothing without --verify.
    home.run_stdin(&["address", "--password-stdin", "--json"], &pw)
        .err("VALIDATION_ERROR");
}

#[test]
fn k1_new_key_is_verified_only_when_derived_after_the_password() {
    let home = Home::new();
    home.create("main", &["--unlock", "30s"]);
    // An agent (no terminal) adds a key through the daemon: not verified.
    let b = home.run(&["create", "b", "--json"]).ok();
    assert_eq!(
        (b["index"].as_i64(), &b["verified"]),
        (Some(1), &json!(false))
    );
    let human = home.run(&["create", "b2"]);
    assert!(human.stdout.contains(NOT_VERIFIED), "{}", human.stdout);
    // A human at a terminal is asked for the password and the key is derived here: verified.
    let mut pty = common::PtyChild::spawn(home.command(&["create", "c", "--json"]));
    pty.wait_for("Wallet password: ", Duration::from_secs(10));
    pty.type_line(PW);
    let out = pty.finish();
    let c: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        (c["index"].as_i64(), &c["verified"]),
        (Some(3), &json!(true)),
        "{c}"
    );
    let seed = store::unlock(home.path(), PW, &chains::derive_addresses)
        .unwrap()
        .0
        .seed;
    assert_eq!(
        chains::derive_addresses(&seed, 3).unwrap()["ethereum"],
        c["address"]
    );
}

#[test]
fn k11_changed_ciphertext_is_noticed_with_no_request_at_all() {
    let home = Home::new();
    home.create("main", &[]);
    home.unlock("30s");
    // Found through the process table: nothing talks to the daemon from here on.
    let pid = only_daemon(&home);
    home.edit_wallet(|v| {
        let ct = v["crypto"]["ciphertext"].as_str().unwrap().to_string();
        let first = if ct.starts_with('0') { "1" } else { "0" };
        v["crypto"]["ciphertext"] = json!(format!("{first}{}", &ct[1..]));
    });
    assert!(
        wait_dead(pid, Duration::from_millis(2500)),
        "the daemon kept the seed with no request to make it look"
    );
}

/// Replaces wallet.json with a FIFO for the guard's lifetime, then puts the file back.
struct FifoWallet {
    path: std::path::PathBuf,
    saved: Vec<u8>,
}

impl FifoWallet {
    fn plant(home: &Home) -> Self {
        let path = home.wallet_json();
        let saved = fs::read(&path).unwrap();
        fs::remove_file(&path).unwrap();
        let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: mkfifo with a valid NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        Self { path, saved }
    }
}

impl Drop for FifoWallet {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        let _ = fs::write(&self.path, &self.saved);
    }
}

#[test]
fn k10_fifo_in_place_of_wallet_json_does_not_stall_the_daemon() {
    let _alone = common::exclusive();
    let home = Home::new();
    home.create("main", &[]);

    // lock works while wallet.json is a FIFO, and the daemon ends on its own: it removes its
    // socket on the way out, which a daemon stuck on the FIFO and then SIGKILLed by `lock`
    // never does (see c14_stale_socket_without_a_daemon_is_wallet_locked).
    home.unlock("30s");
    let fifo = FifoWallet::plant(&home);
    home.run(&["lock", "--json"]).ok();
    assert!(home.daemon_pids().is_empty(), "{:?}", home.daemon_pids());
    assert!(
        !home.sock().exists(),
        "the daemon did not exit by itself (it was killed, leaving d.sock)"
    );
    drop(fifo);

    // With no request at all, the daemon still ends by its expiry.
    home.unlock("2s");
    let pid = only_daemon(&home);
    let fifo = FifoWallet::plant(&home);
    assert!(
        wait_dead(pid, Duration::from_secs(4)),
        "a FIFO in place of wallet.json kept the daemon past its expiry"
    );
    drop(fifo);
}

// ---- Review round 3 (R1-R8) ----

/// A JSON line the daemon wrote, or `None` once it closed its end.
#[cfg(target_os = "linux")]
fn read_json_line(reader: &mut impl std::io::BufRead) -> Option<serde_json::Value> {
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(n) if n > 0 => Some(serde_json::from_str(&line).unwrap()),
        _ => None,
    }
}

#[test]
fn r1_unlock_handshake_over_a_socketpair() {
    let home = Home::new();
    let address = home.create("main", &[])["address"]
        .as_str()
        .unwrap()
        .to_string();
    // `sign-message` signs only under a signed whitelist that allows it (`.prd/04-whitelist.md`).
    home.edit_whitelist(|l| l.policy.allow_message_signing = true);
    // This test plays the unlocking CLI on the other end of the daemon's socketpair.
    let mut daemon = DaemonChild::spawn(home.path());
    assert_eq!(daemon.next()["result"]["stage"], "input");
    let input = zwallet::core::protocol::encode_unlock_input(Duration::from_secs(30), PW).unwrap();
    daemon.send(&input);
    assert_eq!(daemon.next()["result"]["stage"], "decrypted");
    daemon.send(&[zwallet::core::protocol::GO]);
    let ready = daemon.next();
    assert_eq!(ready["ok"], true, "{ready}");
    assert_eq!(ready["result"]["address"], address.as_str(), "{ready}");
    assert_eq!(ready["result"]["pid"], daemon.child.id(), "{ready}");
    let signed = home.run(&["sign-message", "--message", "m", "--json"]).ok();
    assert_eq!(signed["address"], address.as_str());
}

#[cfg(target_os = "linux")]
#[test]
fn r1_daemon_refuses_a_pipe_for_its_channel() {
    let home = Home::new();
    home.create("main", &[]);
    let mut child = common::command_in(home.path(), &[])
        .args(["daemon", "--home"])
        .arg(fs::canonicalize(home.path()).unwrap())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let mut out = std::io::BufReader::new(child.stdout.take().unwrap());
    let first = read_json_line(&mut out);
    let status = child.wait().unwrap();
    let first = first.expect("the daemon said nothing");
    // It never asks for the password over a pipe: its first and last word is the refusal.
    assert_eq!(first["ok"], false, "{first}");
    assert!(
        first["error"].as_str().unwrap().contains("socket"),
        "{first}"
    );
    assert!(!status.success());
    assert!(read_json_line(&mut out).is_none());
}

#[cfg(target_os = "linux")]
#[test]
fn r1_unlock_refuses_ptrace_scope_0_without_the_flag() {
    // Built without `command_in`, which adds --allow-ptrace-scope-0 where the host needs it.
    let raw = |home: &Home, args: &[&str]| {
        let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_zwallet"));
        cmd.env_clear()
            .env("ZWALLET_HOME", home.path())
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = cmd.spawn().unwrap();
        use std::io::Write as _;
        let _ = child
            .stdin
            .take()
            .unwrap()
            .write_all(format!("{PW}\n").as_bytes());
        let out = child.wait_with_output().unwrap();
        common::Run {
            code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    };
    let fresh = Home::new();
    let first = raw(
        &fresh,
        &[
            "create",
            "main",
            "--unlock",
            "30s",
            "--password-stdin",
            "--json",
        ],
    );
    let home = Home::new();
    home.create("main", &[]);
    let unlock = raw(&home, &["unlock", "30s", "--password-stdin", "--json"]);
    if common::ptrace_scope_is_open() {
        // Refused before the prompt, and before anything was created or started.
        for r in [&first, &unlock] {
            let msg = r.err("VALIDATION_ERROR");
            assert!(
                msg.contains("ptrace_scope") && msg.contains("--allow-ptrace-scope-0"),
                "{msg}"
            );
        }
        assert!(!fresh.wallet_json().exists());
        assert!(home.daemon_pids().is_empty());
        // The flag accepts that risk.
        let ok = raw(
            &home,
            &[
                "unlock",
                "30s",
                "--password-stdin",
                "--json",
                "--allow-ptrace-scope-0",
            ],
        )
        .ok();
        assert_eq!(ok["locked"], false, "{ok}");
    } else {
        assert_eq!(first.ok()["locked"], false);
        assert_eq!(unlock.ok()["locked"], false);
    }
}

#[test]
fn r2_lock_stops_a_daemon_behind_a_repointed_symlink() {
    let _alone = common::exclusive();
    let first = Home::new();
    let second = Home::new();
    let links = Home::new();
    // The wallet home is a symlink; its daemon's argv names the directory it pointed at.
    let link = links.path().join("h");
    std::os::unix::fs::symlink(first.path(), &link).unwrap();
    run_in(
        &link,
        &["create", "main", "--password-stdin", "--json"],
        Some(&format!("{PW}\n")),
        &[],
    )
    .ok();
    let pid = run_in(
        &link,
        &["unlock", "30s", "--password-stdin", "--json"],
        Some(&format!("{PW}\n")),
        &[],
    )
    .ok()["daemonPid"]
        .as_i64()
        .unwrap() as i32;
    // A same-user process repoints the link: this home's path no longer leads to that daemon.
    fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(second.path(), &link).unwrap();
    assert!(common::daemon_pids(&link).is_empty());
    let locked = run_in(&link, &["lock", "--json"], None, &[]).ok();
    assert_eq!(locked["wasUnlocked"], true, "{locked}");
    assert!(
        locked["stopped"].as_array().unwrap().contains(&json!(pid)),
        "{locked}"
    );
    assert!(wait_dead(pid, Duration::from_secs(3)));
    assert_eq!(
        common::all_daemon_pids(),
        Vec::<i32>::new(),
        "a zwallet daemon of this user survived lock"
    );
}

#[test]
fn r2_status_reports_the_real_daemon_over_a_listener_saying_locked() {
    let home = Home::new();
    home.create("main", &[]);
    home.unlock("30s");
    let pid = only_daemon(&home);
    // Frozen, so it cannot notice its socket being swapped; a listener that claims the session
    // is locked (even naming the real pid) takes d.sock.
    freeze(pid);
    fs::rename(home.sock(), home.path().join("moved.sock")).unwrap();
    let fake = FakeDaemon::answering(
        &home.sock(),
        json!({ "locked": true, "pid": pid, "wasUnlocked": false }),
    );
    let status = home.run(&["status", "--json"]).ok();
    assert_eq!(status["locked"], false, "{status}");
    assert_eq!(status["daemonPids"], json!([pid]), "{status}");
    drop(fake);
}

#[test]
fn r2_lock_during_an_unlock_ends_locked_with_no_daemon() {
    let _alone = common::exclusive();
    let home = Home::new();
    home.create("main", &[]);
    let mut unlock = home.command(&["unlock", "30s", "--password-stdin", "--json"]);
    unlock
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut unlocking = unlock.spawn().unwrap();
    use std::io::Write as _;
    unlocking
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{PW}\n").as_bytes())
        .unwrap();
    // The unlock's daemon child exists (decrypting, waiting for the go byte, or binding).
    let start = std::time::Instant::now();
    let child = loop {
        if let Some(pid) = home.daemon_pids().first() {
            break *pid;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "the unlock never started its daemon"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    let locked = home.run(&["lock", "--json"]).ok();
    assert!(
        locked["stopped"]
            .as_array()
            .unwrap()
            .contains(&json!(child)),
        "{locked}"
    );
    let out = unlocking.wait_with_output().unwrap();
    assert!(
        home.daemon_pids().is_empty() && common::all_daemon_pids().is_empty(),
        "a daemon outlived lock; unlock said {}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(home.run(&["status", "--json"]).ok()["locked"], true);
}

#[test]
fn r3_daemon_inherits_no_descriptor_above_the_soft_limit() {
    use std::io::Read as _;
    use std::os::fd::AsRawFd;
    use std::os::unix::process::CommandExt;
    const HIGH: libc::c_int = 200;
    const SOFT: libc::rlim_t = 64;
    let home = Home::new();
    home.create("main", &[]);
    // The unlocking CLI inherits a pipe's write end as fd 200 (not close-on-exec), and then its
    // soft RLIMIT_NOFILE is lowered to 64: a scan of fds below the soft limit misses fd 200.
    let (mut reader, writer) = std::io::pipe().unwrap();
    let w = writer.as_raw_fd();
    let mut cmd = home.command(&["unlock", "30s", "--password-stdin", "--json"]);
    cmd.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    // SAFETY: dup2/getrlimit/setrlimit are async-signal-safe.
    unsafe {
        cmd.pre_exec(move || {
            let mut lim = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            if libc::dup2(w, HIGH) == -1 || libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            lim.rlim_cur = SOFT;
            if libc::setrlimit(libc::RLIMIT_NOFILE, &lim) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = cmd.spawn().unwrap();
    drop(cmd);
    drop(writer);
    use std::io::Write as _;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{PW}\n").as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(home.daemon_pids().len(), 1);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut sink = Vec::new();
        let _ = tx.send(reader.read_to_end(&mut sink).map(|_| ()));
    });
    assert!(
        rx.recv_timeout(Duration::from_secs(3)).is_ok(),
        "the key daemon inherited fd {HIGH}, above the CLI's soft descriptor limit"
    );
}

#[test]
fn r5_verify_prints_the_index_and_warns_about_edited_indices() {
    let home = Home::new();
    home.create("main", &[]);
    let pw = format!("{PW}\n");
    let v = home
        .run_stdin(&["address", "--verify", "--password-stdin", "--json"], &pw)
        .ok();
    assert_eq!(v["index"], 0, "{v}");
    assert!(v.get("warning").is_none(), "{v}");
    let l = home
        .run_stdin(&["list", "--verify", "--password-stdin", "--json"], &pw)
        .ok();
    assert!(l.get("warning").is_none(), "{l}");

    // A same-user process moves key 'main' to index 5 (with that index's real addresses, so the
    // cache check passes): an address of this seed, but one zwallet never assigned.
    let seed = store::unlock(home.path(), PW, &chains::derive_addresses)
        .unwrap()
        .0
        .seed;
    let five = chains::derive_addresses(&seed, 5).unwrap();
    home.edit_wallet(|v| {
        v["keys"][0]["index"] = json!(5);
        v["keys"][0]["addresses"] = json!(five);
    });
    let v = home
        .run_stdin(&["address", "--verify", "--password-stdin", "--json"], &pw)
        .ok();
    assert_eq!(
        (&v["index"], &v["verified"], &v["address"]),
        (&json!(5), &json!(true), &json!(five["ethereum"])),
        "{v}"
    );
    let warning = v["warning"].as_str().unwrap_or_else(|| panic!("{v}"));
    assert!(
        warning.contains("at or past nextIndex 1") && warning.contains("belong to no key"),
        "{warning}"
    );
    let human = home.run_stdin(&["address", "--verify", "--password-stdin"], &pw);
    assert!(
        human.stdout.contains("index: 5") && human.stdout.contains("warning: "),
        "{}",
        human.stdout
    );
    let l = home
        .run_stdin(&["list", "--verify", "--password-stdin", "--json"], &pw)
        .ok();
    assert!(l["warning"].as_str().is_some(), "{l}");
}

#[test]
fn r8_daemon_wipes_and_exits_when_its_cli_dies_after_the_go_byte() {
    let home = Home::new();
    home.create("main", &[]);
    // Holding d.lock keeps the daemon from binding (and so from reporting readiness) until this
    // test, playing the CLI, has gone away.
    let squat = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(home.path().join("d.lock"))
        .unwrap();
    squat.try_lock().unwrap();
    let mut daemon = DaemonChild::spawn(home.path());
    assert_eq!(daemon.next()["result"]["stage"], "input");
    let input = zwallet::core::protocol::encode_unlock_input(Duration::from_secs(30), PW).unwrap();
    daemon.send(&input);
    assert_eq!(daemon.next()["result"]["stage"], "decrypted");
    daemon.send(&[zwallet::core::protocol::GO]);
    // The CLI is killed: every handle on its end of the channel is gone.
    daemon.close();
    drop(squat);
    // The daemon binds, its readiness write fails (EPIPE), and it wipes the seed and exits
    // instead of serving a key nobody unlocked.
    let status = daemon.wait_exit(Duration::from_secs(10));
    assert!(
        status.is_some_and(|s| !s.success()),
        "the daemon kept running after its unlocking CLI died: {status:?}"
    );
    assert!(!home.sock().exists(), "it left its socket behind");
    assert!(home.daemon_pids().is_empty());
}

// ---- Custody (review round 4: C1-C3) ----

/// Gives a directory back its permissions when dropped, so its temp home can be removed.
struct Reopen(std::path::PathBuf);

impl Drop for Reopen {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o700));
    }
}

#[test]
fn r4c1_lock_reads_no_home_and_stops_every_daemon() {
    let _alone = common::exclusive();
    let home = Home::new();
    home.create("main", &[]);
    // The home `lock` is given serves no daemon. A listener on its d.sock stands for anything
    // there that could answer slowly or never (a path on a stalled mount): lock must not touch it.
    let elsewhere = Home::new();
    let fake = FakeDaemon::bind(&elsewhere.sock());
    let closed = elsewhere.path().join("closed");
    fs::create_dir(&closed).unwrap();
    fs::set_permissions(&closed, fs::Permissions::from_mode(0o000)).unwrap();
    let _reopen = Reopen(closed.clone());
    let cases = [
        ("HOME and ZWALLET_HOME unset", None),
        (
            "a missing ZWALLET_HOME",
            Some(elsewhere.path().join("missing").join("home")),
        ),
        ("an unreadable ZWALLET_HOME", Some(closed.join("home"))),
        (
            "a ZWALLET_HOME with a listener on d.sock",
            Some(elsewhere.path().to_path_buf()),
        ),
    ];
    let mut failures = Vec::new();
    for (what, zwallet_home) in &cases {
        let pid = home.unlock("30s")["daemonPid"].as_i64().unwrap() as i32;
        let mut cmd = common::command_without_home(&["lock", "--json"]);
        if let Some(h) = zwallet_home {
            cmd.env("ZWALLET_HOME", h);
        }
        let r = common::run_command(cmd, None);
        let v: serde_json::Value = serde_json::from_str(&r.stdout).unwrap_or_default();
        let stopped = v["stopped"]
            .as_array()
            .is_some_and(|s| s.contains(&json!(pid)));
        if r.code != 0 || v["wasUnlocked"] != true || !stopped {
            failures.push(format!(
                "{what}: lock said {:?} / {:?}",
                r.stdout.trim(),
                r.stderr.trim()
            ));
        }
        if !wait_dead(pid, Duration::from_secs(3)) {
            failures.push(format!("{what}: the daemon {pid} of another home survived"));
        }
    }
    assert!(!elsewhere.path().join("missing").exists());
    let contacts = fake.received.lock().unwrap().len();
    if contacts > 0 {
        failures.push(format!(
            "lock connected {contacts} time(s) to the d.sock of the home it was given"
        ));
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn r4c2_unlock_shows_the_index_and_the_verify_warning() {
    let home = Home::new();
    home.create("main", &[]);
    let pw = format!("{PW}\n");
    let clean = home.unlock("30s");
    assert_eq!(clean["index"], 0, "{clean}");
    assert!(clean.get("warning").is_none(), "{clean}");
    // Key 'main' moved to index 5 with that index's real addresses (as in r5): what unlock shows
    // is the seed's address at index 5, so it must say so, with the warning `--verify` gives.
    let seed = store::unlock(home.path(), PW, &chains::derive_addresses)
        .unwrap()
        .0
        .seed;
    let five = chains::derive_addresses(&seed, 5).unwrap();
    home.edit_wallet(|v| {
        v["keys"][0]["index"] = json!(5);
        v["keys"][0]["addresses"] = json!(five);
    });
    let verify = home
        .run_stdin(&["address", "--verify", "--password-stdin", "--json"], &pw)
        .ok();
    assert!(verify["warning"].is_string(), "{verify}");
    let unlocked = home.unlock("30s");
    assert_eq!(
        (
            &unlocked["index"],
            &unlocked["address"],
            &unlocked["verified"]
        ),
        (&json!(5), &json!(five["ethereum"]), &json!(true)),
        "{unlocked}"
    );
    assert_eq!(unlocked["warning"], verify["warning"], "{unlocked}");
    let human = home.run_stdin(&["unlock", "30s", "--password-stdin"], &pw);
    assert!(
        human.code == 0 && human.stdout.contains("index: 5") && human.stdout.contains("warning: "),
        "{} / {}",
        human.stdout,
        human.stderr
    );
}

#[test]
fn r4c3_status_lists_the_daemons_of_other_homes() {
    let a = Home::new();
    let b = Home::new();
    a.create("main", &[]);
    b.create("main", &[]);
    a.unlock("30s");
    b.unlock("30s");
    let (pa, pb) = (only_daemon(&a), only_daemon(&b));
    let others = |home: &Home| {
        let s = home.run(&["status", "--json"]).ok();
        let pids = s["otherDaemonPids"]
            .as_array()
            .unwrap_or_else(|| panic!("no otherDaemonPids: {s}"))
            .clone();
        (s, pids)
    };
    let (s, pids) = others(&a);
    assert_eq!(s["daemonPids"], json!([pa]), "{s}");
    assert!(
        pids.contains(&json!(pb)) && !pids.contains(&json!(pa)),
        "{s}"
    );
    let (s, pids) = others(&b);
    assert!(
        pids.contains(&json!(pa)) && !pids.contains(&json!(pb)),
        "{s}"
    );
    let human = a.run(&["status"]);
    assert!(
        human.stdout.contains(&pb.to_string())
            && human.stdout.contains("`zwallet lock` stops them too"),
        "{}",
        human.stdout
    );
    // A locked home still lists them: `zwallet lock` stops them too.
    a.stop_daemons();
    let (s, pids) = others(&a);
    assert_eq!(s["locked"], true, "{s}");
    assert!(pids.contains(&json!(pb)), "{s}");
}

// The whitelist CLI (`.prd/04-whitelist.md`): `zwallet whitelist …` and `@nickname` recipients.
// Every mutating `whitelist` command stops every zwallet daemon of this user before it edits, so
// the tests that run one take `common::exclusive()`.

/// The EIP-55 example address.
const PAYEE: &str = "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed";
const BOB: &str = "0x3333333333333333333333333333333333333333";
const ROUTER: &str = "0x2222222222222222222222222222222222222222";
/// USDC on Ethereum, a registry token.
const ETH_USDC: &str = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48";
const RESIDUAL: &str = "a listed contract may forward value or act on its arguments";

/// `whitelist <args> --password-stdin --json` with `password` on stdin (`common::command_in`
/// adds `--allow-ptrace-scope-0` where `--unlock` needs it).
fn whitelist_with(home: &Home, args: &[&str], password: &str) -> common::Run {
    let mut all = vec!["whitelist"];
    all.extend_from_slice(args);
    all.extend(["--password-stdin", "--json"]);
    home.run_stdin(&all, &format!("{password}\n"))
}

/// A mutating `whitelist` command with the test password.
fn whitelist_edit(home: &Home, args: &[&str]) -> common::Run {
    whitelist_with(home, args, PW)
}

fn verify_list(home: &Home) -> serde_json::Value {
    home.run_stdin(
        &[
            "whitelist",
            "list",
            "--verify",
            "--password-stdin",
            "--json",
        ],
        &format!("{PW}\n"),
    )
    .ok()
}

fn nicknames(entries: &serde_json::Value) -> Vec<String> {
    entries
        .as_array()
        .unwrap_or_else(|| panic!("not a list: {entries}"))
        .iter()
        .map(|e| {
            e.as_str()
                .or_else(|| e["nickname"].as_str())
                .unwrap_or_else(|| panic!("no nickname: {e}"))
                .to_string()
        })
        .collect()
}

fn rfc3339_secs(v: &serde_json::Value) -> u64 {
    let s = v.as_str().unwrap_or_else(|| panic!("not a time: {v}"));
    zwallet::core::time::parse_rfc3339_utc(s).unwrap_or_else(|| panic!("not RFC 3339: {s}"))
}

#[test]
fn w1_first_add_makes_the_key_and_the_list_reads_while_locked() {
    let _alone = common::exclusive();
    let home = Home::new();
    home.create("main", &[]);
    assert!(!home.path().join("whitelist.key").exists());

    // The first add, typed at a terminal: the password is never echoed or printed.
    let cmd = home.command(&[
        "whitelist",
        "add",
        "alice",
        PAYEE,
        "--chains",
        "base",
        "--json",
    ]);
    let mut pty = common::PtyChild::spawn(cmd);
    // Before the prompt, on the terminal: the change the password authorizes.
    let mut before = String::new();
    let start = std::time::Instant::now();
    while !before.contains("Wallet password: ") {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "no prompt; saw {before:?}"
        );
        before.push_str(&common::drain_pty(&pty.master));
        std::thread::sleep(Duration::from_millis(20));
    }
    let summary = &before[..before.find("Wallet password: ").unwrap()];
    for want in [
        "add alice",
        PAYEE,
        "chains: base",
        "if wallet.json seals no whitelist yet: a new list from the built-in defaults with a new \
         key, gen 1",
        "after the change: the wallet stays locked",
    ] {
        assert!(summary.contains(want), "{want:?} not in {summary:?}");
    }
    pty.type_line(PW);
    let mut tty = String::new();
    let start = std::time::Instant::now();
    while pty.child.try_wait().unwrap().is_none() && start.elapsed() < Duration::from_secs(120) {
        tty.push_str(&common::drain_pty(&pty.master));
        std::thread::sleep(Duration::from_millis(20));
    }
    tty.push_str(&common::drain_pty(&pty.master));
    let out = pty.finish();
    let (stdout, stderr) = (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    );
    for (what, text) in [("terminal", &tty), ("stdout", &stdout), ("stderr", &stderr)] {
        assert!(
            !text.contains(PW),
            "the password reached the {what}: {text:?}"
        );
    }
    assert!(out.status.success(), "{stdout} / {stderr}");
    let added: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(added["gen"], 1, "{added}");
    assert_eq!(added["previousGen"], serde_json::Value::Null, "{added}");
    assert_eq!(added["keyCreated"], true, "{added}");
    assert_eq!(added["change"], "added", "{added}");
    assert_eq!(
        added["entry"],
        json!({
            "nickname": "alice",
            "kind": "address",
            "address": PAYEE.to_ascii_lowercase(),
            "chains": ["base"],
        }),
        "{added}"
    );
    assert_eq!(nicknames(&added["entries"]), ["alice"], "{added}");
    assert_eq!(added["verified"], true, "{added}");
    // An edit leaves the wallet locked unless --unlock is given.
    assert_eq!(added["locked"], true, "{added}");
    assert_eq!(added["stoppedDaemons"], json!([]), "{added}");
    assert!(added.get("expiresAt").is_none(), "{added}");
    assert!(
        added["hint"].as_str().unwrap_or("").contains("--unlock"),
        "{added}"
    );
    for f in ["whitelist.key", "whitelist.json"] {
        assert_eq!(mode(&home.path().join(f)), 0o600, "{f}");
    }
    assert!(!home.path().join("whitelist.next.json").exists());

    // `list` works while locked, without the password: unverified.
    let listed = home.run(&["whitelist", "list", "--json"]).ok();
    assert_eq!(listed["gen"], 1, "{listed}");
    assert_eq!(listed["verified"], false, "{listed}");
    assert_eq!(listed["entries"][0], added["entry"], "{listed}");
    assert_eq!(listed["policy"]["allowMessageSigning"], false, "{listed}");
    assert_eq!(
        listed["policy"]["maxFeeWei"]["base"], "500000000000000",
        "{listed}"
    );
    let human = home.run(&["whitelist", "list"]);
    assert!(
        human.code == 0
            && human.stdout.contains("alice")
            && human.stdout.contains("zwallet whitelist list --verify"),
        "{} / {}",
        human.stdout,
        human.stderr
    );
    // `--verify` checks the signature against wallet.json's seal with the password.
    let verified = verify_list(&home);
    assert_eq!(verified["verified"], true, "{verified}");
    assert_eq!(verified["gen"], 1, "{verified}");
    assert_eq!(verified["sealedGen"], 1, "{verified}");
    assert_eq!(verified["source"], "whitelist.json", "{verified}");
    assert_eq!(verified["entries"], listed["entries"], "{verified}");
    home.run_stdin(
        &[
            "whitelist",
            "list",
            "--verify",
            "--password-stdin",
            "--json",
        ],
        "not the password\n",
    )
    .err("WRONG_PASSWORD");
}

#[test]
fn whitelist_verbs_change_one_generation_each_and_refuse_before_writing() {
    let _alone = common::exclusive();
    let home = Home::new();
    home.create("main", &[]);
    // Without --chains: the default chain (ethereum). "*" only when given.
    let a = whitelist_edit(&home, &["add", "alice", PAYEE]).ok();
    assert_eq!(
        (&a["gen"], &a["entry"]["chains"]),
        (&json!(1), &json!(["ethereum"])),
        "{a}"
    );
    let b = whitelist_edit(&home, &["add", "bob", BOB, "--chains", "*"]).ok();
    assert_eq!(
        (&b["gen"], &b["entry"]["chains"]),
        (&json!(2), &json!(["*"])),
        "{b}"
    );
    assert_eq!(b["keyCreated"], false, "{b}");
    // A contract entry: its selectors (lower-cased), and the residual it carries.
    let r = whitelist_edit(
        &home,
        &[
            "add",
            "router",
            ROUTER,
            "--chains",
            "base,arbitrum",
            "--contract",
            "--selectors",
            "0x12345678,0xABCDEF01",
        ],
    )
    .ok();
    assert_eq!(r["gen"], 3, "{r}");
    assert_eq!(
        r["entry"],
        json!({
            "nickname": "router",
            "kind": "contract",
            "address": ROUTER,
            "chains": ["base", "arbitrum"],
            "selectors": ["0x12345678", "0xabcdef01"],
        }),
        "{r}"
    );
    assert!(
        r["warning"].as_str().unwrap_or("").contains(RESIDUAL),
        "{r}"
    );
    let human = home.run(&["whitelist", "list"]);
    assert!(human.stdout.contains("router"), "{}", human.stdout);

    // Refusals: nothing is written, the generation stays.
    let refused: [(&[&str], &str); 11] = [
        (&["add", "Alice", BOB], "VALIDATION_ERROR"),
        (
            &["add", "carol", "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAeD"],
            "INVALID_ADDRESS",
        ),
        (
            &["add", "carol", BOB, "--chains", "dogechain"],
            "CHAIN_UNSUPPORTED",
        ),
        (
            &[
                "add",
                "usdc",
                ETH_USDC,
                "--chains",
                "ethereum",
                "--contract",
                "--selectors",
                "0xa9059cbb",
            ],
            "VALIDATION_ERROR",
        ),
        (
            &[
                "add",
                "r2",
                "0x4444444444444444444444444444444444444444",
                "--chains",
                "*",
                "--contract",
                "--selectors",
                "0x12345678",
            ],
            "VALIDATION_ERROR",
        ),
        (
            &[
                "add",
                "r3",
                "0x4444444444444444444444444444444444444444",
                "--contract",
            ],
            "VALIDATION_ERROR",
        ),
        (
            &["add", "alice", "0x4444444444444444444444444444444444444444"],
            "VALIDATION_ERROR",
        ),
        (&["add", "carol", PAYEE], "VALIDATION_ERROR"),
        (&["remove", "nobody"], "VALIDATION_ERROR"),
        (&["fee-cap", "base", "0"], "VALIDATION_ERROR"),
        (&["allow-message-signing", "maybe"], "VALIDATION_ERROR"),
    ];
    for (args, code) in refused {
        let msg = whitelist_edit(&home, args).err(code);
        if args[0] == "remove" {
            assert!(msg.contains("zwallet whitelist list"), "{args:?}: {msg}");
        }
    }
    whitelist_with(&home, &["add", "carol", BOB], "not the password").err("WRONG_PASSWORD");
    assert_eq!(verify_list(&home)["gen"], 3);

    // fee-cap in the chain's coin, stored in wei.
    let f = whitelist_edit(&home, &["fee-cap", "base", "0.001"]).ok();
    assert_eq!(f["gen"], 4, "{f}");
    assert_eq!(f["change"], "fee-cap", "{f}");
    assert_eq!(
        f["feeCap"],
        json!({ "chain": "base", "maxFeeWei": "1000000000000000", "maxFee": "0.001 ETH" }),
        "{f}"
    );
    let m = whitelist_edit(&home, &["allow-message-signing", "on"]).ok();
    assert_eq!(
        (&m["gen"], &m["allowMessageSigning"]),
        (&json!(5), &json!(true)),
        "{m}"
    );
    let rm = whitelist_edit(&home, &["remove", "bob"]).ok();
    assert_eq!(rm["gen"], 6, "{rm}");
    assert_eq!(rm["entry"]["nickname"], "bob", "{rm}");
    assert_eq!(nicknames(&rm["entries"]), ["alice", "router"], "{rm}");
    let v = verify_list(&home);
    assert_eq!(v["verified"], true, "{v}");
    assert_eq!(v["gen"], 6, "{v}");
    assert_eq!(v["policy"]["maxFeeWei"]["base"], "1000000000000000", "{v}");
    assert_eq!(v["policy"]["allowMessageSigning"], true, "{v}");
    assert_eq!(nicknames(&v["entries"]), ["alice", "router"], "{v}");

    // reset: a new key and an empty list with the default policy, at the next generation.
    let key_before = fs::read(home.path().join("whitelist.key")).unwrap();
    let z = whitelist_edit(&home, &["reset"]).ok();
    assert_eq!(z["gen"], 7, "{z}");
    assert_eq!(z["change"], "reset", "{z}");
    assert_eq!(z["keyCreated"], true, "{z}");
    assert_eq!(z["entries"], json!([]), "{z}");
    assert_eq!(nicknames(&z["dropped"]), ["alice", "router"], "{z}");
    assert_ne!(
        fs::read(home.path().join("whitelist.key")).unwrap(),
        key_before
    );
    let v = verify_list(&home);
    assert_eq!(
        (&v["verified"], &v["gen"]),
        (&json!(true), &json!(7)),
        "{v}"
    );
    assert_eq!(v["entries"], json!([]), "{v}");
    assert_eq!(v["policy"]["allowMessageSigning"], false, "{v}");
    assert_eq!(v["policy"]["maxFeeWei"]["base"], "500000000000000", "{v}");
}

#[test]
fn w4_a_hand_edited_list_is_shown_but_an_edit_refuses_and_only_reset_starts_over() {
    let _alone = common::exclusive();
    let home = Home::new();
    home.create("main", &[]);
    whitelist_edit(&home, &["add", "alice", PAYEE, "--chains", "base"]).ok();
    let path = home.path().join("whitelist.json");
    let mut file: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let mallory = json!({
        "nickname": "mallory",
        "kind": "address",
        "address": "0x6666666666666666666666666666666666666666",
        "chains": ["*"],
    });
    file["entries"]
        .as_array_mut()
        .unwrap()
        .push(mallory.clone());
    fs::write(&path, serde_json::to_vec_pretty(&file).unwrap()).unwrap();

    // Shown while locked, unverified.
    let listed = home.run(&["whitelist", "list", "--json"]).ok();
    assert_eq!(listed["verified"], false, "{listed}");
    assert_eq!(
        nicknames(&listed["entries"]),
        ["alice", "mallory"],
        "{listed}"
    );
    // --verify says why it is not the signed list.
    let v = verify_list(&home);
    assert_eq!(v["verified"], false, "{v}");
    assert!(
        v["reason"].as_str().unwrap_or("").contains("signature"),
        "{v}"
    );
    assert_eq!(nicknames(&v["entries"]), ["alice", "mallory"], "{v}");

    // An edit never rebuilds it from empty: refused, nothing written, a running session left
    // alone. The message points to `list --verify` and to `reset`.
    let pid = home.unlock("10m")["daemonPid"].as_i64().unwrap() as i32;
    let before = files_of(&home);
    for args in [
        &["add", "bob", BOB, "--chains", "base"][..],
        &["remove", "alice"],
        &["fee-cap", "base", "0.001"],
        &["allow-message-signing", "off"],
    ] {
        let msg = whitelist_edit(&home, args).err("VALIDATION_ERROR");
        for want in [
            "signature",
            "zwallet whitelist list --verify",
            "zwallet whitelist reset",
        ] {
            assert!(msg.contains(want), "{args:?}: {want:?} not in {msg}");
        }
        assert_eq!(files_of(&home), before, "{args:?} wrote something");
    }
    assert_eq!(
        home.daemon_pids(),
        vec![pid],
        "a refused edit stopped the session"
    );

    // A deleted list is refused the same way.
    fs::remove_file(&path).unwrap();
    let msg = whitelist_edit(&home, &["add", "bob", BOB]).err("VALIDATION_ERROR");
    assert!(msg.contains("there is no whitelist.json"), "{msg}");
    assert!(msg.contains("zwallet whitelist reset"), "{msg}");
    fs::write(&path, serde_json::to_vec_pretty(&file).unwrap()).unwrap();

    // Reset starts over: a new key and an empty list, with what it dropped.
    let z = whitelist_edit(&home, &["reset"]).ok();
    assert_eq!(
        (&z["gen"], &z["keyCreated"]),
        (&json!(2), &json!(true)),
        "{z}"
    );
    assert_eq!(z["entries"], json!([]), "{z}");
    assert!(
        z["discarded"]["reason"]
            .as_str()
            .unwrap_or("")
            .contains("signature"),
        "{z}"
    );
    assert_eq!(
        nicknames(&z["discarded"]["entries"]),
        ["alice", "mallory"],
        "{z}"
    );
    let e = whitelist_edit(&home, &["add", "bob", BOB, "--chains", "base"]).ok();
    assert_eq!(
        (&e["gen"], &e["base"]),
        (&json!(3), &json!("current")),
        "{e}"
    );
    assert!(e.get("discarded").is_none(), "{e}");
    let v = verify_list(&home);
    assert_eq!(
        (&v["verified"], &v["gen"]),
        (&json!(true), &json!(3)),
        "{v}"
    );
    assert_eq!(nicknames(&v["entries"]), ["bob"], "{v}");
}

/// wallet.json and every whitelist file of `home`, as bytes (absent: `None`).
fn files_of(home: &Home) -> Vec<(&'static str, Option<Vec<u8>>)> {
    [
        "wallet.json",
        "whitelist.json",
        "whitelist.next.json",
        "whitelist.key",
    ]
    .into_iter()
    .map(|f| (f, fs::read(home.path().join(f)).ok()))
    .collect()
}

#[test]
fn w7_after_a_crash_at_any_write_the_cli_shows_and_continues_a_verified_list() {
    use zwallet::chains::evm::registry::WHITELIST_RULES;
    use zwallet::core::error::WalletError;
    use zwallet::core::whitelist::{self, EditStage, Entry};

    let _alone = common::exclusive();
    let home = Home::new();
    home.create("main", &[]);
    whitelist_edit(&home, &["add", "alice", PAYEE, "--chains", "base"]).ok();
    let mut sealed = 1u64;
    let mut expected = vec!["alice".to_string()];
    for (i, stage) in ["staged", "resealed", "published"].into_iter().enumerate() {
        // The crash: an edit stopped right after one of its writes (in this process, through the
        // core edit's test hook).
        let crashed = format!("c{i}");
        let hook = |s: EditStage<'_>| -> zwallet::core::error::Result<()> {
            let hit = matches!(
                (stage, s),
                ("staged", EditStage::NextWritten(_))
                    | ("resealed", EditStage::Resealed(_))
                    | ("published", EditStage::Published(_))
            );
            if hit {
                Err(WalletError::validation("crash"))
            } else {
                Ok(())
            }
        };
        let address = format!("0x{:040x}", 0xc0 + i);
        let err = whitelist::edit_with_hook(
            home.path(),
            PW,
            &WHITELIST_RULES,
            |l| l.add(Entry::new_address(&crashed, &address, &["base"])),
            &hook,
        )
        .unwrap_err();
        assert_eq!(err.message, "crash", "{stage}");
        let (source, base) = match stage {
            // Before the re-seal: the old generation stands.
            "staged" => ("whitelist.json", "current"),
            // Re-sealed but not renamed: the staged list is the sealed one.
            "resealed" => ("whitelist.next.json", "promoted-next"),
            _ => ("whitelist.json", "current"),
        };
        if stage != "staged" {
            sealed += 1;
            expected.push(crashed.clone());
        }
        let v = verify_list(&home);
        assert_eq!(v["verified"], true, "{stage}: {v}");
        assert_eq!(v["gen"], sealed, "{stage}: {v}");
        assert_eq!(v["sealedGen"], sealed, "{stage}: {v}");
        assert_eq!(v["source"], source, "{stage}: {v}");
        assert_eq!(nicknames(&v["entries"]), expected, "{stage}: {v}");
        // The next CLI edit continues from exactly that list.
        let fixed = format!("d{i}");
        let address = format!("0x{:040x}", 0xd0 + i);
        let e = whitelist_edit(&home, &["add", &fixed, &address, "--chains", "base"]).ok();
        sealed += 1;
        expected.push(fixed);
        assert_eq!(e["base"], base, "{stage}: {e}");
        assert_eq!(e["gen"], sealed, "{stage}: {e}");
        assert_eq!(nicknames(&e["entries"]), expected, "{stage}: {e}");
        assert!(e.get("discarded").is_none(), "{stage}: {e}");
        assert!(!home.path().join("whitelist.next.json").exists(), "{stage}");
    }
}

#[test]
fn w8_an_edit_while_unlocked_stops_every_daemon_and_unlocks_only_when_asked() {
    let _alone = common::exclusive();
    let home = Home::new();
    home.create("main", &[]);
    let other = Home::new();
    other.create("main", &[]);
    let other_pid = other.unlock("10m")["daemonPid"].as_i64().unwrap() as i32;
    let old_pid = home.unlock("1h")["daemonPid"].as_i64().unwrap() as i32;

    // A wrong password, a refused change, or a bad --unlock duration stops nothing.
    whitelist_with(&home, &["add", "alice", PAYEE], "not the password").err("WRONG_PASSWORD");
    whitelist_edit(&home, &["remove", "nobody"]).err("VALIDATION_ERROR");
    whitelist_edit(&home, &["add", "alice", PAYEE, "--unlock", "0s"]).err("VALIDATION_ERROR");
    assert_eq!(home.daemon_pids(), vec![old_pid]);
    assert!(pid_alive(other_pid));

    // Every daemon of this user is stopped before the edit, and the wallet stays locked.
    let e = whitelist_edit(&home, &["add", "alice", PAYEE, "--chains", "base"]).ok();
    let stopped = e["stoppedDaemons"].as_array().unwrap();
    assert!(
        stopped.contains(&json!(old_pid)) && stopped.contains(&json!(other_pid)),
        "{e}"
    );
    assert!(
        wait_dead(old_pid, Duration::from_secs(5)),
        "the session survived the edit"
    );
    assert!(wait_dead(other_pid, Duration::from_secs(5)));
    assert_eq!(e["locked"], true, "{e}");
    assert!(
        e["hint"]
            .as_str()
            .unwrap_or("")
            .contains("--unlock <duration>"),
        "{e}"
    );
    for k in ["expiresAt", "daemonPid", "previousExpiresAt", "whitelist"] {
        assert!(e.get(k).is_none(), "{k}: {e}");
    }
    assert_eq!(home.run(&["status", "--json"]).ok()["locked"], true);
    assert_eq!(other.run(&["status", "--json"]).ok()["locked"], true);

    // --unlock <dur>: a fresh session for the duration typed here, signing under the new list.
    let old_pid = home.unlock("1h")["daemonPid"].as_i64().unwrap() as i32;
    let before = now_secs();
    let e = whitelist_edit(
        &home,
        &["add", "bob", BOB, "--chains", "base", "--unlock", "10m"],
    )
    .ok();
    let after = now_secs();
    assert_eq!(e["gen"], 2, "{e}");
    assert_eq!(e["locked"], false, "{e}");
    assert_eq!(e["duration"], "10m", "{e}");
    assert!(
        e["stoppedDaemons"]
            .as_array()
            .unwrap()
            .contains(&json!(old_pid)),
        "{e}"
    );
    assert!(wait_dead(old_pid, Duration::from_secs(5)));
    let expires = rfc3339_secs(&e["expiresAt"]);
    assert!(
        before + 600 <= expires && expires <= after + 601,
        "{before}+600 <= {expires} <= {after}+601: {e}"
    );
    assert_eq!(e["whitelist"]["gen"], 2, "{e}");
    assert_eq!(e["whitelist"]["verified"], true, "{e}");
    assert_eq!(
        nicknames(&e["whitelist"]["entries"]),
        ["alice", "bob"],
        "{e}"
    );
    let new_pid = e["daemonPid"].as_i64().unwrap() as i32;
    assert_ne!(new_pid, old_pid);
    let status = home.run(&["status", "--json"]).ok();
    assert_eq!(status["daemonPid"], new_pid, "{status}");
    assert_eq!(status["expiresAt"], e["expiresAt"], "{status}");
    assert_eq!(status["whitelist"]["gen"], 2, "{status}");
    let human = whitelist_edit(&home, &["remove", "bob"]);
    assert_eq!(human.code, 0, "{}", human.stderr);
    assert!(home.wait_locked(Duration::from_secs(5)));
}

#[test]
fn at_nickname_recipients_resolve_through_the_list() {
    let home = Home::new();
    home.create("main", &[]);
    let transfer = |to: &str| {
        home.run(&[
            "transfer", "--chain", "base", "--to", to, "--amount", "0.01", "--json",
        ])
    };
    let invoke = |to: &str| {
        home.run(&[
            "invoke",
            "--chain",
            "base",
            "--to",
            to,
            "--data",
            "0xd0e30db0",
            "--json",
        ])
    };
    // No whitelist yet: nobody to resolve.
    let msg = transfer("@alice").err("VALIDATION_ERROR");
    assert!(msg.contains("zwallet whitelist list"), "{msg}");
    home.allow("alice", PAYEE, &["base"]);
    home.allow_contract("router", ROUTER, &["base"], &["0xd0e30db0"]);
    // A listed nickname resolves, and the next check (the lock) answers.
    transfer("@alice").err("WALLET_LOCKED");
    invoke("@router").err("WALLET_LOCKED");
    transfer(PAYEE).err("WALLET_LOCKED");
    // An unknown or malformed nickname is VALIDATION_ERROR naming `zwallet whitelist list`.
    for to in ["@bob", "@Alice", "@"] {
        let msg = transfer(to).err("VALIDATION_ERROR");
        assert!(msg.contains("zwallet whitelist list"), "{to}: {msg}");
        let msg = invoke(to).err("VALIDATION_ERROR");
        assert!(msg.contains("zwallet whitelist list"), "{to}: {msg}");
    }
}

#[test]
fn hand_edited_values_reach_the_terminal_escaped() {
    let home = Home::new();
    home.create("main", &[]);
    home.allow("alice", PAYEE, &["base"]);
    let path = home.path().join("whitelist.json");
    let mut file: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    // A nickname that would clear the screen, and an address with a right-to-left override.
    file["entries"].as_array_mut().unwrap().push(json!({
        "nickname": "mallory\u{1b}[2J",
        "kind": "address",
        "address": "0x6666\u{202e}6666",
        "chains": ["base\u{7}"],
    }));
    fs::write(&path, serde_json::to_vec_pretty(&file).unwrap()).unwrap();
    let pw = format!("{PW}\n");
    for r in [
        home.run(&["whitelist", "list"]),
        home.run_stdin(&["whitelist", "list", "--verify", "--password-stdin"], &pw),
    ] {
        assert_eq!(r.code, 0, "{} / {}", r.stdout, r.stderr);
        for bad in ['\u{1b}', '\u{202e}', '\u{7}'] {
            assert!(
                !r.stdout.contains(bad) && !r.stderr.contains(bad),
                "{bad:?} reached the terminal: {:?}",
                r.stdout
            );
        }
        assert!(r.stdout.contains("mallory"), "{}", r.stdout);
    }
}

#[test]
fn m1_the_change_is_shown_before_the_password_is_read() {
    let _alone = common::exclusive();
    let home = Home::new();
    home.create("main", &[]);
    // Each change: what stderr shows (with --password-stdin) before the password is read.
    let shows = |args: &[&str], password: &str, want: &[&str]| {
        let r = whitelist_with(&home, args, password);
        let summary = r.stderr.clone();
        assert!(
            summary.contains("The password authorizes this whitelist change:"),
            "{args:?}: {summary}"
        );
        for w in want {
            assert!(summary.contains(w), "{args:?}: {w:?} not in {summary}");
        }
        assert!(
            !summary.contains(PW),
            "{args:?}: the password reached stderr"
        );
        r
    };
    // Shown even when the password is then wrong.
    shows(
        &["add", "alice", PAYEE, "--chains", "base"],
        "not the password",
        &[
            "add alice: a payee",
            PAYEE,
            "chains: base",
            "whitelist.json: none yet",
        ],
    )
    .err("WRONG_PASSWORD");
    shows(
        &[
            "add",
            "alice",
            &PAYEE.to_ascii_lowercase(),
            "--chains",
            "base",
        ],
        PW,
        &[
            PAYEE,
            "a new list from the built-in defaults with a new key, gen 1",
        ],
    )
    .ok();
    shows(
        &["add", "bob", BOB, "--chains", "*"],
        PW,
        &[
            "chains: every chain (*)",
            "now: gen 1, 1 entry (alice)",
            "if whitelist.json is the list wallet.json seals: the change lands on it, making gen 2",
            "otherwise: nothing changes",
        ],
    )
    .ok();
    shows(
        &[
            "add",
            "router",
            ROUTER,
            "--chains",
            "base,arbitrum",
            "--contract",
            "--selectors",
            "0x12345678,0xABCDEF01",
        ],
        PW,
        &[
            "add router: a contract",
            RESIDUAL,
            "chains: base, arbitrum",
            "selectors: 0x12345678, 0xabcdef01",
        ],
    )
    .ok();
    shows(
        &["fee-cap", "base", "0.001"],
        PW,
        &["fee-cap base: 0.0005 ETH (500000000000000 wei) -> 0.001 ETH (1000000000000000 wei)"],
    )
    .ok();
    shows(
        &["allow-message-signing", "on"],
        PW,
        &["allow-message-signing: off -> on"],
    )
    .ok();
    shows(
        &["remove", "bob"],
        PW,
        &[
            "remove bob: whitelist.json lists it as a payee",
            BOB,
            "every chain (*)",
        ],
    )
    .ok();
    // Values from the file are escaped: a hand-edited nickname cannot drive the terminal.
    let path = home.path().join("whitelist.json");
    let mut file: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    file["entries"].as_array_mut().unwrap().push(json!({
        "nickname": "mallory\u{1b}[2J",
        "kind": "address",
        "address": "0x6666666666666666666666666666666666666666",
        "chains": ["*"],
    }));
    fs::write(&path, serde_json::to_vec_pretty(&file).unwrap()).unwrap();
    let r = shows(
        &["reset"],
        PW,
        &[
            "reset: a new whitelist key and an empty list",
            "every entry is dropped: alice, router, \"mallory\\u{1b}[2J\"",
            "not verified yet",
        ],
    );
    assert!(!r.stderr.contains('\u{1b}'), "{:?}", r.stderr);
    r.ok();
}

#[test]
fn f2_create_unlock_shows_the_whitelist_its_session_signs_under() {
    let home = Home::new();
    let v = home.create("main", &["--unlock", "10m"]);
    assert_eq!(v["locked"], false, "{v}");
    assert_eq!(v["whitelist"]["verified"], false, "{v}");
    assert_eq!(v["whitelist"]["gen"], serde_json::Value::Null, "{v}");
    assert_eq!(v["whitelist"]["entries"], json!([]), "{v}");
    assert!(
        v["whitelist"]["reason"]
            .as_str()
            .unwrap_or("")
            .contains("no whitelist yet"),
        "{v}"
    );
}

/// Runs `whitelist <args> --json` on a terminal: returns what the terminal showed before the
/// password prompt, then lets `between` act, types the password, and returns the outcome.
fn on_terminal(home: &Home, args: &[&str], between: impl FnOnce()) -> (String, serde_json::Value) {
    let mut all = vec!["whitelist"];
    all.extend_from_slice(args);
    all.push("--json");
    let pty = common::PtyChild::spawn(home.command(&all));
    let mut before = String::new();
    let start = std::time::Instant::now();
    while !before.contains("Wallet password: ") {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "no prompt; saw {before:?}"
        );
        before.push_str(&common::drain_pty(&pty.master));
        std::thread::sleep(Duration::from_millis(20));
    }
    between();
    pty.type_line(PW);
    let out = pty.finish();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let v = serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}: {stdout:?}"));
    let shown = before[..before.find("Wallet password: ").unwrap()].to_string();
    (shown, v)
}

#[test]
fn c2_1_the_summary_says_what_happens_to_signing() {
    let _alone = common::exclusive();
    let home = Home::new();
    home.create("main", &[]);
    // Without --unlock: the wallet stays locked, said before the prompt.
    let (shown, v) = on_terminal(&home, &["add", "alice", PAYEE, "--chains", "base"], || {});
    assert!(
        shown.contains("after the change: the wallet stays locked"),
        "{shown:?}"
    );
    assert!(!shown.contains("signing session"), "{shown:?}");
    assert_eq!(v["locked"], true, "{v}");
    // With --unlock: the new session and when it ends, said before the prompt. The end shown is
    // now + 10m as of the summary: between `before` and the moment the prompt is seen.
    let before = now_secs();
    let seen = std::cell::Cell::new(0);
    let (shown, v) = on_terminal(
        &home,
        &["add", "bob", BOB, "--chains", "base", "--unlock", "10m"],
        || seen.set(now_secs()),
    );
    assert!(
        shown.contains("after the change: a new signing session for 10m (until"),
        "{shown:?}"
    );
    let at = shown
        .find("Z)")
        .unwrap_or_else(|| panic!("no UTC time in {shown:?}"));
    let until = zwallet::core::time::parse_rfc3339_utc(&shown[at - 19..=at])
        .unwrap_or_else(|| panic!("not a time: {:?}", &shown[at - 19..=at]));
    assert!(
        before + 600 <= until && until <= seen.get() + 600,
        "{before}+600 <= {until} <= {}+600",
        seen.get()
    );
    assert!(!shown.contains("stays locked"), "{shown:?}");
    assert_eq!(v["locked"], false, "{v}");
    // With --password-stdin the same lines go to stderr.
    let r = whitelist_edit(&home, &["remove", "bob"]);
    assert!(
        r.stderr
            .contains("after the change: the wallet stays locked"),
        "{}",
        r.stderr
    );
    r.ok();
    let r = whitelist_edit(&home, &["remove", "alice", "--unlock", "1h"]);
    assert!(
        r.stderr
            .contains("after the change: a new signing session for 1h (until"),
        "{}",
        r.stderr
    );
    r.ok();
}

#[test]
fn c2_2_the_change_lands_on_exactly_the_list_shown_or_nothing_changes() {
    let _alone = common::exclusive();
    let home = Home::new();
    home.create("main", &[]);
    whitelist_edit(&home, &["add", "alice", PAYEE, "--chains", "base"]).ok();
    whitelist_edit(&home, &["add", "bob", BOB, "--chains", "base"]).ok();
    let path = home.path().join("whitelist.json");
    let genuine = fs::read(&path).unwrap();
    // A running session: a refused change must leave it running (and so tell which check
    // refused: only the checks before the daemons are stopped can).
    let pid = home.unlock("10m")["daemonPid"].as_i64().unwrap() as i32;

    // The list shown is rewritten while the human types (the same signed content, other bytes):
    // the change is refused before anything is stopped or written.
    let before = files_of(&home);
    let compact = {
        let v: serde_json::Value = serde_json::from_slice(&genuine).unwrap();
        serde_json::to_vec(&v).unwrap()
    };
    assert_ne!(compact, genuine);
    let (_, v) = on_terminal(&home, &["remove", "bob"], || {
        fs::write(&path, &compact).unwrap();
    });
    assert_eq!(v["errorCode"], "VALIDATION_ERROR", "{v}");
    let msg = v["error"].as_str().unwrap();
    assert!(
        msg.contains("whitelist.json changed after it was shown; nothing in the whitelist changed"),
        "{msg}"
    );
    assert!(!msg.contains("stopped"), "{msg}");
    assert_eq!(v["locked"], serde_json::Value::Null, "{v}");
    assert_eq!(
        home.daemon_pids(),
        vec![pid],
        "a swap after the summary stopped the session"
    );
    fs::write(&path, &genuine).unwrap();
    assert_eq!(
        files_of(&home),
        before,
        "the refused change wrote something"
    );

    // A forged list is shown (bob moved to another address), the genuine one restored before the
    // password: the list shown does not verify, so nothing changes either.
    let mut forged: serde_json::Value = serde_json::from_slice(&genuine).unwrap();
    forged["entries"][1]["address"] = json!("0x6666666666666666666666666666666666666666");
    fs::write(&path, serde_json::to_vec_pretty(&forged).unwrap()).unwrap();
    let (shown, v) = on_terminal(&home, &["remove", "bob"], || {
        fs::write(&path, &genuine).unwrap();
    });
    assert!(
        shown.contains("0x6666666666666666666666666666666666666666"),
        "{shown:?}"
    );
    assert_eq!(v["errorCode"], "VALIDATION_ERROR", "{v}");
    let msg = v["error"].as_str().unwrap();
    assert!(
        msg.starts_with(
            "refusing to edit the whitelist: whitelist.json: whitelist signature does not verify"
        ),
        "{msg}"
    );
    assert!(msg.contains("nothing in the whitelist changed"), "{msg}");
    assert_eq!(
        home.daemon_pids(),
        vec![pid],
        "the forged list stopped the session"
    );
    assert_eq!(
        files_of(&home),
        before,
        "the refused change wrote something"
    );

    // An unfinished edit in whitelist.next.json is shown too (the same signed list, other bytes:
    // a byte-identical copy is labelled as a copy of whitelist.json instead).
    fs::write(home.path().join("whitelist.next.json"), &compact).unwrap();
    let r = whitelist_edit(&home, &["remove", "bob"]);
    assert!(
        r.stderr
            .contains("whitelist.next.json: gen 2, 2 entries (alice, bob), an unfinished edit"),
        "{}",
        r.stderr
    );
    let e = r.ok();
    assert_eq!(e["gen"], 3, "{e}");
    assert_eq!(nicknames(&e["entries"]), ["alice"], "{e}");
}

#[test]
fn r3_2_a_wallet_json_from_before_the_first_edit_refuses_every_edit() {
    let _alone = common::exclusive();
    let home = Home::new();
    home.create("main", &[]);
    let fresh = fs::read(home.wallet_json()).unwrap();
    whitelist_edit(&home, &["add", "alice", PAYEE, "--chains", "base"]).ok();
    whitelist_edit(&home, &["fee-cap", "base", "0.0001"]).ok();
    // The agent puts back the wallet.json from before the first edit: no whitelist is sealed,
    // but the real list is still on disk.
    fs::write(home.wallet_json(), &fresh).unwrap();
    let before = files_of(&home);
    for args in [
        &["add", "bob", BOB, "--chains", "base"][..],
        &["remove", "alice"],
        &["fee-cap", "base", "0.05"],
        &["allow-message-signing", "on"],
    ] {
        let r = whitelist_edit(&home, args);
        assert!(
            r.stderr.contains("otherwise: nothing changes"),
            "{args:?}: {}",
            r.stderr
        );
        let msg = r.err("VALIDATION_ERROR");
        for want in [
            "wallet.json seals no whitelist, but whitelist.json exists",
            "a wallet.json restored from before the first edit, or a planted file",
            "nothing in the whitelist changed",
            "zwallet whitelist reset",
            "sets every fee cap back to the built-in default",
        ] {
            assert!(msg.contains(want), "{args:?}: {want:?} not in {msg}");
        }
        assert_eq!(files_of(&home), before, "{args:?} wrote something");
    }
    // `list --verify` explains it the same way, instead of offering the add it refuses.
    let v = verify_list(&home);
    let reason = v["reason"].as_str().unwrap_or("");
    assert!(
        reason.contains("wallet.json seals no whitelist, but whitelist.json exists"),
        "{v}"
    );
    assert!(!reason.contains("creates it"), "{v}");
    // Reset still works: a new key, an empty list, the list it dropped shown.
    let z = whitelist_edit(&home, &["reset"]).ok();
    assert_eq!(
        (&z["gen"], &z["keyCreated"]),
        (&json!(1), &json!(true)),
        "{z}"
    );
    assert_eq!(z["entries"], json!([]), "{z}");
    assert_eq!(nicknames(&z["discarded"]["entries"]), ["alice"], "{z}");
    let v = verify_list(&home);
    assert_eq!(
        (&v["verified"], &v["gen"]),
        (&json!(true), &json!(1)),
        "{v}"
    );
}

/// The one recovery every refusal of an untrusted list gives, and nothing that suggests restoring
/// an older whitelist.json (round 8).
fn assert_recovery_only(text: &str) {
    assert!(
        text.contains(
            "If wallet.json is an older copy, restore the current one if you have it (zwallet \
             keeps no copy of it); otherwise `zwallet whitelist reset` starts over"
        ),
        "{text}"
    );
    assert!(!text.contains(&["put", "back"].join(" ")), "{text}");
    assert!(!text.contains(&["genuine", "copy"].join(" ")), "{text}");
}

#[test]
fn r4_2_a_wallet_json_rolled_back_under_a_newer_list_is_named() {
    let _alone = common::exclusive();
    let home = Home::new();
    home.create("main", &[]);
    whitelist_edit(&home, &["add", "alice", PAYEE, "--chains", "base"]).ok();
    let gen1 = fs::read(home.wallet_json()).unwrap();
    whitelist_edit(&home, &["remove", "alice"]).ok();
    let gen2 = fs::read(home.wallet_json()).unwrap();
    whitelist_edit(&home, &["add", "bob", BOB, "--chains", "base"]).ok();
    // wallet.json alone restored two edits back: the genuine gen-3 list can only mean a
    // rollback.
    fs::write(home.wallet_json(), &gen1).unwrap();
    let rolled = "whitelist.json (gen 3) is newer than the seal (gen 1): wallet.json was rolled \
                  back";
    let v = verify_list(&home);
    assert_eq!(v["verified"], false, "{v}");
    let reason = v["reason"].as_str().unwrap_or("");
    assert!(reason.contains(rolled), "{v}");
    assert_recovery_only(reason);
    let before = files_of(&home);
    let msg = whitelist_edit(&home, &["add", "carol", ROUTER, "--chains", "base"])
        .err("VALIDATION_ERROR");
    assert!(msg.contains(rolled), "{msg}");
    assert_recovery_only(&msg);
    assert_eq!(files_of(&home), before);
    // The daemon refuses with the same reason.
    home.unlock("10m");
    let r = home.run(&["sign-message", "--message", "m", "--json"]);
    let msg = r.err("WHITELIST_INVALID");
    assert!(msg.contains(rolled), "{msg}");
    home.run(&["lock", "--json"]).ok();
    // Restored one edit back: that, or an edit to gen 3 that never finished sealing. Both are
    // named; the advice is the same one recovery.
    fs::write(home.wallet_json(), &gen2).unwrap();
    let v = verify_list(&home);
    let reason = v["reason"].as_str().unwrap_or("");
    assert!(
        reason.contains(
            "whitelist.json (gen 3) is one edit newer than the seal (gen 2), under this wallet's \
             whitelist key: an edit that never finished sealing, or a rolled-back wallet.json"
        ),
        "{v}"
    );
    assert_recovery_only(reason);
}

#[test]
fn r5_1_a_rollback_across_a_reset_is_named_and_no_copy_is_offered() {
    let _alone = common::exclusive();
    let home = Home::new();
    home.create("main", &[]);
    whitelist_edit(&home, &["add", "alice", PAYEE, "--chains", "base"]).ok();
    let gen1 = fs::read(home.wallet_json()).unwrap();
    whitelist_edit(&home, &["reset"]).ok();
    // wallet.json restored to before the reset: the list is the new key's, at gen 2.
    fs::write(home.wallet_json(), &gen1).unwrap();
    let why = "whitelist.json (gen 2) is newer than the seal (gen 1) but not signed by this \
               wallet's whitelist key: a wallet.json rolled back across a reset, a reset that \
               never finished sealing, or a foreign file";
    let v = verify_list(&home);
    let reason = v["reason"].as_str().unwrap_or("");
    assert!(reason.contains(why), "{v}");
    assert_recovery_only(reason);
    let before = files_of(&home);
    let msg =
        whitelist_edit(&home, &["add", "bob", BOB, "--chains", "base"]).err("VALIDATION_ERROR");
    assert!(msg.contains(why), "{msg}");
    assert_recovery_only(&msg);
    assert_eq!(files_of(&home), before);
    // A list at the sealed generation that does not verify: the same one recovery.
    fs::write(home.wallet_json(), &gen1).unwrap();
    fs::write(home.path().join("whitelist.json"), b"{ not a list").unwrap();
    let v = verify_list(&home);
    assert_recovery_only(v["reason"].as_str().unwrap_or(""));
}

#[test]
fn r5_a_first_edit_cut_off_before_its_seal_is_named_by_list_and_list_verify() {
    use zwallet::chains::evm::registry::WHITELIST_RULES;
    use zwallet::core::error::WalletError;
    use zwallet::core::whitelist::{self, EditStage, Entry};

    let home = Home::new();
    home.create("main", &[]);
    // The first edit, stopped right after staging (in this process, through the core edit's
    // test hook): only whitelist.next.json, at gen 1, and no seal.
    let crash = |s: EditStage<'_>| -> zwallet::core::error::Result<()> {
        match s {
            EditStage::NextWritten(_) => Err(WalletError::validation("crash")),
            _ => Ok(()),
        }
    };
    whitelist::edit_with_hook(
        home.path(),
        common::PW,
        &WHITELIST_RULES,
        |l| l.add(Entry::new_address("alice", PAYEE, &["base"])),
        &crash,
    )
    .unwrap_err();
    assert!(!home.path().join("whitelist.json").exists());
    assert!(home.path().join("whitelist.next.json").exists());
    let v = verify_list(&home);
    let reason = v["reason"].as_str().unwrap_or("");
    assert!(
        reason.contains(
            "wallet.json seals no whitelist, but whitelist.next.json exists (a first edit \
             interrupted before it was sealed"
        ),
        "{v}"
    );
    assert!(!reason.contains("creates it"), "{v}");
    let plain = home.run(&["whitelist", "list", "--json"]).ok();
    let reason = plain["reason"].as_str().unwrap_or("");
    assert!(
        reason.contains("there is no whitelist.json, but whitelist.next.json exists"),
        "{plain}"
    );
    assert!(!reason.contains("creates it"), "{plain}");
}

/// The value of the `<name>: ` line in human output.
fn field<'a>(text: &'a str, name: &str) -> &'a str {
    let prefix = format!("{name}: ");
    text.lines()
        .find_map(|l| l.strip_prefix(&prefix))
        .unwrap_or_else(|| panic!("no {name} line in {text:?}"))
}

/// The line above the QR code of an address that holds on every network.
const EVERY_NETWORK: &str = "(the same address on every network `zwallet chains` lists)";

#[test]
fn qr1_address_verify_on_a_terminal_shows_the_address_as_a_qr_code() {
    let home = Home::new();
    let addr = home.create("main", &[])["address"]
        .as_str()
        .unwrap()
        .to_string();
    let prompt = ["Wallet password: "];
    let piped = home.run_stdin(
        &["address", "--verify", "--password-stdin"],
        &format!("{PW}\n"),
    );
    let shown = home.run_on_terminal(&["address", "--verify"], &prompt, &[]);
    // The fields a pipe gets, then a line repeating the address, then the code, in black on
    // white; a phone scanning it reads the address. Verified: no note.
    let (fields, code) = shown
        .split_once(&format!("scan to send to {addr} on ethereum\n"))
        .unwrap_or_else(|| panic!("{shown:?}"));
    assert_eq!(fields, piped.stdout);
    assert!(field(fields, "verified") == "true", "{shown:?}");
    assert!(code.lines().all(|l| l.starts_with('\x1b')), "{shown:?}");
    assert_eq!(common::scan_qr(&shown), [addr.clone()]);
    assert!(!shown.contains("not verified"), "{shown:?}");
    // NO_COLOR: no escape sequences, the light modules drawn for a dark terminal; it scans the
    // same.
    let plain = home.run_on_terminal(&["address", "--verify"], &prompt, &[("NO_COLOR", "1")]);
    assert!(!plain.contains('\x1b'), "{plain:?}");
    assert_eq!(common::scan_qr(&plain), [addr.clone()]);
    // Another chain names that chain.
    let base = home.run_on_terminal(&["address", "--verify", "--chain", "base"], &prompt, &[]);
    assert!(
        base.contains(&format!("scan to send to {addr} on base\n")),
        "{base:?}"
    );
    assert_eq!(common::scan_qr(&base), [addr.clone()]);
    // --no-qr: what a pipe gets. --json: the JSON only.
    let quiet = home.run_on_terminal(&["address", "--verify", "--no-qr"], &prompt, &[]);
    assert_eq!(quiet, piped.stdout);
    let json = home.run_on_terminal(&["address", "--verify", "--json"], &prompt, &[]);
    let v: serde_json::Value =
        serde_json::from_str(&json).unwrap_or_else(|e| panic!("{e}: {json:?}"));
    assert_eq!(
        (&v["address"], &v["verified"]),
        (&json!(addr), &json!(true))
    );
}

#[test]
fn qr2_unlock_and_create_on_a_terminal_show_the_verified_address_as_a_qr_code() {
    let home = Home::new();
    // The first create: the address comes from the seed it just made.
    let created = home.run_on_terminal(
        &["create", "main"],
        &["New wallet password: ", "Repeat password: "],
        &[],
    );
    let addr = field(&created, "address").to_string();
    assert_eq!(field(&created, "verified"), "true");
    assert!(
        created.contains(&format!("scan to send to {addr} {EVERY_NETWORK}\n")),
        "{created:?}"
    );
    assert_eq!(common::scan_qr(&created), [addr.clone()]);

    // unlock: the address its daemon derived from the seed, the code under it.
    let unlocked = home.run_on_terminal(&["unlock", "10m"], &["Wallet password: "], &[]);
    assert_eq!(field(&unlocked, "address"), addr);
    assert_eq!(field(&unlocked, "verified"), "true");
    let label = format!("scan to send to {addr} {EVERY_NETWORK}\n");
    let at = unlocked
        .find(&label)
        .unwrap_or_else(|| panic!("{unlocked:?}"));
    assert!(at > unlocked.find("address: ").unwrap(), "{unlocked:?}");
    assert_eq!(common::scan_qr(&unlocked), [addr.clone()]);
    let quiet = home.run_on_terminal(&["unlock", "10m", "--no-qr"], &["Wallet password: "], &[]);
    assert!(!quiet.contains("scan to send"), "{quiet:?}");
    assert!(common::scan_qr(&quiet).is_empty() && !quiet.contains('\x1b'));
    let json = home.run_on_terminal(&["unlock", "10m", "--json"], &["Wallet password: "], &[]);
    let v: serde_json::Value =
        serde_json::from_str(&json).unwrap_or_else(|e| panic!("{e}: {json:?}"));
    assert_eq!(v["address"], addr.as_str());

    // create --unlock: a new key from the seed the password opened, then its session.
    let sub = home.run_on_terminal(
        &["create", "sub", "--unlock", "10m"],
        &["Wallet password: "],
        &[],
    );
    let sub_addr = field(&sub, "address").to_string();
    assert_ne!(sub_addr, addr);
    assert_eq!(common::scan_qr(&sub), [sub_addr]);
    let quiet = home.run_on_terminal(
        &["create", "sub2", "--unlock", "10m", "--no-qr"],
        &["Wallet password: "],
        &[],
    );
    assert!(common::scan_qr(&quiet).is_empty() && !quiet.contains("scan to send"));

    // An agent's create while unlocked (stdin no terminal): derived by whatever daemon answers,
    // so not verified, and no code even with stdout on a terminal.
    let (master, slave) = common::openpty();
    let mut cmd = home.command(&["create", "agent"]);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(slave.try_clone().unwrap()))
        .stderr(std::process::Stdio::piped());
    let out = cmd.output().unwrap();
    drop(cmd);
    let shown = common::drain_pty(&master).replace("\r\n", "\n");
    drop(slave);
    assert!(out.status.success(), "{:?}", out.stderr);
    assert_eq!(field(&shown, "verified"), "false", "{shown:?}");
    assert!(shown.contains(NOT_VERIFIED), "{shown:?}");
    assert!(!shown.contains("scan to send") && common::scan_qr(&shown).is_empty());
}

#[test]
fn qr3_unverified_addresses_on_a_terminal_print_as_on_a_pipe() {
    let home = Home::new();
    home.create("main", &[]);
    let commands: [&[&str]; 3] = [
        &["address"],
        &["address", "--chain", "base"],
        &["addresses"],
    ];
    for unlocked in [false, true] {
        if unlocked {
            // The daemon cross-checks the cache now; still not verified.
            home.unlock("10m");
        }
        for args in commands {
            let piped = home.run(args);
            assert_eq!(piped.code, 0, "{args:?}: {}", piped.stderr);
            let shown = home.run_on_terminal(args, &[], &[]);
            assert_eq!(shown, piped.stdout, "{args:?} unlocked {unlocked}");
            assert!(shown.contains(NOT_VERIFIED), "{shown:?}");
        }
    }
}
