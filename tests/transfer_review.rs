//! Review/confirm boundary tests. Only an isolated mock signer and loopback RPC; no wallet,
//! password, real daemon, or external network is used.
use alloy::primitives::{U256, keccak256};
use alloy::signers::local::PrivateKeySigner;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use zwallet::chains::{
    self, Net, SendJournal,
    evm::{self, tx},
};
use zwallet::core::whitelist::{Entry, WHITELIST_FILE, Whitelist, WhitelistKey};
use zwallet::wallet::{self, CheckedList, SendTransferOutcome, TransferInput, TransferOptions};

const OTHER: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
const GWEI: u128 = 1_000_000_000;
fn hex(n: u128) -> Value {
    json!(format!("{n:#x}"))
}
fn block(base_fee: Option<u128>) -> Value {
    let zero32 = format!("0x{}", "0".repeat(64));
    let mut b = json!({
        "hash": format!("0x{}", "1".repeat(64)),
        "parentHash": zero32,
        "sha3Uncles": zero32,
        "miner": "0x0000000000000000000000000000000000000000",
        "stateRoot": zero32,
        "transactionsRoot": zero32,
        "receiptsRoot": zero32,
        "logsBloom": format!("0x{}", "0".repeat(512)),
        "difficulty": "0x0",
        "number": "0x100",
        "gasLimit": "0x1c9c380",
        "gasUsed": "0x0",
        "timestamp": "0x6ad6fa17",
        "extraData": "0x",
        "mixHash": zero32,
        "nonce": "0x0000000000000000",
        "size": "0x200",
        "transactions": [],
        "uncles": [],
    });
    if let Some(base) = base_fee {
        b["baseFeePerGas"] = hex(base);
    }
    b
}

