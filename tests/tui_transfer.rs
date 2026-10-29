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
fn warmup_complete(frame: &Frame) -> bool {
    // A previous password pin survives locking. Status can observe the new daemon before
    // Checked finishes, so verified/signing bars alone can still accompany "checking...".
    // Only the completed password worker opens Wallet and restores its normal input hint.
    bar(frame).0.contains("verified by password")
        && bar(frame).1.starts_with("signing (agent too) until ")
        && frame.rows[1].starts_with("+ wallet ")
        && contains(frame, "Ethereum balances")
        && frame.rows[frame.rows.len() - 3].trim_end()
            == "a all networks  c chain  r refresh  L lock  q quit  ? actions"
}

mod warmup_sync {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend, buffer::Cell};
    use zwallet::core::time::{rfc3339_utc, unix_secs};
    use zwallet::tui::actions::Tab;
    use zwallet::tui::app::{App, Effect, Key, Msg, PasswordState};
    use zwallet::tui::{style, term::FilteredBackend, text::Source, ui};
    use zwallet::wallet::{self, KeyRow, Keys, Serving, StatusView, Trust};

    // The same visible frame predicate drives the real PTY and this deterministic event-order
    // regression. No wallet, password, daemon, RPC, timer, or subprocess is involved here.
    fn frame(app: &App) -> Frame {
        let mut terminal = Terminal::new(FilteredBackend::new(TestBackend::new(80, 24))).unwrap();
        let mut placed = Vec::new();
        terminal.draw(|f| placed = ui::draw(f, app)).unwrap();
        assert_eq!(terminal.backend().replaced(), 0);
        let buffer = terminal.backend().inner().buffer();
        let styles: Vec<_> = style::ALL
            .iter()
            .map(|style| {
                let mut cell = Cell::default();
                cell.set_style(*style);
                cell.style()
            })
            .collect();
        for cell in &buffer.content {
            assert_eq!(cell.symbol().len(), 1);
            assert!(cell.symbol().bytes().all(|b| (0x20..=0x7e).contains(&b)));
            assert!(styles.contains(&cell.style()));
        }
        for p in placed {
            let budget = match p.outside.source() {
                Source::WhitelistName | Source::KeyName | Source::RpcHost => 32,
                Source::Quantity => 78,
                Source::TransactionHash => 66,
                Source::Address => 42,
                Source::Expiry | Source::Home => 64,
                Source::Detail => 72,
                Source::Input | Source::Error | Source::Warning => 512,
            };
            assert!(p.outside.text().len() <= budget);
            assert!(p.outside.text().bytes().all(|b| (0x20..=0x7e).contains(&b)));
            assert!(p.area.right() <= 80 && p.area.bottom() <= 24);
            assert!(
                p.area.height == 1
                    || (p.outside.source() == Source::Error && (1..=7).contains(&p.area.height))
            );
            if p.area.height == 1 {
                assert_eq!(usize::from(p.area.width), p.outside.text().len());
            }
            let shown = (p.area.y..p.area.bottom())
                .flat_map(|y| (p.area.x..p.area.right()).map(move |x| buffer[(x, y)].symbol()))
                .collect::<String>();
            assert!(shown.starts_with(p.outside.text()));
            assert!(shown[p.outside.text().len()..].bytes().all(|b| b == b' '));
        }
        Frame {
            rows: (0..24)
                .map(|y| (0..80).map(|x| buffer[(x, y)].symbol()).collect())
                .collect(),
            stray: Vec::new(),
            pending: String::new(),
        }
    }

    fn keys() -> Keys {
        Keys {
            active: "main".into(),
            rows: vec![KeyRow {
                name: "main".into(),
                index: 0,
                address: Some(PAYEE.into()),
                active: true,
            }],
            trust: Trust::Locked,
        }
    }
    fn status(locked: bool) -> StatusView {
        StatusView {
            configured: true,
            locked,
            daemon_pids: if locked { vec![] } else { vec![4242] },
            other_daemon_pids: vec![],
            serving: (!locked).then(|| Serving {
                pid: 4242,
                expires_at: Some(rfc3339_utc(unix_secs(std::time::SystemTime::now()) + 600)),
                whitelist: Value::Null,
            }),
            active: Some("main".into()),
            address: Some((Some(PAYEE.into()), Trust::Locked)),
        }
    }
    fn checked(app: &mut App) {
        app.update(Msg::Checked(Ok((
            wallet::Opened {
                keys: keys().rows,
                active: "main".into(),
                warning: None,
                anchor: Ok(None),
                salted: false,
            },
            Err("fixture has no checked list".into()),
        ))));
    }

    #[test]
    fn warmup_waits_for_checked_wallet_after_status_reports_signing() {
        let mut app = App::new(
            &wallet::net_with(chains::find("ethereum").unwrap(), "http://127.0.0.1:1"),
            Duration::from_secs(3600),
        );
        app.update(Msg::Keys(app.generation(), Ok(keys())));
        app.update(Msg::Status(app.generation(), Ok(status(true))));
        assert_eq!(
            app.update(Msg::Key(Key::Enter)),
            vec![Effect::PrepareUnlock]
        );
        app.update(Msg::Prepared(Ok(())));
        app.update(Msg::PasswordSubmitted);
        checked(&mut app);
        assert!(app.pinned("main"));

        // A whitelist edit has stopped the first session, but its password pin persists.
        app.update(Msg::Keys(app.generation(), Ok(keys())));
        app.update(Msg::Status(app.generation(), Ok(status(true))));
        app.update(Msg::Key(Key::Char('u')));
        for c in "10m".chars() {
            app.update(Msg::Key(Key::Char(c)));
        }
        assert_eq!(
            app.update(Msg::Key(Key::Enter)),
            vec![Effect::PrepareDuration(Duration::from_secs(600))]
        );
        app.update(Msg::Prepared(Ok(())));
        app.update(Msg::PasswordSubmitted);
        assert_eq!(app.password_state(), PasswordState::Checking);

        // The daemon starts before the worker finishes its local password check. A Tick's
        // Status result can arrive here first, making both former readiness bars true.
        app.update(Msg::Status(app.generation(), Ok(status(false))));
        let checking = frame(&app);
        assert!(bar(&checking).0.contains("verified by password"));
        assert!(bar(&checking).1.starts_with("signing (agent too) until "));
        assert!(contains(&checking, "checking..."));
        assert!(
            !warmup_complete(&checking),
            "signing bars do not finish the password worker"
        );
        assert!(app.update(Msg::Key(Key::Char('1'))).is_empty());
        assert_eq!(app.password_state(), PasswordState::Checking);

        checked(&mut app);
        assert_eq!(app.password_state(), PasswordState::Idle);
        assert_eq!(app.tab(), Tab::Wallet);
        // Wait for the wallet's fresh data too, not a stale pane from before the check.
        assert!(!warmup_complete(&frame(&app)));
        app.update(Msg::Keys(app.generation(), Ok(keys())));
        app.update(Msg::Status(app.generation(), Ok(status(false))));
        app.update(Msg::Balances(
            app.generation(),
            "main".into(),
            Ok(wallet::ChainBalances {
                chain: chains::find("ethereum").unwrap(),
                key: "main".into(),
                address: PAYEE.into(),
                trust: Trust::Session,
                native: chains::Balance {
                    symbol: "ETH".into(),
                    decimals: 18,
                    raw: U256::from(10).pow(U256::from(18)),
                },
                tokens: vec![],
                activated: None,
                warning: None,
            }),
        ));
        assert!(warmup_complete(&frame(&app)));
        app.update(Msg::Key(Key::Char('1')));
        assert_eq!(app.tab(), Tab::Keys);
    }
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
    tui.wait_frame(WAIT, warmup_complete);
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

