//! Offline integration tests through the built binary. No network: anything that would need
//! RPC is either rejected before it (validation, WALLET_LOCKED) or pointed at a closed port.

mod common;

use common::{
    DaemonChild, FakeDaemon, Home, KillOnDrop, PW, build_attacker, daemon_argv_impostor, have,
    pid_alive, run_in, wait_dead, wait_until_answering,
};
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
    // One address on every EVM chain; Tron's is its own (`T...`).
    for r in rows {
        if r["chainId"] == "tron" {
            assert!(r["address"].as_str().unwrap().starts_with('T'), "{r}");
        } else {
            assert_eq!(r["address"], address.as_str(), "{r}");
        }
    }
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
    // Tron after Avalanche, Sepolia last; its chain id is the genesis id's last 4 bytes.
    let keys: Vec<&str> = rows.iter().map(|r| r["key"].as_str().unwrap()).collect();
    assert_eq!(
        keys,
        [
            "ethereum",
            "base",
            "arbitrum",
            "optimism",
            "polygon",
            "bsc",
            "avalanche",
            "tron",
            "sepolia"
        ]
    );
    let tron = rows.iter().find(|r| r["key"] == "tron").unwrap();
    assert_eq!(
        tron,
        &json!({
            "key": "tron", "name": "Tron", "family": "tron", "symbol": "TRX",
            "chainId": 728126428, "rpc": "https://tron-rpc.publicnode.com",
        })
    );
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
/// The same on Tron: index 1 of the test mnemonic at `m/44'/195'/0'/0/1`.
const TRON_POISON: &str = "TPjjvMwjPoDC32V2dGDYTkLH4E5LAtBZ6C";

