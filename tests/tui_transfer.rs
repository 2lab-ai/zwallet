//! TUI-5 process acceptance: a real isolated key daemon, raw-mode PTY and loopback RPC.
//! The dispatcher runs this file serially with the other process suites. All Enter input is CR.
mod common;

use alloy::primitives::{U256, keccak256};
use common::{Frame, Home, PW, PtyChild};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use zwallet::chains::{self, SendJournal, evm::tx};
use zwallet::core::whitelist::TransactionLimits;

const WAIT: Duration = Duration::from_secs(60);
const SIZE: (u16, u16) = (80, 24);
const PAYEE: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
const GWEI: u64 = 1_000_000_000;

struct Node {
    url: String,
    base_fee: Arc<AtomicU64>,
    sent: Arc<Mutex<Vec<Vec<u8>>>>,
    journal_before_broadcast: Arc<AtomicBool>,
    journal: PathBuf,
}
impl Node {
    fn new(home: &Home, from: &str) -> Self {
        let base_fee = Arc::new(AtomicU64::new(GWEI));
        let sent = Arc::new(Mutex::new(Vec::new()));
        let journal_before_broadcast = Arc::new(AtomicBool::new(false));
        let journal = SendJournal::new(home.path(), chains::find("ethereum").unwrap(), from)
            .path()
            .to_path_buf();
        let (fee, sends, written, path) = (
            base_fee.clone(),
            sent.clone(),
            journal_before_broadcast.clone(),
            journal.clone(),
        );
        let url = common::tiny_rpc_reply(move |method, params| {
            let base = fee.load(Ordering::SeqCst);
            Ok(match method {
                "eth_chainId" => json!("0x1"),
                "eth_getBalance" => json!("0xde0b6b3a7640000"), // 1 ETH
                "eth_getTransactionCount" => json!("0x7"),
                "eth_getBlockByNumber" => block(base),
                "eth_feeHistory" => json!({
                    "oldestBlock":"0xf6", "baseFeePerGas":vec![quantity(base);11],
                    "gasUsedRatio":vec![0.5;10], "reward":vec![vec![quantity(GWEI/10)];10],
                }),
                "eth_gasPrice" => quantity(base + GWEI / 10),
                "eth_estimateGas" => json!("0x5208"),
                // Registry balance reads return zero so the native asset is the only choice.
                "eth_call" => json!(format!("0x{}", "0".repeat(64))),
                "eth_sendRawTransaction" => {
                    let raw =
                        alloy::hex::decode(params[0].as_str().unwrap().trim_start_matches("0x"))
                            .unwrap();
                    let hash = format!("{:#x}", keccak256(&raw));
                    let recorded = std::fs::read_to_string(&path)
                        .unwrap_or_default()
                        .lines()
                        .any(|line| {
                            serde_json::from_str::<Value>(line)
                                .is_ok_and(|entry| entry["txHash"] == hash && entry["nonce"] == 7)
                        });
                    written.store(recorded, Ordering::SeqCst);
                    sends.lock().unwrap().push(raw);
                    json!(hash)
                }
                "eth_getTransactionReceipt" => {
                    let known = sends
                        .lock()
                        .unwrap()
                        .iter()
                        .any(|raw| params[0] == format!("{:#x}", keccak256(raw)));
                    if known {
                        json!({"transactionHash":params[0],"status":"0x1","blockNumber":"0x100","gasUsed":"0x5208"})
                    } else {
                        Value::Null
                    }
                }
                "eth_getTransactionByHash" => Value::Null,
                other => {
                    return Err(
                        json!({"code":-32601,"message":format!("unexpected mock method {other}")}),
                    );
                }
            })
        });
        Self {
            url,
            base_fee,
            sent,
            journal_before_broadcast,
            journal,
        }
    }
    fn broadcasts(&self) -> usize {
        self.sent.lock().unwrap().len()
    }
}
fn quantity(n: u64) -> Value {
    json!(format!("{n:#x}"))
}
fn block(base_fee: u64) -> Value {
    let zero32 = format!("0x{}", "0".repeat(64));
    json!({
        "hash":format!("0x{}", "1".repeat(64)), "parentHash":zero32,"sha3Uncles":zero32,
        "miner":"0x0000000000000000000000000000000000000000", "stateRoot":zero32,
        "transactionsRoot":zero32,"receiptsRoot":zero32,"logsBloom":format!("0x{}", "0".repeat(512)),
        "difficulty":"0x0","number":"0x100","gasLimit":"0x1c9c380","gasUsed":"0x0",
        "timestamp":"0x6ad6fa17","extraData":"0x","mixHash":zero32,"nonce":"0x0000000000000000",
        "size":"0x200","transactions":[],"uncles":[],"baseFeePerGas":quantity(base_fee),
    })
}
fn contains(frame: &Frame, text: &str) -> bool {
    frame.rows.iter().any(|line| line.contains(text))
}
fn bar(frame: &Frame) -> (&str, &str) {
    let len = frame.rows.len();
    (&frame.rows[len - 2], &frame.rows[len - 1])
}
fn wait(tui: &mut PtyChild, text: &str) -> Frame {
    tui.wait_frame(WAIT, |f| contains(f, text))
}
fn start_and_warm(home: &Home, node: &Node, from: &str) -> PtyChild {
    let mut command = home.command(&["tui", "--allow-ptrace-scope-0"]);
    for chain in chains::CHAINS {
        command.env(chain.rpc_env_var(), "http://127.0.0.1:1");
    }
    command.env("ZWALLET_RPC_ETHEREUM", &node.url);
    let mut tui = PtyChild::spawn_full_screen(command, SIZE);
    tui.wait_frame(WAIT, |f| {
        contains(f, "* main ") && bar(f).1.starts_with("locked | ")
    });
    // V proves the key/list authority independently of warming a signing session.
    tui.send(b"V");
    wait(&mut tui, "Wallet password:");
    tui.send(format!("{PW}\r").as_bytes());
    tui.wait_frame(WAIT, |f| {
        f.rows
            .iter()
            .any(|line| line.contains(from) && line.contains("verified"))
            && bar(f).1.starts_with("locked | ")
    });
    assert_eq!(home.run(&["status", "--json"]).ok()["locked"], true);
    assert_eq!(node.broadcasts(), 0);
    // Enter on the selected, still-locked key warms it; no global key change is requested.
    tui.send(b"\r");
    wait(&mut tui, "Wallet password:");
    tui.send(format!("{PW}\r").as_bytes());
    tui.wait_frame(WAIT, |f| {
        bar(f).0.contains("verified by password")
            && bar(f).1.starts_with("signing (agent too) until ")
    });
    assert_eq!(home.run(&["status", "--json"]).ok()["locked"], false);
    assert_eq!(home.run(&["list", "--json"]).ok()["active"], "main");
    tui
}
fn native_review(tui: &mut PtyChild, from: &str) -> Frame {
    tui.send(b"3");
    wait(tui, "Select an asset");
    tui.send(b"\r");
    tui.wait_frame(WAIT, |f| {
        contains(f, "Pick a password-checked payee") && contains(f, PAYEE)
    });
    tui.send(b"\r");
    wait(tui, "Amount:");
    tui.send(b"0.01\r");
    let review = tui.wait_frame(WAIT, |f| {
        contains(f, "Whitelist:") && contains(f, "Enter reviews the typed confirmation")
    });
    assert!(contains(&review, PAYEE));
    assert!(contains(&review, from));
    assert!(contains(&review, "10000000000000000"));
    assert!(contains(&review, "Nonce: 7"));
    review
}
fn confirm(tui: &mut PtyChild) {
    tui.send(b"\r");
    wait(tui, "Type payee nickname:");
    tui.send(b"alice\r");
}
fn quit(tui: &mut PtyChild) {
    tui.send(b"q");
    let (status, stderr) = tui.wait_exit(WAIT);
    assert!(status.success(), "{status}: {stderr}");
    assert!(stderr.is_empty(), "{stderr:?}");
    let last = tui.final_frame(WAIT);
    assert!(last.stray.is_empty() && last.pending.is_empty(), "{last}");
    let modes = tui.local_modes();
    assert_ne!(modes & libc::ICANON, 0);
    assert_ne!(modes & libc::ECHO, 0);
    let bytes = tui.output();
    let output = String::from_utf8_lossy(&bytes);
    assert!(!output.contains(PW), "password appeared in terminal output");
    assert!(
        !output.contains("broadcasting 0x"),
        "CLI progress escaped the TUI frame"
    );
}

