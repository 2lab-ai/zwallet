//! TUI6 runtime acceptance. These process-global PTYs run only in the serialized full gate.
mod common;
use common::{Home, PW, PtyChild};
use std::time::Duration;
use zwallet::core::whitelist::Entry;
const WAIT: Duration = Duration::from_secs(45);

fn wait(tui: &mut PtyChild, text: &str) {
    tui.wait_frame(WAIT, |frame| frame.rows.join("\n").contains(text));
}
fn start(home: &Home) -> PtyChild {
    let mut command = home.command(&["tui", "--chain", "base"]);
    if common::ptrace_scope_is_open() {
        command.arg("--allow-ptrace-scope-0");
    }
    let mut tui = PtyChild::spawn_full_screen(command, (80, 24));
    tui.wait_frame(WAIT, |frame| frame.rows[22].contains("main"));
    tui.send(b"5");
    wait(&mut tui, "not verified");
    tui
}
fn add(tui: &mut PtyChild, nickname: &str, address: &str, unlock: &str) {
    tui.send(b"a");
    wait(tui, "add payee");
    tui.send(format!("{nickname}\r{address}\rbase\r{unlock}\r").as_bytes());
    wait(tui, "The password authorizes");
    common::open_whitelist_password(tui, WAIT);
    // Raw mode consumes CR as Enter; LF is not a submit key.
    tui.send(format!("{PW}\r").as_bytes());
    wait(tui, &format!("added {nickname}"));
}
fn verified(home: &Home) -> serde_json::Value {
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
fn quit(tui: &mut PtyChild) {
    tui.send(b"q");
    let (status, output) = tui.wait_exit(WAIT);
    assert!(status.success(), "{output}");
    assert!(!String::from_utf8_lossy(&tui.output()).contains(PW));
    assert_eq!(
        tui.local_modes() & (libc::ECHO | libc::ICANON),
        libc::ECHO | libc::ICANON,
        "the terminal is restored after the edit flow"
    );
}

fn files(home: &Home) -> Vec<Option<Vec<u8>>> {
    ["wallet.json", "whitelist.json", "whitelist.next.json"]
        .iter()
        .map(|name| match std::fs::read(home.path().join(name)) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => panic!("reading {name}: {error}"),
        })
        .collect()
}

struct BeforeEdit {
    files: Vec<Option<Vec<u8>>>,
    status: serde_json::Value,
}
fn before_edit(home: &Home) -> BeforeEdit {
    BeforeEdit {
        files: files(home),
        status: home.run(&["status", "--json"]).ok(),
    }
}

fn local_verify(tui: &mut PtyChild, generation: u64) {
    tui.send(b"V");
    wait(tui, "Verify local list with the password");
    wait(tui, "Wallet password");
    tui.send(format!("{PW}\r").as_bytes());
    wait(tui, &format!("verified by password: gen {generation}"));
}

fn list_after_outcome(tui: &mut PtyChild, generation: u64) {
    tui.send(b"\r");
    wait(tui, &format!("verified by password: gen {generation}"));
}

// Summaries wrap at 72 columns. Ignore only frame borders and whitespace when matching a
// sentence across adjacent rows; every byte of its actual words/numbers must still appear.
fn compact(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != '|')
        .collect()
}

/// Visit every review page before opening the password reader. The plan must leave all three
/// authenticated files and the current daemon status intact until the password is submitted.
fn authorize(
    tui: &mut PtyChild,
    home: &Home,
    before: BeforeEdit,
    expected: &[&str],
    outcome: &str,
) {
    let mut frame = tui.wait_frame(WAIT, |frame| {
        common::whitelist_summary_position(frame).is_some()
    });
    let mut pages = Vec::new();
    loop {
        let (start, total) = common::whitelist_summary_position(&frame).expect("review caption");
        assert!(!frame.rows.join("\n").contains("Wallet password"));
        pages.push(compact(&frame.rows.join("\n")));
        if start - 1 + 10 >= total {
            break;
        }
        let next = (start - 1 + 10).min(total.saturating_sub(10)) + 1;
        assert!(next > start, "review must make forward progress");
        tui.send(b"\r");
        frame = tui.wait_frame(WAIT, |frame| {
            common::whitelist_summary_position(frame).is_some_and(|(start, _)| start == next)
        });
    }
    for text in expected.iter().copied().chain([
        "The password authorizes",
        "every zwallet daemon of this user is stopped first",
        "after the change: the wallet stays locked",
    ]) {
        assert!(
            pages.iter().any(|page| page.contains(&compact(text))),
            "summary omitted {text:?}: {pages:?}"
        );
    }
    assert_eq!(files(home), before.files, "review did not write files");
    assert_eq!(home.run(&["status", "--json"]).ok(), before.status);
    tui.send(b"\r");
    wait(tui, "Wallet password");
    assert_eq!(
        files(home),
        before.files,
        "password prompt did not write files"
    );
    assert_eq!(home.run(&["status", "--json"]).ok(), before.status);
    tui.send(format!("{PW}\r").as_bytes());
    wait(tui, outcome);
}