/// Every cached address of key 0 replaced with a valid address of its family that is not this
/// wallet's.
fn poison_cache(home: &Home) {
    home.edit_wallet(|v| {
        for (chain, a) in v["keys"][0]["addresses"].as_object_mut().unwrap() {
            let poison = if chain == "tron" { TRON_POISON } else { POISON };
            *a = serde_json::json!(poison);
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
    // `balance` too: while a session is live it reads the daemon's derivation, not the cache
    // alone. Every network points at a closed port, so nothing would be read anyway.
    let closed: Vec<(String, String)> = chains::CHAINS
        .iter()
        .map(|c| (c.rpc_env_var(), "http://127.0.0.1:1".to_string()))
        .collect();
    let closed: Vec<(&str, &str)> = closed
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    for args in [
        &["address", "--json"][..],
        &["addresses", "--json"],
        &["list", "--json"],
        &["status", "--json"],
        &["change", "main", "--json"],
        &["balance", "--json"],
        &["balance", "--chain", "base", "--json"],
    ] {
        let msg = home.run_env(args, &closed).err("VALIDATION_ERROR");
        assert!(msg.contains(genuine_daemon_mismatch()), "{args:?}: {msg}");
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
        // Both sides floor to whole seconds, so only a wall-clock step back makes the daemon's
        // expiry earlier than `before + secs`: up to 2 s of one is tolerated. Seen once, on
        // 2026-10-28 under load: 2026-10-28T15:09:12Z against an expected 15:09:13Z. The
        // duration arithmetic itself is exact (`parses_all_units`, src/core/time.rs).
        let lo = zwallet::core::time::rfc3339_utc(before + secs - 2);
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
    common::executable::copy_executable(env!("CARGO_BIN_EXE_zwallet"), &exe).unwrap();
    let captured = home.path().join("captured");
    let mut cmd = common::zwallet_at(&exe);
    cmd.env("ZWALLET_HOME", home.path())
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

/// The note on an address shown unverified while a key daemon answers.
const NOT_VERIFIED: &str = "not verified: run `zwallet address --verify` before funding";

/// The note on an address shown while the wallet is locked.
const LOCKED_NOTE: &str = "not verified: the wallet is locked; `zwallet unlock <dur>` or `zwallet address --verify` \
     checks it";

/// Whether a live session verifies an address on this OS. On macOS the CLI asks the kernel for
/// the code-signing hash (cdhash) of the code the process answering on `d.sock` runs, through
/// that connection's audit token, and compares it with its own. On Linux every zwallet
/// process is non-dumpable, which hides its `/proc/<pid>/exe` from its own user, so the key
/// daemon's executable cannot be checked and nothing is verified by session.
///
/// The tests' zwallet is signed on every Mac (the harness signs an x86_64 build, which the linker
/// does not), so this holds on x86_64 macOS too.
fn session_verifies() -> bool {
    cfg!(target_os = "macos")
}

/// Whether the test process itself (the one that hosts a [`FakeDaemon`]) is named when it answers
/// on `d.sock`. On arm64 macOS the linker signed this test binary, so `check_peer` reads its
/// cdhash and names it ([`TEST_PROCESS_NAMED`]); on Linux its readable executable names it. On an
/// x86_64 Mac this test binary is unsigned: `csops` has no cdhash for it, `check_peer` returns
/// `Unknown`, and the commands neither name it nor verify by it. The harness signs the zwallet
/// binary, but a running test binary cannot sign itself. (`cc`-built attackers are signed by
/// [`build_attacker`], so they are named on every Mac.)
fn names_the_test_process() -> bool {
    cfg!(any(
        target_os = "linux",
        all(target_os = "macos", target_arch = "aarch64")
    ))
}

/// What names a peer that runs other code: its cdhash on macOS, its executable on Linux.
const TEST_PROCESS_NAMED: &str = if cfg!(target_os = "macos") {
    "runs other code than this zwallet build"
} else {
    "is not this zwallet build"
};

/// What a cache-vs-daemon mismatch says while this build's key daemon answers: on macOS the CLI
/// knows it is this build's, so its derivation is the seed's and the cache is to blame; on Linux
/// it cannot tell, so either side may be the tampered one.
fn genuine_daemon_mismatch() -> &'static str {
    if session_verifies() {
        "the cache was modified"
    } else {
        "one of them was tampered with"
    }
}

/// The commands that show an address without the human's password.
const ADDRESS_VIEWS: [&[&str]; 4] = [
    &["address", "--json"],
    &["addresses", "--json"],
    &["list", "--json"],
    &["status", "--json"],
];

/// The commands that show an address verified by a live session (where [`session_verifies`]).
const SESSION_VIEWS: [&[&str]; 5] = [
    &["address", "--json"],
    &["address", "--chain", "base", "--json"],
    &["addresses", "--json"],
    &["status", "--json"],
    &["change", "main", "--json"],
];

/// The address a result shows: `address`, or the first row of `addresses`.
fn shown_address(v: &serde_json::Value) -> &str {
    v["address"]
        .as_str()
        .or_else(|| v["addresses"][0]["address"].as_str())
        .unwrap_or_else(|| panic!("no address in {v}"))
}

/// The residual risk the README names: a same-user process swaps in its own wallet.json and
/// unlocks it with its own password through a genuine zwallet daemon. Where a session verifies,
/// that daemon's address is reported verified by session; only the address the human saved from
/// their own first unlock tells them apart. The human's password never opens it.
#[test]
fn k1_swapped_wallet_with_a_genuine_attacker_daemon_is_the_named_residual_risk() {
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
    for args in SESSION_VIEWS {
        let v = home.run(args).ok();
        assert_eq!(v["verified"], session_verifies(), "{args:?}: {v}");
        assert_eq!(shown_address(&v), evil, "{args:?}: {v}");
    }
    // `list` is never verified without the password.
    assert_eq!(home.run(&["list", "--json"]).ok()["verified"], false);
    let human_out = home.run(&["address"]);
    assert_eq!(
        human_out.stdout.contains(NOT_VERIFIED),
        !session_verifies(),
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
    // It read the cache and repeats it (each chain family's), so the daemon cross-check finds
    // no mismatch.
    let tron = derived_tron(&home);
    let fake = FakeDaemon::answering_per_family(
        &home.sock(),
        serde_json::json!({
            "locked": false, "pid": 1, "expiresAt": "2099-01-01T00:00:00Z",
            "chain": "ethereum", "index": 0, "active": "main", "address": addr,
        }),
        &tron,
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
    assert_eq!(
        (&created["verified"], &created["verifiedBy"]),
        (&json!(true), &json!("password")),
        "first create derives in-process"
    );
    let addr = created["address"].as_str().unwrap().to_string();
    let pw = format!("{PW}\n");

    let v = home
        .run_stdin(&["address", "--verify", "--password-stdin", "--json"], &pw)
        .ok();
    assert_eq!(
        (&v["verified"], &v["verifiedBy"], &v["address"]),
        (&json!(true), &json!("password"), &json!(addr))
    );
    let human = home.run_stdin(&["address", "--verify", "--password-stdin"], &pw);
    assert!(!human.stdout.contains("not verified"), "{}", human.stdout);
    let l = home
        .run_stdin(&["list", "--verify", "--password-stdin", "--json"], &pw)
        .ok();
    assert_eq!(
        (&l["verified"], &l["verifiedBy"]),
        (&json!(true), &json!("password"))
    );
    assert_eq!(l["keys"][0]["address"], addr.as_str());

    // unlock reports what its own child derived after the password: verified by the password.
    let unlocked = home.unlock("30s");
    assert_eq!(
        (&unlocked["verified"], &unlocked["verifiedBy"]),
        (&json!(true), &json!("password"))
    );
    // Without --verify, while unlocked: derived by this build's key daemon of this home, so
    // verified by the session where the OS lets the CLI check that (see `session_verifies`).
    let v = home.run(&["address", "--json"]).ok();
    assert_eq!(v["verified"], session_verifies(), "{v}");
    // balance asks the daemon too, rather than trusting the cache alone.
    let rpc = common::tiny_rpc(|method| match method {
        "eth_chainId" => json!("0x1"),
        _ => json!("0x0"),
    });
    let b = home
        .run(&["balance", "--chain", "ethereum", "--rpc", &rpc, "--json"])
        .ok();
    assert_eq!(
        (&b["verified"], &b["address"]),
        (&json!(session_verifies()), &json!(addr))
    );
    home.stop_daemons();

    // A poisoned cache: `list --verify` refuses and says how to repair it (`address --verify`
    // repairs it, see `address_verify_repairs_a_tampered_cache_and_unlock_works_again`); a wrong
    // password and a missing terminal still fail.
    poison_cache(&home);
    let msg = home
        .run_stdin(&["list", "--verify", "--password-stdin", "--json"], &pw)
        .err("VALIDATION_ERROR");
    assert!(
        msg.contains("does not match the master seed")
            && msg.contains("run `zwallet address --verify`"),
        "{msg}"
    );
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
        let mut cmd = common::zwallet_at(env!("CARGO_BIN_EXE_zwallet"));
        cmd.env("ZWALLET_HOME", home.path())
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
        "a zwallet daemon of a test home survived lock"
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

/// A wallet home outside the suite's test homes (`/tmp/zwx-*`, not `/tmp/zwt-*`), standing for
/// the user's real one. Its own commands are scoped to the outside homes: one scoped to the test
/// homes cannot see this home's daemon, and `unlock` checks that it can. Its daemons are stopped
/// when it drops, this home's only.
struct Outside(tempfile::TempDir);

const OUTSIDE_HOMES: &str = "/tmp/zwx-";

impl Outside {
    fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("zwx-")
            .tempdir_in("/tmp")
            .expect("outside home");
        Self(dir)
    }

    fn path(&self) -> &std::path::Path {
        self.0.path()
    }

    fn run_stdin(&self, args: &[&str]) -> common::Run {
        run_in(
            self.path(),
            args,
            Some(&format!("{PW}\n")),
            &[(zwallet::core::protocol::TEST_SCOPE_ENV, OUTSIDE_HOMES)],
        )
    }

    /// Unlocks it for 10 minutes and returns its daemon's pid: long enough that a slow run never
    /// sees it expire (its drop stops it).
    fn unlock(&self) -> i32 {
        self.run_stdin(&["unlock", "10m", "--password-stdin", "--json"])
            .ok()["daemonPid"]
            .as_i64()
            .unwrap() as i32
    }

    /// Its daemons, by the process table, among those of the outside homes.
    fn daemon_pids(&self) -> Vec<i32> {
        zwallet::core::protocol::home_daemon_pids_under(
            std::path::Path::new(OUTSIDE_HOMES),
            self.path(),
        )
        .expect("process table")
    }
}

impl Drop for Outside {
    fn drop(&mut self) {
        let _ = zwallet::core::protocol::stop_daemons_under(
            std::path::Path::new(OUTSIDE_HOMES),
            self.path(),
        );
    }
}

#[test]
fn the_suite_stops_no_daemon_outside_its_test_homes() {
    // `lock` and a whitelist edit stop every daemon in their reach. Run by the suite, that is the
    // daemons of its test homes, never the user's own session.
    let _alone = common::exclusive();
    let outside = Outside::new();
    outside
        .run_stdin(&["create", "main", "--password-stdin", "--json"])
        .ok();
    let home = Home::new();
    home.create("main", &[]);
    let mut failures = Vec::new();
    for what in ["lock", "whitelist add"] {
        let kept = outside.unlock();
        let ended = home.unlock("30s")["daemonPid"].as_i64().unwrap() as i32;
        let (r, field) = match what {
            "lock" => (home.run(&["lock", "--json"]), "stopped"),
            _ => (
                whitelist_edit(&home, &["add", "alice", PAYEE]),
                "stoppedDaemons",
            ),
        };
        let v: serde_json::Value = serde_json::from_str(&r.stdout).unwrap_or_default();
        let stopped = |pid: i32| v[field].as_array().is_some_and(|s| s.contains(&json!(pid)));
        if r.code != 0 || !stopped(ended) || !wait_dead(ended, Duration::from_secs(3)) {
            failures.push(format!(
                "{what} did not stop the daemon {ended} of its test home: {:?} / {:?}",
                r.stdout.trim(),
                r.stderr.trim()
            ));
        }
        // Which of the three happened: an outside daemon that merely expired is not alive, but
        // not listed as stopped.
        let mut seen = Vec::new();
        if stopped(kept) {
            seen.push(format!("{what} listed it as stopped"));
        }
        if !pid_alive(kept) {
            seen.push("it is not alive".to_string());
        }
        if !outside.daemon_pids().contains(&kept) {
            seen.push("the process table no longer lists it for its home".to_string());
        }
        if !seen.is_empty() {
            failures.push(format!(
                "{what}: the daemon {kept} of {}, outside the test homes: {}",
                outside.path().display(),
                seen.join("; ")
            ));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
#[should_panic(expected = "a release zwallet ignores ZWALLET_TEST_DAEMON_SCOPE")]
fn a_release_profile_run_starts_no_zwallet() {
    // `common::zwallet_at` builds every zwallet the tests start, through this launcher with
    // `cfg!(debug_assertions)`: under `cargo test --release` that is false, and it panics before
    // it builds the command. Nothing here spawns it: a launcher that returned fails this test
    // with nothing started.
    let _unspawned = common::zwallet_at_with(env!("CARGO_BIN_EXE_zwallet"), false);
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
    // The key is derived from the password at every edit: no key file, no key flag.
    assert!(added.get("keyCreated").is_none(), "{added}");
    assert!(!home.path().join("whitelist.key").exists());
    assert_eq!(added["change"], "added", "{added}");
    assert_eq!(
        added["entry"],
        json!({
            "nickname": "alice",
            "kind": "address",
            "address": PAYEE,
            "family": "evm",
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
    assert_eq!(mode(&home.path().join("whitelist.json")), 0o600);
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
    assert!(b.get("keyCreated").is_none(), "{b}");
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
            "family": "evm",
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

    // reset: a new key (a new sealed salt) and an empty list with the default policy, at the
    // next generation.
    let sealed = || {
        store::unlock(home.path(), PW, &chains::derive_addresses)
            .unwrap()
            .0
            .policy
    };
    let before = sealed();
    let z = whitelist_edit(&home, &["reset"]).ok();
    assert_eq!(z["gen"], 7, "{z}");
    assert_eq!(z["change"], "reset", "{z}");
    assert_eq!(z["entries"], json!([]), "{z}");
    assert_eq!(nicknames(&z["dropped"]), ["alice", "router"], "{z}");
    let after = sealed();
    for key in ["whitelistSalt", "whitelistPub"] {
        assert!(after[key].is_string(), "{after}");
        assert_ne!(after[key], before[key], "{key}");
    }
    assert!(!home.path().join("whitelist.key").exists());
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
        (&z["gen"], &z["change"]),
        (&json!(2), &json!("reset")),
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
        // The hand-edited entry is no nickname, address or chain `add` would write: each field
        // shows as `<invalid>`, never its own text; the genuine entry shows as it is.
        assert!(
            r.stdout.contains("<invalid>") && r.stdout.contains("alice"),
            "{}",
            r.stdout
        );
    }
    // `--json` keeps every entry (no trimming) but escapes each string: the bidi override and the
    // bell in the hand-edited entry come out as `\u{...}`, so every printed line is ASCII.
    let r = home.run(&["whitelist", "list", "--json"]);
    assert_eq!(r.code, 0, "{}", r.stdout);
    assert!(inert_output(&r.stdout), "{}", r.stdout);
    let v = r.json();
    assert_eq!(v["entries"].as_array().unwrap().len(), 2, "{v}");
    let e = &v["entries"][1];
    assert_eq!(e["nickname"], json!("mallory\\u{1b}[2J"), "{v}");
    assert_eq!(e["address"], json!("0x6666\\u{202e}6666"), "{v}");
    assert_eq!(e["chains"][0], json!("base\\u{7}"), "{v}");
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
            "chains: every EVM chain (*)",
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
            "every EVM chain (*)",
        ],
    )
    .ok();
    // A hand-edited nickname that is not one `add` writes makes the file no list: the summary
    // says it cannot be read, names the field, and never shows the value.
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
            "every entry is dropped: whitelist.json is there but cannot be read, so its entries \
             are unknown",
            "not a whitelist file: the nickname of entry 3 is not in the shape zwallet writes",
        ],
    );
    assert!(
        !r.stderr.contains('\u{1b}') && !r.stderr.contains("mallory"),
        "{:?}",
        r.stderr
    );
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
        (&z["gen"], &z["change"]),
        (&json!(1), &json!("reset")),
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

#[test]
fn a_whitelist_sealed_by_zwallet_0_1_0_is_refused_before_anything_stops_and_reset_starts_over() {
    let _alone = common::exclusive();
    let home = Home::new();
    home.create("main", &[]);
    whitelist_edit(&home, &["add", "alice", PAYEE, "--chains", "base"]).ok();
    // zwallet 0.1.0 sealed no salt (its key sat in whitelist.key, which nothing reads now).
    let lock = store::MetaLock::acquire(home.path()).unwrap();
    let opened = lock.open(PW).unwrap();
    let mut policy = opened.unsealed().policy.clone();
    policy.as_object_mut().unwrap().remove("whitelistSalt");
    lock.reseal(&opened, &policy).unwrap();
    drop(lock);
    let stray = home.path().join("whitelist.key");
    fs::write(&stray, b"{\"version\":1}\n").unwrap();
    let made_by = "this whitelist was made by zwallet 0.1.0, whose key file is no longer used";
    let restored = "or a wallet.json restored from before the upgrade";

    // `list --verify` names the cause, the other way it comes about, and the one recovery.
    let v = verify_list(&home);
    assert_eq!(v["verified"], false, "{v}");
    let reason = v["reason"].as_str().unwrap_or("");
    assert!(reason.contains(made_by), "{v}");
    assert!(reason.contains(restored), "{v}");
    assert_recovery_only(reason);
    // W18: the daemon's view is untrusted, with the cause alone (no remedy for an agent).
    let unlocked = home.unlock("10m");
    let w = &unlocked["whitelist"];
    assert_eq!(
        (&w["verified"], &w["entries"]),
        (&json!(false), &json!([])),
        "{unlocked}"
    );
    assert_eq!(w["reason"], made_by, "{unlocked}");
    let pid = unlocked["daemonPid"].as_i64().unwrap() as i32;
    // Every edit refuses before any daemon is stopped, and writes nothing.
    let before = files_of(&home);
    for args in [
        &["add", "bob", BOB, "--chains", "base"][..],
        &["remove", "alice"],
    ] {
        let msg = whitelist_edit(&home, args).err("VALIDATION_ERROR");
        assert!(msg.contains(made_by), "{args:?}: {msg}");
        assert!(msg.contains(restored), "{args:?}: {msg}");
        assert_recovery_only(&msg);
        assert!(!msg.contains("already stopped"), "{args:?}: {msg}");
        assert_eq!(files_of(&home), before, "{args:?} wrote something");
    }
    assert_eq!(home.daemon_pids(), vec![pid], "a refused edit stopped it");
    // Reset starts over under a new salt; the stray key file is left as it was.
    let z = whitelist_edit(&home, &["reset"]).ok();
    assert_eq!(
        (&z["gen"], &z["change"]),
        (&json!(2), &json!("reset")),
        "{z}"
    );
    assert!(z.get("keyCreated").is_none(), "{z}");
    let e = whitelist_edit(&home, &["add", "bob", BOB, "--chains", "base"]).ok();
    assert_eq!(e["gen"], 3, "{e}");
    let v = verify_list(&home);
    assert_eq!(
        (&v["verified"], &v["gen"]),
        (&json!(true), &json!(3)),
        "{v}"
    );
    assert_eq!(fs::read(&stray).unwrap(), b"{\"version\":1}\n");
}

/// The value of the `<name>: ` line in human output.
fn field<'a>(text: &'a str, name: &str) -> &'a str {
    let prefix = format!("{name}: ");
    text.lines()
        .find_map(|l| l.strip_prefix(&prefix))
        .unwrap_or_else(|| panic!("no {name} line in {text:?}"))
}

/// What the first `create` on a terminal shows where the QR code would be.
const BACKUP_FIRST: &str =
    "QR after backup: run `zwallet backup`, then `zwallet address --verify` (or unlock) to show it";

/// The line above the QR code of an address that holds on every network.
const EVERY_NETWORK: &str = "(the same address on every EVM network in `zwallet chains`)";

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
    // The fields a pipe gets, then two lines repeating the address, then the code, in black on
    // white; a phone scanning it reads the address. Verified: no note. All of it fits an
    // 80-column terminal.
    let (fields, code) = shown
        .split_once(&format!("scan to send to {addr}\n{EVERY_NETWORK}\n"))
        .unwrap_or_else(|| panic!("{shown:?}"));
    assert_eq!(fields, piped.stdout);
    for line in shown.lines() {
        assert!(common::strip_ansi(line).chars().count() <= 80, "{line:?}");
    }
    assert!(field(fields, "verified") == "true", "{shown:?}");
    assert!(code.lines().all(|l| l.starts_with('\x1b')), "{shown:?}");
    assert_eq!(common::scan_qr(&shown), [addr.as_str()]);
    assert!(!shown.contains("not verified"), "{shown:?}");
    // NO_COLOR: no escape sequences, the light modules drawn for a dark terminal; it scans the
    // same.
    let plain = home.run_on_terminal(&["address", "--verify"], &prompt, &[("NO_COLOR", "1")]);
    assert!(!plain.contains('\x1b'), "{plain:?}");
    assert_eq!(common::scan_qr(&plain), [addr.as_str()]);
    // Another chain: the code carries no chain, and the line says so.
    let base = home.run_on_terminal(&["address", "--verify", "--chain", "base"], &prompt, &[]);
    assert!(
        base.contains(&format!("scan to send to {addr}\n{EVERY_NETWORK}\n")),
        "{base:?}"
    );
    assert_eq!(common::scan_qr(&base), [addr.as_str()]);
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
    let new_seed = ["New wallet password: ", "Repeat password: "];
    let home = Home::new();
    // The first create makes the seed (its address is verified), but its recovery phrase is not
    // backed up yet: no code to fund it from, only a line saying how to get one.
    let created = home.run_on_terminal(&["create", "main"], &new_seed, &[]);
    let addr = field(&created, "address").to_string();
    assert_eq!(field(&created, "verified"), "true");
    assert!(
        created.ends_with(&format!(
            "verified: true\nverifiedBy: password\n{BACKUP_FIRST}\n"
        )),
        "{created:?}"
    );
    assert!(!created.contains("scan to send") && !created.contains('\x1b'));
    assert!(common::scan_qr(&created).is_empty(), "{created:?}");
    // The same with --unlock; --no-qr leaves the line out too.
    let other = Home::new();
    let shown = other.run_on_terminal(&["create", "main", "--unlock", "10m"], &new_seed, &[]);
    assert!(shown.ends_with(&format!("\n{BACKUP_FIRST}\n")), "{shown:?}");
    assert!(!shown.contains("scan to send") && common::scan_qr(&shown).is_empty());
    let other = Home::new();
    let shown = other.run_on_terminal(&["create", "main", "--no-qr"], &new_seed, &[]);
    assert!(!shown.contains("QR after backup") && !shown.contains("scan to send"));
    assert!(
        shown.ends_with("verified: true\nverifiedBy: password\n"),
        "{shown:?}"
    );
    // --json and a pipe: as before.
    let other = Home::new();
    let json = other.run_on_terminal(&["create", "main", "--json"], &new_seed, &[]);
    let v: serde_json::Value =
        serde_json::from_str(&json).unwrap_or_else(|e| panic!("{e}: {json:?}"));
    assert_eq!(v["master"], "created");
    let other = Home::new();
    let piped = other.run_stdin(&["create", "main", "--password-stdin"], &format!("{PW}\n"));
    assert_eq!(piped.code, 0, "{}", piped.stderr);
    assert!(
        piped
            .stdout
            .ends_with("verified: true\nverifiedBy: password\n"),
        "{}",
        piped.stdout
    );
    assert!(!piped.stdout.contains("QR after backup") && !piped.stdout.contains("scan to send"));

    // unlock: the address its daemon derived from the seed, the code under it.
    let unlocked = home.run_on_terminal(&["unlock", "10m"], &["Wallet password: "], &[]);
    assert_eq!(field(&unlocked, "address"), addr);
    assert_eq!(field(&unlocked, "verified"), "true");
    let label = format!("scan to send to {addr}\n{EVERY_NETWORK}\n");
    let at = unlocked
        .find(&label)
        .unwrap_or_else(|| panic!("{unlocked:?}"));
    assert!(at > unlocked.find("address: ").unwrap(), "{unlocked:?}");
    assert_eq!(common::scan_qr(&unlocked), [addr.as_str()]);
    let quiet = home.run_on_terminal(&["unlock", "10m", "--no-qr"], &["Wallet password: "], &[]);
    assert!(!quiet.contains("scan to send"), "{quiet:?}");
    assert!(common::scan_qr(&quiet).is_empty() && !quiet.contains('\x1b'));
    let json = home.run_on_terminal(&["unlock", "10m", "--json"], &["Wallet password: "], &[]);
    let v: serde_json::Value =
        serde_json::from_str(&json).unwrap_or_else(|e| panic!("{e}: {json:?}"));
    assert_eq!(v["address"], addr.as_str());
    // Into a pipe: the fields only, as before.
    let piped = home.run_stdin(&["unlock", "10m", "--password-stdin"], &format!("{PW}\n"));
    assert_eq!(piped.code, 0, "{}", piped.stderr);
    assert_eq!(field(&piped.stdout, "address"), addr);
    assert!(!piped.stdout.contains("scan to send") && !piped.stdout.contains('\x1b'));

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
    // A further key without --unlock, the password asked at the terminal: the code too.
    let sub3 = home.run_on_terminal(&["create", "sub3"], &["Wallet password: "], &[]);
    let sub3_addr = field(&sub3, "address").to_string();
    assert_eq!(field(&sub3, "verified"), "true");
    assert!(
        sub3.contains(&format!("scan to send to {sub3_addr}\n{EVERY_NETWORK}\n")),
        "{sub3:?}"
    );
    assert_eq!(common::scan_qr(&sub3), [sub3_addr]);

    // An agent's create while unlocked (stdin no terminal): derived by whatever daemon answers,
    // so not verified, and no code even with stdout on a terminal.
    let mut cmd = home.command(&["create", "agent"]);
    cmd.stdin(std::process::Stdio::null());
    let (out, shown) = common::run_stdout_on_terminal(cmd);
    assert!(out.status.success(), "{:?}", out.stderr);
    assert_eq!(field(&shown, "verified"), "false", "{shown:?}");
    assert!(shown.contains(NOT_VERIFIED), "{shown:?}");
    assert!(!shown.contains("scan to send") && common::scan_qr(&shown).is_empty());
}

#[test]
fn qr3_unverified_addresses_on_a_terminal_print_as_on_a_pipe() {
    let home = Home::new();
    home.create("main", &[]);
    // `status` is left out of the pipe comparison: its `otherDaemonPids` follows the daemons of
    // tests running alongside.
    let commands: [&[&str]; 5] = [
        &["address"],
        &["address", "--chain", "base"],
        &["addresses"],
        &["change", "main"],
        &["status"],
    ];
    // Locked: the cache, with the note saying how to check it, and no code.
    for args in commands {
        let piped = home.run(args);
        assert_eq!(piped.code, 0, "{args:?}: {}", piped.stderr);
        let shown = home.run_on_terminal(args, &[], &[]);
        if args[0] != "status" {
            assert_eq!(shown, piped.stdout, "{args:?}");
        }
        assert!(shown.contains(&format!("\n{LOCKED_NOTE}\n")), "{shown:?}");
        assert!(!shown.contains(NOT_VERIFIED), "{shown:?}");
        assert!(!shown.contains("scan to send") && common::scan_qr(&shown).is_empty());
        let mut json = args.to_vec();
        json.push("--json");
        let v = home.run(&json).ok();
        assert_eq!(v["verified"], false, "{args:?}: {v}");
        assert!(
            v.get("verifiedBy").is_none() && v.get("warning").is_none(),
            "{args:?}: {v}"
        );
    }
    if session_verifies() {
        return;
    }
    // Where no session verifies (Linux): unlocked, the daemon's derivation, still unverified, and
    // no warning (the daemon's executable is hidden, not another one).
    home.unlock("10m");
    for args in commands {
        let piped = home.run(args);
        assert_eq!(piped.code, 0, "{args:?}: {}", piped.stderr);
        let shown = home.run_on_terminal(args, &[], &[]);
        if args[0] != "status" {
            assert_eq!(shown, piped.stdout, "{args:?}");
        }
        assert!(shown.contains(NOT_VERIFIED), "{shown:?}");
        assert!(!shown.contains("warning"), "{shown:?}");
    }
}

// ---- Verified by session: after unlock, without the password ----

#[test]
fn s1_after_unlock_the_address_views_are_verified_by_the_session() {
    let home = Home::new();
    let addr = home.create("main", &[])["address"]
        .as_str()
        .unwrap()
        .to_string();
    home.unlock("10m");
    for args in SESSION_VIEWS {
        let v = home.run(args).ok();
        assert_eq!(shown_address(&v), addr, "{args:?}: {v}");
        assert!(v.get("warning").is_none(), "{args:?}: {v}");
        if session_verifies() {
            assert_eq!(
                (&v["verified"], &v["verifiedBy"]),
                (&json!(true), &json!("session")),
                "{args:?}: {v}"
            );
        } else {
            assert_eq!(v["verified"], false, "{args:?}: {v}");
            assert!(v.get("verifiedBy").is_none(), "{args:?}: {v}");
        }
    }
    // Human, into a pipe: no note when the session verified it, and never a code.
    for args in [
        &["address"][..],
        &["addresses"],
        &["status"],
        &["change", "main"],
    ] {
        let out = home.run(args);
        assert_eq!(out.code, 0, "{args:?}: {}", out.stderr);
        assert_eq!(
            out.stdout.contains("not verified"),
            !session_verifies(),
            "{args:?}: {}",
            out.stdout
        );
        assert!(!out.stdout.contains("scan to send"), "{}", out.stdout);
        // `addresses` prints its rows only; the others print their fields.
        if session_verifies() && args[0] != "addresses" {
            assert!(
                out.stdout.contains("verifiedBy: session\n"),
                "{}",
                out.stdout
            );
        }
    }
    // `list` stays as it was: never verified without the password.
    let l = home.run(&["list", "--json"]).ok();
    assert_eq!(l["verified"], false, "{l}");
}

#[test]
fn s2_after_unlock_the_address_views_on_a_terminal_show_the_qr_code() {
    let home = Home::new();
    let addr = home.create("main", &[])["address"]
        .as_str()
        .unwrap()
        .to_string();
    let tron = derived_tron(&home);
    home.unlock("10m");
    let label = format!("scan to send to {addr}\n{EVERY_NETWORK}\n");
    for args in [
        &["address"][..],
        &["address", "--chain", "base"],
        &["addresses"],
        &["status"],
        &["change", "main"],
    ] {
        // `addresses` draws a second code after the EVM one: the Tron address (05-tron.md §CLI
        // surface).
        let codes: Vec<&str> = if args[0] == "addresses" {
            vec![&addr, &tron]
        } else {
            vec![&addr]
        };
        let shown = home.run_on_terminal(args, &[], &[]);
        // --no-qr is taken everywhere a code may show; then the terminal gets the pipe's text.
        let mut quiet = args.to_vec();
        quiet.push("--no-qr");
        let q = home.run_on_terminal(&quiet, &[], &[]);
        assert!(
            !q.contains("scan to send") && common::scan_qr(&q).is_empty(),
            "{q:?}"
        );
        if args[0] != "status" {
            assert_eq!(q, home.run(&quiet).stdout, "{args:?}");
        }
        if !session_verifies() {
            assert!(!shown.contains("scan to send") && common::scan_qr(&shown).is_empty());
            continue;
        }
        // The fields, then the two lines and the code: one code for the address on every EVM
        // chain, last but for `addresses`' Tron one.
        let at = shown
            .find(&label)
            .unwrap_or_else(|| panic!("{args:?}: {shown:?}"));
        if args[0] != "addresses" {
            assert!(shown[..at].contains("verifiedBy: session\n"), "{shown:?}");
        }
        assert_eq!(common::scan_qr(&shown), codes, "{args:?}");
        assert!(!shown.contains("not verified"), "{shown:?}");
        let plain = home.run_on_terminal(args, &[], &[("NO_COLOR", "1")]);
        assert!(!plain.contains('\x1b'), "{plain:?}");
        assert_eq!(common::scan_qr(&plain), codes, "{args:?}");
        // --json: the JSON only.
        let mut json = args.to_vec();
        json.push("--json");
        let j = home.run_on_terminal(&json, &[], &[]);
        let v: serde_json::Value =
            serde_json::from_str(&j).unwrap_or_else(|e| panic!("{e}: {j:?}"));
        assert_eq!(v["verifiedBy"], "session", "{v}");
    }
}

#[test]
fn s3_a_listener_answering_an_edited_cache_is_not_verified_and_is_named() {
    let home = Home::new();
    home.create("main", &[]);
    // An agent's script edits the cache and listens on d.sock, answering the edited address
    // (each chain family's).
    poison_cache(&home);
    let fake = FakeDaemon::answering_per_family(
        &home.sock(),
        serde_json::json!({
            "locked": false, "pid": 1, "expiresAt": "2099-01-01T00:00:00Z",
            "chain": "ethereum", "index": 0, "active": "main", "address": POISON,
        }),
        TRON_POISON,
    );
    // The listener runs in this test harness binary, not zwallet: its code is not this build's.
    // On macOS the cdhash catches that; on Linux its readable executable is not this build's.
    // Either way the pid is named and nothing is verified ([`names_the_test_process`]: on an
    // x86_64 Mac it is not named, and still not verified).
    let me = std::process::id();
    let want = TEST_PROCESS_NAMED;
    let named = names_the_test_process();
    let pid = format!("(pid {me})");
    let rpc = common::tiny_rpc(|method| match method {
        "eth_chainId" => json!("0x1"),
        _ => json!("0x0"),
    });
    for args in [
        &["address", "--json"][..],
        &["addresses", "--json"],
        &["change", "main", "--json"],
        &["balance", "--chain", "ethereum", "--rpc", &rpc, "--json"],
    ] {
        let v = home.run(args).ok();
        assert_eq!(shown_address(&v), POISON, "{args:?}: {v}");
        assert_eq!(v["verified"], false, "{args:?}: {v}");
        assert!(v.get("verifiedBy").is_none(), "{args:?}: {v}");
        if named {
            let w = v["warning"].as_str().unwrap_or("");
            assert!(w.contains(want) && w.contains(&pid), "{args:?}: {w}");
        } else {
            assert!(v.get("warning").is_none(), "{args:?}: {v}");
        }
    }
    // Human: the warning (where named) and the note; no code on a terminal either.
    let piped = home.run(&["address"]);
    assert_eq!(piped.code, 0, "{}", piped.stderr);
    assert_eq!(
        piped.stdout.contains("warning: the process answering") && piped.stdout.contains(want),
        named,
        "{}",
        piped.stdout
    );
    assert!(piped.stdout.contains(NOT_VERIFIED), "{}", piped.stdout);
    let shown = home.run_on_terminal(&["address"], &[], &[]);
    assert_eq!(shown, piped.stdout);
    assert!(common::scan_qr(&shown).is_empty());
    let addresses = home.run(&["addresses"]);
    assert_eq!(
        addresses.stdout.contains(want),
        named,
        "{}",
        addresses.stdout
    );
    drop(fake);
}

/// After `unlock`, `list` does not ask to verify the active key, which this build's session
/// verifies (`status` and `address` say so); the other keys stay flagged, with what checks them.
#[cfg(target_os = "macos")]
#[test]
fn list_while_this_build_serves_the_session_does_not_ask_to_verify_the_active_key() {
    let home = Home::new();
    home.create("a", &[]);
    home.create("b", &[]);
    // Locked: the note as before, last.
    let locked = home.run(&["list"]);
    assert!(
        locked.stdout.trim_end().ends_with(NOT_VERIFIED),
        "{}",
        locked.stdout
    );
    home.unlock("10m");
    let human = home.run(&["list"]);
    assert_eq!(human.code, 0, "{}", human.stderr);
    assert!(
        !human.stdout.contains("address --verify"),
        "{}",
        human.stdout
    );
    assert!(
        human.stdout.contains("other keys") && human.stdout.contains("`zwallet list --verify`"),
        "{}",
        human.stdout
    );
    assert_eq!(home.run(&["list", "--json"]).ok()["verified"], false);
}

#[test]
fn list_names_a_listener_whose_answers_match_an_edited_cache() {
    // `list` without --verify asks d.sock for each key's address too. A listener that answers
    // the edited cache raises no mismatch there, but it is named all the same, as in `address`.
    let home = Home::new();
    home.create("main", &[]);
    poison_cache(&home);
    let fake = FakeDaemon::answering(
        &home.sock(),
        serde_json::json!({
            "locked": false, "pid": 1, "expiresAt": "2099-01-01T00:00:00Z",
            "chain": "ethereum", "index": 0, "active": "main", "address": POISON,
        }),
    );
    // The listener runs in this test harness binary, not zwallet (see s3). On an x86_64 Mac it is
    // not named ([`names_the_test_process`]): no warning, and still not verified.
    let want = TEST_PROCESS_NAMED;
    let pid = format!("(pid {})", std::process::id());
    let v = home.run(&["list", "--json"]).ok();
    assert_eq!(v["keys"][0]["address"], POISON, "{v}");
    assert_eq!(v["verified"], false, "{v}");
    assert!(v.get("verifiedBy").is_none(), "{v}");
    if !names_the_test_process() {
        assert!(v.get("warning").is_none(), "{v}");
        let human = home.run(&["list"]);
        assert_eq!(human.code, 0, "{}", human.stderr);
        assert!(
            !human.stdout.contains("warning:") && human.stdout.contains(NOT_VERIFIED),
            "{}",
            human.stdout
        );
        drop(fake);
        return;
    }
    let w = v["warning"].as_str().unwrap_or("");
    assert!(
        w.starts_with("the process answering on the key daemon socket")
            && w.contains(want)
            && w.contains(&pid)
            && w.ends_with("run `zwallet lock` and unlock again"),
        "{v}"
    );
    // Human: the warning line, then the note.
    let human = home.run(&["list"]);
    assert_eq!(human.code, 0, "{}", human.stderr);
    let at = human
        .stdout
        .find("\nwarning: the process answering on the key daemon socket")
        .unwrap_or_else(|| panic!("no warning line: {}", human.stdout));
    assert!(human.stdout[at..].contains(want), "{}", human.stdout);
    assert!(
        human.stdout[at..].contains(&format!("\n{NOT_VERIFIED}")),
        "{}",
        human.stdout
    );
    drop(fake);
}

/// Unlocks `home` with a daemon started from the binary at `exe` (a copy), and returns its pids.
fn unlock_from(home: &Home, exe: &std::path::Path) {
    let mut cmd = common::zwallet_at(exe);
    cmd.env("ZWALLET_HOME", home.path())
        .args(["unlock", "10m", "--password-stdin", "--json"]);
    if common::ptrace_scope_is_open() {
        cmd.arg("--allow-ptrace-scope-0");
    }
    common::run_command(cmd, Some(&format!("{PW}\n"))).ok();
}

#[test]
fn s4_a_byte_copy_runs_the_same_code_so_it_may_verify_by_session() {
    // The identity is the code, by its kernel-validated cdhash, not the path or the file. A
    // byte-identical copy at another path has the same cdhash and runs the same code, so a daemon
    // started from it verifies by session on macOS (on Linux nothing verifies by session).
    let home = Home::new();
    let addr = home.create("main", &[])["address"]
        .as_str()
        .unwrap()
        .to_string();
    let dir = tempfile::Builder::new()
        .prefix("zwt-")
        .tempdir_in("/tmp")
        .unwrap();
    let copy = dir.path().join("zwallet");
    common::executable::copy_executable(env!("CARGO_BIN_EXE_zwallet"), &copy).unwrap();
    unlock_from(&home, &copy);
    assert_eq!(home.daemon_pids().len(), 1, "one daemon");
    for args in SESSION_VIEWS {
        let v = home.run(args).ok();
        assert_eq!(shown_address(&v), addr, "{args:?}: {v}");
        assert!(v.get("warning").is_none(), "{args:?}: {v}");
        if session_verifies() {
            assert_eq!(v["verifiedBy"], "session", "{args:?}: {v}");
        } else {
            assert_eq!(v["verified"], false, "{args:?}: {v}");
        }
    }
}

#[test]
fn s5_a_resigned_copy_is_other_code_so_it_is_not_verified() {
    if !session_verifies() {
        return; // The cdhash is only read on macOS.
    }
    // macOS ships codesign: a missing one is a broken host, not a reason to pass silently.
    assert!(
        have("codesign"),
        "s5 needs codesign, which ships with macOS"
    );
    let home = Home::new();
    home.create("main", &[]);
    let dir = tempfile::Builder::new()
        .prefix("zwt-")
        .tempdir_in("/tmp")
        .unwrap();
    let copy = dir.path().join("zwallet");
    common::executable::copy_executable(env!("CARGO_BIN_EXE_zwallet"), &copy).unwrap();
    // Re-sign with a different identifier: same bytes of code, a different code directory, so a
    // different cdhash. The kernel validates the running pages against it.
    let signed = std::process::Command::new("codesign")
        .args(["-f", "-s", "-", "-i", "com.example.other"])
        .arg(&copy)
        .output()
        .expect("run codesign");
    assert!(
        signed.status.success(),
        "codesign: {}",
        String::from_utf8_lossy(&signed.stderr)
    );
    unlock_from(&home, &copy);
    let pids = home.daemon_pids();
    assert_eq!(pids.len(), 1, "{pids:?}");
    for args in SESSION_VIEWS {
        let v = home.run(args).ok();
        assert_eq!(v["verified"], false, "{args:?}: {v}");
        assert!(v.get("verifiedBy").is_none(), "{args:?}: {v}");
        let w = v["warning"].as_str().unwrap_or("");
        assert!(
            w.contains("runs other code than this zwallet build")
                && w.contains(&format!("(pid {})", pids[0])),
            "{args:?}: {w}"
        );
    }
}

#[test]
fn s6_an_attacker_running_its_own_code_on_the_socket_is_not_verified() {
    // rustc links with cc, so a host that built this test has it.
    assert!(
        have("cc"),
        "s6 needs cc (rustc's linker) to build its attacker"
    );
    let home = Home::new();
    home.create("main", &[]);
    // The cache is edited to the attacker's address: a daemon-vs-cache mismatch would hide the
    // point, so the attacker answers the same (poisoned) address the cache now holds.
    poison_cache(&home);
    let dir = tempfile::Builder::new()
        .prefix("zwt-")
        .tempdir_in("/tmp")
        .unwrap();
    let bin = dir.path().join("attacker");
    build_attacker(dir.path(), &bin);
    // It runs with the daemon's argv, so it is in `daemon_pids` (what the old path/UUID check
    // trusted), yet it runs its own code. On macOS the cdhash catches it; on Linux its executable
    // is not this build's.
    let canonical = fs::canonicalize(home.path()).unwrap();
    let mut cmd = std::process::Command::new(&bin);
    cmd.arg("daemon").arg("--home").arg(&canonical);
    let _attacker = KillOnDrop(cmd.spawn().expect("spawn attacker"));
    wait_until_answering(&home, Duration::from_secs(15));
    let want = if session_verifies() {
        "runs other code than this zwallet build"
    } else {
        "is not this zwallet build"
    };
    for args in [&["address", "--json"][..], &["addresses", "--json"]] {
        let v = home.run(args).ok();
        assert_eq!(shown_address(&v), POISON, "{args:?}: {v}");
        assert_eq!(v["verified"], false, "{args:?}: {v}");
        assert!(v.get("verifiedBy").is_none(), "{args:?}: {v}");
        assert!(
            v["warning"].as_str().unwrap_or("").contains(want),
            "{args:?}: {v}"
        );
    }
}

#[test]
fn every_balance_aborts_on_a_cache_mismatch_even_with_good_rows() {
    // `balance` without `--chain`, while unlocked: a cache entry the daemon's derivation
    // contradicts aborts the whole command (VALIDATION_ERROR) before any RPC, even though another
    // chain's row would be good. Without the abort that entry would be one error row among good
    // ones and the command would exit 0; this is a tamper signal, not a read failure.
    let home = Home::new();
    home.create("main", &["--unlock", "30s"]);
    // Poison base's cache only; every EVM chain shares the derived address, so ethereum still
    // agrees.
    home.edit_wallet(|v| {
        v["keys"][0]["addresses"]["base"] = serde_json::json!(POISON);
    });
    // Ethereum answers from a node (a good row: chain id 1, zero balances); every other chain
    // points at a closed port.
    let node = common::tiny_rpc(|method| match method {
        "eth_chainId" => json!("0x1"),
        "eth_call" => json!(format!("0x{:064x}", 0)),
        _ => json!("0x0"),
    });
    let env: Vec<(String, String)> = chains::CHAINS
        .iter()
        .map(|c| {
            let url = if c.key == "ethereum" {
                node.clone()
            } else {
                "http://127.0.0.1:1".to_string()
            };
            (c.rpc_env_var(), url)
        })
        .collect();
    let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let out = home.run_env(&["balance", "--json"], &env);
    assert_eq!(
        out.code, 1,
        "expected the whole command to abort on the mismatch; stdout {}",
        out.stdout
    );
    let v = out.json();
    assert_eq!(v["errorCode"], "VALIDATION_ERROR", "{v}");
    assert!(
        v["error"]
            .as_str()
            .unwrap_or("")
            .contains(genuine_daemon_mismatch()),
        "{v}"
    );
    // The good row really is good: the same node serves `balance --chain ethereum`.
    let one = home
        .run(&["balance", "--chain", "ethereum", "--rpc", &node, "--json"])
        .ok();
    assert_eq!(one["balance"], "0", "{one}");
}

#[test]
fn a_mismatch_answered_by_an_impostor_aborts_and_names_it() {
    // An impostor on d.sock answers an address the cache contradicts: the command still aborts
    // with VALIDATION_ERROR, and the error names the impostor (in the message and as `warning`).
    let home = Home::new();
    home.create("main", &[]);
    let fake = FakeDaemon::answering(
        &home.sock(),
        serde_json::json!({
            "locked": false, "pid": 1, "expiresAt": "2099-01-01T00:00:00Z",
            "chain": "ethereum", "index": 0, "active": "main", "address": POISON,
        }),
    );
    let pid = format!("(pid {})", std::process::id());
    let want = TEST_PROCESS_NAMED;
    // On an x86_64 Mac the test process is not named ([`names_the_test_process`]): the error
    // still aborts and blames neither side, but names no process and carries no `warning`.
    let named = names_the_test_process();
    let closed: Vec<(String, String)> = chains::CHAINS
        .iter()
        .map(|c| (c.rpc_env_var(), "http://127.0.0.1:1".to_string()))
        .collect();
    let closed: Vec<(&str, &str)> = closed
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    for args in [&["balance", "--json"][..], &["address", "--json"]] {
        let out = home.run_env(args, &closed);
        assert_eq!(out.code, 1, "{args:?}: {}", out.stdout);
        let v = out.json();
        assert_eq!(v["errorCode"], "VALIDATION_ERROR", "{args:?}: {v}");
        let msg = v["error"].as_str().unwrap_or("");
        // It does not present the impostor's answer as the seed's derivation, nor blame the cache.
        assert!(
            !msg.contains("the cache was modified") && !msg.contains("derives from the seed"),
            "{args:?}: {msg}"
        );
        for part in [
            format!(
                "the process answering on the key daemon socket reported {POISON} as the \
                 ethereum address of key 'main', but wallet.json caches 0x"
            ),
            ": one of them was tampered with. Do not send funds to either address; run \
             `zwallet address --verify`"
                .to_string(),
        ] {
            assert!(msg.contains(&part), "{args:?}: missing {part:?} in {msg}");
        }
        if !named {
            assert!(
                !msg.contains("That process is not") && !msg.contains("and unlock again"),
                "{args:?}: {msg}"
            );
            assert!(v.get("warning").is_none(), "{args:?}: {v}");
            continue;
        }
        for part in [
            ". That process is not this build's key daemon: ".to_string(),
            want.to_string(),
            pid.clone(),
        ] {
            assert!(msg.contains(&part), "{args:?}: missing {part:?} in {msg}");
        }
        // The relock advice once, worded as in the warning.
        assert_eq!(
            msg.matches("and unlock again").count(),
            1,
            "{args:?}: {msg}"
        );
        assert!(
            msg.ends_with("run `zwallet lock` and unlock again"),
            "{msg}"
        );
        let w = v["warning"].as_str().unwrap_or("");
        assert!(w.contains(want) && w.contains(&pid), "{args:?}: {v}");
    }
    drop(fake);
}

/// Whether `value` (one value from outside, on one line) is printable ASCII only: [`common::
/// ascii_printable`], which does NOT exempt `\n`, so an injected line break fails it.
fn inert(value: &str) -> bool {
    common::ascii_printable(value)
}

/// Whether every byte of a whole output is printable ASCII or the `\n` separator. Split on `\n`
/// only (not `str::lines`, which would also eat a `\r` before it). Pair it with a line-count
/// assertion so an injected line break cannot pass as another output line.
fn inert_output(text: &str) -> bool {
    text.split('\n').all(common::ascii_printable)
}

#[test]
fn an_answer_that_is_no_address_is_rejected_and_shown_escaped() {
    // An impostor answers a terminal escape and a fake second line where an address belongs: it
    // is never shown, compared or verified as an address, and the error shows it escaped, in
    // `--json` and in the human error alike.
    let home = Home::new();
    home.create("main", &[]);
    let evil = "\u{1b}[2J\nwarning: all good, send funds";
    let fake = FakeDaemon::answering(
        &home.sock(),
        serde_json::json!({
            "locked": false, "pid": 1, "expiresAt": "2099-01-01T00:00:00Z",
            "chain": "ethereum", "index": 0, "active": "main", "address": evil,
        }),
    );
    let closed: Vec<(String, String)> = chains::CHAINS
        .iter()
        .map(|c| (c.rpc_env_var(), "http://127.0.0.1:1".to_string()))
        .collect();
    let closed: Vec<(&str, &str)> = closed
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    // `balance` too: an answer that is no address is tampering, not a network row.
    for args in [
        &["address", "--json"][..],
        &["addresses", "--json"],
        &["change", "main", "--json"],
        &["balance", "--json"],
    ] {
        let out = home.run_env(args, &closed);
        assert_eq!(out.code, 1, "{args:?}: {}", out.stdout);
        let v = out.json();
        assert_eq!(v["errorCode"], "VALIDATION_ERROR", "{args:?}: {v}");
        assert!(
            v.get("address").is_none() && v.get("verified").is_none(),
            "{v}"
        );
        let msg = v["error"].as_str().unwrap_or("");
        assert!(msg.contains("which is not an address"), "{args:?}: {msg}");
        // The impostor is named where the test process can be ([`names_the_test_process`]).
        assert_eq!(
            v["warning"].is_string(),
            names_the_test_process(),
            "{args:?}: {v}"
        );
        assert!(inert(msg), "{args:?}: {msg:?}");
        assert!(
            msg.contains("\\u{1b}") && msg.contains("\\u{a}"),
            "{args:?}: {msg:?}"
        );
    }
    // Human: one error line on stderr, nothing on stdout, no raw control character.
    let human = home.run(&["address"]);
    assert_eq!(human.code, 1);
    assert!(human.stdout.is_empty(), "{:?}", human.stdout);
    assert!(inert_output(&human.stderr), "{:?}", human.stderr);
    assert_eq!(
        human.stderr.trim_end().lines().count(),
        1,
        "{:?}",
        human.stderr
    );
    drop(fake);
}

#[test]
fn a_cached_value_that_is_no_address_is_rejected_and_shown_escaped() {
    // wallet.json is anyone's to edit: a cached "address" carrying a terminal escape is never
    // printed as one, locked or unlocked. (Editing the cache does not end a session: the daemon
    // watches only the sealed master.)
    let home = Home::new();
    home.create("main", &[]);
    let genuine = fs::read(home.wallet_json()).unwrap();
    let garbage = "\u{1b}]0;pwned\u{7}\n0xabc";
    for unlocked in [false, true] {
        if unlocked {
            fs::write(home.wallet_json(), &genuine).unwrap();
            home.unlock("10m");
        }
        home.edit_wallet(|v| v["keys"][0]["addresses"]["ethereum"] = serde_json::json!(garbage));
        for args in [
            &["address", "--json"][..],
            &["status", "--json"],
            &["list", "--json"],
        ] {
            let msg = home.run(args).err("VALIDATION_ERROR");
            assert!(
                msg.contains("which is not an address"),
                "unlocked {unlocked} {args:?}: {msg}"
            );
            assert!(inert(&msg), "{args:?}: {msg:?}");
        }
        let human = home.run(&["address"]);
        assert_eq!(human.code, 1);
        assert!(inert_output(&human.stderr), "{:?}", human.stderr);
    }
}

#[test]
fn an_impostor_executable_path_is_shown_escaped() {
    // The warning names the impostor's executable path, which its starter chose: a terminal
    // escape and a line break in it come out escaped, in `--json` and on the terminal alike.
    assert!(
        have("cc"),
        "needs cc (rustc's linker) to build the attacker"
    );
    let home = Home::new();
    home.create("main", &[]);
    // The cache is set to the attacker's answer, so the command shows its warning, not a mismatch.
    poison_cache(&home);
    let dir = tempfile::Builder::new()
        .prefix("zwt-")
        .tempdir_in("/tmp")
        .unwrap();
    let bin = dir.path().join("zw\u{1b}[31mallet\nfake line");
    build_attacker(dir.path(), &bin);
    let canonical = fs::canonicalize(home.path()).unwrap();
    let _attacker = KillOnDrop(
        std::process::Command::new(&bin)
            .arg("daemon")
            .arg("--home")
            .arg(&canonical)
            .spawn()
            .expect("spawn attacker"),
    );
    wait_until_answering(&home, Duration::from_secs(15));
    let v = home.run(&["address", "--json"]).ok();
    assert_eq!(v["verified"], false, "{v}");
    let w = v["warning"].as_str().unwrap_or("");
    assert!(w.contains("zw\\u{1b}[31mallet\\u{a}fake line"), "{w:?}");
    assert!(inert(w), "{w:?}");
    let human = home.run(&["address"]);
    assert_eq!(human.code, 0, "{}", human.stderr);
    assert!(inert_output(&human.stdout), "{:?}", human.stdout);
    let warning_lines: Vec<&str> = human
        .stdout
        .lines()
        .filter(|l| l.starts_with("warning: "))
        .collect();
    assert_eq!(warning_lines.len(), 1, "{:?}", human.stdout);
    assert!(!human.stdout.contains("\nfake line"), "{:?}", human.stdout);
}

#[test]
fn status_names_a_daemon_argv_impostor_that_contradicts_the_cache() {
    // `status` derives only through a daemon the process table lists that says it serves this
    // session. An impostor started with the daemon's argv is listed and says so (its own pid);
    // answering an address the genuine cache contradicts, it makes `status` abort, named.
    assert!(
        have("cc"),
        "needs cc (rustc's linker) to build the attacker"
    );
    let home = Home::new();
    home.create("main", &[]);
    let dir = tempfile::Builder::new()
        .prefix("zwt-")
        .tempdir_in("/tmp")
        .unwrap();
    let bin = dir.path().join("attacker");
    build_attacker(dir.path(), &bin);
    let canonical = fs::canonicalize(home.path()).unwrap();
    let attacker = KillOnDrop(
        std::process::Command::new(&bin)
            .arg("daemon")
            .arg("--home")
            .arg(&canonical)
            .spawn()
            .expect("spawn attacker"),
    );
    let pid = format!("(pid {})", attacker.0.id());
    wait_until_answering(&home, Duration::from_secs(15));
    assert!(home.daemon_pids().contains(&(attacker.0.id() as i32)));
    let want = if session_verifies() {
        "runs other code than this zwallet build"
    } else {
        "is not this zwallet build"
    };
    let out = home.run(&["status", "--json"]);
    assert_eq!(out.code, 1, "{}", out.stdout);
    let v = out.json();
    assert_eq!(v["errorCode"], "VALIDATION_ERROR", "{v}");
    let msg = v["error"].as_str().unwrap_or("");
    assert!(
        msg.contains(&format!(
            "reported {POISON} as the ethereum address of key 'main'"
        )),
        "{msg}"
    );
    assert!(msg.contains(want) && msg.contains(&pid), "{msg}");
    assert!(!msg.contains("the cache was modified"), "{msg}");
    let w = v["warning"].as_str().unwrap_or("");
    assert!(w.contains(want) && w.contains(&pid), "{v}");
}

/// The LC_UUID of a thin little-endian 64-bit Mach-O (what `cargo` and `cc` emit for the host on
/// this machine). `None` if it is not one, or has no LC_UUID.
#[cfg(target_os = "macos")]
fn macho_uuid(bytes: &[u8]) -> Option<[u8; 16]> {
    const MH_MAGIC_64: u32 = 0xFEED_FACF;
    const LC_UUID: u32 = 0x1B;
    let word = |o: usize| -> Option<u32> {
        Some(u32::from_le_bytes(bytes.get(o..o + 4)?.try_into().ok()?))
    };
    if word(0)? != MH_MAGIC_64 {
        return None;
    }
    let ncmds = word(16)? as usize;
    let mut off = 32; // sizeof(mach_header_64)
    for _ in 0..ncmds {
        let cmd = word(off)?;
        let size = word(off + 4)? as usize;
        if cmd == LC_UUID {
            return bytes.get(off + 8..off + 24)?.try_into().ok();
        }
        off = off.checked_add(size)?;
    }
    None
}

/// Overwrites the LC_UUID of the thin Mach-O at `path` with `uuid`.
#[cfg(target_os = "macos")]
fn set_macho_uuid(path: &std::path::Path, uuid: [u8; 16]) {
    const LC_UUID: u32 = 0x1B;
    let mut bytes = fs::read(path).unwrap();
    let word = |b: &[u8], o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
    let ncmds = word(&bytes, 16) as usize;
    let mut off = 32;
    for _ in 0..ncmds {
        let cmd = word(&bytes, off);
        let size = word(&bytes, off + 4) as usize;
        if cmd == LC_UUID {
            bytes[off + 8..off + 24].copy_from_slice(&uuid);
            fs::write(path, &bytes).unwrap();
            return;
        }
        off += size;
    }
    panic!("no LC_UUID in {}", path.display());
}

/// The round-9 exploit, end to end: an attacker's own code that (a) carries the genuine binary's
/// LC_UUID and (b) runs from a path later symlinked to the genuine binary, so the old macOS
/// identity check (`proc_pidpath` + dev/ino + `LOCAL_PEERUUID`) is fully satisfied. The cdhash the
/// kernel validates against the attacker's own code pages still differs, so the current check
/// reports `verified: false`. The RED-critical assertions are the verdict only (no string): at
/// 3005dba this test reports `verified: true`, `verifiedBy: "session"`.
#[cfg(target_os = "macos")]
#[test]
fn s7_own_code_behind_a_symlink_to_zwallet_with_its_uuid_is_not_verified() {
    // The only test that reproduces the round-9 exploit: on macOS it must run, never skip.
    // `cc` and `codesign` both ship with the Xcode Command Line Tools, which rustc needs anyway.
    assert!(
        have("cc") && have("codesign"),
        "s7 needs cc and codesign (Xcode Command Line Tools) to reproduce the round-9 exploit"
    );
    let genuine = std::path::PathBuf::from(env!("CARGO_BIN_EXE_zwallet"));
    let genuine_uuid = macho_uuid(&fs::read(&genuine).unwrap()).expect("genuine LC_UUID");

    let home = Home::new();
    home.create("main", &[]);
    // The attacker answers its own (poisoned) address; the cache is set to it so the daemon-vs-cache
    // check does not short-circuit before the identity verdict.
    poison_cache(&home);

    let dir = tempfile::Builder::new()
        .prefix("zwt-")
        .tempdir_in("/tmp")
        .unwrap();
    let src = dir.path().join("attacker.c");
    let p = dir.path().join("P");
    fs::write(&src, common::ATTACKER_C).unwrap();
    let cc = std::process::Command::new("cc")
        .arg("-o")
        .arg(&p)
        .arg(&src)
        .output()
        .expect("run cc");
    assert!(
        cc.status.success(),
        "cc: {}",
        String::from_utf8_lossy(&cc.stderr)
    );
    // Forge the genuine LC_UUID into the attacker, then ad-hoc re-sign (arm64 will not run a
    // modified Mach-O otherwise). The re-sign must not disturb the forged UUID.
    set_macho_uuid(&p, genuine_uuid);
    let signed = std::process::Command::new("codesign")
        .args(["-f", "-s", "-"])
        .arg(&p)
        .output()
        .expect("run codesign");
    assert!(
        signed.status.success(),
        "codesign: {}",
        String::from_utf8_lossy(&signed.stderr)
    );
    assert_eq!(
        macho_uuid(&fs::read(&p).unwrap()),
        Some(genuine_uuid),
        "re-signing changed the attacker's LC_UUID"
    );

    // Run the attacker from P with the daemon argv (so it is in `daemon_pids`), then replace P
    // with a symlink to the genuine binary so `proc_pidpath` + stat resolve to genuine's dev/ino.
    let canonical = fs::canonicalize(home.path()).unwrap();
    let _attacker = KillOnDrop(
        std::process::Command::new(&p)
            .arg("daemon")
            .arg("--home")
            .arg(&canonical)
            .spawn()
            .expect("spawn attacker from P"),
    );
    wait_until_answering(&home, Duration::from_secs(15));
    fs::remove_file(&p).unwrap();
    std::os::unix::fs::symlink(&genuine, &p).unwrap();

    // RED-critical: the verdict only, no warning string.
    for args in [&["address", "--json"][..], &["addresses", "--json"]] {
        let v = home.run(args).ok();
        assert_eq!(shown_address(&v), POISON, "{args:?}: {v}");
        assert_eq!(v["verified"], false, "{args:?}: {v}");
        assert!(v.get("verifiedBy").is_none(), "{args:?}: {v}");
    }
    // At HEAD the warning names the mismatch (checked here, not in the RED-critical block).
    let v = home.run(&["address", "--json"]).ok();
    assert!(
        v["warning"]
            .as_str()
            .unwrap_or("")
            .contains("runs other code than this zwallet build"),
        "{v}"
    );
}

#[test]
fn list_never_blames_the_cache_for_an_impostor_answer() {
    // `list` compares the cache with what answers on d.sock too. An impostor answering another
    // address than the genuine cache is no derivation from the seed: the error must not say the
    // cache was modified, and it names the impostor.
    let home = Home::new();
    home.create("main", &[]);
    let dir = tempfile::Builder::new()
        .prefix("zwt-")
        .tempdir_in("/tmp")
        .unwrap();
    let attacker = daemon_argv_impostor(&home, dir.path(), &[]);
    let v = home.run(&["list", "--json"]).json();
    assert_eq!(v["errorCode"], "VALIDATION_ERROR", "{v}");
    let msg = v["error"].as_str().unwrap_or("");
    assert!(
        !msg.contains("derives from the seed") && !msg.contains("cache was modified"),
        "{msg}"
    );
    assert!(msg.contains("one of them was tampered with"), "{msg}");
    assert!(
        msg.contains(&format!("(pid {})", attacker.0.id())),
        "the impostor is named: {msg}"
    );
}

#[test]
fn an_impostor_error_reply_is_named_escaped_and_on_one_line() {
    // An impostor answers with ok:false, its error text drawing a fake verified address on the
    // terminal (CR, erase line, new lines, then hiding the rest), and a fake `command` detail.
    // Its text comes out escaped and on one line, after the line naming it; its details are
    // dropped.
    if !names_the_test_process() {
        // On an x86_64 Mac the test process is `Unknown` ([`names_the_test_process`]), whose
        // scalar details are kept, escaped and bounded, by design (as for any peer nothing can be
        // established about; see `a_squatters_array_and_object_details_are_dropped…`). This test
        // is about a peer known to be another program.
        return;
    }
    let home = Home::new();
    home.create("main", &[]);
    let forged = "\r\u{1b}[2Kaddress: 0x000000000000000000000000000000000000dEaD\nverified: \
                  true\nverifiedBy: session\nscan to send to 0xdEaD\n\u{2588}\u{2588}\u{1b}[8m";
    let fake = FakeDaemon::replying(
        &home.sock(),
        serde_json::json!({
            "v": 2, "ok": false, "errorCode": "RPC_ERROR", "error": forged,
            "details": { "command": "zwallet whitelist add x 0xdEaD\n\u{1b}[8m" },
        }),
    );
    let out = home.run(&["address", "--json"]);
    assert_eq!(out.code, 1, "{}", out.stdout);
    let v = out.json();
    let msg = v["error"].as_str().unwrap_or("");
    assert!(inert(msg), "{msg:?}");
    assert!(
        msg.starts_with(&format!(
            "the process answering on the key daemon socket (pid {})",
            std::process::id()
        )) && msg.contains(", and it reported: \\u{d}\\u{1b}[2Kaddress: 0x"),
        "{msg}"
    );
    assert!(
        v.get("command").is_none(),
        "an impostor's details are dropped: {v}"
    );
    assert!(v["warning"].is_string(), "{v}");
    // Human: exactly one line on stderr, nothing on stdout, no terminal control.
    let human = home.run(&["address"]);
    assert_eq!(human.code, 1);
    assert!(human.stdout.is_empty(), "{:?}", human.stdout);
    assert!(inert_output(&human.stderr), "{:?}", human.stderr);
    assert_eq!(human.stderr.lines().count(), 1, "{:?}", human.stderr);
    assert!(!human.stderr.contains("command:"), "{:?}", human.stderr);
    drop(fake);
}

#[test]
fn a_key_name_with_a_control_character_is_refused() {
    // Key names reach messages and output: wallet.json may only hold names `create` accepts.
    let home = Home::new();
    home.create("main", &[]);
    let genuine = fs::read(home.wallet_json()).unwrap();
    for edit in ["name", "active"] {
        fs::write(home.wallet_json(), &genuine).unwrap();
        home.edit_wallet(|v| {
            let evil = serde_json::json!("ma\u{1b}[2J\nin");
            if edit == "name" {
                v["keys"][0]["name"] = evil.clone();
            }
            v["active"] = evil;
        });
        for args in [&["address", "--json"][..], &["list", "--json"]] {
            let msg = home.run(args).err("VALIDATION_ERROR");
            assert!(msg.contains("invalid key name"), "{edit} {args:?}: {msg}");
            assert!(inert(&msg), "{msg:?}");
        }
        let human = home.run(&["list"]);
        assert!(inert_output(&human.stderr), "{:?}", human.stderr);
    }
}

#[test]
fn status_shows_an_impostor_expiry_and_whitelist_escaped() {
    // `status` relays the serving daemon's expiry and whitelist. A daemon-argv impostor (so
    // `status` takes it as serving) sets both to terminal escapes: they come out escaped, in
    // `--json` and on the terminal.
    let home = Home::new();
    home.create("main", &[]);
    // The cache agrees with the impostor's answer, so `status` shows rather than aborts.
    poison_cache(&home);
    let dir = tempfile::Builder::new()
        .prefix("zwt-")
        .tempdir_in("/tmp")
        .unwrap();
    let _attacker = daemon_argv_impostor(
        &home,
        dir.path(),
        &[
            ("ATTACKER_EXPIRES", "2099\\u001b[2J\\u001b]0;x\\u0007"),
            (
                "ATTACKER_WHITELIST",
                "{\"gen\":1,\"verified\":true,\"entries\":[\"ev\\u001b[31mil\",\"\\u202e\\u009b\"]}",
            ),
        ],
    );
    let v = home.run(&["status", "--json"]).ok();
    let expires = v["expiresAt"].as_str().unwrap_or("");
    assert!(inert(expires) && expires.contains("\\u{1b}[2J"), "{v}");
    let entries = v["whitelist"]["entries"].to_string();
    assert!(
        !entries.contains('\u{1b}') && !entries.contains('\u{202e}') && !entries.contains('\u{9b}'),
        "{v}"
    );
    let human = home.run(&["status"]);
    assert_eq!(human.code, 0, "{}", human.stderr);
    for c in ['\u{1b}', '\u{7}', '\u{202e}', '\u{9b}'] {
        assert!(!human.stdout.contains(c), "{c:?} in {:?}", human.stdout);
    }
}

#[test]
fn address_verify_repairs_a_tampered_cache_and_unlock_works_again() {
    // With the password the seed is authoritative: a garbage cache value and a wrong address
    // both stop unlock (which says how to repair), and `address --verify` repairs them from the
    // seed, saying so in `repaired` and a warning. Then unlock works.
    let pw = format!("{PW}\n");
    for garbage in [true, false] {
        let home = Home::new();
        let addr = home.create("main", &[])["address"]
            .as_str()
            .unwrap()
            .to_string();
        let was = if garbage {
            home.edit_wallet(|v| {
                v["keys"][0]["addresses"]["ethereum"] =
                    serde_json::json!("\u{1b}]0;pwned\u{7}\n0xabc")
            });
            "\u{1b}]0;pwned\u{7}\n0xabc".to_string()
        } else {
            poison_cache(&home);
            POISON.to_string()
        };
        for args in [
            &["unlock", "10m", "--password-stdin", "--json"][..],
            &["list", "--verify", "--password-stdin", "--json"],
            &[
                "whitelist",
                "list",
                "--verify",
                "--password-stdin",
                "--json",
            ],
        ] {
            let msg = home.run_stdin(args, &pw).err("VALIDATION_ERROR");
            assert!(
                msg.contains("run `zwallet address --verify` to check the cache against the seed"),
                "{args:?}: {msg}"
            );
        }
        let v = home
            .run_stdin(&["address", "--verify", "--password-stdin", "--json"], &pw)
            .ok();
        assert_eq!(
            (&v["address"], &v["verified"]),
            (&json!(addr), &json!(true)),
            "{v}"
        );
        let repaired = v["repaired"].as_array().unwrap_or_else(|| panic!("{v}"));
        let eth = repaired
            .iter()
            .find(|r| r["chain"] == "ethereum")
            .unwrap_or_else(|| panic!("{v}"));
        assert_eq!((&eth["key"], &eth["now"]), (&json!("main"), &json!(addr)));
        let shown_was = eth["was"].as_str().unwrap();
        assert!(inert(shown_was), "{v}");
        if !garbage {
            assert_eq!(shown_was, was);
            assert_eq!(repaired.len(), chains::CHAINS.len(), "{v}");
        }
        assert!(
            v["warning"]
                .as_str()
                .unwrap_or("")
                .contains("Something modified wallet.json"),
            "{v}"
        );
        // The human form says so too, with nothing raw (tampered again for it).
        home.edit_wallet(|v| v["keys"][0]["addresses"]["ethereum"] = serde_json::json!(was));
        let human = home.run_stdin(&["address", "--verify", "--password-stdin"], &pw);
        assert_eq!(human.code, 0, "{}", human.stderr);
        assert!(
            human.stdout.contains("repaired: ")
                && human.stdout.contains("Something modified wallet.json"),
            "{}",
            human.stdout
        );
        assert!(inert_output(&human.stdout), "{:?}", human.stdout);
        // Repaired once: the next check finds nothing, and unlock works.
        let again = home
            .run_stdin(&["address", "--verify", "--password-stdin", "--json"], &pw)
            .ok();
        assert!(again.get("repaired").is_none(), "{again}");
        assert_eq!(home.unlock("10m")["address"], addr.as_str());
        home.stop_daemons();
    }
}

/// `s` ABI-encoded as one `string` return value.
fn abi_string(s: &str) -> String {
    let b = s.as_bytes();
    let mut hex = format!("{:064x}{:064x}", 32, b.len());
    let mut data: String = b.iter().map(|x| format!("{x:02x}")).collect();
    while !data.len().is_multiple_of(64) {
        data.push('0');
    }
    hex.push_str(&data);
    format!("0x{hex}")
}

/// What a node or a contract writes to draw a fake line on a terminal.
const FORGED: &str = "\r\u{1b}[2Kbalance: 999 ETH\nverified: true\u{9b}\u{202e}\u{1b}[8m";

#[test]
fn rpc_text_reaches_human_output_escaped_and_on_one_line() {
    // An RPC endpoint is anyone's: `--rpc` and ZWALLET_RPC_* can name any server. Its error text
    // and a contract's `symbol()` are escaped where zwallet builds the error or the output, so
    // `--json` and the terminal get the same inert text.
    let token = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48";
    // 1. A JSON-RPC error message (and its data) from the node.
    let failing = common::tiny_rpc_reply(|method, _| match method {
        "eth_chainId" => Ok(json!("0x1")),
        _ => Err(json!({ "code": -32000, "message": FORGED, "data": FORGED })),
    });
    let nowallet = Home::new();
    let call = [
        "call",
        "--chain",
        "ethereum",
        "--to",
        token,
        "--sig",
        "name()(string)",
        "--rpc",
    ];
    let mut args = call.to_vec();
    args.extend([failing.as_str(), "--json"]);
    let msg = nowallet.run(&args).err("RPC_ERROR");
    assert!(msg.starts_with("eth_call failed: "), "{msg}");
    assert!(inert(&msg), "{msg:?}");
    assert!(
        !msg.contains('\u{9b}') && !msg.contains('\u{202e}'),
        "{msg:?}"
    );
    assert!(
        msg.contains("\\u{d}\\u{1b}[2Kbalance: 999 ETH\\u{a}verified: true"),
        "{msg}"
    );
    args.pop();
    let human = nowallet.run(&args);
    assert_eq!(human.code, 1);
    assert!(inert_output(&human.stderr), "{:?}", human.stderr);
    assert_eq!(human.stderr.lines().count(), 1, "{:?}", human.stderr);
    // 2. A token whose `symbol()` is a terminal escape.
    let home = Home::new();
    home.create("main", &[]);
    let symbol_node = common::tiny_rpc_reply(|method, params| match method {
        "eth_chainId" => Ok(json!("0x1")),
        "eth_call" => {
            let tx = &params[0];
            let data = tx["input"].as_str().or(tx["data"].as_str()).unwrap_or("");
            Ok(json!(match &data[..10.min(data.len())] {
                "0x313ce567" => format!("0x{:064x}", 6),
                "0x95d89b41" => abi_string(FORGED),
                _ => format!("0x{:064x}", 5),
            }))
        }
        _ => Ok(json!("0x0")),
    });
    let b = [
        "balance",
        "--chain",
        "ethereum",
        "--token",
        token,
        "--rpc",
        &symbol_node,
    ];
    let mut json_args = b.to_vec();
    json_args.push("--json");
    let v = home.run(&json_args).ok();
    let sym = v["symbol"].as_str().unwrap_or("");
    assert!(inert(sym) && !sym.contains('\u{202e}'), "{v}");
    assert!(sym.starts_with("\\u{d}\\u{1b}[2Kbalance"), "{v}");
    let human = home.run(&b);
    assert_eq!(human.code, 0, "{}", human.stderr);
    assert!(inert_output(&human.stdout), "{:?}", human.stdout);
    assert!(
        !human
            .stdout
            .lines()
            .any(|l| l.starts_with("verified: true")),
        "{:?}",
        human.stdout
    );
}

#[test]
fn a_registry_tokens_recorded_symbol_shows_as_itself_and_any_other_is_escaped() {
    // USDT0 on Arbitrum answers `symbol()` with `USD₮0`, which the registry records: shown as is.
    // The same text from a contract not in the registry, or anything else from the registry's
    // address, is text from outside: escaped to printable ASCII.
    let usdt0 = "0xFd086bC7CD5C481DCC9C85ebE478A1C0b69FCbb9";
    let other = "0x1111111111111111111111111111111111111111";
    let home = Home::new();
    home.create("main", &[]);
    let symbol_of = |token: &str, symbol: &'static str| {
        let node = common::tiny_rpc_reply(move |method, params| match method {
            "eth_chainId" => Ok(json!("0xa4b1")),
            "eth_call" => {
                let tx = &params[0];
                let data = tx["input"].as_str().or(tx["data"].as_str()).unwrap_or("");
                Ok(json!(match &data[..10.min(data.len())] {
                    "0x313ce567" => format!("0x{:064x}", 6),
                    "0x95d89b41" => abi_string(symbol),
                    _ => format!("0x{:064x}", 5),
                }))
            }
            _ => Ok(json!("0x0")),
        });
        let v = home
            .run(&[
                "balance", "--chain", "arbitrum", "--token", token, "--rpc", &node, "--json",
            ])
            .ok();
        v["symbol"].as_str().unwrap_or("").to_string()
    };
    assert_eq!(symbol_of(usdt0, "USD\u{20ae}0"), "USD\u{20ae}0");
    assert_eq!(symbol_of(other, "USD\u{20ae}0"), "USD\\u{20ae}0");
    assert_eq!(
        symbol_of(usdt0, "USD\u{20ae}0\u{1b}[2J"),
        "USD\\u{20ae}0\\u{1b}[2J"
    );
}

#[test]
fn the_write_point_escapes_text_no_source_escaped() {
    // `status` prints the wallet home's path as it is: no source escapes it. A home named with
    // ESC, CR and a solid glyph still reaches the terminal as printable ASCII, on one line,
    // because every human write goes through the write point.
    use std::os::unix::fs::DirBuilderExt;
    let dir = tempfile::Builder::new()
        .prefix("zwt-")
        .tempdir_in("/tmp")
        .unwrap();
    let home = dir.path().join("h\u{1b}[2J\r\u{2b1b}");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&home)
        .unwrap();
    let out = common::run_in(&home, &["status"], None, &[]);
    assert_eq!(out.code, 0, "{:?}", out.stderr);
    assert!(inert_output(&out.stdout), "{:?}", out.stdout);
    let line: Vec<&str> = out
        .stdout
        .lines()
        .filter(|l| l.starts_with("home: "))
        .collect();
    assert_eq!(line.len(), 1, "{:?}", out.stdout);
    assert!(
        line[0].ends_with("/h\\u{1b}[2J\\u{d}\\u{2b1b}"),
        "{:?}",
        line[0]
    );
}

#[test]
fn call_json_is_ascii_on_the_wire_and_parses_back_to_the_contracts_exact_string() {
    // `--json` keeps a contract's string exactly, but the bytes printed are printable ASCII (and
    // the newlines of pretty printing): solid glyphs, an octant, C1 CSI and DEL go out as JSON
    // `\uXXXX` escapes, which a parser turns back into the same string.
    let s = "\u{2b1b}\u{2b1c}\u{1cd00}\u{9b}2J\u{7f}\u{1b}[2J USD\u{20ae}0";
    let encoded = abi_string(s);
    let node = common::tiny_rpc_reply(move |method, _| match method {
        "eth_chainId" => Ok(json!("0x1")),
        "eth_call" => Ok(json!(encoded.clone())),
        _ => Ok(json!("0x0")),
    });
    let home = Home::new();
    let out = home.run(&[
        "call",
        "--chain",
        "ethereum",
        "--to",
        "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
        "--sig",
        "name()(string)",
        "--rpc",
        &node,
        "--json",
    ]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(
        out.stdout
            .bytes()
            .all(|b| b == b'\n' || (0x20..=0x7e).contains(&b)),
        "{:?}",
        out.stdout
    );
    let v: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(v["result"], json!([s]), "the exact string, after parsing");
}

#[test]
fn a_call_string_result_is_faithful_in_json_and_escaped_on_the_terminal() {
    // `call` decodes what the caller asked for: `--json` keeps the contract's string exactly
    // (JSON encodes its control characters), the human text shows it escaped and on one line.
    let node = common::tiny_rpc_reply(|method, _| match method {
        "eth_chainId" => Ok(json!("0x1")),
        "eth_call" => Ok(json!(abi_string(FORGED))),
        _ => Ok(json!("0x0")),
    });
    let home = Home::new();
    let args = [
        "call",
        "--chain",
        "ethereum",
        "--to",
        "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
        "--sig",
        "name()(string)",
        "--rpc",
        &node,
    ];
    let mut json_args = args.to_vec();
    json_args.push("--json");
    let v = home.run(&json_args).ok();
    assert_eq!(v["result"], json!([FORGED]), "{v}");
    let human = home.run(&args);
    assert_eq!(human.code, 0, "{}", human.stderr);
    assert!(inert_output(&human.stdout), "{:?}", human.stdout);
    assert!(!human.stdout.contains('\u{9b}') && !human.stdout.contains('\u{202e}'));
    let result: Vec<&str> = human
        .stdout
        .lines()
        .filter(|l| l.starts_with("result: "))
        .collect();
    assert_eq!(result.len(), 1, "{:?}", human.stdout);
    assert!(
        !human
            .stdout
            .lines()
            .any(|l| l.starts_with("verified: true")),
        "{:?}",
        human.stdout
    );
}

/// Kilobytes of padding, then rows of half blocks, then a fake label: what a sender writes to
/// draw a QR code of its own address by letting the terminal soft-wrap one long line.
fn qr_by_wrapping() -> String {
    let row = "\u{2588}\u{2580}\u{2584} ".repeat(20);
    format!(
        "{}{}scan to send to 0x000000000000000000000000000000000000dEaD",
        " ".repeat(4000),
        format!("{row}\n").repeat(30)
    )
}

#[test]
fn a_long_relayed_error_is_cut_and_draws_nothing_and_its_code_shows() {
    // An impostor's ok:false reply padded to draw a QR code by soft-wrap: the message and its
    // detail come out cut and with no block character, on one line ending in the code.
    let home = Home::new();
    home.create("main", &[]);
    let payload = qr_by_wrapping();
    let fake = FakeDaemon::replying(
        &home.sock(),
        serde_json::json!({
            "v": 2, "ok": false, "errorCode": "RPC_ERROR", "error": payload,
            "details": { "command": payload },
        }),
    );
    let v = home.run(&["address", "--json"]).json();
    let msg = v["error"].as_str().unwrap_or("");
    assert!(inert(msg), "{:?}", &msg[..200]);
    assert!(msg.chars().count() < 1200, "{}", msg.chars().count());
    let human = home.run(&["address"]);
    assert_eq!(human.code, 1);
    let line = human.stderr.trim_end();
    assert_eq!(line.lines().count(), 1, "{:?}", &line[..200]);
    assert!(
        line.starts_with("error: ") && line.ends_with(" [RPC_ERROR]"),
        "{:?}",
        &line[line.len() - 80..]
    );
    assert!(line.chars().count() < 1250, "{}", line.chars().count());
    assert!(inert(line), "{:?}", &line[..200]);
    drop(fake);
}

#[test]
fn a_relayed_error_of_solid_glyphs_draws_nothing() {
    // Glyphs an earlier blacklist missed — ⬛⬜ (U+2B1B/2B1C), a sextant (U+1FB00) and an octant
    // (U+1CD00) — would tile a QR on a terminal as well as the old block elements. The allowlist
    // escapes every one, so `--json` and the human line are printable ASCII, on one line.
    let home = Home::new();
    home.create("main", &[]);
    let payload = format!(
        "{}{}{}",
        "\u{2b1b}\u{2b1c} ".repeat(400),
        "\u{1fb00}".repeat(200),
        "\u{1cd00}".repeat(200)
    );
    let fake = FakeDaemon::replying(
        &home.sock(),
        serde_json::json!({
            "v": 2, "ok": false, "errorCode": "RPC_ERROR", "error": payload,
            "details": { "command": payload },
        }),
    );
    let v = home.run(&["address", "--json"]).json();
    let msg = v["error"].as_str().unwrap_or("");
    assert!(inert(msg), "{:?}", msg.chars().take(80).collect::<String>());
    let human = home.run(&["address"]);
    assert_eq!(human.code, 1);
    let line = human.stderr.trim_end();
    assert_eq!(
        line.lines().count(),
        1,
        "{:?}",
        line.chars().take(120).collect::<String>()
    );
    assert!(
        inert_output(line),
        "{:?}",
        line.chars().take(120).collect::<String>()
    );
    drop(fake);
}

#[test]
fn a_squatters_array_and_object_details_are_dropped_and_its_error_is_bounded() {
    // A squatter on d.sock that a check cannot tell from a key daemon (on Linux a non-dumpable
    // process, `Unknown`) keeps its scalar details; an array of 64 strings of 256 characters and
    // an object of them are dropped, so neither `--json` nor the terminal gets ~16K characters.
    // On macOS the attacker runs other code (`Other`): every detail is dropped there anyway.
    let home = Home::new();
    home.create("main", &[]);
    let dir = tempfile::Builder::new()
        .prefix("zwt-")
        .tempdir_in("/tmp")
        .unwrap();
    let reply = json!({
        "v": 2, "ok": false, "errorCode": "WHITELIST_DENIED", "error": "refused",
        "details": {
            "command": "zwallet whitelist add x 0xdEaD",
            "counterparty": vec!["A".repeat(256); 64],
            "chain": { "k": vec!["B".repeat(256); 64] },
        },
    })
    .to_string();
    let _attacker = daemon_argv_impostor(
        &home,
        dir.path(),
        &[("ATTACKER_REPLY", &reply), ("ATTACKER_NONDUMPABLE", "1")],
    );
    let out = home.run(&["address", "--json"]);
    let head = |s: &str| s.chars().take(300).collect::<String>();
    assert_eq!(out.code, 1, "{}", head(&out.stdout));
    let v = out.json();
    assert!(
        v.get("counterparty").is_none() && v.get("chain").is_none(),
        "{}",
        head(&out.stdout)
    );
    assert!(out.stdout.len() < 1500, "{}", out.stdout.len());
    assert!(inert_output(&out.stdout), "{}", head(&out.stdout));
    #[cfg(target_os = "linux")]
    {
        // Not known who answered: the neutral prefix, and the scalar detail kept.
        let msg = v["error"].as_str().unwrap_or("");
        assert!(
            msg.starts_with("the process answering on the key daemon socket reported: refused"),
            "{msg}"
        );
        assert_eq!(v["command"], "zwallet whitelist add x 0xdEaD", "{v}");
    }
    let human = home.run(&["address"]);
    assert_eq!(human.code, 1);
    assert!(human.stdout.is_empty(), "{:?}", head(&human.stdout));
    assert!(human.stderr.len() < 1500, "{}", human.stderr.len());
    assert!(human.stderr.lines().count() <= 2, "{}", head(&human.stderr));
    assert!(inert_output(&human.stderr), "{}", head(&human.stderr));
}

#[test]
fn status_bounds_a_nested_whitelist_on_the_terminal_and_keeps_every_entry_in_json() {
    // `status` relays the serving daemon's whitelist. A daemon-argv impostor answers 200 entries
    // of solid glyphs and a long reason: `--json` keeps all 200, each escaped, with the reason cut;
    // the terminal gets one inert `whitelist:` line of at most the prose budget.
    let home = Home::new();
    home.create("main", &[]);
    poison_cache(&home);
    let dir = tempfile::Builder::new()
        .prefix("zwt-")
        .tempdir_in("/tmp")
        .unwrap();
    let entries: Vec<String> = (0..200)
        .map(|i| format!("n{i:03}{}", "\u{2b1b}\u{2b1c}".repeat(30)))
        .collect();
    let whitelist = json!({
        "gen": 1, "verified": false, "entries": entries, "reason": "\u{1cd00}".repeat(5000),
    })
    .to_string();
    let _attacker = daemon_argv_impostor(&home, dir.path(), &[("ATTACKER_WHITELIST", &whitelist)]);
    let head = |s: &str| s.chars().take(300).collect::<String>();
    let out = home.run(&["status", "--json"]);
    assert_eq!(out.code, 0, "{}", head(&out.stdout));
    assert!(inert_output(&out.stdout), "{}", head(&out.stdout));
    let v = out.json();
    let shown = v["whitelist"]["entries"].as_array().unwrap();
    assert_eq!(shown.len(), 200, "every entry is kept in --json");
    assert_eq!(
        shown[7],
        json!(format!("n007{}", "\\u{2b1b}\\u{2b1c}".repeat(30)))
    );
    let reason = v["whitelist"]["reason"].as_str().unwrap_or("");
    assert!(reason.chars().count() <= 512 + 8, "{}", reason.len());
    let human = home.run(&["status"]);
    assert_eq!(human.code, 0, "{}", human.stderr);
    assert!(inert_output(&human.stdout), "{}", head(&human.stdout));
    let lines: Vec<&str> = human
        .stdout
        .lines()
        .filter(|l| l.starts_with("whitelist"))
        .collect();
    assert_eq!(lines.len(), 1, "{}", head(&human.stdout));
    assert!(
        lines[0].chars().count() <= "whitelist: ".len() + 512 + 8,
        "{}",
        lines[0].chars().count()
    );
}

#[test]
fn status_shows_a_whitelist_that_is_a_string_as_one_bounded_line() {
    // A daemon-argv impostor answers `status` with a JSON string where the whitelist object
    // belongs: 6000 characters of `#` and space, laid out to fill the human's terminal rows. It
    // becomes one inert string of at most the prose budget, on the terminal and in `--json`.
    let home = Home::new();
    home.create("main", &[]);
    poison_cache(&home);
    let dir = tempfile::Builder::new()
        .prefix("zwt-")
        .tempdir_in("/tmp")
        .unwrap();
    let whitelist = serde_json::to_string(&"## ".repeat(2000)).unwrap();
    let _attacker = daemon_argv_impostor(&home, dir.path(), &[("ATTACKER_WHITELIST", &whitelist)]);
    let human = home.run(&["status"]);
    assert_eq!(human.code, 0, "{}", human.stderr);
    assert!(inert_output(&human.stdout), "{}", human.stdout);
    let lines: Vec<&str> = human
        .stdout
        .lines()
        .filter(|l| l.starts_with("whitelist: "))
        .collect();
    assert_eq!(lines.len(), 1, "{}", human.stdout);
    assert!(
        lines[0].len() <= "whitelist: ".len() + 512,
        "{}",
        lines[0].len()
    );
    let v = home.run(&["status", "--json"]).ok();
    let w = v["whitelist"].as_str().unwrap_or_default();
    assert!(!w.is_empty() && w.len() <= 512, "{}", v["whitelist"]);
}

#[test]
fn whitelist_list_shows_a_file_that_is_no_list_within_one_budget_and_draws_nothing() {
    // whitelist.json is anyone's to write. As `{"entries": [29 rows of `#` and space]}` it is no
    // list (it fails to parse), and its raw entries once came out one per line: a QR code at two
    // characters per module. Now each is `<invalid>`, under a count, all within one budget: with
    // 60 of them the budget runs out, and the rest are counted.
    let home = Home::new();
    home.create("main", &[]);
    let rows: Vec<String> = (0..60)
        .map(|i| {
            (0..62)
                .map(|j| if (i * 7 + j * 3) % 5 < 2 { '#' } else { ' ' })
                .collect()
        })
        .collect();
    fs::write(
        home.path().join("whitelist.json"),
        serde_json::to_vec(&json!({ "entries": rows })).unwrap(),
    )
    .unwrap();
    let r = home.run(&["whitelist", "list"]);
    assert_eq!(r.code, 0, "{} / {}", r.stdout, r.stderr);
    assert!(inert_output(&r.stdout), "{}", r.stdout);
    // No row of the drawing reaches the terminal: no line that is only `#` and space, with or
    // without quotes around it, and the entry lines stay within one prose budget.
    let grid = |l: &str| {
        let core = l.trim().trim_matches('"');
        !core.trim().is_empty() && core.chars().all(|c| c == '#' || c == ' ')
    };
    assert!(!r.stdout.lines().any(grid), "{}", r.stdout);
    let listed: usize = r
        .stdout
        .lines()
        .filter(|l| {
            l.starts_with("  ") && !l.contains("entries, in a file") && !l.starts_with("  ... ")
        })
        .map(str::len)
        .sum();
    assert!(listed <= 512, "{listed}: {}", r.stdout);
    assert!(
        r.stdout
            .contains("60 entries, in a file that is not a whitelist"),
        "{}",
        r.stdout
    );
    let shown = r.stdout.lines().filter(|l| *l == "  <invalid>").count();
    assert!(shown > 0 && shown < 60, "{}", r.stdout);
    assert!(
        r.stdout.contains(&format!(
            "  ... {} more; `zwallet whitelist list --json` shows them",
            60 - shown
        )),
        "{}",
        r.stdout
    );
    // `--json` still has every row, escaped.
    let v = home.run(&["whitelist", "list", "--json"]).ok();
    assert_eq!(v["entries"].as_array().unwrap().len(), 60, "{v}");
}

#[test]
fn whitelist_list_shows_an_unverified_parseable_list_within_one_budget() {
    // A list that parses but does not verify (here its 33 entries are replaced, so the signature
    // is no longer theirs): every field is one `add` accepts, but the nicknames are rows of `m`
    // and `_`, 32 columns of a 33-row grid. Without `--verify` all its entry lines share one
    // prose budget, and the rest are counted with how to see them; `--json` keeps all 33.
    let home = Home::new();
    home.create("main", &[]);
    home.allow("alice", PAYEE, &["base"]);
    let path = home.path().join("whitelist.json");
    let mut v: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let entries: Vec<serde_json::Value> = (0..33)
        .map(|i| {
            let row: String = (0..32)
                .map(|j| {
                    if j == 0 || (i * 5 + j * 3) % 7 < 3 {
                        'm'
                    } else {
                        '_'
                    }
                })
                .collect();
            json!({
                "nickname": row,
                "kind": "address",
                "address": format!("0x{:040x}", i + 1),
                "chains": ["*"],
            })
        })
        .collect();
    v["entries"] = json!(entries);
    fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
    let r = home.run(&["whitelist", "list"]);
    assert_eq!(r.code, 0, "{} / {}", r.stdout, r.stderr);
    assert!(inert_output(&r.stdout), "{}", r.stdout);
    let rows: Vec<&str> = r.stdout.lines().filter(|l| l.starts_with("  m")).collect();
    let listed: usize = rows.iter().map(|l| l.len()).sum();
    assert!(!rows.is_empty() && listed <= 512, "{listed}: {}", r.stdout);
    assert!(
        r.stdout.contains(&format!(
            "  ... {} more; `zwallet whitelist list --verify` shows them all",
            33 - rows.len()
        )),
        "{}",
        r.stdout
    );
    let v = home.run(&["whitelist", "list", "--json"]).ok();
    assert_eq!(v["verified"], false, "{v}");
    assert_eq!(v["entries"].as_array().unwrap().len(), 33, "{v}");
}

/// Rows of a picture made only of `#` and space (quotes around a row aside).
fn grid_row(l: &str) -> bool {
    let core = l.trim().trim_matches('"');
    !core.trim().is_empty() && core.chars().all(|c| c == '#' || c == ' ')
}

/// whitelist.json with its first entry's `field` set to `value` (the signature is then not
/// theirs: the list does not verify, but it is still read and shown).
fn plant_entry(home: &Home, field: &str, value: serde_json::Value) {
    let path = home.path().join("whitelist.json");
    let mut v: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    v["entries"][0][field] = value;
    fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
}

#[test]
fn r6_the_change_summary_shows_no_file_value_outside_adds_shape() {
    // R6-B1. The summary printed before the password once showed an entry's chains as found,
    // each up to 64 characters with spaces, only `, ` between them: one line of 50 rows of `#`
    // and space is a QR code. Such a file is no whitelist now (its chain is not a chain key's
    // shape), so the summary says it cannot be read, naming the field, never the value.
    let home = Home::new();
    home.create("main", &[]);
    home.allow("alice", PAYEE, &["base"]);
    let rows: Vec<String> = (0..50)
        .map(|i| {
            (0..64)
                .map(|j| if (i * 7 + j * 3) % 5 < 2 { '#' } else { ' ' })
                .collect()
        })
        .collect();
    plant_entry(&home, "chains", json!(rows));
    let r = whitelist_with(&home, &["remove", "alice"], "not the password");
    assert!(inert_output(&r.stderr), "{}", r.stderr);
    assert!(!r.stderr.lines().any(grid_row), "{}", r.stderr);
    assert!(
        r.stderr
            .contains("whitelist.json is there but cannot be read (")
            && r.stderr.contains(
                "not a whitelist file: a chain of entry 1 is not in the shape zwallet writes"
            ),
        "{}",
        r.stderr
    );
    // Chains in a chain key's shape that zwallet does not know: `<invalid>`, at most 16.
    let keys: Vec<String> = (0..50).map(|i| format!("m-m{i:02}-mm-m--m")).collect();
    plant_entry(&home, "chains", json!(keys));
    let r = whitelist_with(&home, &["remove", "alice"], "not the password");
    assert!(inert_output(&r.stderr), "{}", r.stderr);
    assert!(!r.stderr.contains("m-m0"), "{}", r.stderr);
    assert!(
        r.stderr
            .contains(&format!("{}and 34 more", "<invalid>, ".repeat(16))),
        "{}",
        r.stderr
    );
}

#[test]
fn r6_the_names_a_summary_shows_from_the_files_share_one_budget() {
    // Both list files with 20 nicknames of 32 characters each, rows of `m` and `_`: what the
    // reset summary names from them, on its "dropped" line and on each file's line, is at most
    // one 512-character budget in all; the rest are counted.
    let home = Home::new();
    home.create("main", &[]);
    home.allow("alice", PAYEE, &["base"]);
    let path = home.path().join("whitelist.json");
    let mut v: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let grid = |file: usize| -> Vec<serde_json::Value> {
        (0..20)
            .map(|i| {
                let row: String = (0..32)
                    .map(|j| {
                        if j == 0 || (i * 5 + j * 3 + file) % 7 < 3 {
                            'm'
                        } else {
                            '_'
                        }
                    })
                    .collect();
                json!({
                    "nickname": row,
                    "kind": "address",
                    "address": format!("0x{:040x}", file * 100 + i + 1),
                    "chains": ["*"],
                })
            })
            .collect()
    };
    v["entries"] = json!(grid(0));
    fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
    v["entries"] = json!(grid(1));
    v["gen"] = json!(2);
    fs::write(
        home.path().join("whitelist.next.json"),
        serde_json::to_vec(&v).unwrap(),
    )
    .unwrap();
    let r = whitelist_with(&home, &["reset"], "not the password");
    assert!(inert_output(&r.stderr), "{}", r.stderr);
    let named: usize = r
        .stderr
        .split([' ', ',', '(', ')', '\n'])
        .filter(|w| w.len() == 32 && w.starts_with('m') && w.chars().all(|c| c == 'm' || c == '_'))
        .map(str::len)
        .sum();
    assert!(named > 0 && named <= 512, "{named}: {}", r.stderr);
    assert!(r.stderr.contains(" more"), "{}", r.stderr);
}

#[test]
fn whitelist_list_shows_a_file_with_a_value_outside_adds_shape_as_no_list() {
    // A parseable file whose nickname is not one `add` writes is no whitelist: `whitelist list`
    // shows the count and `<invalid>` under one budget, and the reason names the field only.
    let home = Home::new();
    home.create("main", &[]);
    home.allow("alice", PAYEE, &["base"]);
    plant_entry(&home, "nickname", json!("## ## ## ##"));
    let r = home.run(&["whitelist", "list"]);
    assert_eq!(r.code, 0, "{} / {}", r.stdout, r.stderr);
    assert!(inert_output(&r.stdout), "{}", r.stdout);
    assert!(
        r.stdout
            .contains("1 entries, in a file that is not a whitelist")
            && r.stdout.contains(
                "not a whitelist file: the nickname of entry 1 is not in the shape zwallet writes"
            )
            && !r.stdout.contains("##"),
        "{}",
        r.stdout
    );
    // `--json` still has the entry as found, escaped.
    let v = home.run(&["whitelist", "list", "--json"]).ok();
    assert_eq!(v["entries"][0]["nickname"], "## ## ## ##", "{v}");
}

#[test]
fn whitelist_list_verify_shows_every_entry_of_a_verified_list() {
    // N3. A list the seal verifies is the human's own: all its entries, no "more" line.
    let home = Home::new();
    home.create("main", &[]);
    for i in 0..8 {
        home.allow(
            &format!("payee{i}"),
            &format!("0x{:040x}", i + 1),
            &["base"],
        );
    }
    let r = home.run_stdin(
        &["whitelist", "list", "--verify", "--password-stdin"],
        &format!("{PW}\n"),
    );
    assert_eq!(r.code, 0, "{} / {}", r.stdout, r.stderr);
    for i in 0..8 {
        assert!(
            r.stdout.contains(&format!("  payee{i} ")),
            "{i}: {}",
            r.stdout
        );
    }
    assert!(!r.stdout.contains(" more"), "{}", r.stdout);
}

#[test]
fn whitelist_list_verify_that_fails_points_to_json_not_to_verify_again() {
    // N2. After a `--verify` that did not verify, the hint for the entries left out is `--json`:
    // running `--verify` again would show the same few.
    let home = Home::new();
    home.create("main", &[]);
    for i in 0..12 {
        home.allow(
            &format!("payee{i:02}"),
            &format!("0x{:040x}", i + 1),
            &["base"],
        );
    }
    plant_entry(&home, "nickname", json!("renamed"));
    let r = home.run_stdin(
        &["whitelist", "list", "--verify", "--password-stdin"],
        &format!("{PW}\n"),
    );
    assert_eq!(r.code, 0, "{} / {}", r.stdout, r.stderr);
    assert!(
        r.stdout
            .contains("more; `zwallet whitelist list --json` shows them")
            && !r.stdout.contains("--verify` shows them all"),
        "{}",
        r.stdout
    );
}

#[test]
fn an_unverified_lists_fee_caps_spend_the_same_budget_as_its_entries() {
    // N5. A list that does not verify, with fee caps of 77 digits on every chain zwallet knows:
    // its entry lines and the caps it shows together stay within one 512-character budget, and
    // the caps left out are counted.
    let home = Home::new();
    home.create("main", &[]);
    home.allow("alice", PAYEE, &["base"]);
    let path = home.path().join("whitelist.json");
    let mut v: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let huge = format!("1{}", "0".repeat(76));
    for chain in [
        "arbitrum",
        "avalanche",
        "base",
        "bsc",
        "ethereum",
        "optimism",
        "polygon",
        "sepolia",
    ] {
        v["policy"]["maxFeeWei"][chain] = json!(huge);
    }
    v["entries"][0]["nickname"] = json!("renamed");
    fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
    let r = home.run(&["whitelist", "list"]);
    assert_eq!(r.code, 0, "{} / {}", r.stdout, r.stderr);
    assert!(inert_output(&r.stdout), "{}", r.stdout);
    let entries: usize = r
        .stdout
        .lines()
        .filter(|l| l.starts_with("  ") && !l.starts_with("  ... "))
        .map(str::len)
        .sum();
    let caps = r
        .stdout
        .lines()
        .find_map(|l| l.strip_prefix("fee caps: "))
        .unwrap_or_else(|| panic!("{}", r.stdout));
    let (shown, more) = caps
        .rsplit_once(", and ")
        .unwrap_or_else(|| panic!("{caps}"));
    assert!(more.ends_with(" more"), "{caps}");
    let shown: usize = shown.split(", ").map(str::len).sum();
    assert!(entries + shown <= 512, "{entries} + {shown}: {}", r.stdout);
}

#[test]
fn a_reset_shows_each_discarded_files_entries_as_one_line_of_at_most_512() {
    // N1. A reset drops a whitelist.json and a whitelist.next.json that are no lists, here each
    // with 2000 characters of `#` and space for its entries: each file's unverified entries are
    // one escaped line of at most 512 characters. A reset stops every daemon of this user, so no
    // other test may run meanwhile.
    let _alone = common::exclusive();
    let home = Home::new();
    home.create("main", &[]);
    home.allow("alice", PAYEE, &["base"]);
    // Two different files (a whitelist.next.json with whitelist.json's very bytes is a leftover
    // copy, not a discarded edit).
    for (file, grid) in [
        ("whitelist.json", "## ".repeat(667)),
        ("whitelist.next.json", " ##".repeat(667)),
    ] {
        fs::write(
            home.path().join(file),
            serde_json::to_vec(&json!({ "entries": grid })).unwrap(),
        )
        .unwrap();
    }
    let r = home.run_stdin(
        &["whitelist", "reset", "--password-stdin"],
        &format!("{PW}\n"),
    );
    assert_eq!(r.code, 0, "{} / {}", r.stdout, r.stderr);
    assert!(inert_output(&r.stdout), "{}", r.stdout);
    let found: Vec<&str> = r
        .stdout
        .lines()
        .filter_map(|l| l.split_once("its unverified entries: ").map(|(_, e)| e))
        .collect();
    assert_eq!(found.len(), 2, "{}", r.stdout);
    for e in found {
        assert!(e.len() <= 512, "{}", e.len());
    }
}

#[test]
fn whitelist_list_shows_a_value_add_would_refuse_as_invalid() {
    // A parseable list with a forged signature: its nickname is a 32-character `#`/space row,
    // which `add` would refuse. `whitelist list` shows `<invalid>` there, not the row.
    let home = Home::new();
    home.create("main", &[]);
    home.allow("alice", PAYEE, &["base"]);
    let path = home.path().join("whitelist.json");
    let mut v: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let row = "## ## ## ## ## ## ## ## ## ## ##";
    assert_eq!(row.len(), 32);
    v["entries"][0]["nickname"] = json!(row);
    fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
    let r = home.run(&["whitelist", "list"]);
    assert_eq!(r.code, 0, "{} / {}", r.stdout, r.stderr);
    assert!(
        r.stdout.contains("<invalid>") && !r.stdout.contains(row),
        "{}",
        r.stdout
    );
    assert!(inert_output(&r.stdout), "{}", r.stdout);
}

/// ABI-encodes `items` as the return data of a function returning `string[]`.
fn abi_string_array(items: &[String]) -> String {
    let word = |n: usize| format!("{n:064x}");
    let tails: Vec<String> = items
        .iter()
        .map(|s| {
            let mut data: String = s.bytes().map(|x| format!("{x:02x}")).collect();
            while !data.len().is_multiple_of(64) {
                data.push('0');
            }
            format!("{}{data}", word(s.len()))
        })
        .collect();
    let mut hex = word(32) + &word(items.len());
    let mut offset = 32 * items.len();
    for t in &tails {
        hex.push_str(&word(offset));
        offset += t.len() / 2;
    }
    for t in &tails {
        hex.push_str(t);
    }
    format!("0x{hex}")
}

#[test]
fn a_call_string_array_result_is_exact_in_json_and_bounded_on_the_terminal() {
    // 64 strings of 128 solid glyphs each: `--json` keeps them exactly; the human `result:` line
    // is one inert line within the prose budget, not 64 pieces each cut on their own.
    let items: Vec<String> = (0..64).map(|_| "\u{2b1b}".repeat(128)).collect();
    let encoded = abi_string_array(&items);
    let node = common::tiny_rpc_reply(move |method, _| match method {
        "eth_chainId" => Ok(json!("0x1")),
        "eth_call" => Ok(json!(encoded.clone())),
        _ => Ok(json!("0x0")),
    });
    let home = Home::new();
    let args = [
        "call",
        "--chain",
        "ethereum",
        "--to",
        "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
        "--sig",
        "names()(string[])",
        "--rpc",
        &node,
    ];
    let mut json_args = args.to_vec();
    json_args.push("--json");
    let v = home.run(&json_args).ok();
    assert_eq!(v["result"], json!([items]), "--json is exact");
    let human = home.run(&args);
    let head = |s: &str| s.chars().take(300).collect::<String>();
    assert_eq!(human.code, 0, "{}", human.stderr);
    assert!(inert_output(&human.stdout), "{}", head(&human.stdout));
    let result: Vec<&str> = human
        .stdout
        .lines()
        .filter(|l| l.starts_with("result: "))
        .collect();
    assert_eq!(result.len(), 1, "{}", head(&human.stdout));
    assert!(
        result[0].chars().count() <= "result: ".len() + 512 + 8,
        "{}",
        result[0].chars().count()
    );
}

#[test]
fn a_long_rpc_error_and_call_string_are_cut_on_the_terminal() {
    // The same from an RPC endpoint: its error text is cut where the error is built; a `call`
    // string is cut on the terminal and kept whole in `--json`.
    let payload = qr_by_wrapping();
    let failing = common::tiny_rpc_reply(move |method, _| match method {
        "eth_chainId" => Ok(json!("0x1")),
        _ => Err(json!({ "code": -32000, "message": qr_by_wrapping() })),
    });
    let home = Home::new();
    let token = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48";
    let call = |rpc: &str, json_out: bool| {
        let mut args = vec![
            "call",
            "--chain",
            "ethereum",
            "--to",
            token,
            "--sig",
            "name()(string)",
            "--rpc",
            rpc,
        ];
        if json_out {
            args.push("--json");
        }
        home.run(&args)
    };
    let msg = call(&failing, true).err("RPC_ERROR");
    assert!(msg.chars().count() < 700, "{}", msg.chars().count());
    assert!(inert(&msg), "{:?}", &msg[..200]);
    let human = call(&failing, false);
    assert_eq!(human.stderr.trim_end().lines().count(), 1);
    assert!(human.stderr.trim_end().ends_with(" [RPC_ERROR]"));
    let encoded = abi_string(&payload);
    let node = common::tiny_rpc_reply(move |method, _| match method {
        "eth_chainId" => Ok(json!("0x1")),
        "eth_call" => Ok(json!(encoded.clone())),
        _ => Ok(json!("0x0")),
    });
    let v = call(&node, true).ok();
    assert_eq!(v["result"], json!([payload]), "--json is exact");
    let human = call(&node, false);
    assert_eq!(human.code, 0, "{}", human.stderr);
    let result: Vec<&str> = human
        .stdout
        .lines()
        .filter(|l| l.starts_with("result: "))
        .collect();
    assert_eq!(result.len(), 1, "{:?}", &human.stdout[..200]);
    assert!(
        result[0].chars().count() < 700,
        "{}",
        result[0].chars().count()
    );
    assert!(inert_output(&human.stdout), "{:?}", &human.stdout[..200]);
}

#[test]
fn unlock_and_create_show_a_whitelist_reason_from_the_file_escaped() {
    // whitelist.json is anyone's to edit: a key the parser does not know is quoted by its message,
    // which the daemon reports as the list's reason, and `unlock` and `create --unlock` show it.
    // A bidi override, an isolate and a line separator there come out escaped.
    let home = Home::new();
    home.create("main", &[]);
    home.allow(
        "alice",
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
        &["base"],
    );
    let path = home.path().join("whitelist.json");
    let mut v: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    v.as_object_mut()
        .unwrap()
        .insert("ok\u{202e}evil\u{2066}\u{2028}\u{1b}[2J".into(), json!(1));
    fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
    let pw = format!("{PW}\n");
    for args in [
        &["unlock", "10m", "--password-stdin", "--json"][..],
        &[
            "create",
            "b",
            "--unlock",
            "10m",
            "--password-stdin",
            "--json",
        ],
    ] {
        let out = home.run_stdin(args, &pw);
        assert_eq!(out.code, 0, "{args:?}: {}", out.stdout);
        let w = out.json()["whitelist"].clone();
        assert_eq!(w["verified"], false, "{args:?}: {w}");
        let reason = w["reason"].as_str().unwrap_or("");
        assert!(
            reason.contains("evil"),
            "{args:?}: the key is still named: {reason}"
        );
        assert!(inert(reason), "{args:?}: {reason:?}");
        home.stop_daemons();
    }
    let human = home.run_stdin(&["unlock", "10m", "--password-stdin"], &pw);
    assert_eq!(human.code, 0, "{}", human.stderr);
    assert!(inert_output(&human.stdout), "{:?}", human.stdout);
    home.stop_daemons();
}

#[test]
fn a_repair_keeps_the_sealed_whitelist_and_the_running_session() {
    // The repair rewrites metadata only: the sealed master (with its whitelist anchor) is
    // untouched, so a session running meanwhile goes on and the whitelist still verifies.
    let home = Home::new();
    home.create("main", &[]);
    home.allow(
        "alice",
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
        &["base"],
    );
    let pid = home.unlock("10m")["daemonPid"].clone();
    poison_cache(&home);
    let pw = format!("{PW}\n");
    let v = home
        .run_stdin(&["address", "--verify", "--password-stdin", "--json"], &pw)
        .ok();
    assert!(v["repaired"].is_array(), "{v}");
    // The human form marks the old value.
    poison_cache(&home);
    let human = home.run_stdin(&["address", "--verify", "--password-stdin"], &pw);
    assert!(
        human
            .stdout
            .contains(&format!("was {POISON} (do not fund), now 0x")),
        "{}",
        human.stdout
    );
    let s = home.run(&["status", "--json"]).ok();
    assert_eq!(
        (&s["locked"], &s["daemonPid"]),
        (&json!(false), &pid),
        "{s}"
    );
    assert_eq!(
        (&s["whitelist"]["verified"], &s["whitelist"]["gen"]),
        (&json!(true), &json!(1)),
        "{s}"
    );
    let w = home
        .run_stdin(
            &[
                "whitelist",
                "list",
                "--verify",
                "--password-stdin",
                "--json",
            ],
            &pw,
        )
        .ok();
    assert_eq!(w["verified"], true, "{w}");
}

// ---- Tron (`.prd/05-tron.md`, TRON-1): the address from the same seed ----

/// The two lines above the QR code of a Tron address.
const TRON_CAPTION: &str = "(Tron only: TRX and TRC20 tokens; not an EVM address)";

/// Key 0's Tron address as the seed derives it, read with the password in this process.
fn derived_tron(home: &Home) -> String {
    let seed = store::unlock(home.path(), PW, &chains::derive_addresses)
        .unwrap()
        .0
        .seed;
    chains::derive_addresses(&seed, 0).unwrap()["tron"].clone()
}

#[test]
fn tr2_create_caches_the_tron_address_and_a_session_verifies_it_without_a_prompt() {
    let home = Home::new();
    let created = home.create("main", &[]);
    assert!(
        created["chains"]
            .as_array()
            .unwrap()
            .contains(&json!("tron")),
        "{created}"
    );
    let tron = derived_tron(&home);
    assert!(tron.starts_with('T') && tron.len() == 34, "{tron}");
    let on_disk: serde_json::Value =
        serde_json::from_slice(&fs::read(home.wallet_json()).unwrap()).unwrap();
    assert_eq!(on_disk["keys"][0]["addresses"]["tron"], tron.as_str());
    // Locked: the cache, not verified, and no code on a terminal.
    let v = home.run(&["address", "--chain", "tron", "--json"]).ok();
    assert_eq!(
        (&v["chain"], &v["address"], &v["verified"]),
        (&json!("tron"), &json!(tron), &json!(false)),
        "{v}"
    );
    let shown = home.run_on_terminal(&["address", "--chain", "tron"], &[], &[]);
    assert!(shown.contains(&format!("\n{LOCKED_NOTE}\n")), "{shown:?}");
    assert!(!shown.contains("scan to send") && common::scan_qr(&shown).is_empty());
    // The tron row of `addresses`.
    let all = home.run(&["addresses", "--json"]).ok();
    let row = all["addresses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["chainId"] == "tron")
        .unwrap_or_else(|| panic!("{all}"));
    assert_eq!(row["address"], tron.as_str());
    // Unlocked: the daemon's derivation, verified by the session where one verifies, no prompt.
    home.unlock("10m");
    let v = home.run(&["address", "--chain", "tron", "--json"]).ok();
    assert_eq!(v["address"], tron.as_str(), "{v}");
    if session_verifies() {
        assert_eq!(
            (&v["verified"], &v["verifiedBy"]),
            (&json!(true), &json!("session")),
            "{v}"
        );
        // Its own code, captioned as Tron's.
        let shown = home.run_on_terminal(&["address", "--chain", "tron"], &[], &[]);
        assert!(
            shown.contains(&format!("scan to send to {tron}\n{TRON_CAPTION}\n")),
            "{shown:?}"
        );
        assert!(!shown.contains(EVERY_NETWORK), "{shown:?}");
        assert_eq!(common::scan_qr(&shown), [tron.as_str()]);
        // `addresses`: two codes, the EVM one then the Tron one, each under its caption.
        let evm = created["address"].as_str().unwrap();
        let shown = home.run_on_terminal(&["addresses"], &[], &[]);
        let evm_at = shown
            .find(&format!("scan to send to {evm}\n{EVERY_NETWORK}\n"))
            .unwrap_or_else(|| panic!("{shown:?}"));
        let tron_at = shown
            .find(&format!("scan to send to {tron}\n{TRON_CAPTION}\n"))
            .unwrap_or_else(|| panic!("{shown:?}"));
        assert!(evm_at < tron_at, "{shown:?}");
        assert_eq!(common::scan_qr(&shown), [evm, tron.as_str()]);
    } else {
        assert_eq!(v["verified"], false, "{v}");
        let shown = home.run_on_terminal(&["address", "--chain", "tron"], &[], &[]);
        assert!(common::scan_qr(&shown).is_empty(), "{shown:?}");
    }
    // With the password: verified, and the code on a terminal.
    let v = home
        .run_stdin(
            &[
                "address",
                "--chain",
                "tron",
                "--verify",
                "--password-stdin",
                "--json",
            ],
            &format!("{PW}\n"),
        )
        .ok();
    assert_eq!(
        (&v["address"], &v["verifiedBy"]),
        (&json!(tron), &json!("password")),
        "{v}"
    );
    let shown = home.run_on_terminal(
        &["address", "--chain", "tron", "--verify"],
        &["Wallet password: "],
        &[],
    );
    assert_eq!(common::scan_qr(&shown), [tron.as_str()]);
}

#[test]
fn tr4_a_tampered_tron_cache_is_refused_and_shown_escaped() {
    let home = Home::new();
    home.create("main", &[]);
    let tron = derived_tron(&home);
    let tampered = [
        // TWer2... with its first letter's case flipped: 34 base58 characters, checksum fails.
        "tWer2Ygk5TEheHp3TPuYeqxmB6SsGZmaL6",
        // TWer2... with its last character changed: checksum fails.
        "TWer2Ygk5TEheHp3TPuYeqxmB6SsGZmaL7",
        // An EVM address where a Tron one belongs.
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
        // A terminal escape and a fake line.
        "T\u{1b}[2J\nverified: true",
    ];
    for value in tampered {
        home.edit_wallet(|v| v["keys"][0]["addresses"]["tron"] = json!(value));
        for args in [
            &["address", "--chain", "tron", "--json"][..],
            &["addresses", "--json"],
        ] {
            let out = home.run(args);
            let msg = out.err("VALIDATION_ERROR");
            assert!(msg.contains("which is not an address"), "{value:?}: {msg}");
            assert!(inert(&msg), "{value:?}: {msg:?}");
            assert!(out.json().get("address").is_none(), "{value:?}");
        }
        let human = home.run(&["address", "--chain", "tron"]);
        assert_eq!(human.code, 1, "{value:?}");
        assert!(human.stdout.is_empty(), "{:?}", human.stdout);
        assert!(inert_output(&human.stderr), "{:?}", human.stderr);
        assert_eq!(human.stderr.lines().count(), 1, "{:?}", human.stderr);
    }
    // A valid address that is not this key's: the daemon's derivation contradicts it (planted
    // after the unlock, which refuses a cache the seed contradicts).
    home.edit_wallet(|v| v["keys"][0]["addresses"]["tron"] = json!(tron));
    home.unlock("10m");
    home.edit_wallet(|v| v["keys"][0]["addresses"]["tron"] = json!(TRON_POISON));
    let msg = home
        .run(&["address", "--chain", "tron", "--json"])
        .err("VALIDATION_ERROR");
    assert!(msg.contains(genuine_daemon_mismatch()), "{msg}");
    assert!(msg.contains(TRON_POISON), "{msg}");
}

#[test]
fn a_daemon_answering_an_evm_address_for_tron_is_never_shown_as_one() {
    let home = Home::new();
    home.create("main", &[]);
    home.edit_wallet(|v| {
        v["keys"][0]["addresses"]
            .as_object_mut()
            .unwrap()
            .remove("tron");
    });
    let fake = FakeDaemon::answering(
        &home.sock(),
        json!({
            "locked": false, "pid": 1, "expiresAt": "2099-01-01T00:00:00Z",
            "chain": "tron", "index": 0, "active": "main", "address": POISON,
        }),
    );
    let out = home.run(&["address", "--chain", "tron", "--json"]);
    let msg = out.err("VALIDATION_ERROR");
    assert!(
        msg.contains(&format!(
            "returned {POISON} as a tron address, which is not an address"
        )),
        "{msg}"
    );
    assert!(out.json().get("address").is_none(), "{}", out.stdout);
    drop(fake);
}

#[test]
fn tr24_sign_message_call_and_invoke_on_tron_are_unsupported_and_so_for_now_are_transfer_and_tx() {
    let home = Home::new();
    let me = home.create("main", &["--unlock", "10m"])["address"]
        .as_str()
        .unwrap()
        .to_string();
    let tron_payee = "TPjjvMwjPoDC32V2dGDYTkLH4E5LAtBZ6C";
    let txid = "21b776c7957ba1213b8616414912c9e81643e15e3e55b59fe143b991fa35a690";
    let rpc = "http://127.0.0.1:1";
    for args in [
        &[
            "sign-message",
            "--chain",
            "tron",
            "--message",
            "hi",
            "--json",
        ][..],
        &[
            "call", "--chain", "tron", "--to", tron_payee, "--sig", "f()", "--rpc", rpc, "--json",
        ],
        &[
            "invoke", "--chain", "tron", "--to", tron_payee, "--sig", "f()", "--rpc", rpc, "--json",
        ],
        &[
            "transfer", "--chain", "tron", "--to", tron_payee, "--amount", "1", "--rpc", rpc,
            "--json",
        ],
        &["tx", "--chain", "tron", txid, "--rpc", rpc, "--json"],
    ] {
        let msg = home.run(args).err("CAPABILITY_UNSUPPORTED");
        assert!(msg.contains("tron"), "{args:?}: {msg}");
    }
    // The same commands on an EVM chain still get past that point (here to the closed port).
    home.run(&[
        "call", "--chain", "base", "--to", &me, "--sig", "f()", "--rpc", rpc, "--json",
    ])
    .err("RPC_ERROR");
}

#[test]
fn tron_whitelist_add_defaults_to_tron_and_shows_its_family_without_changing_signed_shape() {
    let _exclusive = common::exclusive();
    let home = Home::new();
    home.create("main", &[]);
    let address = "TPjjvMwjPoDC32V2dGDYTkLH4E5LAtBZ6C";
    let stored = "0x9705bf55c3dcc6d277ebb8fe2a68762268822ba2";
    let pw = format!("{PW}\n");
    let out = home.run_stdin(
        &[
            "whitelist",
            "add",
            "bob",
            address,
            "--password-stdin",
            "--json",
        ],
        &pw,
    );
    out.ok();
    assert!(out.stderr.contains(address));
    assert!(out.stderr.contains(&format!("stored as {stored}")));
    let file: serde_json::Value =
        serde_json::from_slice(&std::fs::read(home.path().join("whitelist.json")).unwrap())
            .unwrap();
    assert_eq!(file["entries"][0]["address"], stored);
    assert_eq!(file["entries"][0]["chains"], json!(["tron"]));
    let list = home
        .run_stdin(
            &[
                "whitelist",
                "list",
                "--verify",
                "--password-stdin",
                "--json",
            ],
            &pw,
        )
        .ok();
    assert_eq!(list["verified"], true);
    assert_eq!(list["entries"][0]["address"], address);
    assert_eq!(list["entries"][0]["family"], "tron");
    for args in [
        vec!["fee-cap", "tron", "30"],
        vec!["native-cap", "tron", "2"],
        vec![
            "token-cap",
            "tron",
            "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t",
            "1000000",
        ],
        vec!["calldata-cap", "tron", "68"],
    ] {
        let mut cmd = vec!["whitelist"];
        cmd.extend(args);
        cmd.extend(["--password-stdin", "--json"]);
        home.run_stdin(&cmd, &pw).ok();
    }
}

#[test]
fn tron_whitelist_rejects_wrong_families_checksums_and_contract_entries_before_password() {
    let home = Home::new();
    home.create("main", &[]);
    let addr = "TPjjvMwjPoDC32V2dGDYTkLH4E5LAtBZ6C";
    let bad = "TPjjvMwjPoDC32V2dGDYTkLH4E5LAtBZ6D";
    for (address, extra) in [
        (addr, vec!["--chains", "base"]),
        (addr, vec!["--chains", "*"]),
        (BOB, vec!["--chains", "tron"]),
        (addr, vec!["--chains", "tron,base"]),
        (bad, vec![]),
        (addr, vec!["--contract", "--selectors", "0xa9059cbb"]),
    ] {
        let mut args = vec!["whitelist", "add", "bob", address];
        args.extend(extra);
        args.extend(["--password-stdin", "--json"]);
        let out = home.run_stdin(&args, "wrong\n");
        out.err("VALIDATION_ERROR");
        assert!(!out.stderr.contains("password"), "{}", out.stderr);
        assert!(!home.path().join("whitelist.json").exists());
    }
}

// ---- Tron (TRON-2): a key made before this build knew Tron ----

#[test]
fn tr3_a_key_without_a_tron_cache_is_locked_until_unlock_or_verify_adds_it() {
    let home = Home::new();
    let evm = home.create("main", &[])["address"]
        .as_str()
        .unwrap()
        .to_string();
    let tron = derived_tron(&home);
    // As a wallet made before this version: no `tron` in its cache.
    home.edit_wallet(|v| {
        v["keys"][0]["addresses"]
            .as_object_mut()
            .unwrap()
            .remove("tron");
    });
    // Locked: nothing to show, and the error says how to get it.
    let msg = home
        .run(&["address", "--chain", "tron", "--json"])
        .err("WALLET_LOCKED");
    assert!(msg.contains("zwallet unlock"), "{msg}");
    assert!(
        msg.contains("zwallet address --chain tron --verify"),
        "{msg}"
    );
    // `addresses` still lists every other chain; the tron row says why it has no address.
    let all = home.run(&["addresses", "--json"]).ok();
    let rows = all["addresses"].as_array().unwrap();
    for row in rows {
        if row["chainId"] == "tron" {
            assert_eq!(row["errorCode"], "WALLET_LOCKED", "{row}");
            assert!(row.get("address").is_none(), "{row}");
        } else {
            assert_eq!(row["address"], evm.as_str(), "{row}");
        }
    }
    // With the password: added from the seed, said so, and no tamper warning.
    let v = home
        .run_stdin(
            &[
                "address",
                "--chain",
                "tron",
                "--verify",
                "--password-stdin",
                "--json",
            ],
            &format!("{PW}\n"),
        )
        .ok();
    assert_eq!(
        (&v["address"], &v["verifiedBy"]),
        (&json!(tron), &json!("password")),
        "{v}"
    );
    assert_eq!(
        v["added"],
        json!([{ "key": "main", "chain": "tron", "now": tron }]),
        "{v}"
    );
    assert!(
        v.get("warning").is_none() && v.get("repaired").is_none(),
        "{v}"
    );
    // Then cached: the locked read shows it.
    let v = home.run(&["address", "--chain", "tron", "--json"]).ok();
    assert_eq!(v["address"], tron.as_str(), "{v}");
    // A second --verify has nothing to add.
    let v = home
        .run_stdin(
            &["address", "--verify", "--password-stdin", "--json"],
            &format!("{PW}\n"),
        )
        .ok();
    assert!(v.get("added").is_none(), "{v}");
    // The human text names what was added.
    home.edit_wallet(|v| {
        v["keys"][0]["addresses"]
            .as_object_mut()
            .unwrap()
            .remove("tron");
    });
    let human = home.run_stdin(
        &["address", "--chain", "tron", "--verify", "--password-stdin"],
        &format!("{PW}\n"),
    );
    assert_eq!(human.code, 0, "{}", human.stderr);
    assert!(
        human
            .stdout
            .contains(&format!("added: key 'main' tron: {tron}")),
        "{}",
        human.stdout
    );
    assert!(!human.stdout.contains("warning"), "{}", human.stdout);
}

#[test]
fn t44_zero_whitelist_add_refuses_before_password_and_keeps_running_session() {
    let _exclusive = common::exclusive();
    let home = Home::new();
    let pid = home.create("main", &["--unlock", "10m"])["daemonPid"]
        .as_i64()
        .unwrap() as i32;
    let before = fs::read(home.wallet_json()).unwrap();
    let tron_zero = chains::tron::address::from_body(&[0; 20]);
    for address in [
        "0x0000000000000000000000000000000000000000",
        tron_zero.as_str(),
    ] {
        for contract in [false, true] {
            let mut args = vec![
                "whitelist",
                "add",
                "zero",
                address,
                "--password-stdin",
                "--json",
            ];
            if contract {
                args.extend(["--contract", "--selectors", "0x12345678"]);
            }
            // EOF would be a password-input failure if the command ever reached the prompt.
            let out = home.run_stdin(&args, "");
            let message = out.err("VALIDATION_ERROR");
            assert!(message.contains("zero address"), "{message}");
            assert!(
                !out.stderr.contains("The password authorizes"),
                "{}",
                out.stderr
            );
            assert_eq!(before, fs::read(home.wallet_json()).unwrap());
            assert!(!home.path().join("whitelist.json").exists());
            assert!(!home.path().join("whitelist.next.json").exists());
            assert!(pid_alive(pid));
            assert_eq!(
                home.run(&["status", "--json"]).ok()["daemonPids"],
                json!([pid])
            );
        }
    }
}
