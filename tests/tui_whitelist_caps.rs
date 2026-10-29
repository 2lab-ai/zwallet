//! T45–47: real TUI fields -> prepared summary -> password worker -> signed CLI verification.
//! Process-global PTYs run only in the parent's serialized gate, never against a real wallet.
mod common;

use common::{Home, PW, PtyChild};
use serde_json::Value;
use std::time::Duration;

const WAIT: Duration = Duration::from_secs(45);
const USDC: &str = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48";
const PAYEE: &str = "0x3333333333333333333333333333333333333333";

fn wait(tui: &mut PtyChild, text: &str) -> common::Frame {
    tui.wait_frame(WAIT, |frame| frame.rows.join("\n").contains(text))
}

fn start(home: &Home) -> PtyChild {
    let mut command = home.command(&["tui", "--chain", "ethereum"]);
    for chain in zwallet::chains::CHAINS {
        command.env(chain.rpc_env_var(), "http://127.0.0.1:1");
    }
    if common::ptrace_scope_is_open() {
        command.arg("--allow-ptrace-scope-0");
    }
    let mut tui = PtyChild::spawn_full_screen(command, (80, 24));
    tui.wait_frame(WAIT, |frame| {
        frame.rows[22].contains("main") && frame.rows[23].starts_with("locked | ")
    });
    tui.send(b"5");
    wait(&mut tui, "not verified");
    tui
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

fn verified(home: &Home, generation: u64) -> Value {
    assert_eq!(home.run(&["status", "--json"]).ok()["locked"], true);
    assert!(home.daemon_pids().is_empty());
    assert!(!home.path().join("whitelist.next.json").exists());
    let list = home
        .run_stdin(
            &[
                "whitelist",
                "list",
                "--verify",
                "--password-stdin",
                "--json",
            ],
            &format!("{PW}\n"),
        )
        .ok();
    assert_eq!(list["verified"], true);
    assert_eq!(list["gen"], generation);
    list
}

// Only whitespace and the known frame border are ignored when a sentence spans rows.
fn compact(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != '|')
        .collect()
}

fn summary_position(frame: &common::Frame) -> Option<(usize, usize)> {
    frame.rows.iter().find_map(|row| {
        let (_, caption) = row.split_once("Summary rows ")?;
        let (range, total) = caption.split_once(" of ")?;
        let start = range.split_once('-')?.0.parse().ok()?;
        let total = total.split_whitespace().next()?.parse().ok()?;
        Some((start, total))
    })
}

fn authorize(
    tui: &mut PtyChild,
    home: &Home,
    before: &[Option<Vec<u8>>],
    expected: &[&str],
    outcome: &str,
) {
    let status = home.run(&["status", "--json"]).ok();
    let mut frame = tui.wait_frame(WAIT, |frame| summary_position(frame).is_some());
    let mut pages = Vec::new();
    loop {
        let (start, total) = summary_position(&frame).expect("review caption");
        assert!(!frame.rows.join("\n").contains("Wallet password"));
        pages.push(compact(&frame.rows.join("\n")));
        if start - 1 + 10 >= total {
            break;
        }
        let next = (start - 1 + 10).min(total.saturating_sub(10)) + 1;
        assert!(next > start);
        tui.send(b"\r");
        frame = tui.wait_frame(WAIT, |frame| {
            summary_position(frame).is_some_and(|(start, _)| start == next)
        });
    }
    for text in expected.iter().copied().chain([
        "The password authorizes",
        "per transaction, not cumulative",
        "every zwallet daemon of this user is stopped first",
        "after the change: the wallet stays locked",
    ]) {
        assert!(
            pages.iter().any(|page| page.contains(&compact(text))),
            "summary omitted {text:?}"
        );
    }
    assert_eq!(files(home), before, "review must not write policy files");
    assert_eq!(home.run(&["status", "--json"]).ok(), status);
    tui.send(b"\r");
    wait(tui, "Wallet password");
    assert_eq!(files(home), before, "password prompt must not write files");
    // The TUI is in raw mode: CR is Enter, unlike the CLI password pipe's LF.
    tui.send(format!("{PW}\r").as_bytes());
    wait(tui, outcome);
}

fn list_after_outcome(tui: &mut PtyChild, generation: u64) {
    tui.send(b"\r");
    wait(tui, &format!("verified by password: gen {generation}"));
}

fn refused(tui: &mut PtyChild, home: &Home, before: &[Option<Vec<u8>>], error: &str) {
    let frame = wait(tui, error);
    let shown = frame.rows.join("\n");
    assert!(!shown.contains("Wallet password"));
    assert!(summary_position(&frame).is_none());
    assert_eq!(files(home), before, "invalid fields must not write files");
    assert_eq!(home.run(&["status", "--json"]).ok()["locked"], true);
    assert!(home.daemon_pids().is_empty());
    // No password has been sent in this process yet; no reader may have opened.
    assert!(
        !String::from_utf8_lossy(&tui.output())
            .to_ascii_lowercase()
            .contains("password:")
    );
    tui.send(b"\x1b");
    wait(tui, "not verified");
}

fn quit(tui: &mut PtyChild) {
    tui.send(b"q");
    let (status, stderr) = tui.wait_exit(WAIT);
    assert!(status.success(), "{status}");
    assert!(stderr.is_empty());
    assert!(!String::from_utf8_lossy(&tui.output()).contains(PW));
    assert_eq!(
        tui.local_modes() & (libc::ECHO | libc::ICANON),
        libc::ECHO | libc::ICANON
    );
    let frame = tui.final_frame(WAIT);
    assert!(frame.stray.is_empty() && frame.pending.is_empty());
}

