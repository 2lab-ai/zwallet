//! TUI tools use held data and an independently checked list. Only isolated test sockets.
mod common;
use alloy::primitives::eip191_hash_message;
use alloy::signers::{SignerSync, local::PrivateKeySigner};
use common::FakeDaemon;
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
use zwallet::chains::{self, evm};
use zwallet::core::{
    error::ErrorCode,
    whitelist::{WHITELIST_FILE, Whitelist, WhitelistKey},
};
use zwallet::wallet::{self, CheckedList, KeyRow};

struct Fixture {
    dir: tempfile::TempDir,
    checked: CheckedList,
    key: KeyRow,
}
impl Fixture {
    fn new(allow: bool) -> Self {
        let dir = tempfile::Builder::new()
            .prefix("zwt-tools-")
            .tempdir_in("/tmp")
            .unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut list = Whitelist::empty(&evm::registry::WHITELIST_RULES);
        list.generation = 1;
        list.policy.allow_message_signing = allow;
        let seal = WhitelistKey::from_secret(&[7; 32]);
        let signed = seal.sign(&list);
        std::fs::write(
            dir.path().join(WHITELIST_FILE),
            signed.to_file_bytes().unwrap(),
        )
        .unwrap();
        let checked = wallet::check_list(dir.path(), &Ok(Some(seal.anchor_for(&signed)))).unwrap();
        let key = KeyRow {
            name: "main".into(),
            index: 0,
            address: Some(signer().address().to_checksum(None)),
            active: true,
        };
        Self { dir, checked, key }
    }
    fn review(&self, text: &str) -> zwallet::core::error::Result<wallet::MessageReview> {
        wallet::review_message(
            self.dir.path(),
            chains::find("ethereum").unwrap(),
            &self.checked,
            &self.key,
            text,
        )
    }
}
fn signer() -> PrivateKeySigner {
    format!("{:064x}", 1).parse().unwrap()
}
fn signed(message: &[u8]) -> Value {
    let digest = eip191_hash_message(message);
    let signature = signer().sign_hash_sync(&digest).unwrap();
    json!({"digest":format!("{digest:#x}"),"signature":format!("0x{}",alloy::hex::encode(signature.as_bytes()))})
}

#[tokio::test]
async fn reviewed_message_keeps_all_512_bytes_last_byte_and_spaces() {
    for text in [
        "a".repeat(512),
        format!("{}b", "a".repeat(511)),
        "  exact bytes ".into(),
    ] {
        let f = Fixture::new(true);
        let daemon = FakeDaemon::answering(&f.dir.path().join("d.sock"), signed(text.as_bytes()));
        let review = f.review(&text).unwrap();
        assert_eq!(review.bytes(), text.as_bytes());
        assert_eq!(review.rows().concat().replace(['[', ']'], ""), text);
        assert_eq!(review.rows().len(), text.len().div_ceil(76));
        if text.starts_with(' ') {
            assert_eq!(
                review.metadata(),
                "14 bytes; leading spaces 2, trailing spaces 1"
            );
        }
        let result = wallet::sign_message(review).await.unwrap();
        assert_eq!(result["message"], text);
        assert_eq!(
            result["digest"],
            format!("{:#x}", eip191_hash_message(text.as_bytes()))
        );
        let requests = daemon.received.lock().unwrap();
        assert_eq!(requests.len(), 1);
        let request: Value = serde_json::from_slice(&requests[0]).unwrap();
        assert_eq!(request["op"], "sign_message");
        assert_eq!(
            alloy::hex::decode(request["messageHex"].as_str().unwrap()).unwrap(),
            text.as_bytes()
        );
    }
}

#[test]
fn forbidden_or_oversized_paste_is_wholly_refused() {
    let f = Fixture::new(true);
    for text in [
        "a".repeat(513),
        "line\nline".into(),
        "tab\t".into(),
        "nul\0".into(),
        "del\u{7f}".into(),
        "한국어".into(),
        "█".into(),
    ] {
        let err = f.review(&text).unwrap_err();
        assert_eq!(err.code, ErrorCode::ValidationError);
        assert!(err.message.contains("zwallet sign-message"));
    }
    let all_printable: String = (0x20..=0x7e).map(char::from).collect();
    assert_eq!(
        f.review(&all_printable).unwrap().bytes(),
        all_printable.as_bytes()
    );
}

#[tokio::test]
async fn checked_policy_and_fingerprint_win_over_forged_daemon_claims() {
    let denied = Fixture::new(false);
    let daemon = FakeDaemon::answering(
        &denied.dir.path().join("d.sock"),
        json!({"allowMessageSigning":true}),
    );
    assert_eq!(
        denied.review("hi").unwrap_err().code,
        ErrorCode::WhitelistDenied
    );
    assert!(daemon.received.lock().unwrap().is_empty());
    let f = Fixture::new(true);
    let daemon = FakeDaemon::answering(&f.dir.path().join("d.sock"), signed(b"hi"));
    let review = f.review("hi").unwrap();
    std::fs::write(f.dir.path().join(WHITELIST_FILE), b"{}").unwrap();
    assert_eq!(f.review("hi").unwrap_err().code, ErrorCode::ValidationError);
    assert_eq!(
        wallet::sign_message(review).await.unwrap_err().code,
        ErrorCode::ValidationError
    );
    assert!(daemon.received.lock().unwrap().is_empty());
}