// These two cross-unit cases are compiled in H13, then executed only after H14 is integrated.
fn verified_list(home: &Home) -> Value {
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
fn authorize_whitelist_edit(tui: &mut PtyChild, outcome: &str) {
    wait(tui, "The password authorizes");
    // The summary owns Enter until its last page; End makes that review step explicit.
    tui.send(b"\x1b[F\r");
    wait(tui, "Wallet password");
    tui.send(format!("{PW}\r").as_bytes());
    wait(tui, outcome);
}

#[test]
fn t2_t6_t7_pty_warm_send_then_whitelist_edit_stops_all_and_requires_new_warmup() {
    let _exclusive = common::exclusive();
    let home = Home::new();
    let created = home.create("main", &[]);
    let from = created["address"].as_str().unwrap();
    home.allow("alice", PAYEE, &["ethereum"]);
    let other = Home::new();
    other.create("other", &[]);
    let node = Node::new(&home, from);
    let mut tui = start_and_warm(&home, &node, from);
    native_review(&mut tui, from);
    confirm(&mut tui);
    let mined = tui.wait_frame(WAIT, |f| {
        contains(f, "Hash: 0x") && contains(f, "mined: success")
    });
    assert_eq!(node.broadcasts(), 1);
    assert!(node.journal_before_broadcast.load(Ordering::SeqCst));
    let hash = format!("{:#x}", keccak256(&node.sent.lock().unwrap()[0]));
    assert!(contains(&mined, &hash));

    // A second test-scoped home makes the edit's stop-all contract observable.
    other.unlock("10m");
    assert_eq!(home.run(&["status", "--json"]).ok()["locked"], false);
    assert_eq!(other.run(&["status", "--json"]).ok()["locked"], false);
    tui.send(b"5");
    wait(&mut tui, "verified by password: gen 1");
    tui.send(b"a");
    wait(&mut tui, "add payee");
    tui.send(b"bob\r0x3333333333333333333333333333333333333333\rethereum\r\r");
    authorize_whitelist_edit(&mut tui, "added bob");
    tui.wait_frame(WAIT, |f| bar(f).1.starts_with("locked | "));
    assert_eq!(home.run(&["status", "--json"]).ok()["locked"], true);
    assert_eq!(other.run(&["status", "--json"]).ok()["locked"], true);
    let checked = verified_list(&home);
    assert_eq!(checked["verified"], true);
    assert_eq!(checked["gen"], 2);
    assert!(checked["entries"].as_array().unwrap().iter().any(|entry| {
        entry["nickname"] == "bob"
            && entry["address"] == "0x3333333333333333333333333333333333333333"
    }));
    tui.send(b"\r");
    let list = tui.wait_frame(WAIT, |f| {
        contains(f, "verified by password: gen 2") && bar(f).1.starts_with("locked | ")
    });
    assert!(contains(&list, "bob"));

    // Observe the refreshed key rows as well as the lock bar before opening another send.
    tui.send(b"1");
    tui.wait_frame(WAIT, |f| {
        contains(f, "* main ") && bar(f).1.starts_with("locked | ")
    });
    // Returning to the prior receipt never repeats it. Explicitly start a new payment.
    tui.send(b"3");
    wait(&mut tui, "Send recorded");
    tui.send(b"n");
    let refused = wait(&mut tui, "wallet is locked");
    assert!(!contains(&refused, "Select an asset"));
    assert_eq!(node.broadcasts(), 1);
    tui.send(b"u");
    wait(&mut tui, "unlock duration");
    tui.send(b"10m\r");
    wait(&mut tui, "Wallet password:");
    tui.send(format!("{PW}\r").as_bytes());
    tui.wait_frame(WAIT, warmup_complete);
    tui.send(b"1");
    tui.wait_frame(WAIT, |f| {
        contains(f, "* main ") && bar(f).0.contains("verified by password")
    });
    tui.send(b"3");
    wait(&mut tui, "Select an asset");
    assert_eq!(
        node.broadcasts(),
        1,
        "warming again must not resubmit the prior send"
    );
    quit(&mut tui);
    assert_eq!(node.broadcasts(), 1);
}

#[test]
fn t44_pty_legacy_zero_is_flagged_excluded_from_picker_and_removable() {
    let _exclusive = common::exclusive();
    let home = Home::new();
    let created = home.create("main", &[]);
    let from = created["address"].as_str().unwrap();
    home.allow("alice", PAYEE, &["ethereum"]);
    // Core signing/sealing emulates an existing legacy entry. The add UI deliberately refuses it.
    let zero = "0x0000000000000000000000000000000000000000";
    common::seed_whitelist(home.path(), |list| {
        list.entries
            .push(zwallet::core::whitelist::Entry::new_address(
                "z",
                zero,
                &["ethereum"],
            ));
    });
    let checked = verified_list(&home);
    assert_eq!(checked["verified"], true);
    assert_eq!(checked["gen"], 2);
    assert!(
        checked["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["nickname"] == "z" && entry["address"] == zero)
    );
    let node = Node::new(&home, from);
    let mut tui = start_and_warm(&home, &node, from);
    tui.send(b"5");
    wait(&mut tui, "verified by password: gen 2");
    tui.send(b"j");
    let flagged = tui.wait_frame(WAIT, |f| {
        contains(f, "selected for d: z") && contains(f, "zero address; unavailable as payee")
    });
    assert!(contains(&flagged, zero));
    tui.send(b"3");
    wait(&mut tui, "Select an asset");
    tui.send(b"\r");
    let picker = tui.wait_frame(WAIT, |f| {
        contains(f, "Pick a password-checked payee") && contains(f, PAYEE)
    });
    assert!(contains(&picker, "alice"));
    assert!(!contains(&picker, zero));
    assert!(
        !picker
            .rows
            .iter()
            .any(|row| row.trim_matches(|c| c == ' ' || c == '|') == "z")
    );
    assert_eq!(node.broadcasts(), 0);
    assert!(!node.journal.exists());
    // The picker consumes typed address characters, so leave it explicitly before switching tabs.
    tui.send(b"\x1b");
    wait(&mut tui, "Select an asset");
    tui.send(b"5");
    tui.wait_frame(WAIT, |f| {
        contains(f, "verified by password: gen 2") && contains(f, "selected for d: z")
    });
    tui.send(b"d");
    let remove = wait(&mut tui, "remove selected entry");
    assert!(contains(&remove, "remove: z"));
    tui.send(b"\r"); // blank duration: remain locked after the mutation
    authorize_whitelist_edit(&mut tui, "removed z");
    tui.wait_frame(WAIT, |f| bar(f).1.starts_with("locked | "));
    let checked = verified_list(&home);
    assert_eq!(checked["verified"], true);
    assert_eq!(checked["gen"], 3);
    let entries = checked["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["nickname"], "alice");
    tui.send(b"\r");
    let remaining = wait(&mut tui, "verified by password: gen 3");
    assert!(!contains(&remaining, "zero address"));
    assert_eq!(node.broadcasts(), 0);
    assert!(!node.journal.exists());
    quit(&mut tui);
}