#[test]
fn t6_pty_verified_warm_native_send_rechecks_fees_then_journals_and_mines() {
    let _exclusive = common::exclusive();
    let home = Home::new();
    let created = home.create("main", &[]);
    let from = created["address"].as_str().unwrap();
    home.allow("alice", PAYEE, &["ethereum"]);
    let node = Node::new(&home, from);
    let mut tui = start_and_warm(&home, &node, from);
    let review = native_review(&mut tui, from);
    assert!(contains(&review, "Whitelist: allowed"));
    assert_eq!(node.broadcasts(), 0);
    assert!(!node.journal.exists());

    // A similar nickname must not sign or leave the review.
    tui.send(b"\r");
    wait(&mut tui, "Type payee nickname:");
    tui.send(b"alixe\r");
    wait(&mut tui, "type the exact payee nickname");
    assert_eq!(node.broadcasts(), 0);
    assert!(!node.journal.exists());
    node.base_fee.store(2 * GWEI, Ordering::SeqCst);
    tui.send(b"\x7f\x7f\x7f\x7f\x7falice\r");
    wait(&mut tui, "review changed; nothing signed");
    assert_eq!(node.broadcasts(), 0);
    assert!(!node.journal.exists());

    // The lower fresh fee fits the newly reviewed upper bound, so one confirmed send proceeds.
    node.base_fee.store(GWEI, Ordering::SeqCst);
    confirm(&mut tui);
    let mined = tui.wait_frame(WAIT, |f| {
        contains(f, "Hash: 0x") && contains(f, "Nonce: 7") && contains(f, "mined: success")
    });
    assert_eq!(node.broadcasts(), 1);
    assert!(node.journal_before_broadcast.load(Ordering::SeqCst));
    let signed = node.sent.lock().unwrap();
    let decoded = tx::decode_signed(&signed[0]).unwrap();
    assert_eq!(decoded.chain_id, Some(1));
    assert_eq!(decoded.nonce, 7);
    assert_eq!(decoded.to, Some(PAYEE.parse().unwrap()));
    assert_eq!(decoded.value, U256::from(10_000_000_000_000_000u64));
    assert_eq!(
        decoded.signer,
        from.parse::<alloy::primitives::Address>().unwrap()
    );
    let hash = format!("{:#x}", decoded.hash);
    assert!(contains(&mined, &hash));
    let entries: Vec<Value> = std::fs::read_to_string(&node.journal)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["txHash"], hash);
    assert_eq!(entries[0]["amountRaw"], "10000000000000000");
    drop(signed);
    quit(&mut tui);
    assert_eq!(node.broadcasts(), 1);
}

#[test]
fn t6_pty_native_policy_cap_refuses_without_journal_or_broadcast() {
    let _exclusive = common::exclusive();
    let home = Home::new();
    let created = home.create("main", &[]);
    let from = created["address"].as_str().unwrap();
    home.allow("alice", PAYEE, &["ethereum"]);
    home.edit_whitelist(|list| {
        list.policy.transaction_limits.insert(
            "ethereum".into(),
            TransactionLimits {
                max_native_wei: Some("5000000000000000".into()),
                ..TransactionLimits::default()
            },
        );
    });
    let node = Node::new(&home, from);
    let mut tui = start_and_warm(&home, &node, from);
    let review = native_review(&mut tui, from);
    assert!(contains(&review, "Whitelist: DENIED"));
    tui.send(b"\r");
    let denied = wait(&mut tui, "native value cap");
    assert!(!contains(&denied, "Type payee nickname:"));
    assert_eq!(node.broadcasts(), 0);
    assert!(!node.journal.exists());
    quit(&mut tui);
    assert_eq!(node.broadcasts(), 0);
}
