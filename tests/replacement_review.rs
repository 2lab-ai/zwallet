//! Replacement proof/review/confirm boundary tests. Only an isolated mock signer and loopback RPC; no wallet,
//! password, real daemon, or external network is used.
mod common;
use alloy::consensus::TxEnvelope;
use alloy::eips::eip2718::Decodable2718;
use alloy::primitives::{Address, Bytes, U128};
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
use zwallet::wallet::replacement::{self, Action, Options, OriginalIntent, SendReplacementOutcome};
use zwallet::wallet::{self, CheckedList, TransferInput, TransferOptions};

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
    uncertain: Arc<AtomicBool>,
    net: Net,
    base: Arc<AtomicU64>,
    #[allow(dead_code)]
    balance: Arc<AtomicU64>,
    l1: Arc<AtomicU64>,
    latest: Arc<AtomicU64>,
    pending: Arc<AtomicU64>,
    original: Arc<Mutex<Option<Value>>>,
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
        let latest = Arc::new(AtomicU64::new(7));
        let pending = Arc::new(AtomicU64::new(8));
        let original = Arc::new(Mutex::new(None::<Value>));
        let sent = Arc::new(AtomicUsize::new(0));
        let uncertain = Arc::new(AtomicBool::new(false));
        let ambiguous = uncertain.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let (b, l, p, n, end) = (
            base.clone(),
            l1.clone(),
            pending.clone(),
            sent.clone(),
            stop.clone(),
        );
        let funds = balance.clone();
        let mined = latest.clone();
        let tx = original.clone();
        let thread = std::thread::spawn(move || {
            while !end.load(Ordering::SeqCst) {
                if let Ok((stream, _)) = listener.accept() {
                    let (b, l, p, n) = (b.clone(), l.clone(), p.clone(), n.clone());
                    let funds = funds.clone();
                    let ambiguous = ambiguous.clone();
                    let (mined, tx) = (mined.clone(), tx.clone());
                    std::thread::spawn(move || {
                        serve(stream, move |m, args| {
                            let base = u128::from(b.load(Ordering::SeqCst));
                            match m {
                                "eth_chainId" => hex(evm::config(chain).unwrap().chain_id.into()),
                                "eth_getBalance" => hex(funds.load(Ordering::SeqCst).into()),
                                "eth_getTransactionCount" => {
                                    hex(u128::from(if args[1] == "latest" {
                                        mined.load(Ordering::SeqCst)
                                    } else {
                                        p.load(Ordering::SeqCst)
                                    }))
                                }
                                "eth_getBlockByNumber" => block(Some(base)),
                                "eth_feeHistory" => {
                                    json!({"oldestBlock":"0xf6", "baseFeePerGas":vec![hex(base);11],
                                "gasUsedRatio":vec![0.5;10], "reward":vec![vec![hex(GWEI/10)];10]})
                                }
                                "eth_gasPrice" => hex(base + GWEI / 10),
                                "eth_estimateGas" => json!("0x5208"),
                                // decimals() answers the registry's USDC units; anything else
                                // reads the L1 fee (or a balance) from `l1`.
                                "eth_call"
                                    if args[0]["data"]
                                        .as_str()
                                        .or(args[0]["input"].as_str())
                                        .is_some_and(|d| d.starts_with("0x313ce567")) =>
                                {
                                    json!(format!("0x{:064x}", 6))
                                }
                                "eth_call" => json!(format!("0x{:064x}", l.load(Ordering::SeqCst))),
                                "eth_sendRawTransaction" => {
                                    n.fetch_add(1, Ordering::SeqCst);
                                    let bytes = alloy::hex::decode(
                                        args[0].as_str().unwrap().trim_start_matches("0x"),
                                    )
                                    .unwrap();
                                    if ambiguous.load(Ordering::SeqCst) {
                                        Value::Null
                                    } else {
                                        json!(format!("{:#x}", keccak256(bytes)))
                                    }
                                }
                                "eth_getTransactionByHash" => tx
                                    .lock()
                                    .unwrap()
                                    .as_ref()
                                    .filter(|v| v["hash"] == args[0])
                                    .cloned()
                                    .unwrap_or(Value::Null),
                                "eth_getTransactionReceipt" => Value::Null,
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
            uncertain,
            net,
            base,
            balance,
            l1,
            latest,
            pending,
            original,
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
    #[allow(dead_code)]
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
    #[allow(dead_code)]
    async fn review(&self, node: &Node) -> wallet::TransferReview {
        wallet::review_transfer(self.dir.path(), &node.net, &self.checked, self.input(node))
            .await
            .unwrap()
    }
}

fn key(f: &Fixture) -> wallet::KeyRow {
    wallet::KeyRow {
        name: "main".into(),
        index: 0,
        address: Some(f.from.clone()),
        active: true,
    }
}

fn original(node: &Node, f: &Fixture, signer: u8, token: bool, invoke: bool) -> chains::JournalRow {
    let private: PrivateKeySigner = format!("{signer:064x}").parse().unwrap();
    let to: Address = OTHER.parse().unwrap();
    let token_addr: Address = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"
        .parse()
        .unwrap();
    let unsigned = tx::UnsignedEvmTx {
        chain_id: evm::config(node.net.chain).unwrap().chain_id,
        nonce: 7,
        gas_limit: 25_200,
        fees: tx::EvmFees::Eip1559 {
            max_fee_per_gas: U128::from(2 * GWEI),
            max_priority_fee_per_gas: U128::from(GWEI / 10),
        },
        to: if token { token_addr } else { to },
        value: if token { U256::ZERO } else { U256::from(123) },
        input: if token {
            tx::erc20_transfer_calldata(to, U256::from(123))
        } else if invoke {
            Bytes::from_static(&[0x12, 0x34, 0x56, 0x78])
        } else {
            Bytes::new()
        },
    };
    let signed = tx::sign_decoded(
        &private,
        &tx::decode_unsigned(&tx::encoded_for_signing(&unsigned)).unwrap(),
    )
    .unwrap();
    let envelope = TxEnvelope::decode_2718(&mut signed.raw.as_ref()).unwrap();
    let mut value = serde_json::to_value(&envelope).unwrap();
    value["hash"] = json!(format!("{:#x}", signed.hash));
    value["from"] = json!(f.from);
    value["blockNumber"] = Value::Null;
    value["blockHash"] = Value::Null;
    *node.original.lock().unwrap() = Some(value);
    chains::JournalRow {
        chain: node.net.chain.key,
        tx_hash: format!("{:#x}", signed.hash),
        from: f.from.clone(),
        to: OTHER.into(),
        nonce: Some(7),
        expiration_ms: None,
        timestamp: 0,
        kind: if token {
            "token-transfer"
        } else if invoke {
            "invoke"
        } else {
            "transfer"
        },
        amount_raw: Some(U256::from(123)),
        value: "123 units".into(),
        token: token.then(|| token_addr.to_checksum(None)),
    }
}

async fn recovered(node: &Node, f: &Fixture, row: &chains::JournalRow) -> OriginalIntent {
    replacement::replaced_intent(&node.net, &row.tx_hash, &key(f), 7, row)
        .await
        .unwrap()
}
async fn review(
    node: &Node,
    f: &Fixture,
    original: &OriginalIntent,
    hash: &str,
    action: Action,
) -> replacement::ReplacementReview {
    replacement::review_replacement(
        f.dir.path(),
        &node.net,
        &f.checked,
        original,
        hash,
        action,
        Options::default(),
    )
    .await
    .unwrap()
}
fn signed(f: &Fixture) -> tx::DecodedUnsigned {
    let request = f.signer.received.lock().unwrap().last().unwrap().clone();
    tx::decode_unsigned(
        &alloy::hex::decode(request["tx"].as_str().unwrap().trim_start_matches("0x")).unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn t27_restart_speedup_proves_original_and_signs_once_at_same_nonce() {
    // A registry token's transfer recovered from the node is also an invoke's bytes: its restart
    // speed-up is refused and only a cancel is offered
    // (journal_rewritten_to_token_transfer_cannot_authorize_a_pending_invoke).
    {
        let f = Fixture::new(false);
        let node = Node::new("ethereum");
        let row = original(&node, &f, 1, false, false);
        let original = recovered(&node, &f, &row).await;
        let review = review(&node, &f, &original, &row.tx_hash, Action::SpeedUp).await;
        assert_eq!(f.signer.count(), 0);
        assert_eq!(review.confirmation(), "alice");
        assert!(matches!(
            replacement::send_replacement(review, "alice", None)
                .await
                .unwrap(),
            SendReplacementOutcome::Sent(_)
        ));
        assert_eq!(f.signer.count(), 1);
        let actual = signed(&f);
        assert_eq!(actual.nonce(), 7);
        assert_eq!(actual.to(), original.decoded().to());
        assert_eq!(actual.value(), original.decoded().value());
        assert_eq!(actual.input(), original.decoded().input());
        match actual.fees() {
            tx::EvmFees::Eip1559 {
                max_fee_per_gas,
                max_priority_fee_per_gas,
            } => {
                assert!(max_fee_per_gas.to::<u128>() >= 2 * GWEI * 125 / 100);
                assert!(max_priority_fee_per_gas.to::<u128>() >= GWEI / 10 * 125 / 100);
            }
            _ => panic!("wrong type"),
        }
        assert_eq!(
            SendJournal::new(f.dir.path(), node.net.chain, &f.from)
                .recent()
                .unwrap()[0]
                .nonce,
            Some(7)
        );
    }
}

#[tokio::test]
async fn t27_journal_tamper_forged_sender_hash_fields_or_missing_pin_sign_nothing() {
    let f = Fixture::new(false);
    let node = Node::new("ethereum");
    let row = original(&node, &f, 1, false, false);
    for mutate in 0..5 {
        let mut changed = row.clone();
        match mutate {
            0 => changed.amount_raw = Some(U256::from(124)),
            1 => changed.to = f.from.clone(),
            2 => changed.nonce = Some(8),
            3 => changed.kind = "token-transfer",
            _ => changed.token = Some(OTHER.into()),
        }
        assert!(
            replacement::replaced_intent(&node.net, &row.tx_hash, &key(&f), 7, &changed)
                .await
                .is_err()
        );
    }
    let mut unverified = key(&f);
    unverified.address = None;
    assert!(
        replacement::replaced_intent(&node.net, &row.tx_hash, &unverified, 7, &row)
            .await
            .unwrap_err()
            .message
            .contains("needs: V")
    );
    let forged = original(&node, &f, 2, false, false);
    assert!(
        replacement::replaced_intent(&node.net, &forged.tx_hash, &key(&f), 7, &forged)
            .await
            .is_err()
    );
    let row = original(&node, &f, 1, false, false);
    node.original.lock().unwrap().as_mut().unwrap()["value"] = json!("0x7c");
    assert!(
        replacement::replaced_intent(&node.net, &row.tx_hash, &key(&f), 7, &row)
            .await
            .is_err()
    );
    assert_eq!(f.signer.count(), 0);
    assert_eq!(node.sent.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn t28_cancel_is_fixed_zero_self_and_requires_exact_cancel() {
    let f = Fixture::new(false);
    let node = Node::new("ethereum");
    let row = original(&node, &f, 1, false, false);
    let original = recovered(&node, &f, &row).await;
    let r = review(&node, &f, &original, &row.tx_hash, Action::Cancel).await;
    assert!(
        r.fields()["race"]
            .as_str()
            .unwrap()
            .contains("if the original is mined first")
    );
    assert!(
        replacement::send_replacement(r, "alice", None)
            .await
            .is_err()
    );
    assert_eq!(f.signer.count(), 0);
    let r = review(&node, &f, &original, &row.tx_hash, Action::Cancel).await;
    assert!(matches!(
        replacement::send_replacement(r, "cancel", None)
            .await
            .unwrap(),
        SendReplacementOutcome::Sent(_)
    ));
    let actual = signed(&f);
    assert_eq!(actual.nonce(), 7);
    assert_eq!(actual.to(), f.from.parse::<Address>().unwrap());
    assert_eq!(actual.value(), U256::ZERO);
    assert!(actual.input().is_empty());
    assert_eq!(f.signer.count(), 1);
}

#[tokio::test]
async fn t27_t28_fee_increase_and_op_l1_increase_require_new_review_without_signing() {
    for action in [Action::SpeedUp, Action::Cancel] {
        for chain in ["ethereum", "base"] {
            let f = Fixture::new(false);
            let node = Node::new(chain);
            let row = original(&node, &f, 1, false, false);
            let original = recovered(&node, &f, &row).await;
            let r = review(&node, &f, &original, &row.tx_hash, action).await;
            let typed = r.confirmation().to_string();
            let previous = r.max_total_fee();
            if chain == "base" {
                node.l1.store(10_001, Ordering::SeqCst);
            } else {
                node.base.store(2 * GWEI as u64, Ordering::SeqCst);
            }
            let SendReplacementOutcome::ReviewChanged(next) =
                replacement::send_replacement(r, &typed, None)
                    .await
                    .unwrap()
            else {
                panic!("signed increased fee")
            };
            assert!(next.max_total_fee() > previous);
            assert_eq!(f.signer.count(), 0);
        }
    }
}

#[tokio::test]
async fn t27_t28_current_policy_cap_and_consumed_nonce_sign_nothing() {
    for action in [Action::SpeedUp, Action::Cancel] {
        let mut f = Fixture::new(false);
        let node = Node::new("ethereum");
        let row = original(&node, &f, 1, false, false);
        let original = recovered(&node, &f, &row).await;
        f.checked
            .list
            .policy
            .set_max_fee_wei("ethereum", U256::from(1));
        let r = review(&node, &f, &original, &row.tx_hash, action).await;
        let typed = r.confirmation().to_string();
        assert!(r.verdict().is_err());
        assert!(
            replacement::send_replacement(r, &typed, None)
                .await
                .is_err()
        );
        assert_eq!(f.signer.count(), 0);
    }
    let f = Fixture::new(false);
    let node = Node::new("ethereum");
    let row = original(&node, &f, 1, false, false);
    let original = recovered(&node, &f, &row).await;
    let r = review(&node, &f, &original, &row.tx_hash, Action::SpeedUp).await;
    node.latest.store(8, Ordering::SeqCst);
    assert!(
        replacement::send_replacement(r, "alice", None)
            .await
            .is_err()
    );
    assert_eq!(f.signer.count(), 0);
}

#[tokio::test]
async fn t29_retry_uses_retained_review_even_with_no_hash_or_modified_deleted_journal() {
    for tamper in [false, true] {
        let f = Fixture::new(false);
        let node = Node::new("ethereum");
        node.pending.store(7, Ordering::SeqCst);
        let r = f.review(&node).await;
        let original = OriginalIntent::from_transfer(&r.retained_intent()).unwrap();
        let hash = format!("0x{}", "a".repeat(64));
        if tamper {
            SendJournal::new(f.dir.path(),node.net.chain,&f.from).append(&json!({"chain":"ethereum","from":f.from,"nonce":7,"txHash":hash,"to":f.from,"amountRaw":"999999999999999999","kind":"transfer","ts":"2000-01-01T00:00:00Z","value":"0.999 ETH"})).unwrap();
        }
        let retry = review(&node, &f, &original, &hash, Action::Retry).await;
        assert_eq!(retry.intent().nonce(), 7);
        assert_eq!(retry.replaced_hash(), hash);
        assert!(matches!(
            replacement::send_replacement(retry, "alice", None)
                .await
                .unwrap(),
            SendReplacementOutcome::Sent(_)
        ));
        assert_eq!(f.signer.count(), 1);
        let actual = signed(&f);
        assert_eq!(actual.nonce(), 7);
        assert_eq!(actual.to(), r.intent().to());
        assert_eq!(actual.value(), r.intent().value());
    }
}

#[tokio::test]
async fn t29_restart_cannot_retry_and_unknown_original_never_authorizes_any_action() {
    let f = Fixture::new(false);
    let node = Node::new("ethereum");
    let row = original(&node, &f, 1, false, false);
    let proven = recovered(&node, &f, &row).await;
    assert!(
        replacement::review_replacement(
            f.dir.path(),
            &node.net,
            &f.checked,
            &proven,
            &row.tx_hash,
            Action::Retry,
            Options::default()
        )
        .await
        .is_err()
    );
    *node.original.lock().unwrap() = None;
    let error = replacement::replaced_intent(&node.net, &row.tx_hash, &key(&f), 7, &row)
        .await
        .unwrap_err();
    assert!(error.message.contains("also without d") && error.message.contains("10 minutes"));
    for action in [Action::SpeedUp, Action::Cancel] {
        assert!(
            replacement::review_replacement(
                f.dir.path(),
                &node.net,
                &f.checked,
                &proven,
                &row.tx_hash,
                action,
                Options::default()
            )
            .await
            .is_err()
        );
    }
    assert_eq!(f.signer.count(), 0);
}

#[tokio::test]
async fn t27_session_speedup_ignores_journal_edits_and_preserves_reviewed_intent() {
    let f = Fixture::new(false);
    let node = Node::new("ethereum");
    node.pending.store(7, Ordering::SeqCst);
    let reviewed = f.review(&node).await;
    let retained = OriginalIntent::from_transfer(&reviewed.retained_intent()).unwrap();
    let signer: PrivateKeySigner = format!("{:064x}", 1).parse().unwrap();
    let signed_original = tx::sign_decoded(&signer, reviewed.intent()).unwrap();
    let envelope = TxEnvelope::decode_2718(&mut signed_original.raw.as_ref()).unwrap();
    let mut value = serde_json::to_value(envelope).unwrap();
    value["hash"] = json!(format!("{:#x}", signed_original.hash));
    value["from"] = json!(f.from);
    value["blockNumber"] = Value::Null;
    value["blockHash"] = Value::Null;
    *node.original.lock().unwrap() = Some(value);
    node.pending.store(8, Ordering::SeqCst);
    SendJournal::new(f.dir.path(),node.net.chain,&f.from).append(&json!({"chain":"ethereum","from":f.from,"nonce":7,"txHash":format!("{:#x}",signed_original.hash),"to":f.from,"amountRaw":"999","kind":"transfer","ts":"2000-01-01T00:00:00Z","value":"999 ETH"})).unwrap();
    let r = review(
        &node,
        &f,
        &retained,
        &format!("{:#x}", signed_original.hash),
        Action::SpeedUp,
    )
    .await;
    assert!(matches!(
        replacement::send_replacement(r, "alice", None)
            .await
            .unwrap(),
        SendReplacementOutcome::Sent(_)
    ));
    assert_eq!(f.signer.count(), 1);
    let actual = signed(&f);
    assert_eq!(actual.to(), reviewed.intent().to());
    assert_eq!(actual.value(), reviewed.intent().value());
    assert_eq!(actual.input(), reviewed.intent().input());
    assert_eq!(actual.nonce(), reviewed.intent().nonce());
}

#[tokio::test]
async fn invoke_kind_survives_proof_and_requires_current_contract_selector() {
    let mut f = Fixture::new(false);
    let node = Node::new("ethereum");
    let row = original(&node, &f, 1, false, true);
    let original = recovered(&node, &f, &row).await;
    assert_eq!(original.kind(), "invoke");
    assert!(
        replacement::review_replacement(
            f.dir.path(),
            &node.net,
            &f.checked,
            &original,
            &row.tx_hash,
            Action::SpeedUp,
            Options::default()
        )
        .await
        .is_err()
    );
    f.checked.list.entries.clear();
    f.checked.list.entries.push(Entry::new_contract(
        "router",
        OTHER,
        &["ethereum"],
        &["0x12345678"],
    ));
    let r = review(&node, &f, &original, &row.tx_hash, Action::SpeedUp).await;
    assert!(matches!(
        replacement::send_replacement(r, "router", None)
            .await
            .unwrap(),
        SendReplacementOutcome::Sent(_)
    ));
    assert_eq!(f.signer.count(), 1);
    assert_eq!(signed(&f).input(), original.decoded().input());
    assert_eq!(
        SendJournal::new(f.dir.path(), node.net.chain, &f.from)
            .recent()
            .unwrap()[0]
            .kind,
        "invoke"
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
fn press(app: &mut App, key: Key) -> Vec<Effect> {
    let work = app.update(Msg::Key(key));
    frame(app);
    work
}
fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        assert!(press(app, Key::Char(c)).is_empty());
    }
}

async fn drive_ui(app: &mut App, f: &Fixture, work: Vec<Effect>) -> Vec<Effect> {
    use zwallet::tui::app::transfer::TransferJob;
    let mut queue = std::collections::VecDeque::from(work);
    let mut pending = Vec::new();
    while let Some(effect) = queue.pop_front() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        match effect {
            Effect::Tools(id) => {
                let job = app.take_tool_job(id).expect("single use tool job");
                assert!(app.take_tool_job(id).is_none());
                job.run(id, f.dir.path(), &tx).await;
            }
            Effect::Replacement(id) => {
                let job = app
                    .take_replacement_job(id)
                    .expect("single use replacement job");
                assert!(app.take_replacement_job(id).is_none());
                job.run(id, f.dir.path(), &tx).await;
            }
            Effect::Transfer(id) => {
                let Some(job) = app.take_transfer_job(id) else {
                    continue;
                };
                if matches!(job, TransferJob::Receipt { .. }) {
                    continue;
                }
                job.run(id, f.dir.path(), &tx).await;
            }
            effect => {
                pending.push(effect);
                continue;
            }
        }
        while let Ok(msg) = rx.try_recv() {
            queue.extend(app.update(msg));
            frame(app);
        }
    }
    pending
}
fn write_row(f: &Fixture, node: &Node, row: &chains::JournalRow) {
    let value = json!({"ts":zwallet::core::time::rfc3339_utc(row.timestamp),"chain":row.chain,
        "txHash":row.tx_hash,"from":row.from,"to":row.to,"nonce":row.nonce,
        "kind":row.kind,"amountRaw":row.amount_raw.map(|v|v.to_string()),
        "value":"0.000000000000000123 ETH","token":row.token});
    std::fs::write(
        SendJournal::new(f.dir.path(), node.net.chain, &f.from).path(),
        format!("{value}\n"),
    )
    .unwrap();
}

// T34's integration proof uses real App forms/jobs and counts the entire wire path at
// FakeDaemon.received. Its upstream is only the deterministic test signer above.
fn invoke_fixture() -> (Fixture, common::FakeDaemon) {
    let mut f = Fixture::new(false);
    f.checked.list.entries = vec![Entry::new_contract(
        "router",
        OTHER,
        &["ethereum"],
        &["0x414bf389"],
    )];
    resign_invoke_list(&mut f);
    let upstream = f.dir.path().join("signer.sock");
    std::fs::rename(f.dir.path().join("d.sock"), &upstream).unwrap();
    let capture = common::FakeDaemon::proxy(&f.dir.path().join("d.sock"), upstream);
    (f, capture)
}
fn resign_invoke_list(f: &mut Fixture) {
    f.checked.list.generation += 1;
    let key = WhitelistKey::from_secret(&[7; 32]);
    let signed = key.sign(&f.checked.list);
    std::fs::write(
        f.dir.path().join(WHITELIST_FILE),
        signed.to_file_bytes().unwrap(),
    )
    .unwrap();
    f.checked = wallet::check_list(f.dir.path(), &Ok(Some(key.anchor_for(&signed)))).unwrap();
}
fn captured(capture: &common::FakeDaemon) -> Vec<tx::DecodedUnsigned> {
    capture
        .received
        .lock()
        .unwrap()
        .iter()
        .map(|bytes| {
            let request: Value = serde_json::from_slice(bytes).unwrap();
            assert_eq!(request["op"], "sign_tx");
            tx::decode_unsigned(&alloy::hex::decode(request["tx"].as_str().unwrap()).unwrap())
                .unwrap()
        })
        .collect()
}
async fn invoke_from_tools(f: &Fixture, node: &Node, capture: &common::FakeDaemon) -> App {
    use zwallet::tui::app::tools::Stage;
    node.pending.store(7, Ordering::SeqCst);
    let mut app = tui_app(f, node);
    press(&mut app, Key::Char('6'));
    let work = press(&mut app, Key::Char('i'));
    drive_ui(&mut app, f, work).await;
    assert_eq!(app.tools().stage, Stage::Contracts);
    press(&mut app, Key::Enter);
    type_text(
        &mut app,
        "exactInputSingle((address,address,uint24,address,uint256,uint256,uint256,uint160))",
    );
    press(&mut app, Key::Enter);
    type_text(&mut app, &format!("({OTHER},{OTHER},3000,{OTHER},1,2,3,0)"));
    press(&mut app, Key::Enter);
    // Replace the default zero, without a second authoritative input path.
    press(&mut app, Key::Home);
    press(&mut app, Key::Delete);
    type_text(&mut app, "0.001");
    let work = press(&mut app, Key::Enter);
    drive_ui(&mut app, f, work).await;
    assert_eq!(app.tools().stage, Stage::InvokeReview, "{}", frame(&app));
    let reviewed = app.tools().invoke.as_ref().unwrap().intent().clone();
    assert_eq!(&reviewed.input()[..4], &[0x41, 0x4b, 0xf3, 0x89]);
    assert_eq!(reviewed.value(), U256::from(1_000_000_000_000_000u64));
    assert!(captured(capture).is_empty());
    press(&mut app, Key::Enter);
    type_text(&mut app, "router");
    let work = press(&mut app, Key::Enter);
    assert!(press(&mut app, Key::Enter).is_empty(), "replayed submit");
    drive_ui(&mut app, f, work).await;
    assert_eq!(captured(capture), vec![reviewed]);
    let status = app.transfer().status.as_ref().unwrap();
    assert_eq!(
        app.tools().retained[&status.hash].intent.decoded(),
        &captured(capture)[0]
    );
    assert_eq!(
        SendJournal::new(f.dir.path(), node.net.chain, &f.from)
            .recent()
            .unwrap()[0]
            .kind,
        "invoke"
    );
    node.pending.store(8, Ordering::SeqCst);
    app
}
fn node_invoke(node: &Node, f: &Fixture, decoded: &tx::DecodedUnsigned, signer: u8) -> String {
    let private: PrivateKeySigner = format!("{signer:064x}").parse().unwrap();
    let signed = tx::sign_decoded(&private, decoded).unwrap();
    let envelope = TxEnvelope::decode_2718(&mut signed.raw.as_ref()).unwrap();
    let mut value = serde_json::to_value(envelope).unwrap();
    let hash = format!("{:#x}", signed.hash);
    value["hash"] = json!(hash);
    value["from"] = json!(f.from);
    value["blockNumber"] = Value::Null;
    value["blockHash"] = Value::Null;
    *node.original.lock().unwrap() = Some(value);
    hash
}
async fn confirm_invoke_replacement(app: &mut App, f: &Fixture, nickname: &str) {
    assert_eq!(
        app.replacement().stage,
        zwallet::tui::app::replacement::Stage::Review,
        "{}",
        frame(app)
    );
    press(app, Key::Enter);
    type_text(app, nickname);
    let work = press(app, Key::Enter);
    assert!(press(app, Key::Enter).is_empty());
    drive_ui(app, f, work).await;
}
fn assert_invoke_replacement(before: &tx::DecodedUnsigned, after: &tx::DecodedUnsigned) {
    assert_eq!(after.nonce(), before.nonce());
    assert_eq!(after.to(), before.to());
    assert_eq!(after.value(), before.value());
    assert_eq!(after.input(), before.input());
    assert!(after.fees().max_price() >= before.fees().max_price() * 5 / 4);
    match (before.fees(), after.fees()) {
        (
            tx::EvmFees::Eip1559 {
                max_priority_fee_per_gas: old,
                ..
            },
            tx::EvmFees::Eip1559 {
                max_priority_fee_per_gas: new,
                ..
            },
        ) => assert!(new >= old * U128::from(5) / U128::from(4)),
        other => panic!("expected type-2 fees: {other:?}"),
    }
}

#[tokio::test]
async fn t34_g_j_tools_progress_authorizes_only_same_nonce_invoke_or_zero_self_cancel() {
    for action in ['s', 'x'] {
        let (f, capture) = invoke_fixture();
        let node = Node::new("ethereum");
        let mut app = invoke_from_tools(&f, &node, &capture).await;
        let original = captured(&capture)[0].clone();
        // The node proves pending status, but no local journal can supply the intent.
        // Only the opaque Tools review can authorize this same-session replacement.
        let hash = node_invoke(&node, &f, &original, 1);
        assert_eq!(app.transfer().status.as_ref().unwrap().hash, hash);
        std::fs::remove_file(SendJournal::new(f.dir.path(), node.net.chain, &f.from).path())
            .unwrap();
        let work = press(&mut app, Key::Char(action));
        drive_ui(&mut app, &f, work).await;
        confirm_invoke_replacement(
            &mut app,
            &f,
            if action == 'x' { "cancel" } else { "router" },
        )
        .await;
        let requests = captured(&capture);
        assert_eq!(
            requests.len(),
            2,
            "one original plus exactly one replacement"
        );
        if action == 's' {
            assert_invoke_replacement(&original, &requests[1]);
        } else {
            assert_eq!(requests[1].nonce(), original.nonce());
            assert_eq!(requests[1].to().to_checksum(None), f.from);
            assert!(requests[1].value().is_zero());
            assert!(requests[1].input().is_empty());
        }
    }
}

#[tokio::test]
async fn t34_g_restarted_invoke_proves_node_bytes_and_retains_every_argument() {
    let (f, capture) = invoke_fixture();
    let node = Node::new("ethereum");
    let first = invoke_from_tools(&f, &node, &capture).await;
    let original = captured(&capture)[0].clone();
    let hash = node_invoke(&node, &f, &original, 1);
    assert_eq!(first.transfer().status.as_ref().unwrap().hash, hash);
    let row = SendJournal::new(f.dir.path(), node.net.chain, &f.from)
        .recent()
        .unwrap()
        .remove(0);
    let mut restarted = tui_app(&f, &node);
    history_review(&mut restarted, &f, &node, &row, 's').await;
    confirm_invoke_replacement(&mut restarted, &f, "router").await;
    let requests = captured(&capture);
    assert_eq!(requests.len(), 2);
    assert_invoke_replacement(&original, &requests[1]);
}

#[tokio::test]
async fn t34_h_current_resigned_contract_or_selector_refuses_speedup_and_retry() {
    for (remove_contract, action) in [(true, 's'), (false, 's'), (true, 'R'), (false, 'R')] {
        let (mut f, capture) = invoke_fixture();
        let node = Node::new("ethereum");
        node.uncertain.store(action == 'R', Ordering::SeqCst);
        let mut app = invoke_from_tools(&f, &node, &capture).await;
        f.checked.list.entries = if remove_contract {
            vec![]
        } else {
            vec![Entry::new_contract(
                "router",
                OTHER,
                &["ethereum"],
                &["0x12345678"],
            )]
        };
        resign_invoke_list(&mut f);
        app.update(Msg::Checked(Ok((
            wallet::Opened {
                keys: vec![key(&f)],
                active: "main".into(),
                warning: None,
                anchor: Ok(None),
                salted: true,
            },
            Ok(f.checked.clone()),
        ))));
        press(&mut app, Key::Char('3'));
        let work = press(&mut app, Key::Char(action));
        drive_ui(&mut app, &f, work).await;
        assert_eq!(
            app.replacement().stage,
            zwallet::tui::app::replacement::Stage::Failed,
            "{}",
            frame(&app)
        );
        assert_eq!(
            captured(&capture).len(),
            1,
            "policy refusal must add zero SignTx"
        );
    }
}

#[tokio::test]
async fn t34_i_l_restart_refuses_rewritten_journal_forged_signer_and_changed_arguments() {
    for attack in ["journal-to", "journal-value", "signer", "arguments"] {
        let (f, capture) = invoke_fixture();
        let node = Node::new("ethereum");
        let _first = invoke_from_tools(&f, &node, &capture).await;
        let original = captured(&capture)[0].clone();
        let mut row = SendJournal::new(f.dir.path(), node.net.chain, &f.from)
            .recent()
            .unwrap()
            .remove(0);
        let hash = node_invoke(&node, &f, &original, if attack == "signer" { 2 } else { 1 });
        row.tx_hash = hash.clone();
        match attack {
            "journal-to" => row.to = f.from.clone(),
            "journal-value" => row.amount_raw = Some(U256::from(999)),
            "arguments" => {
                let mut fields = node.original.lock().unwrap();
                let fields = fields.as_mut().unwrap();
                let mut bytes = original.input().to_vec();
                *bytes.last_mut().unwrap() ^= 1;
                fields["input"] = json!(format!("0x{}", alloy::hex::encode(bytes)));
            }
            _ => {}
        }
        write_row(&f, &node, &row);
        let mut restarted = tui_app(&f, &node);
        let work = detail(
            &mut restarted,
            &f,
            &node,
            &hash,
            zwallet::history::State::Pending,
        );
        drive_ui(&mut restarted, &f, work).await;
        assert!(
            !restarted.replacement_available().0,
            "{attack}: {}",
            frame(&restarted)
        );
        for key in ['s', 'x'] {
            assert!(press(&mut restarted, Key::Char(key)).is_empty());
        }
        assert_eq!(
            captured(&capture).len(),
            1,
            "{attack}: zero additional SignTx"
        );
    }
}

#[tokio::test]
async fn t34_k_uncertain_tools_invoke_retries_retained_intent_but_restart_cannot() {
    let (f, capture) = invoke_fixture();
    let node = Node::new("ethereum");
    node.uncertain.store(true, Ordering::SeqCst);
    let mut app = invoke_from_tools(&f, &node, &capture).await;
    let original = captured(&capture)[0].clone();
    let hash = app.transfer().status.as_ref().unwrap().hash.clone();
    assert!(app.tools().retained[&hash].uncertain);
    assert!(app.replacement_available().1);
    // The missing local row cannot create or change retry authority.
    std::fs::remove_file(SendJournal::new(f.dir.path(), node.net.chain, &f.from).path()).unwrap();
    let mut restarted = tui_app(&f, &node);
    let work = detail(
        &mut restarted,
        &f,
        &node,
        &hash,
        zwallet::history::State::NotFound,
    );
    drive_ui(&mut restarted, &f, work).await;
    assert!(!restarted.replacement_available().1);
    assert!(frame(&restarted).contains("pay twice"));
    assert!(press(&mut restarted, Key::Char('R')).is_empty());
    assert_eq!(captured(&capture).len(), 1);
    let work = press(&mut app, Key::Char('R'));
    drive_ui(&mut app, &f, work).await;
    node.uncertain.store(false, Ordering::SeqCst);
    confirm_invoke_replacement(&mut app, &f, "router").await;
    let requests = captured(&capture);
    assert_eq!(requests.len(), 2);
    assert_invoke_replacement(&original, &requests[1]);
}
fn detail(
    app: &mut App,
    f: &Fixture,
    node: &Node,
    hash: &str,
    state: zwallet::history::State,
) -> Vec<Effect> {
    use zwallet::chains::evm::rpc::{MovementsState, TxDetail};
    use zwallet::tui::app::history::{DetailRead, HistoryTarget, TransactionDetail};
    press(app, Key::Char('2'));
    app.update(Msg::HistoryOwner(
        HistoryTarget {
            generation: app.generation(),
            key: "main".into(),
            endpoint: zwallet::tui::app::RpcChoice {
                chain: node.net.chain.key,
                url: node.net.rpc_url.clone(),
            },
            checked: app.checked_list().map(|c| c.fingerprint),
        },
        Ok((f.from.clone(), wallet::Trust::Session)),
    ));
    press(app, Key::Char('/'));
    type_text(app, hash);
    let work = press(app, Key::Enter);
    let request = work
        .into_iter()
        .find_map(|e| {
            if let Effect::LoadTransaction(r) = e {
                Some(r)
            } else {
                None
            }
        })
        .expect("lookup effect");
    app.update(Msg::Transaction(
        request,
        Ok(DetailRead {
            list_unchanged: true,
            detail: TransactionDetail::Evm(TxDetail {
                chain: node.net.chain.key,
                tx_hash: hash.into(),
                state,
                success: None,
                from: Some(f.from.clone()),
                to: Some(OTHER.into()),
                contract_creation: false,
                nonce: (state == zwallet::history::State::Pending).then_some(7),
                journal_nonce: None,
                value: Some(U256::from(123)),
                block_number: None,
                gas_used: None,
                effective_gas_price: None,
                l1_fee: None,
                fee: None,
                confirmations: None,
                movements: vec![],
                movements_state: MovementsState::Pending,
                hidden_movements: 0,
                hint: "",
            }),
        }),
    ))
}
async fn history_review(
    app: &mut App,
    f: &Fixture,
    node: &Node,
    row: &chains::JournalRow,
    key: char,
) {
    write_row(f, node, row);
    let work = detail(app, f, node, &row.tx_hash, zwallet::history::State::Pending);
    assert!(
        !app.replacement_available().0,
        "unproved node sender must not enable s/x"
    );
    drive_ui(app, f, work).await;
    assert!(app.replacement_available().0, "{}", frame(app));
    let work = press(app, Key::Char(key));
    drive_ui(app, f, work).await;
    assert_eq!(
        app.replacement().stage,
        zwallet::tui::app::replacement::Stage::Review,
        "{}",
        frame(app)
    );
}
#[tokio::test]
async fn tui_t27_t28_proof_then_typed_review_signs_once_and_retains_status() {
    for action in ['s', 'x'] {
        let f = Fixture::new(false);
        let node = Node::new("ethereum");
        let row = original(&node, &f, 1, false, false);
        let mut app = tui_app(&f, &node);
        history_review(&mut app, &f, &node, &row, action).await;
        let rendered = frame(&app);
        assert!(rendered.contains("nonce 7"));
        assert!(rendered.contains(&row.tx_hash));
        press(&mut app, Key::Enter);
        type_text(&mut app, "wrong");
        assert!(press(&mut app, Key::Enter).is_empty());
        assert_eq!(f.signer.count(), 0);
        for _ in 0..5 {
            press(&mut app, Key::Backspace);
        }
        type_text(&mut app, if action == 'x' { "cancel" } else { "alice" });
        let work = press(&mut app, Key::Enter);
        assert!(app.sending());
        assert!(press(&mut app, Key::Enter).is_empty());
        drive_ui(&mut app, &f, work).await;
        assert_eq!(f.signer.count(), 1);
        let actual = signed(&f);
        assert_eq!(actual.nonce(), 7);
        if action == 'x' {
            assert_eq!(actual.to().to_checksum(None), f.from);
            assert!(actual.value().is_zero());
            assert!(actual.input().is_empty());
            assert_eq!(
                app.replacement_status_notes(),
                vec![replacement::CANCEL_RACE]
            );
        } else {
            assert_eq!(actual.to().to_checksum(None), OTHER);
            assert_eq!(actual.value(), U256::from(123));
        }
        assert_ne!(app.transfer().status.as_ref().unwrap().hash, row.tx_hash);
    }
}
#[tokio::test]
async fn tui_t27_bad_original_never_offers_actions_and_late_proof_cannot_reopen_detail() {
    for bad in 0..4 {
        let f = Fixture::new(false);
        let node = Node::new("ethereum");
        let mut row = original(&node, &f, if bad == 1 { 2 } else { 1 }, false, false);
        if bad == 0 {
            row.amount_raw = Some(U256::from(999));
        }
        if bad == 2 {
            node.original.lock().unwrap().as_mut().unwrap()["value"] = json!("0x999");
        }
        write_row(&f, &node, &row);
        let mut app = tui_app(&f, &node);
        let work = detail(
            &mut app,
            &f,
            &node,
            &row.tx_hash,
            zwallet::history::State::Pending,
        );
        if bad == 3 {
            press(&mut app, Key::Esc);
        }
        drive_ui(&mut app, &f, work).await;
        assert!(!app.replacement_available().0);
        assert_eq!(f.signer.count(), 0);
        if bad != 3 {
            assert!(press(&mut app, Key::Char('s')).is_empty());
            assert!(press(&mut app, Key::Char('x')).is_empty());
        } else {
            assert!(app.history().detail.is_none());
        }
    }
}
#[tokio::test]
async fn tui_t27_t28_fee_recheck_and_deferred_lock_never_sign_or_hang() {
    for action in ['s', 'x'] {
        let f = Fixture::new(false);
        let node = Node::new("ethereum");
        let row = original(&node, &f, 1, false, false);
        let mut app = tui_app(&f, &node);
        history_review(&mut app, &f, &node, &row, action).await;
        press(&mut app, Key::Enter);
        type_text(&mut app, if action == 'x' { "cancel" } else { "alice" });
        node.base.store((10 * GWEI) as u64, Ordering::SeqCst);
        let work = press(&mut app, Key::Enter);
        assert!(app.sending());
        assert!(press(&mut app, Key::Char('L')).is_empty());
        assert!(app.locking());
        let effects = drive_ui(&mut app, &f, work).await;
        assert_eq!(effects, vec![Effect::LockAll]);
        assert_eq!(f.signer.count(), 0);
        assert!(!app.sending());
        assert_eq!(
            app.replacement().stage,
            zwallet::tui::app::replacement::Stage::Review
        );
        assert!(press(&mut app, Key::Enter).is_empty());
        assert_eq!(f.signer.count(), 0);
        app.update(Msg::Locked(Ok(vec![])));
        press(&mut app, Key::Char('q'));
        assert!(app.quitting());
    }
}
#[tokio::test]
async fn tui_t29_restart_unknown_warning_survives_aged_and_missing_journal_without_retry() {
    for exists in [true, false] {
        let f = Fixture::new(false);
        let node = Node::new("ethereum");
        let row = original(&node, &f, 1, false, false);
        if exists {
            write_row(&f, &node, &row);
        }
        *node.original.lock().unwrap() = None;
        let mut app = tui_app(&f, &node);
        let work = detail(
            &mut app,
            &f,
            &node,
            &row.tx_hash,
            zwallet::history::State::NotFound,
        );
        assert!(work.is_empty());
        assert_eq!(
            app.history_replacement_note(),
            Some(replacement::RETRY_WARNING)
        );
        let shown = frame(&app);
        assert!(shown.contains("it can pay twice"), "{shown}");
        assert!(!app.replacement_available().0 && !app.replacement_available().1);
        assert!(!app.history_replacement_hint().contains("R retry"));
        assert!(press(&mut app, Key::Char('R')).is_empty());
        assert_eq!(f.signer.count(), 0);
    }
}

#[tokio::test]
async fn tui_t29_same_session_uncertain_retry_ignores_journal_edit_and_preserves_nonce() {
    let f = Fixture::new(false);
    let node = Node::new("ethereum");
    node.pending.store(7, Ordering::SeqCst);
    node.uncertain.store(true, Ordering::SeqCst);
    let mut app = tui_app(&f, &node);
    let work = press(&mut app, Key::Char('3'));
    drive_ui(&mut app, &f, work).await;
    let work = press(&mut app, Key::Enter);
    drive_ui(&mut app, &f, work).await;
    press(&mut app, Key::Enter);
    type_text(&mut app, "0.01");
    let work = press(&mut app, Key::Enter);
    drive_ui(&mut app, &f, work).await;
    press(&mut app, Key::Enter);
    type_text(&mut app, "alice");
    let work = press(&mut app, Key::Enter);
    drive_ui(&mut app, &f, work).await;
    assert_eq!(f.signer.count(), 1);
    let original = signed(&f);
    let first_hash = app.transfer().status.as_ref().unwrap().hash.clone();
    assert!(app.transfer().status.as_ref().unwrap().uncertain);
    assert!(app.replacement_available().1);
    std::fs::write(
        SendJournal::new(f.dir.path(), node.net.chain, &f.from).path(),
        b"corrupt local claim\n",
    )
    .unwrap();
    let work = press(&mut app, Key::Char('R'));
    drive_ui(&mut app, &f, work).await;
    assert_eq!(
        app.replacement().review.as_ref().unwrap().replaced_hash(),
        first_hash
    );
    press(&mut app, Key::Enter);
    type_text(&mut app, "alice");
    node.uncertain.store(false, Ordering::SeqCst);
    let work = press(&mut app, Key::Enter);
    drive_ui(&mut app, &f, work).await;
    assert_eq!(f.signer.count(), 2);
    let retried = signed(&f);
    assert_eq!(retried.nonce(), original.nonce());
    assert_eq!(retried.to(), original.to());
    assert_eq!(retried.value(), original.value());
    assert_eq!(retried.input(), original.input());
    assert_ne!(app.transfer().status.as_ref().unwrap().hash, first_hash);
}

#[tokio::test]
async fn tui_cancel_uncertain_retry_keeps_cancel_confirmation_without_self_entry() {
    let f = Fixture::new(false);
    let node = Node::new("ethereum");
    let row = original(&node, &f, 1, false, false);
    let mut app = tui_app(&f, &node);
    history_review(&mut app, &f, &node, &row, 'x').await;
    press(&mut app, Key::Enter);
    type_text(&mut app, "cancel");
    node.uncertain.store(true, Ordering::SeqCst);
    let work = press(&mut app, Key::Enter);
    drive_ui(&mut app, &f, work).await;
    assert!(app.transfer().status.as_ref().unwrap().uncertain);
    assert!(app.replacement_available().1);
    assert!(
        frame(&app).contains("it can pay twice"),
        "an uncertain cancel must keep the duplicate-payment warning"
    );
    assert!(frame(&app).contains("a cancel replaces nonce N"));
    press(&mut app, Key::Char('w'));
    assert!(frame(&app).contains("it can pay twice") && frame(&app).contains("Wait from now"));
    press(&mut app, Key::Esc);
    let work = press(&mut app, Key::Char('R'));
    drive_ui(&mut app, &f, work).await;
    assert_eq!(
        app.replacement().stage,
        zwallet::tui::app::replacement::Stage::Review,
        "{}",
        frame(&app)
    );
    assert_eq!(
        app.replacement().review.as_ref().unwrap().confirmation(),
        "cancel"
    );
    press(&mut app, Key::Enter);
    type_text(&mut app, "cancel");
    node.uncertain.store(false, Ordering::SeqCst);
    let work = press(&mut app, Key::Enter);
    drive_ui(&mut app, &f, work).await;
    assert_eq!(f.signer.count(), 2);
    let actual = signed(&f);
    assert_eq!(actual.nonce(), 7);
    assert_eq!(actual.to().to_checksum(None), f.from);
    assert!(actual.value().is_zero() && actual.input().is_empty());
    assert_eq!(
        app.replacement_status_notes(),
        vec![replacement::CANCEL_RACE]
    );
}

#[tokio::test]
async fn tui_replacement_review_exposes_full_long_quantity_and_invoke_calldata() {
    let mut f = Fixture::new(false);
    let node = Node::new("ethereum");
    let mut row = original(&node, &f, 1, false, true);
    f.checked.list.entries.clear();
    f.checked.list.entries.push(Entry::new_contract(
        "router",
        OTHER,
        &["ethereum"],
        &["0x12345678"],
    ));
    let proven = recovered(&node, &f, &row).await;
    let d = proven.decoded();
    let value = U256::from_str_radix(&format!("1{}987654321", "0".repeat(66)), 10).unwrap();
    let data = alloy::hex::decode(format!("12345678{}deadbeefcafe4321", "42".repeat(90))).unwrap();
    let unsigned = tx::UnsignedEvmTx {
        chain_id: d.chain_id(),
        nonce: d.nonce(),
        gas_limit: d.gas_limit(),
        fees: d.fees(),
        to: d.to(),
        value,
        input: data.into(),
    };
    let signer: PrivateKeySigner = format!("{:064x}", 1).parse().unwrap();
    let signed = tx::sign_decoded(
        &signer,
        &tx::decode_unsigned(&tx::encoded_for_signing(&unsigned)).unwrap(),
    )
    .unwrap();
    let envelope = TxEnvelope::decode_2718(&mut signed.raw.as_ref()).unwrap();
    let mut json = serde_json::to_value(envelope).unwrap();
    row.tx_hash = format!("{:#x}", signed.hash);
    row.amount_raw = Some(value);
    json["hash"] = json!(row.tx_hash);
    json["from"] = json!(f.from);
    json["blockNumber"] = Value::Null;
    json["blockHash"] = Value::Null;
    *node.original.lock().unwrap() = Some(json);
    let mut app = tui_app(&f, &node);
    history_review(&mut app, &f, &node, &row, 's').await;
    let mut all = String::new();
    for _ in 0..60 {
        all.push_str(&frame(&app));
        press(&mut app, Key::Char('j'));
    }
    assert!(
        all.contains("987654321"),
        "the tail of the reviewed quantity was lost"
    );
    assert!(
        all.contains("deadbeefcafe4321"),
        "the reviewed calldata cannot be inspected"
    );
    assert_eq!(f.signer.count(), 0);
}

#[tokio::test]
async fn tui_uncertain_retry_disappears_after_a_mined_receipt() {
    use zwallet::core::error::{ErrorCode, WalletError};
    use zwallet::tui::app::transfer::TransferMsg;
    let f = Fixture::new(false);
    let node = Node::new("ethereum");
    node.pending.store(7, Ordering::SeqCst);
    let review = f.review(&node).await;
    let original = OriginalIntent::from_transfer(&review.retained_intent()).unwrap();
    let hash = format!("0x{}", "12".repeat(32));
    let mut app = tui_app(&f, &node);
    app.register_retained_replacement(&hash, original);
    let id = app.begin_external_send();
    app.update(Msg::Transfer(
        id,
        TransferMsg::Progress {
            hash: hash.clone(),
            nonce: 7,
            journal: SendJournal::new(f.dir.path(), node.net.chain, &f.from)
                .path()
                .into(),
        },
    ));
    let effects = app.update(Msg::Transfer(
        id,
        TransferMsg::Sent(Err(WalletError::new(
            ErrorCode::BroadcastUncertain,
            "uncertain",
        ))),
    ));
    assert!(app.replacement_available().1);
    let watch = effects
        .into_iter()
        .find_map(|e| {
            if let Effect::Transfer(id) = e {
                Some(id)
            } else {
                None
            }
        })
        .unwrap();
    app.update(Msg::Transfer(
        watch,
        TransferMsg::Receipt(Ok(Some(json!({"success":true})))),
    ));
    assert!(
        !app.replacement_available().1,
        "a mined receipt resolves uncertainty"
    );
    assert!(app.replacement_status_notes().is_empty());
    assert!(
        press(&mut app, Key::Char('R')).is_empty(),
        "resolved sends must not dispatch retry preparation"
    );
    assert_eq!(f.signer.count(), 0);
}

/// A pending invoke of a registry token's `transfer` (value 0) whose journal row is rewritten
/// to `token-transfer` with the same token and the decoded recipient and amount. The list allows
/// the recipient's address but holds no contract entry for the token (none can). The journal must
/// not choose the more permissive token-transfer rule: zero `SignTx`, in the library and on the
/// screen. A cancel needs no entry and stays available.
#[tokio::test]
async fn journal_rewritten_to_token_transfer_cannot_authorize_a_pending_invoke() {
    const USDC_ETH: &str = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48";
    let invoke_row = |rewritten: &chains::JournalRow| chains::JournalRow {
        kind: "invoke",
        to: USDC_ETH.into(),
        amount_raw: Some(U256::ZERO),
        token: None,
        ..rewritten.clone()
    };
    // The library.
    let f = Fixture::new(false);
    let node = Node::new("ethereum");
    let rewritten = original(&node, &f, 1, true, false);
    assert_eq!(rewritten.kind, "token-transfer");
    let journaled = recovered(&node, &f, &invoke_row(&rewritten)).await;
    assert!(
        replacement::review_replacement(
            f.dir.path(),
            &node.net,
            &f.checked,
            &journaled,
            &rewritten.tx_hash,
            Action::SpeedUp,
            Options::default()
        )
        .await
        .is_err(),
        "the invoke as journaled needs a contract entry"
    );
    let refused =
        match replacement::replaced_intent(&node.net, &rewritten.tx_hash, &key(&f), 7, &rewritten)
            .await
        {
            Err(e) => Some(e),
            Ok(original) => match replacement::review_replacement(
                f.dir.path(),
                &node.net,
                &f.checked,
                &original,
                &rewritten.tx_hash,
                Action::SpeedUp,
                Options::default(),
            )
            .await
            {
                Err(e) => Some(e),
                Ok(review) => {
                    let typed = review.confirmation().to_string();
                    replacement::send_replacement(review, &typed, None)
                        .await
                        .err()
                }
            },
        };
    assert_eq!(
        f.signer.count(),
        0,
        "the rewritten journal row got a SignTx"
    );
    assert_eq!(node.sent.load(Ordering::SeqCst), 0);
    assert!(refused.is_some());
    // The screen: a restarted TUI, the rewritten row, `s`.
    let f = Fixture::new(false);
    let node = Node::new("ethereum");
    let rewritten = original(&node, &f, 1, true, false);
    write_row(&f, &node, &rewritten);
    let mut app = tui_app(&f, &node);
    let work = detail(
        &mut app,
        &f,
        &node,
        &rewritten.tx_hash,
        zwallet::history::State::Pending,
    );
    drive_ui(&mut app, &f, work).await;
    let work = press(&mut app, Key::Char('s'));
    drive_ui(&mut app, &f, work).await;
    assert_eq!(
        app.replacement().stage,
        zwallet::tui::app::replacement::Stage::Failed,
        "{}",
        frame(&app)
    );
    assert_eq!(f.signer.count(), 0, "the screen signed a rewritten row");
    assert!(frame(&app).contains("same bytes"), "{}", frame(&app));
    // A cancel is the same zero self-transfer for either kind, and stays available.
    let f = Fixture::new(false);
    let node = Node::new("ethereum");
    let rewritten = original(&node, &f, 1, true, false);
    let mut app = tui_app(&f, &node);
    history_review(&mut app, &f, &node, &rewritten, 'x').await;
    press(&mut app, Key::Enter);
    type_text(&mut app, "cancel");
    let work = press(&mut app, Key::Enter);
    drive_ui(&mut app, &f, work).await;
    assert_eq!(f.signer.count(), 1);
    let actual = signed(&f);
    assert_eq!(actual.nonce(), 7);
    assert_eq!(actual.to().to_checksum(None), f.from);
    assert!(actual.value().is_zero() && actual.input().is_empty());
}

/// This session's own token-transfer review keeps its category: its speed-up signs once with the
/// reviewed bytes, whatever the journal row says.
#[tokio::test]
async fn t27_session_token_speedup_keeps_the_reviewed_category() {
    let f = Fixture::new(false);
    let node = Node::new("ethereum");
    node.pending.store(7, Ordering::SeqCst);
    let mut input = f.input(&node);
    input.token = Some("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".into());
    let reviewed = wallet::review_transfer(f.dir.path(), &node.net, &f.checked, input)
        .await
        .unwrap();
    assert!(matches!(
        reviewed.intent().action(),
        tx::Action::TransferCall { .. }
    ));
    let retained = OriginalIntent::from_transfer(&reviewed.retained_intent()).unwrap();
    assert_eq!(retained.kind(), "token-transfer");
    let hash = node_invoke(&node, &f, reviewed.intent(), 1);
    node.pending.store(8, Ordering::SeqCst);
    let r = review(&node, &f, &retained, &hash, Action::SpeedUp).await;
    assert!(matches!(
        replacement::send_replacement(r, "alice", None)
            .await
            .unwrap(),
        SendReplacementOutcome::Sent(_)
    ));
    assert_eq!(f.signer.count(), 1);
    let actual = signed(&f);
    assert_eq!(actual.nonce(), reviewed.intent().nonce());
    assert_eq!(actual.to(), reviewed.intent().to());
    assert_eq!(actual.input(), reviewed.intent().input());
}

/// D1: the replacement review is not a text field; the global tab keys act there.
#[tokio::test]
async fn tui_replacement_review_passes_the_global_tab_keys() {
    use zwallet::tui::actions::Tab;
    let f = Fixture::new(false);
    let node = Node::new("ethereum");
    let row = original(&node, &f, 1, false, false);
    let mut app = tui_app(&f, &node);
    history_review(&mut app, &f, &node, &row, 's').await;
    assert_eq!(app.tab(), Tab::Send);
    press(&mut app, Key::Char('4'));
    assert_eq!(app.tab(), Tab::Receive, "{}", frame(&app));
    assert_eq!(f.signer.count(), 0);
}

/// D3: the invoke and raw-invoke reviews show the pending and repeat guards' verdicts before the
/// nickname, as the transfer review does, so `p`/`d` are chosen knowingly. The repeat key is the
/// CLI's chain/kind/contract/value/token plus encoded calldata: identical bytes repeat,
/// while different arguments to the same contract and value do not.
#[tokio::test]
async fn tools_invoke_and_raw_reviews_show_the_pending_and_repeat_verdicts() {
    use zwallet::tui::app::tools::Stage;
    let (f, capture) = invoke_fixture();
    let node = Node::new("ethereum");
    let mut app = invoke_from_tools(&f, &node, &capture).await;
    node.pending.store(7, Ordering::SeqCst);
    for (raw, same) in [(false, true), (true, true), (false, false), (true, false)] {
        press(&mut app, Key::Char('6'));
        let work = press(&mut app, Key::Char(if raw { 'd' } else { 'i' }));
        drive_ui(&mut app, &f, work).await;
        assert_eq!(app.tools().stage, Stage::Contracts, "{}", frame(&app));
        press(&mut app, Key::Enter);
        if raw {
            let data = if same {
                format!("0x{}", alloy::hex::encode(captured(&capture)[0].input()))
            } else {
                format!("0x414bf389{}", "00".repeat(32 * 8))
            };
            type_text(&mut app, &data);
        } else {
            type_text(
                &mut app,
                "exactInputSingle((address,address,uint24,address,uint256,uint256,uint256,uint160))",
            );
            press(&mut app, Key::Enter);
            let args = if same {
                format!("({OTHER},{OTHER},3000,{OTHER},1,2,3,0)")
            } else {
                format!("({OTHER},{OTHER},500,{OTHER},9,8,7,0)")
            };
            type_text(&mut app, &args);
        }
        press(&mut app, Key::Enter);
        press(&mut app, Key::Home);
        press(&mut app, Key::Delete);
        type_text(&mut app, "0.001");
        let work = press(&mut app, Key::Enter);
        drive_ui(&mut app, &f, work).await;
        assert_eq!(app.tools().stage, Stage::InvokeReview, "{}", frame(&app));
        let shown = frame(&app);
        let expected = if same {
            "repeats 1 blocked"
        } else {
            "repeats 0 clear"
        };
        assert!(shown.contains(expected), "raw {raw}, same {same}:\n{shown}");
        // N1: a guard with nothing to block reads `clear`, not `blocked`.
        assert!(shown.contains("pending 0 clear"), "raw {raw}:\n{shown}");
        if same {
            let work = press(&mut app, Key::Char('d'));
            drive_ui(&mut app, &f, work).await;
            assert!(
                frame(&app).contains("repeats 1 OVERRIDE"),
                "{}",
                frame(&app)
            );
        }
        press(&mut app, Key::Esc);
    }
    assert_eq!(captured(&capture).len(), 1, "a review signs nothing");
}

/// Round-2 audit: a replacement review left with 4 and reopened with 3 still confirms, with
/// exactly one SignTx at the replaced nonce.
#[tokio::test]
async fn tui_replacement_review_survives_a_tab_round_trip_and_signs_once() {
    use zwallet::tui::actions::Tab;
    let f = Fixture::new(false);
    let node = Node::new("ethereum");
    let row = original(&node, &f, 1, false, false);
    let mut app = tui_app(&f, &node);
    history_review(&mut app, &f, &node, &row, 's').await;
    assert_eq!(app.tab(), Tab::Send);
    for k in ['3', '4', '3'] {
        let work = press(&mut app, Key::Char(k));
        drive_ui(&mut app, &f, work).await;
    }
    assert_eq!(app.tab(), Tab::Send);
    assert_eq!(
        app.replacement().stage,
        zwallet::tui::app::replacement::Stage::Review,
        "{}",
        frame(&app)
    );
    press(&mut app, Key::Enter);
    type_text(&mut app, "alice");
    let work = press(&mut app, Key::Enter);
    drive_ui(&mut app, &f, work).await;
    assert_eq!(f.signer.count(), 1, "{}", frame(&app));
    let actual = signed(&f);
    assert_eq!(actual.nonce(), 7);
    assert_eq!(actual.to().to_checksum(None), OTHER);
}

#[tokio::test]
async fn task_attribution_is_retained_explicit_and_bound_before_replacement_review() {
    let f = Fixture::new(false);
    let node = Node::new("ethereum");
    let path = f.dir.path().join("task-fixture.json");
    let task = json!({"version":1,"id":"fixture","chain":"ethereum","owner":f.from,
        "createdAt":1,"baselineSource":"http://127.0.0.1:9","allocations":{},
        "baseline":{"native":"0"},"intents":[]});
    std::fs::write(&path, serde_json::to_vec(&task).unwrap()).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    node.pending.store(7, Ordering::SeqCst);
    let mut input = f.input(&node);
    input.options.task_id = Some("fixture".into());
    let transfer = wallet::review_transfer(f.dir.path(), &node.net, &f.checked, input)
        .await
        .unwrap();
    assert_eq!(transfer.fields()["taskId"], "fixture");
    let retained = OriginalIntent::from_transfer(&transfer.retained_intent()).unwrap();
    assert_eq!(retained.task_id(), Some("fixture"));
    node.pending.store(8, Ordering::SeqCst);
    let row = original(&node, &f, 1, false, false);
    let recovered = recovered(&node, &f, &row).await;
    assert_eq!(
        recovered.task_id(),
        None,
        "mutable history must not infer task attribution"
    );
    let attributed = recovered.with_task_id(Some("fixture".into())).unwrap();
    let reviewed = review(&node, &f, &attributed, &row.tx_hash, Action::SpeedUp).await;
    assert_eq!(reviewed.fields()["taskId"], "fixture");
    assert_eq!(reviewed.fields()["replaces"], row.tx_hash);
    assert_eq!(reviewed.retained_intent().task_id(), Some("fixture"));
    let wrong = attributed.with_task_id(Some("missing".into())).unwrap();
    assert!(
        replacement::review_replacement(
            f.dir.path(),
            &node.net,
            &f.checked,
            &wrong,
            &row.tx_hash,
            Action::SpeedUp,
            Options::default()
        )
        .await
        .is_err()
    );
    assert_eq!(f.signer.count(), 0);
}