fn locked_verified(home: &Home, generation: u64) -> serde_json::Value {
    assert_eq!(home.run(&["status", "--json"]).ok()["locked"], true);
    let list = verified(home);
    assert_eq!(list["verified"], true);
    assert_eq!(list["gen"], generation);
    list
}
#[test]
fn t7_t11_whitelist_add_uses_real_worker_and_returns_verified_locked_or_explicitly_unlocked() {
    let _exclusive = common::exclusive();
    let home = Home::new();
    home.create("main", &["--unlock", "10m"]);
    let mut tui = start(&home);
    add(
        &mut tui,
        "bob",
        "0x3333333333333333333333333333333333333333",
        "",
    );
    assert_eq!(home.run(&["status", "--json"]).ok()["locked"], true);
    let list = verified(&home);
    assert_eq!(list["verified"], true);
    assert_eq!(list["gen"], 1);
    tui.send(b"\r");
    wait(&mut tui, "verified by password: gen 1");
    add(
        &mut tui,
        "alice",
        "0x4444444444444444444444444444444444444444",
        "10m",
    );
    assert_eq!(home.run(&["status", "--json"]).ok()["locked"], false);
    let list = verified(&home);
    assert_eq!(list["gen"], 2);
    assert_eq!(list["entries"].as_array().unwrap().len(), 2);
    quit(&mut tui);
}
#[test]
fn t11_whitelist_password_cancel_and_retry_do_not_echo_secret() {
    let _exclusive = common::exclusive();
    let home = Home::new();
    home.create("main", &[]);
    let before = std::fs::read(home.wallet_json()).unwrap();
    let mut tui = start(&home);
    tui.send(b"a");
    wait(&mut tui, "add payee");
    tui.send(b"bob\r0x3333333333333333333333333333333333333333\rbase\r\r");
    wait(&mut tui, "The password authorizes");
    common::open_whitelist_password(&mut tui, WAIT);
    tui.send(b"cancelled-secret\x1b");
    wait(&mut tui, "not verified");
    assert_eq!(before, std::fs::read(home.wallet_json()).unwrap());
    assert!(!home.path().join("whitelist.json").exists());
    add(
        &mut tui,
        "bob",
        "0x3333333333333333333333333333333333333333",
        "",
    );
    assert_eq!(verified(&home)["gen"], 1);
    assert!(!String::from_utf8_lossy(&tui.output()).contains("cancelled-secret"));
    quit(&mut tui);
}