#[test]
fn t45_native_cap_shows_units_and_impact_saves_signed_wei_and_none_removes_it() {
    let _exclusive = common::exclusive();
    let home = Home::new();
    home.create("main", &[]);
    home.allow("seed", PAYEE, &["ethereum"]);
    let before = files(&home);
    let mut tui = start(&home);

    tui.send(b"n");
    wait(&mut tui, "per-transaction native cap");
    // Chain is prefilled; 19 fractional digits exceed ETH's 18 before any password.
    tui.send(b"\r1.0000000000000000001\r\r");
    refused(&mut tui, &home, &before, "19 fractional digits");
    assert!(
        verified(&home, 1)["policy"]["transactionLimits"]["ethereum"]["maxNativeWei"].is_null()
    );

    tui.send(b"n");
    wait(&mut tui, "per-transaction native cap");
    tui.send(b"\r0.01\r\r");
    authorize(
        &mut tui,
        &home,
        &before,
        &[
            "native-cap ethereum",
            "native value (wei)",
            "10000000000000000",
        ],
        "native-cap on ethereum:",
    );
    assert_eq!(
        verified(&home, 2)["policy"]["transactionLimits"]["ethereum"]["maxNativeWei"],
        "10000000000000000"
    );
    let before = files(&home);
    list_after_outcome(&mut tui, 2);
    tui.send(b"n");
    wait(&mut tui, "per-transaction native cap");
    tui.send(b"\rnone\r\r");
    authorize(
        &mut tui,
        &home,
        &before,
        &[
            "native-cap ethereum",
            "10000000000000000 -> none (unbounded)",
        ],
        "native-cap on ethereum:",
    );
    assert!(
        verified(&home, 3)["policy"]["transactionLimits"]["ethereum"]["maxNativeWei"].is_null()
    );
    quit(&mut tui);
}

#[test]
fn t46_token_cap_rejects_nonregistry_then_saves_exact_base_units_and_none_removes_it() {
    let _exclusive = common::exclusive();
    let home = Home::new();
    home.create("main", &[]);
    home.allow("seed", PAYEE, &["ethereum"]);
    let before = files(&home);
    let mut tui = start(&home);

    tui.send(b"t");
    wait(&mut tui, "per-transaction token cap");
    tui.send(format!("\r{PAYEE}\r1000000\r\r").as_bytes());
    refused(&mut tui, &home, &before, "registry token on this chain");
    verified(&home, 1);

    tui.send(b"t");
    wait(&mut tui, "per-transaction token cap");
    tui.send(format!("\r{USDC}\r1000000\r\r").as_bytes());
    authorize(
        &mut tui,
        &home,
        &before,
        &[
            "token-cap ethereum",
            "USDC",
            "decimals 6",
            "smallest base units",
            "1000000",
        ],
        "token-cap on ethereum:",
    );
    let token = USDC.to_ascii_lowercase();
    assert_eq!(
        verified(&home, 2)["policy"]["transactionLimits"]["ethereum"]["maxTokenBaseUnits"][&token],
        "1000000"
    );
    let before = files(&home);
    list_after_outcome(&mut tui, 2);
    tui.send(b"t");
    wait(&mut tui, "per-transaction token cap");
    tui.send(format!("\r{USDC}\rnone\r\r").as_bytes());
    authorize(
        &mut tui,
        &home,
        &before,
        &[
            "token-cap ethereum",
            "1000000 -> none (unbounded)",
            "decimals 6",
        ],
        "token-cap on ethereum:",
    );
    assert!(
        verified(&home, 3)["policy"]["transactionLimits"]["ethereum"]["maxTokenBaseUnits"][&token]
            .is_null()
    );
    quit(&mut tui);
}

#[test]
fn t47_calldata_cap_rejects_overflow_saves_zero_as_empty_data_and_none_removes_it() {
    let _exclusive = common::exclusive();
    let home = Home::new();
    home.create("main", &[]);
    home.allow("seed", PAYEE, &["ethereum"]);
    let before = files(&home);
    let mut tui = start(&home);

    tui.send(b"b");
    wait(&mut tui, "per-transaction calldata cap");
    tui.send(b"\r4294967296\r");
    refused(
        &mut tui,
        &home,
        &before,
        "complete this field before continuing",
    );
    verified(&home, 1);

    tui.send(b"b");
    wait(&mut tui, "per-transaction calldata cap");
    tui.send(b"\r0\r\r");
    authorize(
        &mut tui,
        &home,
        &before,
        &[
            "calldata-cap ethereum",
            "calldata bytes",
            "not an L1 fee cap",
            "-> 0",
        ],
        "calldata-cap on ethereum:",
    );
    assert_eq!(
        verified(&home, 2)["policy"]["transactionLimits"]["ethereum"]["maxCalldataBytes"],
        0
    );
    let before = files(&home);
    list_after_outcome(&mut tui, 2);
    tui.send(b"b");
    wait(&mut tui, "per-transaction calldata cap");
    tui.send(b"\rnone\r\r");
    authorize(
        &mut tui,
        &home,
        &before,
        &["calldata-cap ethereum", "0 -> none (unbounded)"],
        "calldata-cap on ethereum:",
    );
    assert!(
        verified(&home, 3)["policy"]["transactionLimits"]["ethereum"]["maxCalldataBytes"].is_null()
    );
    quit(&mut tui);
}