struct Node {
    net: Net,
    base: Arc<AtomicU64>,
    balance: Arc<AtomicU64>,
    l1: Arc<AtomicU64>,
    pending: Arc<AtomicBool>,
    mined: Arc<AtomicBool>,
    token_decimals: Arc<AtomicU64>,
    sent: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Node {
    fn new(chain: &'static str) -> Self {
        let chain = chains::find(chain).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let net = Net {
            chain,
            rpc_url: format!("http://{}", listener.local_addr().unwrap()),
        };
        let base = Arc::new(AtomicU64::new(GWEI as u64));
        let balance = Arc::new(AtomicU64::new(10u64.pow(18)));
        let l1 = Arc::new(AtomicU64::new(10_000));
        let pending = Arc::new(AtomicBool::new(false));
        let mined = Arc::new(AtomicBool::new(false));
        let token_decimals = Arc::new(AtomicU64::new(6));
        let sent = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (b, l, p, n, end) = (
            base.clone(),
            l1.clone(),
            pending.clone(),
            sent.clone(),
            stop.clone(),
        );
        let funds = balance.clone();
        let receipt = mined.clone();
        let decimals = token_decimals.clone();
        let thread = std::thread::spawn(move || {
            while !end.load(Ordering::SeqCst) {
                if let Ok((stream, _)) = listener.accept() {
                    let (b, l, p, n) = (b.clone(), l.clone(), p.clone(), n.clone());
                    let funds = funds.clone();
                    let receipt = receipt.clone();
                    let decimals = decimals.clone();
                    std::thread::spawn(move || {
                        serve(stream, move |m, args| {
                            let base = u128::from(b.load(Ordering::SeqCst));
                            match m {
                                "eth_chainId" => hex(evm::config(chain).unwrap().chain_id.into()),
                                "eth_getBalance" => hex(funds.load(Ordering::SeqCst).into()),
                                "eth_getTransactionCount" => {
                                    hex(if args[1] == "latest" && p.load(Ordering::SeqCst) {
                                        6
                                    } else {
                                        7
                                    })
                                }
                                "eth_getBlockByNumber" => block(Some(base)),
                                "eth_feeHistory" => {
                                    json!({"oldestBlock":"0xf6", "baseFeePerGas":vec![hex(base);11],
                                "gasUsedRatio":vec![0.5;10], "reward":vec![vec![hex(GWEI/10)];10]})
                                }
                                "eth_gasPrice" => hex(base + GWEI / 10),
                                "eth_estimateGas" => json!("0x5208"),
                                "eth_call" => {
                                    let input = args[0]
                                        .get("data")
                                        .or_else(|| args[0].get("input"))
                                        .and_then(Value::as_str)
                                        .unwrap_or("");
                                    if input.starts_with("0x313ce567") {
                                        json!(format!("0x{:064x}", decimals.load(Ordering::SeqCst)))
                                    } else if input.starts_with("0x95d89b41") {
                                        json!(format!("0x55534443{}", "00".repeat(28)))
                                    } else {
                                        json!(format!("0x{:064x}", l.load(Ordering::SeqCst)))
                                    }
                                }
                                "eth_sendRawTransaction" => {
                                    n.fetch_add(1, Ordering::SeqCst);
                                    let bytes = alloy::hex::decode(
                                        args[0].as_str().unwrap().trim_start_matches("0x"),
                                    )
                                    .unwrap();
                                    json!(format!("{:#x}", keccak256(bytes)))
                                }
                                "eth_getTransactionReceipt" if receipt.load(Ordering::SeqCst) => {
                                    json!({
                                        "transactionHash":args[0],"status":"0x1","blockNumber":"0x10","gasUsed":"0x5208"
                                    })
                                }
                                "eth_getTransactionByHash" | "eth_getTransactionReceipt" => {
                                    Value::Null
                                }
                                _ => panic!("unexpected mock method {m}"),
                            }
                        })
                    });
                } else {
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
        });
        Self {
            net,
            base,
            balance,
            l1,
            pending,
            mined,
            token_decimals,
            sent,
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for Node {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
    }
}
fn serve(stream: TcpStream, answer: impl Fn(&str, &Value) -> Value) {
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);
    loop {
        let mut len = 0;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                return;
            }
            if line.trim().is_empty() {
                break;
            }
            if let Some((k, v)) = line.split_once(':')
                && k.eq_ignore_ascii_case("content-length")
            {
                len = v.trim().parse::<usize>().unwrap();
            }
        }
        let mut body = vec![0; len];
        if reader.read_exact(&mut body).is_err() {
            return;
        }
        let req: Value = serde_json::from_slice(&body).unwrap();
        let result = answer(req["method"].as_str().unwrap(), &req["params"]);
        let body = json!({"jsonrpc":"2.0","id":req["id"],"result":result}).to_string();
        if write!(
            writer,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        )
        .is_err()
        {
            return;
        }
    }
}

struct Signer {
    received: Arc<Mutex<Vec<Value>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Signer {
    fn new(home: &std::path::Path, tamper: bool) -> (Self, String) {
        let key: PrivateKeySigner = format!("{:064x}", 1).parse().unwrap();
        let address = key.address().to_checksum(None);
        let listener = UnixListener::bind(home.join("d.sock")).unwrap();
        listener.set_nonblocking(true).unwrap();
        let received = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (requests, end) = (received.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            while !end.load(Ordering::SeqCst) {
                if let Ok((mut stream, _)) = listener.accept() {
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut bytes = Vec::new();
                    stream.read_to_end(&mut bytes).unwrap();
                    let request: Value = serde_json::from_slice(&bytes).unwrap();
                    assert_eq!(request["op"], "sign_tx");
                    requests.lock().unwrap().push(request.clone());
                    let mut bytes = alloy::hex::decode(
                        request["tx"].as_str().unwrap().trim_start_matches("0x"),
                    )
                    .unwrap();
                    if tamper {
                        // Decode a different, valid intent signed by the very same key.
                        let d = tx::decode_unsigned(&bytes).unwrap();
                        bytes = tx::encoded_for_signing(&tx::UnsignedEvmTx {
                            chain_id: d.chain_id(),
                            nonce: d.nonce() + 1,
                            gas_limit: d.gas_limit(),
                            fees: d.fees(),
                            to: d.to(),
                            value: d.value(),
                            input: d.input().clone(),
                        });
                    }
                    let decoded = tx::decode_unsigned(&bytes).unwrap();
                    let signed = tx::sign_decoded(&key, &decoded).unwrap();
                    let reply = json!({"v":2,"ok":true,"result":{"rawTx":format!("0x{}",alloy::hex::encode(signed.raw))}});
                    writeln!(stream, "{reply}").unwrap();
                } else {
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
        });
        (
            Self {
                received,
                stop,
                thread: Some(thread),
            },
            address,
        )
    }
    fn count(&self) -> usize {
        self.received.lock().unwrap().len()
    }
}
impl Drop for Signer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
    }
}
struct Fixture {
    // Stop the fake signer before deleting the socket's home.
    signer: Signer,
    dir: tempfile::TempDir,
    checked: CheckedList,
    from: String,
}
impl Fixture {
    fn new(tamper: bool) -> Self {
        let dir = tempfile::Builder::new()
            .prefix("zwt-send-review-")
            .tempdir_in("/tmp")
            .unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let (signer, from) = Signer::new(dir.path(), tamper);
        let mut list = Whitelist::empty(&evm::registry::WHITELIST_RULES);
        list.generation = 1;
        list.entries
            .push(Entry::new_address("alice", OTHER, &["*"]));
        let key = WhitelistKey::from_secret(&[7; 32]);
        let signed = key.sign(&list);
        std::fs::write(
            dir.path().join(WHITELIST_FILE),
            signed.to_file_bytes().unwrap(),
        )
        .unwrap();
        let checked = wallet::check_list(dir.path(), &Ok(Some(key.anchor_for(&signed)))).unwrap();
        Self {
            signer,
            dir,
            checked,
            from,
        }
    }
    fn input(&self, node: &Node) -> TransferInput {
        TransferInput {
            index: 0,
            key_name: "main".into(),
            from: self.from.clone(),
            payee: wallet::transfer_payees(&self.checked, node.net.chain).remove(0),
            amount: "0.01".into(),
            token: None,
            options: TransferOptions::default(),
        }
    }
    async fn review(&self, node: &Node) -> wallet::TransferReview {
        wallet::review_transfer(self.dir.path(), &node.net, &self.checked, self.input(node))
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn review_uses_checked_memory_without_lock_or_any_signer_request() {
    let f = Fixture::new(false);
    let node = Node::new("ethereum");
    let lock = chains::SendLock::acquire(f.dir.path(), node.net.chain, &f.from).unwrap();
    std::fs::remove_file(f.dir.path().join(WHITELIST_FILE)).unwrap();
    let review = f.review(&node).await;
    assert!(review.verdict().is_ok());
    assert!(review.sufficient_funds());
    assert_eq!(review.payee().address(), OTHER);
    assert_eq!(f.signer.count(), 0);
    drop(lock);
    assert!(
        wallet::send_transfer(review, None)
            .await
            .unwrap_err()
            .message
            .contains("changed")
    );
    assert_eq!(f.signer.count(), 0);
}

#[tokio::test]
async fn same_generation_resigned_payee_change_signs_nothing() {
    let f = Fixture::new(false);
    let node = Node::new("ethereum");
    let review = f.review(&node).await;
    let mut changed = f.checked.list.clone();
    changed.entries[0].address = f.from.to_lowercase();
    let signed = WhitelistKey::from_secret(&[7; 32]).sign(&changed);
    std::fs::write(
        f.dir.path().join(WHITELIST_FILE),
        signed.to_file_bytes().unwrap(),
    )
    .unwrap();
    assert!(
        wallet::send_transfer(review, None)
            .await
            .unwrap_err()
            .message
            .contains("changed")
    );
    assert_eq!(f.signer.count(), 0);
    assert_eq!(node.sent.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn rising_fee_returns_new_review_and_zero_sign_tx() {
    let f = Fixture::new(false);
    let node = Node::new("ethereum");
    let review = f.review(&node).await;
    let old = review.max_total_fee();
    node.base.store(2 * GWEI as u64, Ordering::SeqCst);
    let SendTransferOutcome::ReviewChanged(new) =
        wallet::send_transfer(review, None).await.unwrap()
    else {
        panic!("signed rising fee")
    };
    assert!(new.max_total_fee() > old);
    assert_eq!(f.signer.count(), 0);
    assert_eq!(node.sent.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn falling_fee_signs_once_with_exact_reviewed_intent_and_journal_before_broadcast() {
    let f = Fixture::new(false);
    let node = Node::new("ethereum");
    let review = f.review(&node).await;
    let intent = review.intent().clone();
    node.base.store(GWEI as u64 / 2, Ordering::SeqCst);
    let journal = SendJournal::new(f.dir.path(), node.net.chain, &f.from);
    let path = journal.path().to_owned();
    let broadcasts = node.sent.clone();
    let notices = Arc::new(AtomicUsize::new(0));
    let n = notices.clone();
    let progress = Arc::new(move |line: &str| {
        assert!(line.starts_with("broadcasting 0x"));
        assert!(path.exists());
        assert_eq!(broadcasts.load(Ordering::SeqCst), 0);
        n.fetch_add(1, Ordering::SeqCst);
    });
    let SendTransferOutcome::Sent(sent) =
        wallet::send_transfer(review, Some(progress)).await.unwrap()
    else {
        panic!("fee decrease refused")
    };
    assert_eq!(f.signer.count(), 1);
    assert_eq!(node.sent.load(Ordering::SeqCst), 1);
    assert_eq!(notices.load(Ordering::SeqCst), 1);
    assert!(sent.fields.get("rawTx").is_none());
    let req = f.signer.received.lock().unwrap()[0].clone();
    let payload = alloy::hex::decode(req["tx"].as_str().unwrap().trim_start_matches("0x")).unwrap();
    let actual = tx::decode_unsigned(&payload).unwrap();
    assert_eq!(actual.chain_id(), intent.chain_id());
    assert_eq!(actual.to(), intent.to());
    assert_eq!(actual.value(), intent.value());
    assert_eq!(actual.input(), intent.input());
    assert_eq!(actual.nonce(), intent.nonce());
}

#[tokio::test]
async fn op_l1_rise_refuses_and_fall_signs_while_execution_fee_is_unchanged() {
    for rises in [true, false] {
        let f = Fixture::new(false);
        let node = Node::new("base");
        let review = f.review(&node).await;
        let gas = review.max_gas_cost();
        assert_eq!(review.l1_fee(), Some(U256::from(10_000)));
        node.l1
            .store(if rises { 10_001 } else { 9_999 }, Ordering::SeqCst);
        match wallet::send_transfer(review, None).await.unwrap() {
            SendTransferOutcome::ReviewChanged(next) => {
                assert!(rises);
                assert_eq!(next.max_gas_cost(), gas);
                assert_eq!(f.signer.count(), 0);
            }
            SendTransferOutcome::Sent(_) => {
                assert!(!rises);
                assert_eq!(f.signer.count(), 1);
            }
        }
    }
}

#[tokio::test]
async fn pending_and_duplicate_overrides_are_held_from_review() {
    for pending in [true, false] {
        for allowed in [false, true] {
            let f = Fixture::new(false);
            let node = Node::new("ethereum");
            node.pending.store(pending, Ordering::SeqCst);
            if !pending {
                SendJournal::new(f.dir.path(),node.net.chain,&f.from).append(&json!({
            "ts":zwallet::core::time::now_rfc3339(),"chain":"ethereum","from":f.from,
            "nonce":6,"txHash":format!("0x{}","11".repeat(32)),"to":OTHER,"value":"0.01 ETH",
            "kind":"transfer","amountRaw":"10000000000000000"
        })).unwrap();
            }
            let mut input = f.input(&node);
            input.options.allow_pending = allowed;
            input.options.allow_duplicate = allowed;
            let review = wallet::review_transfer(f.dir.path(), &node.net, &f.checked, input)
                .await
                .unwrap();
            if pending {
                assert_eq!(review.fields()["pendingCount"], 1);
            } else {
                assert_eq!(
                    review.fields()["duplicateSends"].as_array().unwrap().len(),
                    1
                );
            }
            let result = wallet::send_transfer(review, None).await;
            if allowed {
                assert!(matches!(result.unwrap(), SendTransferOutcome::Sent(_)));
            } else {
                assert!(result.is_err());
            }
            assert_eq!(f.signer.count(), usize::from(allowed));
        }
    }
}

#[tokio::test]
async fn valid_signature_for_different_intent_never_broadcasts_or_journals() {
    let f = Fixture::new(true);
    let node = Node::new("ethereum");
    let review = f.review(&node).await;
    let err = wallet::send_transfer(review, None).await.unwrap_err();
    assert!(err.message.contains("different transaction"));
    assert_eq!(f.signer.count(), 1);
    assert_eq!(node.sent.load(Ordering::SeqCst), 0);
    assert!(
        !SendJournal::new(f.dir.path(), node.net.chain, &f.from)
            .path()
            .exists()
    );
}

#[test]
fn picker_and_fee_inputs_reject_ineligible_choices() {
    let mut f = Fixture::new(false);
    let chain = chains::find("ethereum").unwrap();
    f.checked.list.entries.push(Entry::new_address(
        "zero",
        "0x0000000000000000000000000000000000000000",
        &["*"],
    ));
    f.checked
        .list
        .entries
        .push(Entry::new_address("base", &f.from, &["base"]));
    f.checked.list.entries.push(Entry::new_contract(
        "contract",
        &f.from,
        &["ethereum"],
        &["0xd0e30db0"],
    ));
    let payees = wallet::transfer_payees(&f.checked, chain);
    assert_eq!(payees.len(), 1);
    assert_eq!(payees[0].nickname(), "alice");
    assert_eq!(wallet::parse_fee_ceiling("5").unwrap(), 5 * GWEI);
    for invalid in ["0", "0.5.1", "999999999999999999999999999999999999999"] {
        assert!(wallet::parse_fee_ceiling(invalid).is_err());
    }
}

#[tokio::test]
async fn changed_funds_return_a_new_review_without_signing() {
    let f = Fixture::new(false);
    let node = Node::new("ethereum");
    let review = f.review(&node).await;
    node.balance.store(1, Ordering::SeqCst);
    let SendTransferOutcome::ReviewChanged(next) =
        wallet::send_transfer(review, None).await.unwrap()
    else {
        panic!("signed with changed funds")
    };
    assert!(!next.sufficient_funds());
    assert_eq!(f.signer.count(), 0);
    assert_eq!(
        wallet::send_transfer(*next, None).await.unwrap_err().code,
        zwallet::core::error::ErrorCode::InsufficientFunds
    );
    assert_eq!(f.signer.count(), 0);
}

#[tokio::test]
async fn exact_payload_signed_by_another_key_never_broadcasts() {
    let mut f = Fixture::new(false);
    f.from = OTHER.into();
    let node = Node::new("ethereum");
    let review = f.review(&node).await;
    let err = wallet::send_transfer(review, None).await.unwrap_err();
    assert!(err.message.contains("signed as"));
    assert_eq!(f.signer.count(), 1);
    assert_eq!(node.sent.load(Ordering::SeqCst), 0);
    assert!(
        !SendJournal::new(f.dir.path(), node.net.chain, &f.from)
            .path()
            .exists()
    );
}

fn tui_app(f: &Fixture, node: &Node) -> zwallet::tui::app::App {
    use zwallet::tui::app::{App, Msg};
    let mut app = App::new(&node.net, Duration::from_secs(3600));
    app.configure(f.dir.path(), false);
    let row = wallet::KeyRow {
        name: "main".into(),
        index: 0,
        address: Some(f.from.clone()),
        active: true,
    };
    app.update(Msg::Checked(Ok((
        wallet::Opened {
            keys: vec![row.clone()],
            active: "main".into(),
            warning: None,
            anchor: Ok(None),
            salted: true,
        },
        Ok(f.checked.clone()),
    ))));
    let generation = app.generation();
    app.update(Msg::Keys(
        generation,
        Ok(wallet::Keys {
            active: "main".into(),
            rows: vec![row],
            trust: wallet::Trust::Locked,
        }),
    ));
    app.update(Msg::Status(
        generation,
        Ok(wallet::StatusView {
            configured: true,
            locked: false,
            daemon_pids: vec![1],
            other_daemon_pids: vec![],
            serving: None,
            active: Some("main".into()),
            address: Some((Some(f.from.clone()), wallet::Trust::Session)),
        }),
    ));
    app
}

#[tokio::test]
async fn tui_send_tab_starts_the_checked_asset_picker() {
    let f = Fixture::new(false);
    let node = Node::new("ethereum");
    let mut app = tui_app(&f, &node);
    let effects = app.update(zwallet::tui::app::Msg::Key(zwallet::tui::app::Key::Char(
        '3',
    )));
    assert!(
        effects
            .iter()
            .any(|effect| format!("{effect:?}").starts_with("Transfer(")),
        "the send tab must start a real checked asset picker"
    );
}

use zwallet::tui::app::transfer::{ReceiptState, Stage, TransferJob, TransferMsg};
use zwallet::tui::app::{App, Effect, Key, Msg};

fn frame(app: &App) -> String {
    use ratatui::{Terminal, backend::TestBackend, buffer::Cell};
    use zwallet::tui::{style, term::FilteredBackend, text::Source, ui};
    let mut terminal = Terminal::new(FilteredBackend::new(TestBackend::new(80, 24))).unwrap();
    let mut placed = Vec::new();
    terminal.draw(|f| placed = ui::draw(f, app)).unwrap();
    assert_eq!(terminal.backend().replaced(), 0);
    let buffer = terminal.backend().inner().buffer();
    let styles: Vec<_> = style::ALL
        .iter()
        .map(|style| {
            let mut c = Cell::default();
            c.set_style(*style);
            c.style()
        })
        .collect();
    for cell in &buffer.content {
        assert!(styles.contains(&cell.style()));
    }
    let rows: Vec<String> = (0..24)
        .map(|y| (0..80).map(|x| buffer[(x, y)].symbol()).collect())
        .collect();
    for p in placed {
        let max = match p.outside.source() {
            Source::WhitelistName | Source::KeyName | Source::RpcHost => 32,
            Source::Quantity => 78,
            Source::TransactionHash => 66,
            Source::Detail => 72,
            Source::Home | Source::Expiry => 64,
            Source::Address => 42,
            Source::Input | Source::Error | Source::Warning | Source::ListRow => 512,
        };
        assert!(p.outside.text().len() <= max);
        assert!(p.outside.text().bytes().all(|b| (32..=126).contains(&b)));
        assert!(p.area.height == 1 || p.area.height <= 7);
        assert!(p.area.x + p.area.width <= 80 && p.area.y + p.area.height <= 24);
        if p.area.height == 1 {
            let drawn =
                &rows[p.area.y as usize][p.area.x as usize..(p.area.x + p.area.width) as usize];
            assert_eq!(drawn, p.outside.text());
        }
    }
    rows.join("\n")
}
fn key(app: &mut App, key: Key) -> Vec<Effect> {
    let work = app.update(Msg::Key(key));
    frame(app);
    work
}
fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        assert!(key(app, Key::Char(c)).is_empty());
    }
}
async fn drive(app: &mut App, f: &Fixture, effects: Vec<Effect>) -> Vec<(u64, TransferJob)> {
    let mut queue = std::collections::VecDeque::from(effects);
    let mut waits = Vec::new();
    while let Some(effect) = queue.pop_front() {
        let Effect::Transfer(id) = effect else {
            continue;
        };
        let Some(job) = app.take_transfer_job(id) else {
            continue;
        };
        if matches!(job, TransferJob::Receipt { .. }) {
            waits.push((id, job));
            continue;
        }
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        job.run(id, f.dir.path(), &tx).await;
        while let Ok(msg) = rx.try_recv() {
            queue.extend(app.update(msg));
            frame(app);
        }
    }
    waits
}
async fn to_review(app: &mut App, f: &Fixture) -> Vec<(u64, TransferJob)> {
    let effects = key(app, Key::Char('3'));
    drive(app, f, effects).await;
    assert_eq!(app.transfer().stage, Stage::Assets);
    let effects = key(app, Key::Enter);
    drive(app, f, effects).await;
    assert_eq!(app.transfer().stage, Stage::Payees);
    // No recipient can be typed or pasted (T6b): the step has no field. Its keys are a list's,
    // so a digit or `c` acts globally (D1); a pasted address and the other hex letters do nothing.
    let selected = app.transfer().selected;
    let pasted = app.update(Msg::Paste(zwallet::tui::app::UserText(
        zeroize::Zeroizing::new(OTHER.into()),
    )));
    assert!(pasted.is_empty());
    for c in "abdefxABCDEF".chars() {
        assert!(key(app, Key::Char(c)).is_empty(), "{c}");
    }
    assert_eq!(app.transfer().selected, selected);
    assert_eq!(app.transfer().stage, Stage::Payees);
    assert!(app.transfer().field.is_none() && app.transfer().draft.is_none());
    key(app, Key::Enter);
    assert_eq!(app.transfer().stage, Stage::Amount);
    type_text(app, "0.01");
    let effects = key(app, Key::Enter);
    let waits = drive(app, f, effects).await;
    assert_eq!(app.transfer().stage, Stage::Review);
    assert_eq!(f.signer.count(), 0);
    let screen = frame(app);
    assert!(screen.contains(OTHER));
    assert!(screen.contains(&f.from));
    assert!(screen.contains("10000000000000000"));
    assert!(screen.contains("? actions"));
    let review = app.transfer().review.as_ref().unwrap();
    assert!(screen.contains(&review.max_total_fee().to_string()));
    waits
}
fn confirm(app: &mut App) -> Vec<Effect> {
    key(app, Key::Enter);
    assert_eq!(app.transfer().stage, Stage::Confirm);
    type_text(app, "alice");
    key(app, Key::Enter)
}

#[tokio::test]
async fn tui_end_to_end_confirm_progress_deadline_extend_and_mined() {
    let f = Fixture::new(false);
    let node = Node::new("base");
    let mut app = tui_app(&f, &node);
    to_review(&mut app, &f).await;
    assert!(frame(&app).contains("10000"));
    let work = confirm(&mut app);
    assert_eq!(app.transfer().stage, Stage::Sending);
    assert!(key(&mut app, Key::Enter).is_empty());
    let waits = drive(&mut app, &f, work).await;
    assert_eq!(f.signer.count(), 1);
    assert_eq!(app.transfer().stage, Stage::Status);
    let status = app.transfer().status.as_ref().unwrap();
    assert_eq!(status.wait, Duration::from_secs(90));
    let hash = status.hash.clone();
    assert!(frame(&app).contains(&hash));
    assert!(app.transfer().retained.is_some());
    assert_eq!(waits.len(), 1);
    app.update(Msg::Transfer(waits[0].0, TransferMsg::Receipt(Ok(None))));
    assert_eq!(
        app.transfer().status.as_ref().unwrap().state,
        ReceiptState::Unknown
    );
    assert!(frame(&app).contains("unknown - run zwallet tx"));
    key(&mut app, Key::Char('w'));
    type_text(&mut app, "0");
    assert!(key(&mut app, Key::Enter).is_empty());
    assert_eq!(
        app.transfer().status.as_ref().unwrap().wait,
        Duration::from_secs(90)
    );
    key(&mut app, Key::Backspace);
    type_text(&mut app, "1x");
    // The cancel shortcut must not steal x from a focused wait-duration field.
    assert_eq!(app.transfer().stage, Stage::Wait);
    assert_eq!(
        app.replacement().stage,
        zwallet::tui::app::replacement::Stage::Idle
    );
    assert!(key(&mut app, Key::Enter).is_empty());
    assert_eq!(
        app.transfer().status.as_ref().unwrap().wait,
        Duration::from_secs(90)
    );
    key(&mut app, Key::Backspace);
    type_text(&mut app, "5m");
    let work = key(&mut app, Key::Enter);
    let mut waits = drive(&mut app, &f, work).await;
    let (id, job) = waits.remove(0);
    assert_eq!(
        app.transfer().status.as_ref().unwrap().wait,
        Duration::from_secs(300)
    );
    node.mined.store(true, Ordering::SeqCst);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    job.run(id, f.dir.path(), &tx).await;
    app.update(rx.recv().await.unwrap());
    assert!(frame(&app).contains("mined: success"));
    assert_eq!(f.signer.count(), 1);
}

#[tokio::test]
async fn tui_fees_or_l1_rise_are_new_reviews_and_decreases_sign_once() {
    for chain in ["ethereum", "base"] {
        for rises in [true, false] {
            let f = Fixture::new(false);
            let node = Node::new(chain);
            let mut app = tui_app(&f, &node);
            to_review(&mut app, &f).await;
            if chain == "base" {
                node.l1
                    .store(if rises { 10_001 } else { 9_999 }, Ordering::SeqCst);
            } else {
                node.base.store(
                    if rises {
                        2 * GWEI as u64
                    } else {
                        GWEI as u64 / 2
                    },
                    Ordering::SeqCst,
                );
            }
            let work = confirm(&mut app);
            drive(&mut app, &f, work).await;
            assert_eq!(f.signer.count(), usize::from(!rises));
            if rises {
                assert_eq!(app.transfer().stage, Stage::Review);
                assert!(frame(&app).contains("nothing signed"));
            } else {
                assert_eq!(app.transfer().stage, Stage::Status);
            }
        }
    }
}

#[tokio::test]
async fn tui_stale_list_and_changed_metadata_index_never_submit_sign_tx() {
    for stale in [true, false] {
        let f = Fixture::new(false);
        let node = Node::new("ethereum");
        let mut app = tui_app(&f, &node);
        to_review(&mut app, &f).await;
        if stale {
            let mut list = f.checked.list.clone();
            list.entries[0].address = f.from.to_lowercase();
            std::fs::write(
                f.dir.path().join(WHITELIST_FILE),
                WhitelistKey::from_secret(&[7; 32])
                    .sign(&list)
                    .to_file_bytes()
                    .unwrap(),
            )
            .unwrap();
            let work = confirm(&mut app);
            drive(&mut app, &f, work).await;
            assert!(app.checked_list().is_none());
            assert!(frame(&app).contains("changed"));
        } else {
            app.update(Msg::Keys(
                app.generation(),
                Ok(wallet::Keys {
                    active: "main".into(),
                    trust: wallet::Trust::Locked,
                    rows: vec![wallet::KeyRow {
                        name: "main".into(),
                        index: 99,
                        address: Some(f.from.clone()),
                        active: true,
                    }],
                }),
            ));
            assert!(key(&mut app, Key::Enter).is_empty());
            assert!(app.verified_key().is_err());
            assert!(frame(&app).contains("index changed"));
        }
        assert_eq!(f.signer.count(), 0);
        assert_eq!(node.sent.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn tui_pending_and_repeat_guards_require_separate_override_keys() {
    for pending in [true, false] {
        let f = Fixture::new(false);
        let node = Node::new("ethereum");
        node.pending.store(pending, Ordering::SeqCst);
        if !pending {
            SendJournal::new(f.dir.path(),node.net.chain,&f.from).append(&json!({
            "ts":zwallet::core::time::now_rfc3339(),"chain":"ethereum","from":f.from,"nonce":6,
            "txHash":format!("0x{}","11".repeat(32)),"to":OTHER,"value":"0.01 ETH","kind":"transfer","amountRaw":"10000000000000000"
        })).unwrap();
        }
        let mut app = tui_app(&f, &node);
        to_review(&mut app, &f).await;
        let work = confirm(&mut app);
        drive(&mut app, &f, work).await;
        assert_eq!(f.signer.count(), 0);
        assert_eq!(app.transfer().stage, Stage::Failed);
        let work = key(&mut app, Key::Char(if pending { 'p' } else { 'd' }));
        drive(&mut app, &f, work).await;
        assert!(frame(&app).contains("OVERRIDE"));
        assert!(frame(&app).contains("may pay twice"));
        let work = confirm(&mut app);
        drive(&mut app, &f, work).await;
        assert_eq!(f.signer.count(), 1);
    }
}

#[tokio::test]
async fn tui_fee_field_invalid_input_preserves_review_and_valid_ceiling_reprepares() {
    let f = Fixture::new(false);
    let node = Node::new("ethereum");
    let mut app = tui_app(&f, &node);
    to_review(&mut app, &f).await;
    let old = app.transfer().review.as_ref().unwrap().max_total_fee();
    key(&mut app, Key::Char('g'));
    type_text(&mut app, "0");
    assert!(key(&mut app, Key::Enter).is_empty());
    assert_eq!(app.transfer().review.as_ref().unwrap().max_total_fee(), old);
    key(&mut app, Key::Backspace);
    type_text(&mut app, "5");
    let work = key(&mut app, Key::Enter);
    drive(&mut app, &f, work).await;
    assert_eq!(
        app.transfer()
            .review
            .as_ref()
            .unwrap()
            .options()
            .max_fee_wei,
        Some(5 * GWEI)
    );
    assert_eq!(f.signer.count(), 0);
}

#[tokio::test]
async fn tui_quit_waits_for_owned_send_and_replayed_effect_cannot_submit_twice() {
    let f = Fixture::new(false);
    let node = Node::new("ethereum");
    let mut app = tui_app(&f, &node);
    to_review(&mut app, &f).await;
    let mut work = confirm(&mut app);
    work.extend(work.clone());
    key(&mut app, Key::Char('q'));
    assert!(!app.quitting());
    assert!(app.quit_pending());
    assert!(frame(&app).contains("quits when done"));
    drive(&mut app, &f, work).await;
    assert!(app.quitting());
    assert_eq!(f.signer.count(), 1);
}

#[tokio::test]
async fn tui_registry_token_send_uses_exact_registry_units_and_rejects_rpc_decimal_swap() {
    for bad_decimals in [false, true] {
        let f = Fixture::new(false);
        let node = Node::new("ethereum");
        let mut app = tui_app(&f, &node);
        let work = key(&mut app, Key::Char('3'));
        drive(&mut app, &f, work).await;
        key(&mut app, Key::Down);
        assert_eq!(app.transfer().assets[1].symbol, "USDC");
        let work = key(&mut app, Key::Enter);
        drive(&mut app, &f, work).await;
        key(&mut app, Key::Enter);
        type_text(&mut app, "0.01");
        if bad_decimals {
            node.token_decimals.store(18, Ordering::SeqCst);
        }
        let work = key(&mut app, Key::Enter);
        drive(&mut app, &f, work).await;
        if bad_decimals {
            assert_eq!(app.transfer().stage, Stage::Failed);
            assert_eq!(f.signer.count(), 0);
            assert!(frame(&app).contains("differs from the requested"));
        } else {
            assert_eq!(app.transfer().stage, Stage::Review);
            assert!(frame(&app).contains("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"));
            assert!(frame(&app).contains("10000"));
            let work = confirm(&mut app);
            drive(&mut app, &f, work).await;
            assert_eq!(f.signer.count(), 1);
            let retained = app.transfer().retained.as_ref().unwrap();
            assert!(
                matches!(retained.decoded().action(),tx::Action::TransferCall{amount,..} if *amount==U256::from(10_000))
            );
        }
    }
}

#[tokio::test]
async fn tui_deferred_lock_finishes_send_before_stopping_and_quitting() {
    let f = Fixture::new(false);
    let node = Node::new("ethereum");
    let mut app = tui_app(&f, &node);
    to_review(&mut app, &f).await;
    let work = confirm(&mut app);
    assert!(key(&mut app, Key::Char('L')).is_empty());
    key(&mut app, Key::Char('q'));
    assert!(!app.quitting());
    let Effect::Transfer(id) = work[0] else {
        panic!("missing owned send")
    };
    let job = app.take_transfer_job(id).unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    job.run(id, f.dir.path(), &tx).await;
    let mut stops = 0;
    while let Ok(msg) = rx.try_recv() {
        stops += app
            .update(msg)
            .iter()
            .filter(|e| matches!(e, Effect::LockAll))
            .count();
        frame(&app);
    }
    assert_eq!(f.signer.count(), 1);
    assert_eq!(node.sent.load(Ordering::SeqCst), 1);
    assert_eq!(stops, 1);
    assert!(!app.quitting(), "q must also wait for the requested stop");
    app.update(Msg::Locked(Ok(vec![])));
    assert!(app.quitting());
}

#[tokio::test]
async fn tui_extending_wait_cancels_only_the_superseded_receipt_poll() {
    let f = Fixture::new(false);
    let node = Node::new("ethereum");
    let mut app = tui_app(&f, &node);
    to_review(&mut app, &f).await;
    let work = confirm(&mut app);
    let mut old = drive(&mut app, &f, work).await;
    key(&mut app, Key::Char('w'));
    type_text(&mut app, "5m");
    let work = key(&mut app, Key::Enter);
    let mut new = drive(&mut app, &f, work).await;
    let (old_id, old_job) = old.remove(0);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::time::timeout(
        Duration::from_secs(1),
        old_job.run(old_id, f.dir.path(), &tx),
    )
    .await
    .expect("superseded receipt poll did not cancel");
    assert!(
        rx.try_recv().is_err(),
        "cancelled watch must not publish a stale receipt"
    );
    node.mined.store(true, Ordering::SeqCst);
    let (id, job) = new.remove(0);
    job.run(id, f.dir.path(), &tx).await;
    app.update(rx.recv().await.unwrap());
    assert!(frame(&app).contains("mined: success"));
    assert_eq!(f.signer.count(), 1);
    assert_eq!(node.sent.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn tui_picker_filters_checked_payees_and_rejects_forged_session_and_stale_list() {
    let mut f = Fixture::new(false);
    let node = Node::new("ethereum");
    f.checked.list.entries.extend([
        Entry::new_address("zero", "0x0000000000000000000000000000000000000000", &["*"]),
        Entry::new_address("baseOnly", &f.from, &["base"]),
        Entry::new_contract("contractOnly", &f.from, &["ethereum"], &["0xd0e30db0"]),
    ]);
    let mut app = tui_app(&f, &node);
    let work = key(&mut app, Key::Char('3'));
    drive(&mut app, &f, work).await;
    let work = key(&mut app, Key::Enter);
    drive(&mut app, &f, work).await;
    assert_eq!(app.transfer().payees.len(), 1);
    let screen = frame(&app);
    assert!(screen.contains("alice"));
    for excluded in ["zero", "baseOnly", "contractOnly"] {
        assert!(!screen.contains(excluded));
    }
    std::fs::write(f.dir.path().join(WHITELIST_FILE), b"changed").unwrap();
    key(&mut app, Key::Esc);
    let work = key(&mut app, Key::Enter);
    drive(&mut app, &f, work).await;
    assert_eq!(app.transfer().stage, Stage::Failed);
    assert!(app.transfer().payees.is_empty());
    assert!(app.checked_list().is_none());
    assert_eq!(f.signer.count(), 0);

    // Session-derived addresses never establish password-derived key/index/list authority.
    let mut unverified = App::new(&node.net, Duration::from_secs(3600));
    unverified.configure(f.dir.path(), false);
    let generation = unverified.generation();
    unverified.update(Msg::Keys(
        generation,
        Ok(wallet::Keys {
            active: "main".into(),
            trust: wallet::Trust::Session,
            rows: vec![wallet::KeyRow {
                name: "main".into(),
                index: 0,
                address: Some(f.from.clone()),
                active: true,
            }],
        }),
    ));
    unverified.update(Msg::Status(
        generation,
        Ok(wallet::StatusView {
            configured: true,
            locked: false,
            daemon_pids: vec![1],
            other_daemon_pids: vec![],
            serving: None,
            active: Some("main".into()),
            address: Some((Some(f.from.clone()), wallet::Trust::Session)),
        }),
    ));
    key(&mut unverified, Key::Enter);
    let work = key(&mut unverified, Key::Char('3'));
    assert!(!work.iter().any(|e| matches!(e, Effect::Transfer(_))));
    assert!(unverified.transfer().payees.is_empty());
    assert!(unverified.checked_list().is_none());
    assert_eq!(f.signer.count(), 0);
}

/// D1: the payee step is a list, not a text field, so the global tab keys act there
/// (.prd/03-tui.md §Screens and keys: "Global, outside a text field: 1–7 tabs, [ ]").
#[tokio::test]
async fn tui_payee_step_passes_the_global_tab_keys() {
    use zwallet::tui::actions::Tab;
    let f = Fixture::new(false);
    let node = Node::new("ethereum");
    let mut app = tui_app(&f, &node);
    for (press, tab) in [
        (Key::Char('2'), Tab::Wallet),
        (Key::Char(']'), Tab::Receive),
        (Key::Char('['), Tab::Wallet),
    ] {
        let work = key(&mut app, Key::Char('3'));
        drive(&mut app, &f, work).await;
        if app.transfer().stage == Stage::Assets {
            let work = key(&mut app, Key::Enter);
            drive(&mut app, &f, work).await;
        }
        assert_eq!(app.transfer().stage, Stage::Payees, "{}", frame(&app));
        key(&mut app, press);
        assert_eq!(
            app.tab(),
            tab,
            "{press:?} on the payee step:\n{}",
            frame(&app)
        );
    }
    assert_eq!(f.signer.count(), 0);
}