#[test]
fn t7_t23_pty_warm_verify_then_remove_fee_cap_and_reset_match_cli() {
    let _exclusive = common::exclusive();
    let home = Home::new();
    home.create("main", &[]);
    home.edit_whitelist(|list| {
        list.add(Entry::new_address(
            "bob",
            "0x3333333333333333333333333333333333333333",
            &["base"],
        ))
        .unwrap();
        list.add(Entry::new_address(
            "carol",
            "0x4444444444444444444444444444444444444444",
            &["base"],
        ))
        .unwrap();
        list.policy.allow_message_signing = true;
    });
    let initial = verified(&home);
    home.unlock("10m");
    let before = files(&home);
    let status = home.run(&["status", "--json"]).ok();
    assert_eq!(status["locked"], false);
    let pid = home.daemon_pid().expect("warm session");
    let mut tui = start(&home);
    assert_eq!(files(&home), before);
    assert_eq!(home.run(&["status", "--json"]).ok(), status);
    assert!(!String::from_utf8_lossy(&tui.output()).contains("Wallet password"));
    local_verify(&mut tui, 1);
    assert_eq!(files(&home), before, "V is read-only");
    assert_eq!(home.run(&["status", "--json"]).ok(), status);
    assert!(
        common::pid_alive(pid),
        "V leaves the original session alive"
    );

    wait(&mut tui, "selected for d: bob");
    let before = before_edit(&home);
    tui.send(b"d");
    wait(&mut tui, "remove: bob");
    tui.send(b"\r");
    authorize(&mut tui, &home, before, &["remove bob:"], "removed bob");
    assert!(common::wait_dead(pid, WAIT));
    let removed = locked_verified(&home, 2);
    assert_eq!(removed["entries"].as_array().unwrap().len(), 1);
    assert_eq!(removed["entries"][0]["nickname"], "carol");
    list_after_outcome(&mut tui, 2);

    let before = before_edit(&home);
    tui.send(b"f");
    wait(&mut tui, "fee cap");
    // The chain field starts at the active base chain; blank duration keeps it locked.
    tui.send(b"\r0.001\r\r");
    authorize(
        &mut tui,
        &home,
        before,
        &["fee-cap base:", "-> 0.001 ETH", "1000000000000000 wei"],
        "fee cap on base set to 0.001 ETH",
    );
    let capped = locked_verified(&home, 3);
    assert_eq!(capped["policy"]["maxFeeWei"]["base"], "1000000000000000");
    assert_eq!(capped["entries"], removed["entries"]);
    list_after_outcome(&mut tui, 3);

    let before = before_edit(&home);
    tui.send(b"R");
    wait(&mut tui, "reset policy and rotate its signing key");
    tui.send(b"\r");
    authorize(
        &mut tui,
        &home,
        before,
        &[
            "reset: a new whitelist key and an empty list",
            "every entry is dropped: carol",
            "all per-transaction native/token/calldata limits are removed",
            "fee caps after the reset, the built-in defaults:",
            "message signing after the reset: off",
            "the reset allows nothing else (no payee, no contract)",
            "restore the current one instead if you have it",
        ],
        "reset: a new key and an empty list",
    );
    let reset = locked_verified(&home, 4);
    assert!(reset["entries"].as_array().unwrap().is_empty());
    assert_eq!(reset["policy"]["maxFeeWei"], initial["policy"]["maxFeeWei"]);
    assert_eq!(reset["policy"]["allowMessageSigning"], false);
    quit(&mut tui);
}

