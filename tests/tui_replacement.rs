//! TUI9 PTY acceptance. Parent runs serially; every home and RPC is isolated.
//! Actual daemon sockets remain untouched; mock tests count SignTx at the protocol boundary.
mod common;
use alloy::{
    consensus::{Transaction, TxEnvelope},
    eips::eip2718::Decodable2718,
    primitives::{U256, keccak256},
};
use common::{Frame, Home, PW, PtyChild};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use zwallet::chains::{self, SendJournal, evm::tx};
const WAIT: Duration = Duration::from_secs(60);
const SIZE: (u16, u16) = (80, 24);
const PAYEE: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
const GWEI: u64 = 1_000_000_000;
struct Node {
    url: String,
    sent: Arc<Mutex<Vec<Vec<u8>>>>,
    unknown: Arc<AtomicBool>,
}
impl Node {
    fn new() -> Self {
        let sent = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
        let unknown = Arc::new(AtomicBool::new(false));
        let (s, u) = (sent.clone(), unknown.clone());
        let url = common::tiny_rpc_reply(move |method, params| {
            Ok(match method {
                "eth_chainId" => json!("0x1"),
                "eth_getBalance" => json!("0xde0b6b3a7640000"),
                "eth_getTransactionCount" => json!("0x7"),
                "eth_getBlockByNumber" => block(GWEI),
                "eth_blockNumber" => json!("0x100"),
                "eth_feeHistory" => {
                    json!({"oldestBlock":"0xf6","baseFeePerGas":vec![quantity(GWEI);11],"gasUsedRatio":vec![0.5;10],"reward":vec![vec![quantity(GWEI/10)];10]})
                }
                "eth_gasPrice" => quantity(GWEI + GWEI / 10),
                "eth_estimateGas" => json!("0x5208"),
                "eth_call" => json!(format!("0x{}", "0".repeat(64))),
                "eth_sendRawTransaction" => {
                    let raw =
                        alloy::hex::decode(params[0].as_str().unwrap().trim_start_matches("0x"))
                            .unwrap();
                    let hash = format!("{:#x}", keccak256(&raw));
                    s.lock().unwrap().push(raw);
                    if u.load(Ordering::SeqCst) {
                        Value::Null
                    } else {
                        json!(hash)
                    }
                }
                "eth_getTransactionReceipt" => Value::Null,
                "eth_getTransactionByHash" => {
                    if u.load(Ordering::SeqCst) {
                        Value::Null
                    } else {
                        s.lock()
                            .unwrap()
                            .iter()
                            .find(|raw| params[0] == format!("{:#x}", keccak256(raw)))
                            .map(|raw| {
                                let envelope =
                                    TxEnvelope::decode_2718(&mut raw.as_slice()).unwrap();
                                let decoded = tx::decode_signed(raw).unwrap();
                                let mut v = serde_json::to_value(envelope).unwrap();
                                v["hash"] = json!(format!("{:#x}", keccak256(raw)));
                                v["from"] = json!(decoded.signer.to_checksum(None));
                                v["blockNumber"] = Value::Null;
                                v["blockHash"] = Value::Null;
                                v
                            })
                            .unwrap_or(Value::Null)
                    }
                }
                _ => Value::Null,
            })
        });
        Self { url, sent, unknown }
    }
    fn last_hash(&self) -> String {
        format!(
            "{:#x}",
            keccak256(self.sent.lock().unwrap().last().unwrap())
        )
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
#[track_caller]
fn wait(tui: &mut PtyChild, text: &str) -> Frame {
    tui.wait_frame(WAIT, |f| contains(f, text))
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

fn start(home: &Home, node: &Node, from: &str) -> PtyChild {
    let mut command = home.command(&["tui", "--allow-ptrace-scope-0"]);
    for chain in chains::CHAINS {
        command.env(chain.rpc_env_var(), "http://127.0.0.1:1");
    }
    command.env("ZWALLET_RPC_ETHEREUM", &node.url);
    let mut tui = PtyChild::spawn_full_screen(command, SIZE);
    wait(&mut tui, "* main ");
    tui.send(b"V");
    wait(&mut tui, "Wallet password:");
    tui.send(format!("{PW}\r").as_bytes());
    tui.wait_frame(WAIT, |f| {
        f.rows
            .iter()
            .any(|line| line.contains(from) && line.contains("verified"))
    });
    tui.send(b"\r");
    tui.wait_frame(WAIT, |f| bar(f).0.contains("verified by password"));
    tui
}
/// Looks `hash` up on the wallet tab, where `start` leaves the TUI (Enter on a key of an
/// unlocked wallet opens it there). The tab submits a lookup only once it knows the open key's
/// address (`history.scope`); until then Enter only says "wait for the selected key's address,
/// then press Enter". So the hash is typed only once the history pane shows `loaded`, text it
/// draws only from a history read, which starts only after the address is known: a row's
/// `[journal]` tag, or "No transactions in these sources." when the journal is gone. Not `2`
/// first: on the wallet tab it reloads the tab, which forgets the address while the screen still
/// shows the pane loaded, and the hash went in too early under load (issue #52).
#[track_caller]
fn lookup(tui: &mut PtyChild, hash: &str, loaded: &str) {
    tui.wait_frame(WAIT, |f| contains(f, loaded));
    tui.send(b"/");
    wait(tui, "Transaction hash:");
    tui.send(format!("{hash}\r").as_bytes());
}
/// What the history pane draws once it has read the journal's rows, or found none.
const JOURNAL_ROW: &str = "[journal]";
const NO_ROWS: &str = "No transactions in these sources.";
fn replacement_confirm(tui: &mut PtyChild, label: &str) {
    tui.send(b"\r");
    wait(tui, "Type the reviewed nickname or cancel:");
    tui.send(format!("{label}\r").as_bytes());
}
fn setup() -> (Home, Node, String) {
    let home = Home::new();
    let created = home.create("main", &[]);
    let from = created["address"].as_str().unwrap().to_string();
    home.allow("alice", PAYEE, &["ethereum"]);
    home.unlock("1h");
    (home, Node::new(), from)
}
#[test]
fn t27_t28_pty_restart_original_proof_speedup_and_cancel_verify_signed_bytes() {
    let _exclusive = common::exclusive();
    for (action, label) in [(b's', "alice"), (b'x', "cancel")] {
        let (home, node, from) = setup();
        let output = home
            .command(&[
                "transfer", "--chain", "ethereum", "--to", "@alice", "--amount", "0.01", "--rpc",
                &node.url, "--json",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let first = node.sent.lock().unwrap()[0].clone();
        let first_hash = node.last_hash();
        let mut tui = start(&home, &node, &from);
        lookup(&mut tui, &first_hash, JOURNAL_ROW);
        wait(&mut tui, "s speed up   x cancel");
        assert_eq!(node.sent.lock().unwrap().len(), 1);
        tui.send(&[action]);
        wait(&mut tui, "nonce 7");
        replacement_confirm(&mut tui, label);
        tui.wait_frame(WAIT, |f| {
            contains(f, "pending; checking") && node.sent.lock().unwrap().len() == 2
        });
        assert_eq!(node.sent.lock().unwrap().len(), 2);
        let raw = node.sent.lock().unwrap()[1].clone();
        let signed = TxEnvelope::decode_2718(&mut raw.as_slice()).unwrap();
        assert_eq!(
            tx::decode_signed(&raw).unwrap().signer.to_checksum(None),
            from
        );
        assert_eq!(signed.nonce(), 7);
        let old = TxEnvelope::decode_2718(&mut first.as_slice()).unwrap();
        if action == b's' {
            assert_eq!(signed.to(), old.to());
            assert_eq!(signed.value(), old.value());
            assert_eq!(signed.input(), old.input());
        } else {
            assert_eq!(signed.to().unwrap().to_checksum(None), from);
            assert_eq!(signed.value(), U256::ZERO);
            assert!(signed.input().is_empty());
            wait(&mut tui, "a cancel replaces nonce N");
        }
        assert_ne!(node.last_hash(), first_hash);
        quit(&mut tui);
    }
}
#[test]
fn t29_pty_uncertain_retry_then_restart_unknown_has_no_retry_even_without_journal() {
    let _exclusive = common::exclusive();
    let (home, node, from) = setup();
    node.unknown.store(true, Ordering::SeqCst);
    let mut tui = start(&home, &node, &from);
    native_review(&mut tui, &from);
    confirm(&mut tui);
    wait(&mut tui, "R retry this uncertain send");
    assert_eq!(node.sent.lock().unwrap().len(), 1);
    let first = node.sent.lock().unwrap()[0].clone();
    let original_hash = node.last_hash();
    std::fs::write(
        SendJournal::new(home.path(), chains::find("ethereum").unwrap(), &from).path(),
        b"tampered claim\n",
    )
    .unwrap();
    tui.send(b"R");
    wait(&mut tui, "nonce 7");
    node.unknown.store(false, Ordering::SeqCst);
    replacement_confirm(&mut tui, "alice");
    tui.wait_frame(WAIT, |f| {
        contains(f, "pending; checking") && node.sent.lock().unwrap().len() == 2
    });
    assert_eq!(node.sent.lock().unwrap().len(), 2);
    let raw = node.sent.lock().unwrap()[1].clone();
    let last = TxEnvelope::decode_2718(&mut raw.as_slice()).unwrap();
    assert_eq!(
        tx::decode_signed(&raw).unwrap().signer.to_checksum(None),
        from
    );
    let old = TxEnvelope::decode_2718(&mut first.as_slice()).unwrap();
    assert_eq!(last.nonce(), old.nonce());
    assert_eq!(last.to(), old.to());
    assert_eq!(last.value(), old.value());
    assert_eq!(last.input(), old.input());
    quit(&mut tui);
    node.unknown.store(true, Ordering::SeqCst);
    let path = SendJournal::new(home.path(), chains::find("ethereum").unwrap(), &from)
        .path()
        .to_path_buf();
    let rows = std::fs::read_to_string(&path)
        .unwrap()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .map(|mut row| {
            row["ts"] = json!("2000-01-01T00:00:00Z");
            row.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&path, format!("{rows}\n")).unwrap();
    for remove in [false, true] {
        if remove {
            std::fs::remove_file(
                SendJournal::new(home.path(), chains::find("ethereum").unwrap(), &from).path(),
            )
            .unwrap();
        }
        let mut tui = start(&home, &node, &from);
        lookup(
            &mut tui,
            &original_hash,
            if remove { NO_ROWS } else { JOURNAL_ROW },
        );
        let shown = wait(&mut tui, "it can pay twice");
        assert!(!contains(&shown, "R retry"));
        tui.send(b"R");
        wait(&mut tui, "Retry is only available");
        assert_eq!(node.sent.lock().unwrap().len(), 2);
        quit(&mut tui);
    }
}