#[tokio::test]
async fn malformed_wrong_message_and_wrong_key_signatures_are_refused() {
    for mut reply in [
        json!({"signature":"0x00","digest":format!("{:#x}",eip191_hash_message(b"hello"))}),
        signed(b"different"),
        signed(b"hello"),
    ] {
        let mut f = Fixture::new(true);
        if reply == signed(b"hello") {
            f.key.address = Some(format!("0x{}", "11".repeat(20)));
        }
        reply["address"] = json!(f.key.address);
        let daemon = FakeDaemon::answering(&f.dir.path().join("d.sock"), reply);
        assert_eq!(
            wallet::sign_message(f.review("hello").unwrap())
                .await
                .unwrap_err()
                .code,
            ErrorCode::ValidationError
        );
        assert_eq!(daemon.received.lock().unwrap().len(), 1);
    }
}

use alloy::primitives::{U256, keccak256};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::net::UnixListener;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use zwallet::chains::{Net, SendJournal, evm::tx};
use zwallet::core::whitelist::Entry;
use zwallet::wallet::{InvokeData, InvokeInput, SendInvokeOutcome, TransferOptions};

fn string_abi(text: &str) -> Value {
    let encoded = alloy::dyn_abi::DynSolValue::String(text.into()).abi_encode();
    json!(format!("0x{}", alloy::hex::encode(encoded)))
}
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
    nonce: Arc<AtomicU64>,
    pending: Arc<AtomicBool>,
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
        let nonce = Arc::new(AtomicU64::new(7));
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
        let next_nonce = nonce.clone();
        let thread = std::thread::spawn(move || {
            while !end.load(Ordering::SeqCst) {
                if let Ok((stream, _)) = listener.accept() {
                    let (b, l, p, n) = (b.clone(), l.clone(), p.clone(), n.clone());
                    let funds = funds.clone();
                    let next_nonce = next_nonce.clone();
                    std::thread::spawn(move || {
                        serve(stream, move |m, args| {
                            let base = u128::from(b.load(Ordering::SeqCst));
                            match m {
                                "eth_chainId" => hex(evm::config(chain).unwrap().chain_id.into()),
                                "eth_getBalance" => hex(funds.load(Ordering::SeqCst).into()),
                                "eth_getTransactionCount" => {
                                    hex(if args[1] == "latest" && p.load(Ordering::SeqCst) {
                                        u128::from(next_nonce.load(Ordering::SeqCst) - 1)
                                    } else {
                                        u128::from(next_nonce.load(Ordering::SeqCst))
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
                                    let data = args[0]["input"]
                                        .as_str()
                                        .or_else(|| args[0]["data"].as_str())
                                        .unwrap();
                                    match data.get(..10).unwrap_or("") {
                                        "0x70a08231" => json!(format!("0x{:064x}", 1_234_567u64)),
                                        "0x313ce567" => json!(format!("0x{:064x}", 6)),
                                        "0x95d89b41" => {
                                            string_abi(&format!("\x1b{}", "M".repeat(40)))
                                        }
                                        "0x06fdde03" => string_abi(&"\x1b█".repeat(300)),
                                        _ => json!(format!("0x{:064x}", l.load(Ordering::SeqCst))),
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
            nonce,
            pending,
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

struct TxSigner {
    received: Arc<Mutex<Vec<Value>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl TxSigner {
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
impl Drop for TxSigner {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
    }
}

const ROUTER: &str = "0x2222222222222222222222222222222222222222";
impl Fixture {
    fn router(&mut self) {
        self.checked.list.entries.push(Entry::new_contract(
            "router",
            ROUTER,
            &["ethereum", "base"],
            &["0x414bf389"],
        ));
        self.checked.list.generation += 1;
        let seal = WhitelistKey::from_secret(&[7; 32]);
        let signed = seal.sign(&self.checked.list);
        std::fs::write(
            self.dir.path().join(WHITELIST_FILE),
            signed.to_file_bytes().unwrap(),
        )
        .unwrap();
        self.checked =
            wallet::check_list(self.dir.path(), &Ok(Some(seal.anchor_for(&signed)))).unwrap();
    }
    fn invoke_input(&self, node: &Node, raw: bool) -> InvokeInput {
        InvokeInput {
            key: self.key.clone(),
            contract: wallet::invoke_contracts(&self.checked, node.net.chain).remove(0),
            call: if raw {
                InvokeData::Raw(format!("0x414bf389{:064x}", 7))
            } else {
                InvokeData::Signature {
                    sig:"exactInputSingle((address,address,uint24,address,uint256,uint256,uint256,uint160))".into(),
                    args:vec![format!("({ROUTER},{ROUTER},3000,{ROUTER},1,2,3,0)")],
                }
            },
            value: "0.001".into(),
            options: TransferOptions::default(),
        }
    }
}

#[tokio::test]
async fn call_matches_cli_and_contract_text_is_inert_and_bounded() {
    let node = Node::new("ethereum");
    let f = Fixture::new(true);
    let input = wallet::CallInput {
        from: None,
        to: ROUTER.into(),
        sig: "balanceOf(address)(uint256)".into(),
        args: vec![f.key.address.clone().unwrap()],
    };
    let result = wallet::call(&node.net, &input).await.unwrap();
    assert_eq!(result.fields["result"], json!(["1234567"]));
    assert_eq!(result.rendered_result, "[\"1234567\"]");
    let cli = std::process::Command::new(env!("CARGO_BIN_EXE_zwallet"))
        .env_clear()
        .env("ZWALLET_HOME", f.dir.path())
        .args([
            "--json",
            "call",
            "--chain",
            "ethereum",
            "--rpc",
            &node.net.rpc_url,
            "--to",
            ROUTER,
            "--sig",
            &input.sig,
            &input.args[0],
        ])
        .output()
        .unwrap();
    assert!(
        cli.status.success(),
        "{}",
        String::from_utf8_lossy(&cli.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&cli.stdout).unwrap(),
        result.fields
    );
    let malicious = wallet::call(
        &node.net,
        &wallet::CallInput {
            sig: "name()(string)".into(),
            args: vec![],
            ..input
        },
    )
    .await
    .unwrap();
    assert_eq!(malicious.fields["result"][0], "\x1b█".repeat(300));
    assert!(malicious.rendered_result.contains("\\u{1b}\\u{2588}"));
    assert_eq!(malicious.rendered_result.len(), 512);
    assert_eq!(malicious.raw.len(), 512);
    assert!(
        malicious
            .rendered_result
            .bytes()
            .all(|b| (0x20..=0x7e).contains(&b))
    );
    assert!(malicious.rendered_result.len().div_ceil(78) <= 7);
}

#[tokio::test]
async fn arbitrary_token_balance_has_capped_symbol_and_exact_amount() {
    let node = Node::new("ethereum");
    let balance = wallet::token_balance(&node.net, &signer().address().to_checksum(None), ROUTER)
        .await
        .unwrap();
    assert_eq!(balance.raw, U256::from(1_234_567));
    assert_eq!(balance.decimals, 6);
    assert_eq!(balance.symbol.len(), 32);
    assert!(balance.symbol.starts_with("\\u{1b}"));
    assert!(balance.symbol.bytes().all(|b| (0x20..=0x7e).contains(&b)));
    assert_eq!(node.sent.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn signature_and_raw_invokes_sign_only_the_reviewed_contract_input_and_value() {
    for raw in [false, true] {
        let node = Node::new("ethereum");
        let mut f = Fixture::new(true);
        f.router();
        let (signer, _) = TxSigner::new(f.dir.path(), false);
        let mut input = f.invoke_input(&node, raw);
        let expected = input.call.encode().unwrap().0;
        assert_eq!(&expected[..4], &[0x41, 0x4b, 0xf3, 0x89]);
        let review = wallet::review_invoke(f.dir.path(), &node.net, &f.checked, input.clone())
            .await
            .unwrap();
        assert!(review.verdict().is_ok());
        assert_eq!(review.intent().to(), evm::parse_address(ROUTER).unwrap());
        assert_eq!(review.intent().input(), &expected);
        assert_eq!(
            review.intent().value(),
            U256::from(1_000_000_000_000_000u64)
        );
        assert_eq!(signer.count(), 0);
        // Edits to the form after review cannot affect the owned review.
        input.call = InvokeData::Raw("0xdeadbeef".into());
        input.value = "9999".into();
        let out = wallet::send_invoke(review, "router", None).await.unwrap();
        let SendInvokeOutcome::Sent(out) = out else {
            panic!("unchanged review should send")
        };
        assert_eq!(signer.count(), 1);
        assert_eq!(node.sent.load(Ordering::SeqCst), 1);
        assert!(out.fields.get("rawTx").is_none());
        let requests = signer.received.lock().unwrap();
        let bytes =
            alloy::hex::decode(requests[0]["tx"].as_str().unwrap().trim_start_matches("0x"))
                .unwrap();
        let decoded = tx::decode_unsigned(&bytes).unwrap();
        assert_eq!(decoded.to(), evm::parse_address(ROUTER).unwrap());
        assert_eq!(decoded.input(), &expected);
        assert_eq!(decoded.nonce(), 7);
        let journal = SendJournal::new(
            f.dir.path(),
            node.net.chain,
            f.key.address.as_deref().unwrap(),
        );
        let lines = std::fs::read_to_string(journal.display()).unwrap();
        let row: Value = serde_json::from_str(lines.lines().next().unwrap()).unwrap();
        assert_eq!(row["kind"], "invoke");
        assert_eq!(row["to"], ROUTER);
    }
}

#[tokio::test]
async fn invoke_rejects_unlisted_selector_changed_list_and_wrong_nickname_without_signing() {
    let node = Node::new("ethereum");
    let mut f = Fixture::new(true);
    f.router();
    let (signer, _) = TxSigner::new(f.dir.path(), false);
    let input = f.invoke_input(&node, true);
    let mut bad = input.clone();
    bad.call = InvokeData::Raw("0xdeadbeef".into());
    let error = wallet::review_invoke(f.dir.path(), &node.net, &f.checked, bad)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::WhitelistDenied);
    assert!(error.message.contains("0x414bf389"));
    let review = wallet::review_invoke(f.dir.path(), &node.net, &f.checked, input.clone())
        .await
        .unwrap();
    assert_eq!(
        wallet::send_invoke(review, "Router", None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::ValidationError
    );
    let review = wallet::review_invoke(f.dir.path(), &node.net, &f.checked, input.clone())
        .await
        .unwrap();
    std::fs::write(f.dir.path().join(WHITELIST_FILE), b"{}").unwrap();
    assert_eq!(
        wallet::send_invoke(review, "router", None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::ValidationError
    );
    assert_eq!(
        wallet::review_invoke(f.dir.path(), &node.net, &f.checked, input)
            .await
            .unwrap_err()
            .code,
        ErrorCode::ValidationError
    );
    assert_eq!(signer.count(), 0);
    assert_eq!(node.sent.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn invoke_fee_increase_and_funds_change_require_another_review() {
    for funds in [false, true] {
        let node = Node::new("ethereum");
        let mut f = Fixture::new(true);
        f.router();
        let (signer, _) = TxSigner::new(f.dir.path(), false);
        let review = wallet::review_invoke(
            f.dir.path(),
            &node.net,
            &f.checked,
            f.invoke_input(&node, true),
        )
        .await
        .unwrap();
        if funds {
            node.balance.store(0, Ordering::SeqCst);
        } else {
            node.base.store((2 * GWEI) as u64, Ordering::SeqCst);
        }
        assert!(matches!(
            wallet::send_invoke(review, "router", None).await.unwrap(),
            SendInvokeOutcome::ReviewChanged(_)
        ));
        assert_eq!(signer.count(), 0);
        assert_eq!(node.sent.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn invoke_same_key_but_altered_signed_intent_never_broadcasts() {
    let node = Node::new("ethereum");
    let mut f = Fixture::new(true);
    f.router();
    let (signer, _) = TxSigner::new(f.dir.path(), true);
    let review = wallet::review_invoke(
        f.dir.path(),
        &node.net,
        &f.checked,
        f.invoke_input(&node, true),
    )
    .await
    .unwrap();
    assert_eq!(
        wallet::send_invoke(review, "router", None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::ValidationError
    );
    assert_eq!(signer.count(), 1);
    assert_eq!(node.sent.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn new_invoke_never_bypasses_pending_or_replacement_intent_checks() {
    let node = Node::new("ethereum");
    let mut f = Fixture::new(true);
    f.router();
    let (signer, _) = TxSigner::new(f.dir.path(), false);
    let mut input = f.invoke_input(&node, true);
    input.options.nonce = Some(7);
    assert_eq!(
        wallet::review_invoke(f.dir.path(), &node.net, &f.checked, input)
            .await
            .unwrap_err()
            .code,
        ErrorCode::ValidationError
    );
    node.pending.store(true, Ordering::SeqCst);
    let review = wallet::review_invoke(
        f.dir.path(),
        &node.net,
        &f.checked,
        f.invoke_input(&node, true),
    )
    .await
    .unwrap();
    assert_eq!(
        wallet::send_invoke(review, "router", None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::ValidationError
    );
    assert_eq!(signer.count(), 0);
    assert_eq!(node.sent.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn nonce_and_op_data_fee_changes_do_not_silently_alter_an_invoke() {
    for chain in ["ethereum", "base"] {
        let node = Node::new(chain);
        let mut f = Fixture::new(true);
        f.router();
        let (signer, _) = TxSigner::new(f.dir.path(), false);
        let review = wallet::review_invoke(
            f.dir.path(),
            &node.net,
            &f.checked,
            f.invoke_input(&node, true),
        )
        .await
        .unwrap();
        if chain == "base" {
            node.l1.store(20_000, Ordering::SeqCst);
        } else {
            node.nonce.store(8, Ordering::SeqCst);
        }
        assert!(matches!(
            wallet::send_invoke(review, "router", None).await.unwrap(),
            SendInvokeOutcome::ReviewChanged(_)
        ));
        assert_eq!(signer.count(), 0);
        assert_eq!(node.sent.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn checked_fee_cap_refuses_before_any_signature_and_input_key_is_held() {
    let node = Node::new("ethereum");
    let mut f = Fixture::new(true);
    f.checked
        .list
        .policy
        .max_fee_wei
        .insert("ethereum".into(), "1".into());
    f.router();
    let (signer, _) = TxSigner::new(f.dir.path(), false);
    let review = wallet::review_invoke(
        f.dir.path(),
        &node.net,
        &f.checked,
        f.invoke_input(&node, true),
    )
    .await
    .unwrap();
    assert_eq!(
        review.verdict().as_ref().unwrap_err().code,
        ErrorCode::FeeCapExceeded
    );
    assert_eq!(
        wallet::send_invoke(review, "router", None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::FeeCapExceeded
    );
    assert_eq!(signer.count(), 0);
    assert_eq!(node.sent.load(Ordering::SeqCst), 0);
    let mut f = Fixture::new(true);
    let daemon = FakeDaemon::answering(&f.dir.path().join("d.sock"), signed(b"unchanged"));
    let review = f.review("unchanged").unwrap();
    f.key.index = 99;
    f.key.name = "changed".into();
    f.key.address = None;
    let output = wallet::sign_message(review).await.unwrap();
    assert_eq!(output["key"], "main");
    let request: Value = serde_json::from_slice(&daemon.received.lock().unwrap()[0]).unwrap();
    assert_eq!(request["index"], 0);
}

use zeroize::Zeroizing;
use zwallet::tui::app::tools::{Stage as ToolStage, ToolResult};
use zwallet::tui::{
    self,
    app::{App, Effect, Key, Msg, UserText},
};
use zwallet::wallet::{Keys, Opened, StatusView, Trust};

fn opened_app(f: &Fixture, node: &Node) -> App {
    let mut app = App::new(&node.net, Duration::from_secs(3600));
    app.configure(f.dir.path(), false);
    let effects = app.update(Msg::Checked(Ok((
        Opened {
            keys: vec![f.key.clone()],
            active: "main".into(),
            warning: None,
            anchor: Ok(None),
            salted: false,
        },
        Ok(f.checked.clone()),
    ))));
    for effect in effects {
        match effect {
            Effect::LoadKeys(g) => {
                app.update(Msg::Keys(
                    g,
                    Ok(Keys {
                        active: "main".into(),
                        rows: vec![f.key.clone()],
                        trust: Trust::Session,
                    }),
                ));
            }
            Effect::LoadStatus(g) => {
                app.update(Msg::Status(
                    g,
                    Ok(StatusView {
                        configured: true,
                        locked: false,
                        daemon_pids: vec![1],
                        other_daemon_pids: vec![],
                        serving: None,
                        active: Some("main".into()),
                        address: Some((f.key.address.clone(), Trust::Session)),
                    }),
                ));
            }
            _ => {}
        }
    }
    app.update(Msg::Key(Key::Char('6')));
    app
}
fn paste(app: &mut App, text: &str) {
    app.update(Msg::Paste(UserText(Zeroizing::new(text.into()))));
}
async fn tool_jobs(app: &mut App, effects: Vec<Effect>, home: &std::path::Path) {
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    for effect in effects {
        if let Effect::Tools(id) = effect {
            let job = app.take_tool_job(id).expect("one owned job");
            assert!(
                app.take_tool_job(id).is_none(),
                "duplicate effect must not sign twice"
            );
            job.run(id, home, &sender).await;
        }
    }
    while let Ok(message) = receiver.try_recv() {
        app.update(message);
    }
}
fn screen(app: &App) -> String {
    use ratatui::buffer::Cell;
    use zwallet::tui::{style, text::Source};
    let mut terminal = ratatui::Terminal::new(zwallet::tui::term::FilteredBackend::new(
        ratatui::backend::TestBackend::new(80, 24),
    ))
    .unwrap();
    let mut placed = Vec::new();
    terminal.draw(|f| placed = tui::ui::draw(f, app)).unwrap();
    assert_eq!(
        terminal.backend().replaced(),
        0,
        "outside text must never reach the terminal filter raw"
    );
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
        assert_eq!(cell.symbol().len(), 1, "not a single ASCII cell");
        assert!(cell.symbol().bytes().all(|b| (0x20..=0x7e).contains(&b)));
        assert!(styles.contains(&cell.style()), "style: {:?}", cell.style());
    }
    for p in placed {
        // These contract budgets are independent of the production Source::budget values.
        let max = match p.outside.source() {
            Source::WhitelistName | Source::KeyName | Source::RpcHost => 32,
            Source::Quantity => 78,
            Source::TransactionHash => 66,
            Source::Detail => 72,
            Source::Home | Source::Expiry => 64,
            Source::Address => 42,
            Source::Input | Source::Error | Source::Warning | Source::ListRow => 512,
        };
        assert!(
            p.outside.text().len() <= max,
            "source: {:?}",
            p.outside.source()
        );
        assert!(p.outside.text().bytes().all(|b| (0x20..=0x7e).contains(&b)));
        assert!(p.area.right() <= 80 && p.area.bottom() <= 24);
        assert!(
            p.area.height == 1
                || (p.outside.source() == Source::Error && (1..=7).contains(&p.area.height)),
            "outside block: {:?}",
            p.area
        );
        if p.area.height == 1 {
            assert_eq!(usize::from(p.area.width), p.outside.text().len());
        }
        let shown = (p.area.y..p.area.bottom())
            .flat_map(|y| (p.area.x..p.area.right()).map(move |x| buffer[(x, y)].symbol()))
            .collect::<String>();
        assert!(
            shown.starts_with(p.outside.text()),
            "outside placement: {:?}",
            p.area
        );
        assert!(shown[p.outside.text().len()..].bytes().all(|b| b == b' '));
    }
    (0..24)
        .map(|y| (0..80).map(|x| buffer[(x, y)].symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn tools_ui_renders_all_message_bytes_and_edit_drops_authority_to_sign() {
    for text in [
        "a".repeat(512),
        format!("{}b", "a".repeat(511)),
        "  exact bytes ".into(),
    ] {
        let f = Fixture::new(true);
        let node = Node::new("ethereum");
        let daemon = FakeDaemon::answering(&f.dir.path().join("d.sock"), signed(text.as_bytes()));
        let mut app = opened_app(&f, &node);
        app.update(Msg::Key(Key::Char('m')));
        paste(&mut app, &text);
        let effects = app.update(Msg::Key(Key::Enter));
        tool_jobs(&mut app, effects, f.dir.path()).await;
        assert_eq!(app.tools().stage, ToolStage::MessageReview);
        let drawn = screen(&app);
        let lines: Vec<_> = drawn
            .lines()
            .filter(|line| line.starts_with("|["))
            .collect();
        assert_eq!(lines.len(), text.len().div_ceil(76), "{drawn}");
        let reconstructed = lines
            .iter()
            .map(|line| line.strip_prefix("|[").unwrap().split(']').next().unwrap())
            .collect::<String>();
        assert_eq!(reconstructed, text);
        if text.starts_with(' ') {
            assert!(drawn.contains("leading spaces 2, trailing spaces 1"));
        }
        app.update(Msg::Key(Key::Char('e')));
        assert!(app.tools().message.is_none());
        assert!(daemon.received.lock().unwrap().is_empty());
        let effects = app.update(Msg::Key(Key::Enter)); // Builds a new review; never a SignMessage.
        tool_jobs(&mut app, effects, f.dir.path()).await;
        assert!(daemon.received.lock().unwrap().is_empty());
        let effects = app.update(Msg::Key(Key::Enter));
        tool_jobs(&mut app, effects, f.dir.path()).await;
        assert_eq!(app.tools().stage, ToolStage::MessageConfirm);
        paste(&mut app, "sign");
        let effects = app.update(Msg::Key(Key::Enter));
        tool_jobs(&mut app, effects, f.dir.path()).await;
        assert!(matches!(app.tools().result, Some(ToolResult::Signature(_))));
        let request: Value = serde_json::from_slice(&daemon.received.lock().unwrap()[0]).unwrap();
        assert_eq!(
            alloy::hex::decode(request["messageHex"].as_str().unwrap()).unwrap(),
            text.as_bytes()
        );
    }
}

#[tokio::test]
async fn tools_ui_whole_paste_refusal_and_tampered_index_send_no_requests() {
    let f = Fixture::new(true);
    let node = Node::new("ethereum");
    let daemon = FakeDaemon::answering(&f.dir.path().join("d.sock"), signed(b"safe"));
    let mut app = opened_app(&f, &node);
    app.update(Msg::Key(Key::Char('m')));
    for text in ["a".repeat(513), "ok\nnotok".into(), "ok█".into()] {
        paste(&mut app, &text);
        assert_eq!(app.tools().field.as_ref().unwrap().value(), "");
        assert!(screen(&app).contains("zwallet sign-message"));
    }
    paste(&mut app, "safe");
    let effects = app.update(Msg::Key(Key::Enter));
    tool_jobs(&mut app, effects, f.dir.path()).await;
    let effects = app.update(Msg::Key(Key::Char('r'))); // consumed by review; no global refresh.
    assert!(effects.is_empty());
    // Drive the genuine periodic key read generation and change only public metadata's index.
    app.update(Msg::Key(Key::Esc));
    let effects = app.update(Msg::Key(Key::Char('1')));
    let mut changed = f.key.clone();
    changed.index = 99;
    for effect in effects {
        if let Effect::LoadKeys(g) = effect {
            app.update(Msg::Keys(
                g,
                Ok(Keys {
                    active: "main".into(),
                    rows: vec![changed.clone()],
                    trust: Trust::Session,
                }),
            ));
        }
    }
    app.update(Msg::Key(Key::Char('6')));
    app.update(Msg::Key(Key::Char('m')));
    assert_eq!(app.tools().stage, ToolStage::Error);
    assert!(screen(&app).contains("key index changed"));
    assert!(daemon.received.lock().unwrap().is_empty());
}

#[tokio::test]
async fn tools_ui_calls_and_tokens_use_shared_operations_without_a_send_action() {
    let f = Fixture::new(true);
    let node = Node::new("ethereum");
    let mut app = opened_app(&f, &node);
    app.update(Msg::Key(Key::Char('C')));
    paste(&mut app, ROUTER);
    app.update(Msg::Key(Key::Enter));
    paste(&mut app, "name()(string)");
    let effects = app.update(Msg::Key(Key::Enter));
    tool_jobs(&mut app, effects, f.dir.path()).await;
    assert_eq!(app.tools().stage, ToolStage::Result);
    let drawn = screen(&app);
    assert!(drawn.contains("\\u{1b}\\u{2588}"));
    assert!(!drawn.contains('█'));
    assert!(
        drawn
            .lines()
            .filter(|l| l.starts_with("|[\"") || l.contains("\\u{"))
            .count()
            <= 7
    );
    app.update(Msg::Key(Key::Esc));
    app.update(Msg::Key(Key::Char('t')));
    paste(&mut app, ROUTER);
    let effects = app.update(Msg::Key(Key::Enter));
    tool_jobs(&mut app, effects, f.dir.path()).await;
    assert!(screen(&app).contains("Read only."));
    assert_eq!(node.sent.load(Ordering::SeqCst), 0);
    assert!(matches!(app.tools().result, Some(ToolResult::Token { .. })));
}

#[tokio::test]
async fn tools_ui_raw_invoke_uses_nickname_and_shared_journaled_send_status() {
    let mut f = Fixture::new(true);
    f.router();
    let node = Node::new("ethereum");
    let (signer, _) = TxSigner::new(f.dir.path(), false);
    let mut app = opened_app(&f, &node);
    let effects = app.update(Msg::Key(Key::Char('d')));
    tool_jobs(&mut app, effects, f.dir.path()).await;
    assert_eq!(app.tools().stage, ToolStage::Contracts);
    app.update(Msg::Key(Key::Enter));
    paste(&mut app, "0x414bf389");
    app.update(Msg::Key(Key::Enter));
    let effects = app.update(Msg::Key(Key::Enter));
    tool_jobs(&mut app, effects, f.dir.path()).await;
    assert_eq!(app.tools().stage, ToolStage::InvokeReview);
    assert!(screen(&app).contains("selector 0x414bf389"));
    app.update(Msg::Key(Key::Enter));
    paste(&mut app, "wrong");
    assert!(app.update(Msg::Key(Key::Enter)).is_empty());
    assert_eq!(signer.count(), 0);
    for _ in 0..5 {
        app.update(Msg::Key(Key::Backspace));
    }
    paste(&mut app, "router");
    let effects = app.update(Msg::Key(Key::Enter));
    tool_jobs(&mut app, effects, f.dir.path()).await;
    assert_eq!(signer.count(), 1);
    assert_eq!(node.sent.load(Ordering::SeqCst), 1);
    assert_eq!(app.tab(), zwallet::tui::actions::Tab::Send);
    assert!(
        app.transfer().status.is_some(),
        "fsynced hash must survive while receipt pending"
    );
    let hash = app.transfer().status.as_ref().unwrap().hash.clone();
    let retained = &app.tools().retained[&hash];
    assert_eq!(
        retained.intent.decoded().input().as_ref(),
        &[0x41, 0x4b, 0xf3, 0x89]
    );
    assert!(!retained.uncertain);
    app.update(Msg::Key(Key::Char('6')));
    assert!(app.tools().retained.contains_key(&hash));
}

#[tokio::test]
async fn invoke_review_scroll_exposes_every_calldata_byte_and_distinguishes_arguments() {
    let mut observed = Vec::new();
    for (raw, length) in [(true, 0), (true, 512), (true, 2011), (false, 0)] {
        for last in [7, 8] {
            let mut f = Fixture::new(true);
            f.router();
            let node = Node::new("ethereum");
            let (signer, _) = TxSigner::new(f.dir.path(), false);
            let mut app = opened_app(&f, &node);
            let work = app.update(Msg::Key(Key::Char(if raw { 'd' } else { 'i' })));
            tool_jobs(&mut app, work, f.dir.path()).await;
            app.update(Msg::Key(Key::Enter));
            let data = if raw {
                let data = format!("0x414bf389{}{:064x}", "11".repeat(length), last);
                paste(&mut app, &data);
                app.update(Msg::Key(Key::Enter));
                data
            } else {
                let sig = "exactInputSingle((address,address,uint24,address,uint256,uint256,uint256,uint160))";
                let args = vec![format!("({ROUTER},{ROUTER},3000,{ROUTER},1,2,3,{last})")];
                paste(&mut app, sig);
                app.update(Msg::Key(Key::Enter));
                paste(&mut app, &args[0]);
                app.update(Msg::Key(Key::Enter));
                let expected = wallet::InvokeData::Signature {
                    sig: sig.into(),
                    args,
                }
                .encode()
                .unwrap()
                .0;
                assert_eq!(expected.last(), Some(&last));
                format!("0x{}", alloy::hex::encode(expected))
            };
            let work = app.update(Msg::Key(Key::Enter));
            tool_jobs(&mut app, work, f.dir.path()).await;
            assert_eq!(app.tools().stage, ToolStage::InvokeReview);
            let held = app
                .tools()
                .invoke
                .as_ref()
                .unwrap()
                .intent()
                .input()
                .clone();
            assert_eq!(format!("0x{}", alloy::hex::encode(&held)), data);
            let initial = screen(&app);
            let mut all_rows = Vec::new();
            for step in 0..100 {
                let drawn = screen(&app); // Also asserts the independent H7 cell/style budgets.
                let rows: Vec<_> = drawn
                    .lines()
                    .skip(2)
                    .take(16)
                    .map(|row| row[1..79].trim().to_owned())
                    .collect();
                if let Some(end) = rows.iter().position(|r| r == "End of reviewed calldata") {
                    all_rows.extend_from_slice(&rows[..=end]);
                    break;
                }
                all_rows.push(rows[0].clone());
                assert!(app.update(Msg::Key(Key::Char('j'))).is_empty());
                assert!(step < 99, "complete calldata must remain reachable");
            }
            let start = all_rows
                .iter()
                .position(|r| r.starts_with("Complete calldata ("))
                .unwrap();
            let end = all_rows
                .iter()
                .position(|r| r == "End of reviewed calldata")
                .unwrap();
            let shown = all_rows[start + 1..end].concat();
            assert_eq!(shown, data, "no omitted argument or trailing byte");
            observed.push(shown);
            assert_eq!(signer.count(), 0, "scrolling never signs");
            app.update(Msg::Key(Key::Home));
            assert_eq!(screen(&app), initial);
            for option in ['p', 'd'] {
                app.update(Msg::Key(Key::Char('j')));
                assert!(app.tools().review_scroll > 0);
                let work = app.update(Msg::Key(Key::Char(option)));
                tool_jobs(&mut app, work, f.dir.path()).await;
                assert_eq!(app.tools().review_scroll, 0);
            }
            app.update(Msg::Key(Key::Char('j')));
            app.update(Msg::Key(Key::Char('g')));
            paste(&mut app, "100");
            let work = app.update(Msg::Key(Key::Enter));
            tool_jobs(&mut app, work, f.dir.path()).await;
            assert_eq!(app.tools().review_scroll, 0);
            app.update(Msg::Key(Key::Char('j')));
            let scroll = app.tools().review_scroll;
            app.update(Msg::Key(Key::Enter));
            assert_eq!(app.tools().stage, ToolStage::Confirm);
            paste(&mut app, "router");
            app.update(Msg::Key(Key::Home));
            assert_eq!(app.tools().field.as_ref().unwrap().cursor(), 0);
            app.update(Msg::Key(Key::End));
            assert_eq!(app.tools().field.as_ref().unwrap().cursor(), 6);
            assert_eq!(
                app.tools().review_scroll,
                scroll,
                "confirmation field owns Home/End"
            );
            assert!(screen(&app).contains("Type the contract nickname exactly:"));
            assert_eq!(app.tools().invoke.as_ref().unwrap().intent().input(), &held);
            assert_eq!(signer.count(), 0);
        }
    }
    for pair in observed.chunks_exact(2) {
        assert_ne!(pair[0], pair[1]);
    }
}

#[tokio::test]
async fn unlisted_raw_selector_stays_in_form_and_names_allowed_selectors() {
    let mut f = Fixture::new(true);
    f.router();
    let node = Node::new("ethereum");
    let daemon = FakeDaemon::answering(
        &f.dir.path().join("d.sock"),
        json!({"allowMessageSigning":true}),
    );
    let mut app = opened_app(&f, &node);
    let effects = app.update(Msg::Key(Key::Char('d')));
    tool_jobs(&mut app, effects, f.dir.path()).await;
    app.update(Msg::Key(Key::Enter));
    paste(&mut app, "0xdeadbeef");
    app.update(Msg::Key(Key::Enter));
    let effects = app.update(Msg::Key(Key::Enter));
    tool_jobs(&mut app, effects, f.dir.path()).await;
    assert_eq!(app.tools().stage, ToolStage::Field);
    assert_eq!(app.tools().field.as_ref().unwrap().value(), "0xdeadbeef");
    assert!(screen(&app).contains("0x414bf389"));
    assert!(daemon.received.lock().unwrap().is_empty());
}

#[tokio::test]
async fn metadata_index_change_while_reviewed_is_refused_before_sign_message() {
    let f = Fixture::new(true);
    let node = Node::new("ethereum");
    let daemon = FakeDaemon::answering(&f.dir.path().join("d.sock"), signed(b"held"));
    let mut app = opened_app(&f, &node);
    let generation = app
        .update(Msg::Key(Key::Char('6')))
        .into_iter()
        .find_map(|e| match e {
            Effect::LoadKeys(g) => Some(g),
            _ => None,
        })
        .unwrap();
    app.update(Msg::Key(Key::Char('m')));
    paste(&mut app, "held");
    let effects = app.update(Msg::Key(Key::Enter));
    tool_jobs(&mut app, effects, f.dir.path()).await;
    assert_eq!(app.tools().stage, ToolStage::MessageReview);
    let mut changed = f.key.clone();
    changed.index = 99;
    app.update(Msg::Keys(
        generation,
        Ok(Keys {
            active: "main".into(),
            rows: vec![changed],
            trust: Trust::Session,
        }),
    ));
    assert!(app.update(Msg::Key(Key::Enter)).is_empty());
    assert_eq!(app.tools().stage, ToolStage::Error);
    assert!(daemon.received.lock().unwrap().is_empty());
}

#[tokio::test]
async fn quitting_and_locking_during_message_signature_wait_for_the_owned_job() {
    let f = Fixture::new(true);
    let node = Node::new("ethereum");
    let daemon = FakeDaemon::answering(&f.dir.path().join("d.sock"), signed(b"held"));
    let mut app = opened_app(&f, &node);
    app.update(Msg::Key(Key::Char('m')));
    paste(&mut app, "held");
    let effects = app.update(Msg::Key(Key::Enter));
    tool_jobs(&mut app, effects, f.dir.path()).await;
    app.update(Msg::Key(Key::Enter));
    paste(&mut app, "sign");
    let effects = app.update(Msg::Key(Key::Enter));
    let id = effects
        .into_iter()
        .find_map(|e| match e {
            Effect::Tools(id) => Some(id),
            _ => None,
        })
        .unwrap();
    app.update(Msg::Key(Key::Char('L')));
    app.update(Msg::Key(Key::Interrupt));
    assert!(app.quit_pending());
    assert!(!app.quitting());
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    app.take_tool_job(id)
        .unwrap()
        .run(id, f.dir.path(), &sender)
        .await;
    let effects = app.update(receiver.recv().await.unwrap());
    assert_eq!(effects, vec![Effect::LockAll]);
    assert!(!app.quitting());
    app.update(Msg::Locked(Ok(vec![])));
    assert!(app.quitting());
    assert_eq!(daemon.received.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn reviewed_message_requires_exact_typed_sign_before_any_request() {
    let f = Fixture::new(true);
    let node = Node::new("ethereum");
    let daemon = FakeDaemon::answering(&f.dir.path().join("d.sock"), signed(b"held"));
    let mut app = opened_app(&f, &node);
    app.update(Msg::Key(Key::Char('m')));
    paste(&mut app, "held");
    let effects = app.update(Msg::Key(Key::Enter));
    tool_jobs(&mut app, effects, f.dir.path()).await;
    assert!(
        app.update(Msg::Key(Key::Enter)).is_empty(),
        "review Enter must open confirmation, not create a signing job"
    );
    assert_eq!(app.tools().field.as_ref().unwrap().value(), "");
    paste(&mut app, "wrong");
    assert!(app.update(Msg::Key(Key::Enter)).is_empty());
    assert!(daemon.received.lock().unwrap().is_empty());
    for _ in 0..5 {
        app.update(Msg::Key(Key::Backspace));
    }
    paste(&mut app, "sign");
    let effects = app.update(Msg::Key(Key::Enter));
    tool_jobs(&mut app, effects, f.dir.path()).await;
    assert_eq!(daemon.received.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn lock_requested_during_invoke_refresh_is_completed_when_review_changes() {
    let mut f = Fixture::new(true);
    f.router();
    let node = Node::new("ethereum");
    let (signer, _) = TxSigner::new(f.dir.path(), false);
    let mut app = opened_app(&f, &node);
    let effects = app.update(Msg::Key(Key::Char('d')));
    tool_jobs(&mut app, effects, f.dir.path()).await;
    app.update(Msg::Key(Key::Enter));
    paste(&mut app, "0x414bf389");
    app.update(Msg::Key(Key::Enter));
    let effects = app.update(Msg::Key(Key::Enter));
    tool_jobs(&mut app, effects, f.dir.path()).await;
    app.update(Msg::Key(Key::Char('j')));
    assert!(app.tools().review_scroll > 0);
    app.update(Msg::Key(Key::Enter));
    paste(&mut app, "router");
    let id = app
        .update(Msg::Key(Key::Enter))
        .into_iter()
        .find_map(|e| match e {
            Effect::Tools(id) => Some(id),
            _ => None,
        })
        .unwrap();
    app.update(Msg::Key(Key::Char('L')));
    assert!(app.locking());
    node.base.store((2 * GWEI) as u64, Ordering::SeqCst);
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    app.take_tool_job(id)
        .unwrap()
        .run(id, f.dir.path(), &sender)
        .await;
    assert_eq!(
        app.update(receiver.recv().await.unwrap()),
        vec![Effect::LockAll]
    );
    assert_eq!(
        app.tools().review_scroll,
        0,
        "changed review returns to its beginning"
    );
    app.update(Msg::Locked(Ok(vec![])));
    assert!(!app.locking());
    app.update(Msg::Key(Key::Char('q')));
    assert!(app.quitting());
    assert_eq!(signer.count(), 0);
}

#[tokio::test]
async fn locking_blocks_new_message_reviews_before_the_daemon_has_stopped() {
    let f = Fixture::new(true);
    let node = Node::new("ethereum");
    let daemon = FakeDaemon::answering(&f.dir.path().join("d.sock"), signed(b"held"));
    let mut app = opened_app(&f, &node);
    assert_eq!(app.update(Msg::Key(Key::Char('L'))), vec![Effect::LockAll]);
    assert!(app.locking());
    app.update(Msg::Key(Key::Char('m')));
    assert_eq!(app.tools().stage, ToolStage::Error);
    assert!(daemon.received.lock().unwrap().is_empty());
}