#[test]
fn t24_pty_contract_selectors_warning_registry_refusal_and_individual_removal() {
    let _exclusive = common::exclusive();
    let home = Home::new();
    home.create("main", &[]);
    home.allow(
        "bob",
        "0x3333333333333333333333333333333333333333",
        &["base"],
    );
    let mut tui = start(&home);
    let before = before_edit(&home);
    tui.send(b"C");
    wait(&mut tui, "add contract (EVM only)");
    tui.send(b"router\r0x5555555555555555555555555555555555555555\rethereum\r0x414bf389\r\r");
    authorize(
        &mut tui,
        &home,
        before,
        &[
            "add router: a contract",
            "chains: ethereum",
            "selectors: 0x414bf389",
        ],
        "added router",
    );
    let warning = "a listed contract may forward value or act on its arguments";
    tui.wait_frame(WAIT, |frame| {
        compact(&frame.rows.join("\n")).contains(&compact(warning))
    });
    let added = locked_verified(&home, 2);
    let router = added["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["nickname"] == "router")
        .unwrap();
    assert_eq!(router["kind"], "contract");
    assert_eq!(
        router["address"],
        "0x5555555555555555555555555555555555555555"
    );
    assert_eq!(router["chains"], serde_json::json!(["ethereum"]));
    assert_eq!(router["selectors"], serde_json::json!(["0x414bf389"]));
    list_after_outcome(&mut tui, 2);

    home.unlock("10m");
    let pid = home.daemon_pid().expect("session before invalid contract");
    let before = files(&home);
    let status = home.run(&["status", "--json"]).ok();
    let token = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48";
    let cli_refusal = home
        .run_stdin(
            &[
                "whitelist",
                "add",
                "token",
                token,
                "--contract",
                "--chains",
                "ethereum",
                "--selectors",
                "0x414bf389",
                "--password-stdin",
                "--json",
            ],
            "",
        )
        .err("VALIDATION_ERROR");
    assert!(cli_refusal.contains("a registry token cannot be a contract entry"));
    tui.send(b"C");
    wait(&mut tui, "add contract (EVM only)");
    let output_start = tui.output().len();
    tui.send(format!("token\r{token}\rethereum\r0x414bf389\r\r").as_bytes());
    wait(&mut tui, "registry token cannot be");
    let refusal = tui.frame().rows.join("\n");
    assert!(refusal.contains("add contract (EVM only)"));
    assert!(!String::from_utf8_lossy(&tui.output()[output_start..]).contains("Wallet password"));
    assert_eq!(files(&home), before);
    assert_eq!(home.run(&["status", "--json"]).ok(), status);
    assert!(
        common::pid_alive(pid),
        "invalid contract must not stop a session"
    );

    tui.send(b"\x1b");
    wait(&mut tui, "verified by password: gen 2");
    wait(&mut tui, "selected for d: bob");
    tui.send(b"j");
    wait(&mut tui, "selected for d: router");
    let before = before_edit(&home);
    tui.send(b"d");
    wait(&mut tui, "remove: router");
    tui.send(b"\r");
    authorize(
        &mut tui,
        &home,
        before,
        &["remove router:", "a contract", "ethereum"],
        "removed router",
    );
    let removed = locked_verified(&home, 3);
    assert_eq!(removed["entries"].as_array().unwrap().len(), 1);
    assert_eq!(removed["entries"][0]["nickname"], "bob");
    assert!(common::wait_dead(pid, WAIT));
    quit(&mut tui);
}

#[test]
fn t25_pty_message_policy_discloses_old_and_new_and_cli_verifies_both_directions() {
    let _exclusive = common::exclusive();
    let home = Home::new();
    home.create("main", &[]);
    home.allow(
        "bob",
        "0x3333333333333333333333333333333333333333",
        &["base"],
    );
    let mut tui = start(&home);
    local_verify(&mut tui, 1);
    for (generation, on, summary, outcome) in [
        (
            2,
            true,
            "allow-message-signing: off -> on",
            "message signing allowed",
        ),
        (
            3,
            false,
            "allow-message-signing: on -> off",
            "message signing refused",
        ),
    ] {
        let before = before_edit(&home);
        tui.send(b"m");
        wait(&mut tui, "message signing (left/right toggles)");
        tui.send(b"\r\r");
        authorize(
            &mut tui,
            &home,
            before,
            &[summary, "as spending authority"],
            outcome,
        );
        let list = locked_verified(&home, generation);
        assert_eq!(list["policy"]["allowMessageSigning"], on);
        assert_eq!(list["entries"][0]["nickname"], "bob");
        list_after_outcome(&mut tui, generation);
    }
    quit(&mut tui);
}

#[test]
fn t44_pty_zero_add_is_refused_before_password_stop_or_write() {
    let _exclusive = common::exclusive();
    let home = Home::new();
    home.create("main", &["--unlock", "10m"]);
    let pid = home
        .daemon_pid()
        .expect("session before zero-address refusal");
    let before = files(&home);
    let status = home.run(&["status", "--json"]).ok();
    let mut tui = start(&home);
    tui.send(b"a");
    wait(&mut tui, "add payee");
    tui.send(b"z\r0x0000000000000000000000000000000000000000\rbase\r\r");
    wait(
        &mut tui,
        "the zero address cannot be added to the whitelist",
    );
    assert!(!String::from_utf8_lossy(&tui.output()).contains("Wallet password"));
    assert_eq!(files(&home), before);
    assert_eq!(home.run(&["status", "--json"]).ok(), status);
    assert!(common::pid_alive(pid));
    tui.send(b"\x1b");
    wait(&mut tui, "not verified");
    quit(&mut tui);
}
