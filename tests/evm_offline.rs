//! EVM send-path tests through the built binary against a local mock JSON-RPC node. No network:
//! the mock listens on 127.0.0.1 and answers only what each test scripts. Every wallet is a temp
//! wallet whose daemon is locked (or killed) when the test ends.

mod common;

use alloy::consensus::{Transaction, TxEnvelope};
use alloy::eips::eip2718::Decodable2718;
use alloy::primitives::keccak256;
use common::Home;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A second hardhat account, used as a plain recipient.
const OTHER: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
const USDC: &str = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48";
const GWEI: u128 = 1_000_000_000;
/// The mock account's next free nonce (`pending`). [`healthy`] has nothing pending, so its mined
/// count (`latest`) is the same.
const PENDING_NONCE: u64 = 7;
/// [`pending_node`]'s mined count: nonces 5 and 6 are pending there.
const LATEST_NONCE: u64 = 5;

enum Reply {
    Result(Value),
    Error(&'static str),
    /// A non-200 HTTP status with this plain-text body (an API gateway's error page).
    Http(u16, &'static str),
    /// Read the request, then close the connection without answering.
    Drop,
}

type Handler = dyn Fn(&str, &Value) -> Reply + Send + Sync;

struct MockRpc {
    url: String,
    calls: Arc<Mutex<Vec<(String, Value)>>>,
}

impl MockRpc {
    fn start(handler: impl Fn(&str, &Value) -> Reply + Send + Sync + 'static) -> Self {
        Self::start_at("", handler)
    }

    /// `path` is appended to the URL (the mock ignores it); used to plant a fake API key.
    fn start_at(
        path: &str,
        handler: impl Fn(&str, &Value) -> Reply + Send + Sync + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}{path}", listener.local_addr().unwrap());
        let calls = Arc::new(Mutex::new(Vec::new()));
        let handler: Arc<Handler> = Arc::new(handler);
        let log = calls.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (handler, log) = (handler.clone(), log.clone());
                std::thread::spawn(move || serve(stream, &*handler, &log));
            }
        });
        Self { url, calls }
    }

    fn calls(&self, method: &str) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, _)| m == method)
            .map(|(_, p)| p.clone())
            .collect()
    }

    /// The raw signed transactions the CLI handed to `eth_sendRawTransaction`.
    fn sent(&self) -> Vec<String> {
        self.calls("eth_sendRawTransaction")
            .iter()
            .map(|p| p[0].as_str().unwrap().to_string())
            .collect()
    }
}

/// One keep-alive connection: HTTP/1.1 requests with a Content-Length body, until EOF.
fn serve(stream: TcpStream, handler: &Handler, log: &Mutex<Vec<(String, Value)>>) {
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);
    loop {
        let mut len = 0usize;
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                return;
            }
            let l = line.trim_end();
            if l.is_empty() {
                break;
            }
            if let Some((k, v)) = l.split_once(':')
                && k.eq_ignore_ascii_case("content-length")
            {
                len = v.trim().parse().unwrap();
            }
        }
        let mut body = vec![0u8; len];
        reader.read_exact(&mut body).unwrap();
        let req: Value = serde_json::from_slice(&body).unwrap();
        let method = req["method"].as_str().unwrap().to_string();
        let params = req["params"].clone();
        log.lock().unwrap().push((method.clone(), params.clone()));
        let (status, text) = match handler(&method, &params) {
            Reply::Drop => return,
            Reply::Result(r) => (
                "200 OK".to_string(),
                json!({ "jsonrpc": "2.0", "id": req["id"], "result": r }).to_string(),
            ),
            Reply::Error(msg) => (
                "200 OK".to_string(),
                json!({
                    "jsonrpc": "2.0", "id": req["id"], "error": { "code": -32000, "message": msg },
                })
                .to_string(),
            ),
            Reply::Http(code, body) => (format!("{code} Error"), body.to_string()),
        };
        let head = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            text.len()
        );
        if writer.write_all(head.as_bytes()).is_err() || writer.write_all(text.as_bytes()).is_err()
        {
            return;
        }
    }
}

fn hex(n: u128) -> Value {
    json!(format!("{n:#x}"))
}

fn tx_hash_of(raw: &str) -> String {
    let bytes = alloy::hex::decode(raw.trim_start_matches("0x")).unwrap();
    format!("{:#x}", keccak256(bytes))
}

/// The nonce a raw signed transaction consumes.
fn nonce_of(raw: &str) -> u64 {
    let bytes = alloy::hex::decode(raw.trim_start_matches("0x")).unwrap();
    TxEnvelope::decode_2718(&mut bytes.as_slice())
        .unwrap()
        .nonce()
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

/// A healthy, funded EIP-1559 node: base fee 1 gwei, tips 0.1 gwei, 1 ETH balance, nothing
/// pending.
fn healthy(method: &str, params: &Value) -> Reply {
    fee_node(method, params, Some(GWEI), GWEI / 10, GWEI + GWEI / 10)
}

/// The hash the mock gives the transaction pending at nonce `n` (5 or 6 on [`pending_node`]).
fn pending_tx(n: u64) -> String {
    format!("0x{}", format!("{n:02x}").repeat(32))
}

/// `base` with nonces 5 and 6 pending from `me`: `latest` is 5, and the transactions there
/// ([`pending_tx`]) are known, unmined, at 1 wei fees (so a replacement's bump applies to the
/// estimate).
fn pending_node(
    me: &str,
    base: impl Fn(&str, &Value) -> Reply + Send + Sync + 'static,
) -> impl Fn(&str, &Value) -> Reply + Send + Sync + 'static {
    let me = me.to_lowercase();
    move |m, p| match m {
        "eth_getTransactionCount" if p[1] == "latest" => Reply::Result(hex(LATEST_NONCE.into())),
        "eth_getTransactionByHash" => {
            match (LATEST_NONCE..PENDING_NONCE).find(|n| p[0] == pending_tx(*n).as_str()) {
                Some(n) => Reply::Result(json!({
                    "hash": p[0], "from": me, "nonce": hex(n.into()), "blockNumber": null,
                    "maxFeePerGas": "0x1", "maxPriorityFeePerGas": "0x1",
                })),
                None => base(m, p),
            }
        }
        _ => base(m, p),
    }
}

/// The send journal of `me` on Ethereum in `home`.
fn journal_path(home: &Home, me: &str) -> std::path::PathBuf {
    home.path()
        .join(format!("sends-1-{}.jsonl", me.to_lowercase()))
}

/// A node with the given base fee (`None` = legacy chain), feeHistory tip, and eth_gasPrice.
fn fee_node(method: &str, params: &Value, base: Option<u128>, tip: u128, gas_price: u128) -> Reply {
    Reply::Result(match method {
        "eth_chainId" => json!("0x1"),
        "eth_getBalance" => hex(10u128.pow(18)),
        "eth_getTransactionCount" => hex(PENDING_NONCE.into()),
        "eth_getBlockByNumber" => block(base),
        "eth_feeHistory" => json!({
            "oldestBlock": "0xf6",
            "baseFeePerGas": vec![hex(base.unwrap_or(0)); 11],
            "gasUsedRatio": vec![0.5; 10],
            "reward": vec![vec![hex(tip)]; 10],
        }),
        "eth_gasPrice" => hex(gas_price),
        "eth_estimateGas" => json!("0x5208"),
        "eth_sendRawTransaction" => json!(tx_hash_of(params[0].as_str().unwrap())),
        "eth_getTransactionByHash" | "eth_getTransactionReceipt" => Value::Null,
        other => panic!("mock: unexpected {other}"),
    })
}

fn unlocked() -> (Home, String) {
    let home = Home::new();
    let me = home.create("t", &["--unlock", "10m"])["address"]
        .as_str()
        .unwrap()
        .to_string();
    (home, me)
}

fn transfer<'a>(rpc: &'a str, extra: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec![
        "transfer", "--chain", "ethereum", "--to", OTHER, "--amount", "0.01", "--rpc", rpc,
        "--json",
    ];
    args.extend_from_slice(extra);
    args
}

#[test]
fn dry_run_prints_no_broadcastable_raw_tx() {
    let (home, me) = unlocked();
    let rpc = MockRpc::start(healthy);
    let v = home.run(&transfer(&rpc.url, &["--dry-run"])).ok();
    assert_eq!(v["dryRun"], true);
    assert!(v.get("rawTx").is_none(), "dry-run printed rawTx: {v:#}");
    assert!(
        !v.to_string().contains("0x02f8"),
        "signed bytes leaked: {v:#}"
    );
    assert_eq!(v["recoveredFrom"], me.as_str());
    assert!(v["txHash"].as_str().unwrap().starts_with("0x"));
    assert_eq!(v["signerDecoded"]["action"]["kind"], "native-transfer");
    assert_eq!(v["pendingNonces"], json!([]));
    // Nothing broadcast, nothing journaled.
    assert!(!journal_path(&home, &me).exists());
    let inv = home
        .run(&[
            "invoke",
            "--chain",
            "ethereum",
            "--to",
            OTHER,
            "--data",
            "0xd0e30db0",
            "--rpc",
            &rpc.url,
            "--dry-run",
            "--json",
        ])
        .ok();
    assert!(
        inv.get("rawTx").is_none(),
        "invoke dry-run printed rawTx: {inv:#}"
    );
    assert!(rpc.sent().is_empty(), "a dry-run broadcast");
}

#[test]
fn transfer_refuses_zero_or_token_recipient_unless_forced() {
    let (home, _) = unlocked();
    // A closed port: the refusal must happen before any RPC.
    let closed = "http://127.0.0.1:1";
    let zero = "0x0000000000000000000000000000000000000000";
    let base = |to: &'static str| {
        vec![
            "transfer", "--chain", "ethereum", "--to", to, "--amount", "1", "--rpc", closed,
            "--json",
        ]
    };
    let msg = home.run(&base(zero)).err("INVALID_ADDRESS");
    assert!(msg.contains("--force"), "{msg}");
    let mut token_to_itself = base(USDC);
    token_to_itself.extend(["--token", USDC]);
    let msg = home.run(&token_to_itself).err("INVALID_ADDRESS");
    assert!(msg.contains("--force"), "{msg}");
    let lower = USDC.to_lowercase();
    let mut lowercase = base(USDC);
    lowercase.extend(["--token", &lower]);
    home.run(&lowercase).err("INVALID_ADDRESS");

    // --force skips only this check; the next step is the (closed) RPC.
    let mut forced = base(zero);
    forced.push("--force");
    home.run(&forced).err("RPC_ERROR");
    token_to_itself.push("--force");
    home.run(&token_to_itself).err("RPC_ERROR");
}

#[test]
fn lost_send_reply_is_broadcast_uncertain_with_hash_nonce_from() {
    let (home, me) = unlocked();
    let rpc = MockRpc::start(|m, p| match m {
        "eth_sendRawTransaction" => Reply::Drop,
        _ => healthy(m, p),
    });
    let r = home.run(&transfer(&rpc.url, &[]));
    let msg = r.err("BROADCAST_UNCERTAIN");
    let v = r.json();
    println!("{v:#}");
    let sent = rpc.sent();
    assert_eq!(sent.len(), 1, "the CLI must send exactly once");
    let hash = tx_hash_of(&sent[0]);
    assert_eq!(v["txHash"], hash.as_str(), "{v:#}");
    assert_eq!(v["nonce"], PENDING_NONCE);
    assert_eq!(v["from"], me.as_str());
    // A retry must name what it replaces, so the hint carries the hash.
    assert!(
        msg.contains(&format!("retry with --nonce 7 --replace {hash}")),
        "{msg}"
    );
    assert_eq!(
        v["journal"],
        journal_path(&home, &me).display().to_string().as_str()
    );
    // It looked for the transaction by the hash it computed, more than once.
    let polls = rpc.calls("eth_getTransactionByHash");
    assert!(polls.len() >= 2, "{polls:?}");
    assert!(polls.iter().all(|p| p[0] == hash.as_str()));
}

#[test]
fn send_error_for_a_tx_the_node_knows_is_success() {
    let (home, _) = unlocked();
    let known = Arc::new(Mutex::new(None::<String>));
    let k = known.clone();
    let rpc = MockRpc::start(move |m, p| match m {
        "eth_sendRawTransaction" => {
            *k.lock().unwrap() = Some(tx_hash_of(p[0].as_str().unwrap()));
            Reply::Error("already known")
        }
        "eth_getTransactionByHash" => match k.lock().unwrap().clone() {
            Some(h) if p[0] == h.as_str() => Reply::Result(json!({ "hash": h, "nonce": "0x7" })),
            _ => Reply::Result(Value::Null),
        },
        _ => healthy(m, p),
    });
    let v = home.run(&transfer(&rpc.url, &[])).ok();
    let hash = known.lock().unwrap().clone().unwrap();
    assert_eq!(v["txHash"], hash.as_str(), "{v:#}");
    assert_eq!(v["nonce"], PENDING_NONCE);
    assert!(
        v["broadcastWarning"]
            .as_str()
            .unwrap()
            .contains("already known")
    );
    assert_eq!(rpc.sent().len(), 1);
}

#[test]
fn node_rejection_is_rpc_error_not_broadcast_uncertain() {
    let (home, me) = unlocked();
    let answer = Arc::new(Mutex::new(""));
    let a = answer.clone();
    let rpc = MockRpc::start(move |m, p| match m {
        "eth_sendRawTransaction" => Reply::Error(*a.lock().unwrap()),
        _ => healthy(m, p),
    });
    // geth's wording (most clients share it), matched case-insensitively. "nonce too low" and
    // "replacement transaction underpriced" are not here: they may mean the transaction itself is
    // already in, so they are looked up first (see `nonce_taken_*`).
    let rejections = [
        "INSUFFICIENT FUNDS for gas * price + value",
        "intrinsic gas too low",
        "exceeds block gas limit",
        "max fee per gas less than block base fee: maxFeePerGas: 1, baseFee: 2",
        // Below the pool's minimum price (not a replacement).
        "transaction underpriced: gas tip cap 1, minimum needed 25000000",
    ];
    // Each answer is a separate case, not a retry: --allow-duplicate.
    let args = transfer(&rpc.url, &["--allow-duplicate"]);
    for (i, why) in rejections.into_iter().enumerate() {
        *answer.lock().unwrap() = why;
        let r = home.run(&args);
        let msg = r.err("RPC_ERROR");
        assert!(
            msg.contains("the node rejected the transaction; nothing was broadcast"),
            "{msg}"
        );
        assert!(msg.contains(why), "{msg}");
        // A retry must reuse the nonce, so that it can never become a second send.
        assert!(msg.contains("--nonce 7"), "{msg}");
        // Underpriced: the way to pay more at the same nonce.
        assert_eq!(
            msg.contains("retry with --nonce 7 --bump"),
            why.starts_with("transaction underpriced"),
            "{msg}"
        );
        let v = r.json();
        let sent = rpc.sent();
        assert_eq!(sent.len(), i + 1, "the CLI must send exactly once");
        assert_eq!(v["txHash"], tx_hash_of(&sent[i]).as_str(), "{v:#}");
        assert_eq!(v["nonce"], PENDING_NONCE);
        assert_eq!(v["from"], me.as_str());
    }
    // These refuse the transaction itself: no lookup by hash.
    assert!(rpc.calls("eth_getTransactionByHash").is_empty());
    // Any other error text still asks for the transaction and stays uncertain.
    *answer.lock().unwrap() = "internal error";
    home.run(&args).err("BROADCAST_UNCERTAIN");
    assert!(!rpc.calls("eth_getTransactionByHash").is_empty());
}

/// A node whose `eth_sendRawTransaction` fails with `error`. With `known` it reports the
/// transaction it was sent by that transaction's own hash (as a mined one), otherwise nothing.
fn send_error_node(error: &'static str, known: bool) -> MockRpc {
    let sent = Arc::new(Mutex::new(None::<String>));
    let s = sent.clone();
    MockRpc::start(move |m, p| match m {
        "eth_sendRawTransaction" => {
            *s.lock().unwrap() = Some(tx_hash_of(p[0].as_str().unwrap()));
            Reply::Error(error)
        }
        "eth_getTransactionByHash" => match s.lock().unwrap().clone() {
            Some(h) if known && p[0] == h.as_str() => {
                Reply::Result(json!({ "hash": h, "nonce": "0x7", "blockNumber": "0x99" }))
            }
            _ => Reply::Result(Value::Null),
        },
        _ => healthy(m, p),
    })
}

#[test]
fn nonce_too_low_for_the_same_tx_is_success() {
    // The identical transaction is already in (resent after a lost reply): the node refuses the
    // copy, and the lookup by the computed hash finds the original.
    let (home, _) = unlocked();
    for why in [
        "nonce too low: address 0xf39F, tx: 7 state: 8",
        "replacement transaction underpriced",
    ] {
        let rpc = send_error_node(why, true);
        // Two separate cases in one home, not a retry.
        let v = home.run(&transfer(&rpc.url, &["--allow-duplicate"])).ok();
        let sent = rpc.sent();
        assert_eq!(sent.len(), 1, "the CLI must send exactly once");
        assert_eq!(v["txHash"], tx_hash_of(&sent[0]).as_str(), "{v:#}");
        assert_eq!(v["nonce"], PENDING_NONCE);
        let warning = v["broadcastWarning"].as_str().unwrap();
        assert!(warning.contains(why), "{warning}");
        assert!(warning.contains("so it was sent"), "{warning}");
    }
}

#[test]
fn nonce_taken_by_another_tx_says_look_up_before_sending_again() {
    let (home, me) = unlocked();
    // Without --nonce: the pending nonce went to another transaction meanwhile.
    let rpc = send_error_node("nonce too low: address 0xf39F, tx: 7 state: 8", false);
    let r = home.run(&transfer(&rpc.url, &[]));
    let msg = r.err("RPC_ERROR");
    let v = r.json();
    println!("{v:#}");
    assert!(
        msg.contains("nonce 7 is taken by another transaction from this key, pending or mined"),
        "{msg}"
    );
    assert!(
        msg.contains(
            "If you were retrying an earlier send, look up that send's txHash before sending \
             again"
        ),
        "{msg}"
    );
    assert!(!msg.contains("next nonce"), "{msg}");
    assert!(!msg.contains("rejected"), "{msg}");
    let hash = tx_hash_of(&rpc.sent()[0]);
    // The refused transaction will never be found: it is not `txHash`.
    assert!(v.get("txHash").is_none(), "{v:#}");
    assert_eq!(v["refusedTxHash"], hash.as_str(), "{v:#}");
    assert_eq!(v["nonce"], PENDING_NONCE);
    assert_eq!(v["from"], me.as_str());
    let polls = rpc.calls("eth_getTransactionByHash");
    assert!(!polls.is_empty() && polls.iter().all(|p| p[0] == hash.as_str()));

    // With --nonce (a retry): the earlier send, recorded in the journal, may be the one that
    // took the nonce.
    let earlier = format!("0x{}", "e1".repeat(32));
    let line = json!({ "ts": "2026-10-20T00:00:00Z", "chain": "ethereum", "from": me,
                       "nonce": 7, "txHash": earlier, "to": OTHER, "value": "0.01 ETH",
                       "kind": "transfer" });
    for why in ["nonce too low", "replacement transaction underpriced"] {
        std::fs::write(journal_path(&home, &me), format!("{line}\n")).unwrap();
        let rpc = send_error_node(why, false);
        let r = home.run(&transfer(&rpc.url, &["--nonce", "7"]));
        let msg = r.err("BROADCAST_UNCERTAIN");
        println!("{msg}");
        let refused = tx_hash_of(&rpc.sent()[0]);
        for part in [
            "nonce 7 is taken by another transaction from this key, pending or mined",
            "This retry was refused and will not be found",
            "look up the txHash of your earlier send at nonce 7 from its own output (or the send \
             journal below). The send journal ",
            "While it may be pending or mined, do not send this payment without --nonce; to \
             replace it use --nonce 7 --replace <earlier txHash>",
        ] {
            assert!(msg.contains(part), "{part}: {msg}");
        }
        assert!(!msg.contains("next nonce"), "{msg}");
        // The journal's earlier send is named; the refused retry is not offered as it.
        assert!(msg.contains(&earlier), "{msg}");
        assert!(!msg.contains(&format!("{refused} (")), "{msg}");
        let v = r.json();
        assert_eq!(v["nonce"], 7, "{v:#}");
        assert!(v.get("txHash").is_none(), "{v:#}");
        assert_eq!(v["refusedTxHash"], refused.as_str(), "{v:#}");
        assert_eq!(
            v["journal"],
            journal_path(&home, &me).display().to_string().as_str()
        );
        if why.starts_with("replacement") {
            assert!(
                msg.contains("--replace <its txHash> and a larger --bump"),
                "{msg}"
            );
        }
    }
}

#[test]
fn replacing_a_pending_nonce_pays_a_quarter_more() {
    let (home, me) = unlocked();
    let wei = |v: &Value, k: &str| -> u128 {
        v[k].as_str()
            .unwrap_or_else(|| panic!("{k}: {v:#}"))
            .parse()
            .unwrap()
    };
    let rpc = MockRpc::start(pending_node(&me, healthy));
    let six = pending_tx(6);
    // Nonce 7 is the next free one (no replacement), 6 is pending (replaced, and named).
    let fresh = home
        .run(&transfer(&rpc.url, &["--nonce", "7", "--dry-run"]))
        .ok();
    let repl = home
        .run(&transfer(
            &rpc.url,
            &["--nonce", "6", "--replace", &six, "--dry-run"],
        ))
        .ok();
    assert_eq!(fresh["replacesPending"], false);
    assert_eq!(fresh["feeBump"], Value::Null);
    assert_eq!(repl["replacesPending"], true);
    assert_eq!(repl["feeBump"], "1.25x");
    for field in ["maxFeePerGas", "maxPriorityFeePerGas"] {
        let (est, bumped) = (wei(&fresh, field), wei(&repl, field));
        assert!(
            est > 0 && bumped * 4 >= est * 5,
            "{field}: {est} -> {bumped}"
        );
    }
    // The signed bytes carry the bumped price, not only the printed fields.
    let (est, bumped) = (
        wei(&fresh["signerDecoded"], "maxGasPrice"),
        wei(&repl["signerDecoded"], "maxGasPrice"),
    );
    assert!(
        bumped * 4 >= est * 5,
        "signed maxGasPrice: {est} -> {bumped}"
    );

    // Legacy chain: gasPrice 3 gwei becomes at least 3.75 gwei, and --max-fee-gwei is checked
    // against the bumped price.
    let legacy = MockRpc::start(pending_node(&me, |m, p| fee_node(m, p, None, 0, 3 * GWEI)));
    let v = home
        .run(&transfer(
            &legacy.url,
            &["--nonce", "6", "--replace", &six, "--dry-run"],
        ))
        .ok();
    assert!(wei(&v, "gasPrice") * 4 >= 3 * GWEI * 5, "{v:#}");
    assert_eq!(v["feeBump"], "1.25x");
    home.run(&transfer(
        &legacy.url,
        &["--nonce", "7", "--dry-run", "--max-fee-gwei", "3.5"],
    ))
    .ok();
    let msg = home
        .run(&transfer(
            &legacy.url,
            &[
                "--nonce",
                "6",
                "--replace",
                &six,
                "--dry-run",
                "--max-fee-gwei",
                "3.5",
            ],
        ))
        .err("VALIDATION_ERROR");
    assert!(msg.contains("--max-fee-gwei"), "{msg}");
    assert!(rpc.sent().is_empty() && legacy.sent().is_empty());
}

#[test]
fn replacing_a_pending_nonce_must_name_the_transaction_it_replaces() {
    let (home, me) = unlocked();
    let rpc = MockRpc::start(pending_node(&me, healthy));
    // An earlier send at nonce 6 in this home's journal.
    let six = pending_tx(6);
    let line = json!({ "ts": "2026-10-20T00:00:00Z", "chain": "ethereum", "from": me,
                       "nonce": 6, "txHash": six, "to": OTHER, "value": "0.01 ETH",
                       "kind": "transfer" });
    std::fs::write(journal_path(&home, &me), format!("{line}\n")).unwrap();
    for args in [
        vec!["--nonce", "6", "--dry-run"],
        vec!["--nonce", "6", "--bump", "2"],
    ] {
        let r = home.run(&transfer(&rpc.url, &args));
        let msg = r.err("VALIDATION_ERROR");
        assert!(
            msg.starts_with(
                "nonce 6 belongs to a pending transaction; pass --replace <its txHash> to \
                 replace it (see the send journal); nothing was signed. The send journal "
            ),
            "{msg}"
        );
        // The journal names it.
        assert!(msg.contains(&six), "{msg}");
        let v = r.json();
        assert_eq!(v["nonce"], 6);
        assert_eq!(v["pendingSends"][0]["txHash"], six.as_str(), "{v:#}");
    }
    // Nonce 5 has no journal entry: sent from elsewhere, so the explorer is named instead.
    let msg = home
        .run(&transfer(&rpc.url, &["--nonce", "5", "--dry-run"]))
        .err("VALIDATION_ERROR");
    assert!(msg.contains("has no entry at nonce 5"), "{msg}");
    assert!(
        msg.contains(&format!("https://etherscan.io/address/{me}")),
        "{msg}"
    );
    // The next free nonce replaces nothing, so it needs no --replace.
    let v = home
        .run(&transfer(&rpc.url, &["--nonce", "7", "--dry-run"]))
        .ok();
    assert_eq!(v["replacesPending"], false);
    assert!(rpc.sent().is_empty());
}

#[test]
fn bump_and_replace_outbid_the_pending_transaction() {
    let (home, me) = unlocked();
    let wei = |v: &Value, k: &str| -> u128 {
        v[k].as_str()
            .unwrap_or_else(|| panic!("{k}: {v:#}"))
            .parse()
            .unwrap()
    };
    let rpc = MockRpc::start(pending_node(&me, healthy));
    let est = home
        .run(&transfer(&rpc.url, &["--nonce", "7", "--dry-run"]))
        .ok();
    assert_eq!(est["feeBump"], Value::Null);
    // --bump 2: twice the estimate, signed, whether it replaces nonce 6 or takes the free nonce
    // 7 (an explicit bump applies even where nothing is pending).
    let six = pending_tx(6);
    for args in [
        vec![
            "--nonce",
            "6",
            "--replace",
            &six,
            "--bump",
            "2",
            "--dry-run",
        ],
        vec!["--nonce", "7", "--bump", "2", "--dry-run"],
    ] {
        let v = home.run(&transfer(&rpc.url, &args)).ok();
        assert_eq!(v["feeBump"], "2x", "{args:?}");
        for field in ["maxFeePerGas", "maxPriorityFeePerGas"] {
            assert_eq!(wei(&v, field), 2 * wei(&est, field), "{field} {args:?}");
        }
        assert_eq!(
            wei(&v["signerDecoded"], "maxGasPrice"),
            2 * wei(&est, "maxFeePerGas")
        );
    }
    // Out of range, malformed, or without --nonce: refused before any RPC.
    for bad in [
        vec!["--nonce", "6", "--bump", "1.05"],
        vec!["--nonce", "6", "--bump", "3.5"],
        vec!["--nonce", "6", "--bump", "x"],
        vec!["--bump", "2"],
        vec!["--replace", "0x01"],
    ] {
        let mut args = bad.clone();
        args.push("--dry-run");
        home.run(&transfer("http://127.0.0.1:1", &args))
            .err("VALIDATION_ERROR");
    }
    // The ceiling still applies after the bump: 3 gwei legacy x 2.5 = 7.5 gwei > 6 gwei.
    let legacy = MockRpc::start(|m, p| fee_node(m, p, None, 0, 3 * GWEI));
    let msg = home
        .run(&transfer(
            &legacy.url,
            &["--nonce", "7", "--bump", "2.5", "--dry-run"],
        ))
        .err("VALIDATION_ERROR");
    assert!(msg.contains("the fees include the 2.5x bump"), "{msg}");

    // --replace: the original's fees (4 gwei cap, 1 gwei tip) beat the estimate, so the
    // replacement pays bump x the original.
    let original = format!("0x{}", "cd".repeat(32));
    let (o, from) = (original.clone(), me.clone());
    let inner = pending_node(&me, healthy);
    let node = MockRpc::start(move |m, p| match m {
        "eth_getTransactionByHash" if p[0] == o.as_str() => Reply::Result(json!({
            "hash": o, "from": from.to_lowercase(), "nonce": "0x6", "blockNumber": null,
            "maxFeePerGas": hex(4 * GWEI), "maxPriorityFeePerGas": hex(GWEI),
        })),
        _ => inner(m, p),
    });
    let v = home
        .run(&transfer(
            &node.url,
            &["--nonce", "6", "--replace", &original, "--dry-run"],
        ))
        .ok();
    assert_eq!(v["replacesTx"], original.as_str());
    assert_eq!(v["feeBump"], "1.25x");
    assert_eq!(wei(&v, "maxFeePerGas"), 5 * GWEI);
    assert_eq!(wei(&v, "maxPriorityFeePerGas"), GWEI + GWEI / 4);
    // The original must be this key's transaction at that nonce, and known to the node.
    let msg = home
        .run(&transfer(
            &node.url,
            &["--nonce", "5", "--replace", &original, "--dry-run"],
        ))
        .err("VALIDATION_ERROR");
    assert!(msg.contains("nonce 6"), "{msg}");
    // A hash the node does not know: at a nonce it holds another pending transaction for, the
    // hash is the wrong one; at the free nonce, the slot is free and a retry without --replace
    // is allowed (so the two answers never send the agent back and forth).
    let unknown = format!("0x{}", "ef".repeat(32));
    let msg = home
        .run(&transfer(
            &node.url,
            &["--nonce", "6", "--replace", &unknown, "--dry-run"],
        ))
        .err("RPC_ERROR");
    assert!(
        msg.contains("but it holds another pending transaction from this key at nonce 6"),
        "{msg}"
    );
    assert!(msg.contains("pass that to --replace"), "{msg}");
    let msg = home
        .run(&transfer(
            &node.url,
            &["--nonce", "7", "--replace", &unknown, "--dry-run"],
        ))
        .err("RPC_ERROR");
    assert!(
        msg.contains("retry with --nonce 7 and no --replace"),
        "{msg}"
    );
    home.run(&transfer(&node.url, &["--nonce", "7", "--dry-run"]))
        .ok();
    let inner = pending_node(&me, healthy);
    let stranger = MockRpc::start(move |m, p| match m {
        "eth_getTransactionByHash" => Reply::Result(json!({
            "hash": p[0], "from": OTHER, "nonce": "0x6", "blockNumber": null, "gasPrice": hex(GWEI),
        })),
        _ => inner(m, p),
    });
    let msg = home
        .run(&transfer(
            &stranger.url,
            &["--nonce", "6", "--replace", &original, "--dry-run"],
        ))
        .err("VALIDATION_ERROR");
    assert!(msg.contains(OTHER), "{msg}");
    assert!(rpc.sent().is_empty() && node.sent().is_empty() && legacy.sent().is_empty());
}

#[test]
fn failure_before_the_send_is_rpc_error_and_says_nothing_was_sent() {
    let (home, me) = unlocked();
    // `prepare` connects once (eth_chainId); the broadcast's own connect is the second call.
    let n = Arc::new(AtomicUsize::new(0));
    let c = n.clone();
    let rpc = MockRpc::start(move |m, p| match m {
        "eth_chainId" if c.fetch_add(1, Ordering::SeqCst) >= 1 => Reply::Error("node is down"),
        _ => healthy(m, p),
    });
    let r = home.run(&transfer(&rpc.url, &[]));
    let msg = r.err("RPC_ERROR");
    assert!(msg.contains("nothing was broadcast"), "{msg}");
    // It was signed already: a retry must reuse the nonce.
    assert!(msg.contains("--nonce 7"), "{msg}");
    assert!(!msg.contains("so a retry is safe"), "{msg}");
    assert_eq!(r.json()["nonce"], PENDING_NONCE);
    assert!(rpc.sent().is_empty());
    // The journal line is written right before the send request, which never went out.
    assert!(!journal_path(&home, &me).exists());
    assert!(!r.stderr.contains("broadcasting"), "{}", r.stderr);
}

#[test]
fn explicit_nonce_is_signed_and_must_be_replaceable_or_next() {
    let (home, me) = unlocked();
    let rpc = MockRpc::start(pending_node(&me, healthy));
    let (five, six) = (pending_tx(5), pending_tx(6));
    let v = home
        .run(&transfer(
            &rpc.url,
            &["--nonce", "6", "--replace", &six, "--dry-run"],
        ))
        .ok();
    assert_eq!(v["nonce"], 6);
    assert_eq!(v["signerDecoded"]["nonce"], 6);
    assert_eq!(v["replacesPending"], true);
    let next = home
        .run(&transfer(&rpc.url, &["--nonce", "7", "--dry-run"]))
        .ok();
    assert_eq!(next["signerDecoded"]["nonce"], 7);
    assert_eq!(next["replacesPending"], false);
    let lo = home
        .run(&transfer(&rpc.url, &["--nonce", "4", "--dry-run"]))
        .err("VALIDATION_ERROR");
    assert!(lo.contains("nonce 4 is already mined"), "{lo}");
    assert!(
        lo.contains("that send (or its replacement) went through"),
        "{lo}"
    );
    assert!(
        lo.contains("look up its txHash before sending anything new"),
        "{lo}"
    );
    // Never a pointer to another nonce: that is how a retry becomes a second send.
    assert!(!lo.contains("next free nonce"), "{lo}");
    let hi = home
        .run(&transfer(&rpc.url, &["--nonce", "8", "--dry-run"]))
        .err("VALIDATION_ERROR");
    assert!(hi.contains("gap"), "{hi}");
    assert!(!hi.contains("next free nonce is"), "{hi}");
    let inv = home
        .run(&[
            "invoke",
            "--chain",
            "ethereum",
            "--to",
            OTHER,
            "--data",
            "0x",
            "--nonce",
            "5",
            "--replace",
            &five,
            "--rpc",
            &rpc.url,
            "--dry-run",
            "--json",
        ])
        .ok();
    assert_eq!(inv["signerDecoded"]["nonce"], 5);
    assert!(rpc.sent().is_empty());
}

#[test]
fn rpc_returning_another_hash_is_reconciled_by_the_computed_hash() {
    let (home, me) = unlocked();
    let bogus = format!("0x{}", "ab".repeat(32));
    // Not found by its own hash: BROADCAST_UNCERTAIN, with both hashes.
    let b = bogus.clone();
    let rpc = MockRpc::start(move |m, p| match m {
        "eth_sendRawTransaction" => Reply::Result(json!(b)),
        _ => healthy(m, p),
    });
    let r = home.run(&transfer(&rpc.url, &[]));
    let msg = r.err("BROADCAST_UNCERTAIN");
    let v = r.json();
    println!("{v:#}");
    let hash = tx_hash_of(&rpc.sent()[0]);
    assert_eq!(v["txHash"], hash.as_str(), "{v:#}");
    assert_eq!(v["rpcTxHash"], bogus.as_str());
    assert_eq!(v["nonce"], PENDING_NONCE);
    assert_eq!(v["from"], me.as_str());
    assert!(
        msg.contains(&format!("--nonce 7 --replace {hash}")),
        "{msg}"
    );
    let polls = rpc.calls("eth_getTransactionByHash");
    assert!(!polls.is_empty() && polls.iter().all(|p| p[0] == hash.as_str()));

    // Found by its own hash: sent, with the odd answer noted.
    let sent = Arc::new(Mutex::new(None::<String>));
    let (s, b) = (sent.clone(), bogus.clone());
    let rpc = MockRpc::start(move |m, p| match m {
        "eth_sendRawTransaction" => {
            *s.lock().unwrap() = Some(tx_hash_of(p[0].as_str().unwrap()));
            Reply::Result(json!(b))
        }
        "eth_getTransactionByHash" => match s.lock().unwrap().clone() {
            Some(h) if p[0] == h.as_str() => Reply::Result(json!({ "hash": h })),
            _ => Reply::Result(Value::Null),
        },
        _ => healthy(m, p),
    });
    // A separate case in the same home, not a retry.
    let v = home.run(&transfer(&rpc.url, &["--allow-duplicate"])).ok();
    assert_eq!(v["txHash"], tx_hash_of(&rpc.sent()[0]).as_str(), "{v:#}");
    assert_eq!(v["rpcTxHash"], bogus.as_str(), "{v:#}");
    assert!(
        v["broadcastWarning"]
            .as_str()
            .unwrap()
            .contains("not the transaction's own hash"),
        "{v:#}"
    );
}

#[test]
fn a_send_while_another_from_the_key_is_in_flight_fails_fast_and_signs_nothing() {
    let (home, me) = unlocked();
    // A node whose nonces count what it accepted (mined at once), and which takes a while to
    // accept. Waiting for the first send would let a second attempt at the same payment sign
    // the next nonce and land as well; it must fail at once instead.
    let accepted = Arc::new(AtomicUsize::new(0));
    let a = accepted.clone();
    let rpc = MockRpc::start(move |m, p| match m {
        "eth_getTransactionCount" => Reply::Result(hex(
            u128::from(PENDING_NONCE) + a.load(Ordering::SeqCst) as u128
        )),
        "eth_sendRawTransaction" => {
            std::thread::sleep(Duration::from_secs(4));
            a.fetch_add(1, Ordering::SeqCst);
            Reply::Result(json!(tx_hash_of(p[0].as_str().unwrap())))
        }
        _ => healthy(m, p),
    });
    let args = transfer(&rpc.url, &[]);
    let lock_name = format!("nonce-1-{}.lock", me.to_lowercase());
    std::thread::scope(|s| {
        let first = s.spawn(|| home.run(&args));
        // Once the node has the first raw transaction, the first send holds the lock until the
        // node answers.
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while rpc.sent().is_empty() {
            assert!(
                std::time::Instant::now() < deadline,
                "the first send never arrived"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let calls_before = rpc.calls.lock().unwrap().len();
        let msg = home.run(&args).err("VALIDATION_ERROR");
        // It did not wait: the node is still holding the first send.
        assert_eq!(accepted.load(Ordering::SeqCst), 0, "the second send waited");
        assert!(
            msg.starts_with("another send from this key on ethereum is in progress (lock "),
            "{msg}"
        );
        let named = format!("{lock_name}); nothing was signed or broadcast. ");
        for part in [
            named.as_str(),
            "If it is an earlier attempt at this same payment, wait for it to finish and look up \
             its result before sending again; for a different payment, retry after it finishes",
        ] {
            assert!(msg.contains(part), "{msg}");
        }
        // Not one RPC call: it stopped before reading the nonce.
        assert_eq!(rpc.calls.lock().unwrap().len(), calls_before);
        assert_eq!(first.join().unwrap().ok()["nonce"], PENDING_NONCE);
    });
    assert_eq!(rpc.sent().len(), 1);
    // Once the first send is done, the lock is free for the next payment (the same recipient
    // and amount, so it must say it is a second payment).
    let mut next = args.clone();
    next.push("--allow-duplicate");
    assert_eq!(home.run(&next).ok()["nonce"], PENDING_NONCE + 1);
    let nonces: Vec<u64> = rpc.sent().iter().map(|raw| nonce_of(raw)).collect();
    assert_eq!(nonces, vec![PENDING_NONCE, PENDING_NONCE + 1]);
    // The per-(chain, account) lock file in the home.
    let lock = home.path().join(&lock_name);
    assert!(lock.exists(), "{}", lock.display());
}

#[test]
fn receipt_trouble_after_the_send_is_exit_0_with_nonce() {
    let (home, _) = unlocked();
    let rpc = MockRpc::start(|m, p| match m {
        "eth_getTransactionReceipt" => Reply::Error("receipts unavailable"),
        _ => healthy(m, p),
    });
    let v = home.run(&transfer(&rpc.url, &["--wait"])).ok();
    assert_eq!(v["status"], "unknown");
    assert_eq!(v["nonce"], PENDING_NONCE);
    let hash = tx_hash_of(&rpc.sent()[0]);
    assert_eq!(v["txHash"], hash.as_str());
    let hint = v["hint"].as_str().unwrap();
    assert!(
        hint.ends_with(&format!(
            "to speed up or cancel use --nonce 7 --replace {hash}; a run without --nonce is a \
             second payment"
        )),
        "{hint}"
    );
}

#[test]
fn the_journal_line_is_on_disk_before_the_node_sees_the_raw_tx() {
    let (home, me) = unlocked();
    let journal = journal_path(&home, &me);
    // What the journal held at the moment the node received the raw transaction.
    let seen = Arc::new(Mutex::new(None::<String>));
    let (s, j) = (seen.clone(), journal.clone());
    let rpc = MockRpc::start(move |m, p| match m {
        "eth_sendRawTransaction" => {
            *s.lock().unwrap() = Some(std::fs::read_to_string(&j).unwrap_or_default());
            healthy(m, p)
        }
        _ => healthy(m, p),
    });
    let r = home.run(&transfer(&rpc.url, &[]));
    let v = r.ok();
    let hash = tx_hash_of(&rpc.sent()[0]);
    let at_send = seen.lock().unwrap().clone().unwrap();
    assert_eq!(at_send.lines().count(), 1, "{at_send}");
    let line: Value = serde_json::from_str(at_send.lines().next().unwrap()).unwrap();
    for (k, want) in [
        ("chain", json!("ethereum")),
        ("from", json!(me)),
        ("nonce", json!(PENDING_NONCE)),
        ("txHash", json!(hash)),
        ("to", json!(OTHER)),
        ("value", json!("0.01 ETH")),
        ("kind", json!("transfer")),
    ] {
        assert_eq!(line[k], want, "{k}: {line}");
    }
    assert!(line["ts"].as_str().unwrap().ends_with('Z'), "{line}");
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(&journal).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    // The same fields, and what to do if the command is cut off, on stderr.
    assert!(
        r.stderr.contains(&format!(
            "broadcasting {hash} (nonce 7 on ethereum); if this command does not finish, look \
             this hash up before sending again; a retry must pass --nonce 7 --replace {hash}"
        )),
        "{}",
        r.stderr
    );
    assert!(r.stderr.contains(&line.to_string()), "{}", r.stderr);
    assert_eq!(r.stderr.lines().count(), 1, "{}", r.stderr);
    assert_eq!(v["journal"], journal.display().to_string().as_str());
    // An invoke and a token transfer are journaled under their own kind.
    let inv = home
        .run(&[
            "invoke",
            "--chain",
            "ethereum",
            "--to",
            OTHER,
            "--data",
            "0xd0e30db0",
            "--value",
            "0.5",
            "--rpc",
            &rpc.url,
            "--json",
        ])
        .ok();
    let text = std::fs::read_to_string(&journal).unwrap();
    let last: Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
    assert_eq!(last["kind"], "invoke");
    assert_eq!(last["value"], "0.5 ETH");
    assert_eq!(last["txHash"], inv["txHash"]);
}

#[test]
fn a_fresh_send_while_one_is_pending_is_refused_unless_allow_pending() {
    let (home, me) = unlocked();
    // Accepted transactions stay pending: `latest` stays at 7.
    let accepted = Arc::new(AtomicUsize::new(0));
    let a = accepted.clone();
    let rpc = MockRpc::start(move |m, p| match m {
        "eth_getTransactionCount" if p[1] == "pending" => {
            Reply::Result(hex(
                u128::from(PENDING_NONCE) + a.load(Ordering::SeqCst) as u128
            ))
        }
        "eth_sendRawTransaction" => {
            a.fetch_add(1, Ordering::SeqCst);
            healthy(m, p)
        }
        _ => healthy(m, p),
    });
    let args = transfer(&rpc.url, &[]);
    let first = home.run(&args).ok();
    let first_hash = first["txHash"].as_str().unwrap().to_string();
    // The harness cut the first run off and runs it again: refused, naming the pending send.
    let r = home.run(&args);
    let msg = r.err("VALIDATION_ERROR");
    println!("{msg}");
    for part in [
        "a transaction from this key is still pending on ethereum at nonce 7; nothing was \
         signed.",
        "If this command repeats a payment sent earlier (for example a run that was cut off), \
         do not run it again",
        "use --nonce 7 --replace <its txHash>",
        "Only for a different payment, pass --allow-pending",
    ] {
        assert!(msg.contains(part), "{part}: {msg}");
    }
    assert!(msg.contains(&first_hash), "{msg}");
    let v = r.json();
    assert_eq!(v["pendingNonces"], json!([7]));
    assert_eq!(v["pendingSends"][0]["txHash"], first_hash.as_str());
    assert_eq!(
        v["journal"],
        journal_path(&home, &me).display().to_string().as_str()
    );
    assert_eq!(rpc.sent().len(), 1, "the re-run was broadcast");
    // A dry-run reports the pending nonces instead of refusing.
    let dry = home.run(&transfer(&rpc.url, &["--dry-run"])).ok();
    assert_eq!(dry["pendingNonces"], json!([7]));
    assert_eq!(dry["duplicateSends"][0]["txHash"], first_hash.as_str());
    // --allow-pending alone: the same recipient and amount as the pending send is still a
    // likely repeat.
    let v = home.run(&transfer(&rpc.url, &["--allow-pending"])).json();
    assert_eq!(v["errorCode"], "VALIDATION_ERROR", "{v:#}");
    assert_eq!(v["duplicateSends"][0]["txHash"], first_hash.as_str());
    // A second payment, said so explicitly.
    let v = home
        .run(&transfer(
            &rpc.url,
            &["--allow-pending", "--allow-duplicate"],
        ))
        .ok();
    assert_eq!(v["nonce"], PENDING_NONCE + 1);
    assert_eq!(v["pendingNonces"], json!([7]));
    assert_eq!(v["duplicateSends"][0]["txHash"], first_hash.as_str());
    assert_eq!(rpc.sent().len(), 2);
}

#[test]
fn the_send_lock_is_held_through_wait() {
    let (home, _) = unlocked();
    // Never mined: --wait runs to its timeout.
    let rpc = MockRpc::start(healthy);
    let args = transfer(&rpc.url, &["--wait", "--wait-timeout", "4s"]);
    std::thread::scope(|s| {
        let first = s.spawn(|| home.run(&args));
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while rpc.calls("eth_getTransactionReceipt").is_empty() {
            assert!(std::time::Instant::now() < deadline, "never waited");
            std::thread::sleep(Duration::from_millis(20));
        }
        // The first send is waiting for its receipt: a second one fails at once.
        let started = std::time::Instant::now();
        let msg = home.run(&transfer(&rpc.url, &[])).err("VALIDATION_ERROR");
        assert!(
            msg.starts_with("another send from this key on ethereum is in progress"),
            "{msg}"
        );
        assert!(started.elapsed() < Duration::from_secs(3), "it waited");
        let v = first.join().unwrap().ok();
        // Not mined within --wait-timeout: pending, with the safe next step.
        assert_eq!(v["status"], "pending");
        let hash = v["txHash"].as_str().unwrap();
        assert_eq!(
            v["hint"],
            format!(
                "still pending; to speed up or cancel use --nonce 7 --replace {hash}; a run \
                 without --nonce is a second payment"
            )
            .as_str()
        );
    });
    assert_eq!(rpc.sent().len(), 1);
}

#[test]
fn human_errors_print_the_send_details() {
    let (home, me) = unlocked();
    let rpc = MockRpc::start(|m, p| match m {
        "eth_sendRawTransaction" => Reply::Drop,
        _ => healthy(m, p),
    });
    let mut args = transfer(&rpc.url, &[]);
    args.retain(|a| *a != "--json");
    let r = home.run(&args);
    assert_eq!(r.code, 1, "{}", r.stdout);
    let hash = tx_hash_of(&rpc.sent()[0]);
    let journal = journal_path(&home, &me).display().to_string();
    for line in [
        format!("txHash: {hash}"),
        "nonce: 7".to_string(),
        format!("from: {me}"),
        format!("journal: {journal}"),
    ] {
        assert!(r.stderr.lines().any(|l| l == line), "{line}: {}", r.stderr);
    }
    assert!(r.stderr.contains("[BROADCAST_UNCERTAIN]"), "{}", r.stderr);
    // A refused transaction is named as such.
    let refused = send_error_node("nonce too low", false);
    let mut args = transfer(&refused.url, &["--allow-duplicate"]);
    args.retain(|a| *a != "--json");
    let r = home.run(&args);
    let hash = tx_hash_of(&refused.sent()[0]);
    assert!(
        r.stderr
            .lines()
            .any(|l| l == format!("refusedTxHash: {hash}")),
        "{}",
        r.stderr
    );
    assert!(
        !r.stderr.lines().any(|l| l.starts_with("txHash:")),
        "{}",
        r.stderr
    );
}

#[test]
fn rpc_fees_above_the_ceiling_need_max_fee_gwei() {
    let (home, _) = unlocked();
    // Base fee 1 gwei, eth_gasPrice 1.1 gwei, but feeHistory claims 500 gwei tips.
    let greedy = MockRpc::start(|m, p| fee_node(m, p, Some(GWEI), 500 * GWEI, GWEI + GWEI / 10));
    let msg = home
        .run(&transfer(&greedy.url, &["--dry-run"]))
        .err("VALIDATION_ERROR");
    assert!(msg.contains("--max-fee-gwei"), "{msg}");
    let v = home
        .run(&transfer(
            &greedy.url,
            &["--dry-run", "--max-fee-gwei", "1000.5"],
        ))
        .ok();
    assert_eq!(v["maxPriorityFeePerGas"], (500 * GWEI).to_string());
    home.run(&transfer(
        &greedy.url,
        &["--dry-run", "--max-fee-gwei", "100"],
    ))
    .err("VALIDATION_ERROR");

    // Normal fees pass without the flag; the flag also lowers the ceiling.
    let normal = MockRpc::start(healthy);
    home.run(&transfer(&normal.url, &["--dry-run"])).ok();
    home.run(&transfer(
        &normal.url,
        &["--dry-run", "--max-fee-gwei", "1"],
    ))
    .err("VALIDATION_ERROR");
    for bad in ["0", "-1", "1e3", "abc"] {
        home.run(&transfer(
            &normal.url,
            &["--dry-run", "--max-fee-gwei", bad],
        ))
        .err("VALIDATION_ERROR");
    }

    // Legacy chain (no base fee): gasPrice is eth_gasPrice; the flag can still lower the cap.
    let legacy = MockRpc::start(|m, p| fee_node(m, p, None, 0, 3 * GWEI));
    let v = home.run(&transfer(&legacy.url, &["--dry-run"])).ok();
    assert_eq!(v["type"], "legacy");
    assert_eq!(v["gasPrice"], (3 * GWEI).to_string());
    home.run(&transfer(
        &legacy.url,
        &["--dry-run", "--max-fee-gwei", "2"],
    ))
    .err("VALIDATION_ERROR");
    // A legacy node's eth_gasPrice is not its own ceiling: 10 gwei is above 6 x 1 gwei.
    let pricey = MockRpc::start(|m, p| fee_node(m, p, None, 0, 10 * GWEI));
    let msg = home
        .run(&transfer(&pricey.url, &["--dry-run"]))
        .err("VALIDATION_ERROR");
    assert!(msg.contains("gasPrice 10 gwei"), "{msg}");
    assert!(msg.contains("--max-fee-gwei"), "{msg}");
    home.run(&transfer(
        &pricey.url,
        &["--dry-run", "--max-fee-gwei", "10"],
    ))
    .ok();
    assert!(greedy.sent().is_empty() && normal.sent().is_empty() && legacy.sent().is_empty());
}

#[test]
fn rpc_url_path_and_query_are_never_printed() {
    let home = Home::new();
    let secret_env = "https://eth.example.com/v2/SECRETKEY1?apikey=SECRETKEY2";
    let v = home
        .run_env(&["chains", "--json"], &[("ZWALLET_RPC_BASE", secret_env)])
        .ok();
    let base = v
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["key"] == "base")
        .unwrap();
    assert_eq!(base["rpc"], "https://eth.example.com/<redacted>");
    let human = home.run_env(&["chains"], &[("ZWALLET_RPC_BASE", secret_env)]);
    assert!(!human.stdout.contains("SECRETKEY"), "{}", human.stdout);
    // A key in a subdomain label; the default hosts stay readable.
    let key_host = "https://k3y0a1b2c3d4e5f6.base.example.com";
    let v = home
        .run_env(&["chains", "--json"], &[("ZWALLET_RPC_BASE", key_host)])
        .ok();
    for row in v.as_array().unwrap() {
        let shown = row["rpc"].as_str().unwrap();
        if row["key"] == "base" {
            assert_eq!(shown, "https://<redacted>.base.example.com");
        } else {
            assert!(shown.ends_with("-rpc.publicnode.com"), "{shown}");
            assert!(!shown.contains("redacted"), "{shown}");
        }
    }
    let human = home.run_env(&["chains"], &[("ZWALLET_RPC_BASE", key_host)]);
    assert!(!human.stdout.contains("k3y0a1b2"), "{}", human.stdout);

    home.create("t", &[]);
    let leaks = |r: &common::Run| r.stdout.contains("SECRETKEY") || r.stderr.contains("SECRETKEY");
    // Transport error text (reqwest quotes the URL).
    let closed = home.run(&[
        "balance",
        "--chain",
        "ethereum",
        "--rpc",
        "http://127.0.0.1:1/SECRETKEY3",
        "--json",
    ]);
    closed.err("RPC_ERROR");
    assert!(!leaks(&closed), "{}", closed.stdout);
    // Wrong chain id.
    let wrong = MockRpc::start_at("/v3/SECRETKEY4?k=SECRETKEY5", |m, p| match m {
        "eth_chainId" => Reply::Result(json!("0x2")),
        _ => healthy(m, p),
    });
    let r = home.run(&[
        "balance", "--chain", "ethereum", "--rpc", &wrong.url, "--json",
    ]);
    r.err("RPC_ERROR");
    assert!(!leaks(&r), "{}", r.stdout);
    // Unparseable URL.
    let r = home.run(&[
        "balance",
        "--chain",
        "ethereum",
        "--rpc",
        "not a url/SECRETKEY6",
        "--json",
    ]);
    r.err("VALIDATION_ERROR");
    assert!(!leaks(&r), "{}", r.stdout);
    // An HTTP error body that echoes the key: as the full path and query, and bare.
    let echo = MockRpc::start_at("/v3/SECRETKEY7?apikey=SECRETKEY8", |_, _| {
        Reply::Http(
            401,
            "invalid project id SECRETKEY7 for /v3/SECRETKEY7?apikey=SECRETKEY8 (key SECRETKEY8)",
        )
    });
    let r = home.run(&[
        "balance", "--chain", "ethereum", "--rpc", &echo.url, "--json",
    ]);
    let msg = r.err("RPC_ERROR");
    // Only the status and the origin: none of the body, whatever it quotes.
    let origin = echo.url.split("/v3").next().unwrap();
    assert!(msg.ends_with(&format!("HTTP 401 from {origin}")), "{msg}");
    assert!(!msg.contains("invalid project id"), "{msg}");
    assert!(!leaks(&r), "{}", r.stdout);
}

/// A node whose nonces count what it accepted, mined at once (`latest` = `pending`).
fn mining_node() -> MockRpc {
    let accepted = Arc::new(AtomicUsize::new(0));
    MockRpc::start(move |m, p| match m {
        "eth_getTransactionCount" => Reply::Result(hex(
            u128::from(PENDING_NONCE) + accepted.load(Ordering::SeqCst) as u128
        )),
        "eth_sendRawTransaction" => {
            accepted.fetch_add(1, Ordering::SeqCst);
            healthy(m, p)
        }
        // Any token contract: 6 decimals, symbol "USDC", a large balance.
        "eth_call" => {
            let tx = &p[0];
            let data = tx["input"].as_str().or(tx["data"].as_str()).unwrap_or("");
            let word = |n: u128| format!("{n:064x}");
            Reply::Result(json!(match &data[..10.min(data.len())] {
                "0x313ce567" => format!("0x{}", word(6)),
                "0x70a08231" => format!("0x{}", word(10u128.pow(12))),
                "0x95d89b41" => format!("0x{}{}{:0<64}", word(32), word(4), "55534443"),
                other => panic!("mock: unexpected eth_call {other}"),
            }))
        }
        _ => healthy(m, p),
    })
}

/// `transfer` of `amount` (instead of 0.01) to OTHER.
fn transfer_amount<'a>(rpc: &'a str, amount: &'a str, extra: &[&'a str]) -> Vec<&'a str> {
    let mut args = transfer(rpc, extra);
    args[6] = amount;
    args
}

#[test]
fn a_repeat_of_a_recent_send_is_refused_unless_allow_duplicate() {
    let (home, me) = unlocked();
    let rpc = mining_node();
    let args = transfer(&rpc.url, &[]);
    let first = home.run(&args).ok();
    let first_hash = first["txHash"].as_str().unwrap().to_string();
    assert_eq!(first["duplicateSends"], json!([]));
    // The first was mined (nothing pending), and the harness runs the same command again.
    let r = home.run(&args);
    let msg = r.err("VALIDATION_ERROR");
    println!("{msg}");
    assert!(
        msg.starts_with(
            "this looks like a repeat of a recent send; look those hashes up first. If this \
             really is a second payment, pass --allow-duplicate"
        ),
        "{msg}"
    );
    assert!(
        msg.contains(&format!("{first_hash} (nonce 7, sent ")),
        "{msg}"
    );
    assert!(msg.ends_with("; nothing was signed"), "{msg}");
    let v = r.json();
    let dup = &v["duplicateSends"][0];
    assert_eq!(dup["txHash"], first_hash.as_str(), "{v:#}");
    assert_eq!(dup["nonce"], 7);
    assert!(dup["ts"].as_str().unwrap().ends_with('Z'), "{v:#}");
    assert_eq!(
        v["journal"],
        journal_path(&home, &me).display().to_string().as_str()
    );
    assert_eq!(rpc.sent().len(), 1, "the repeat was broadcast");
    // A dry-run reports it instead.
    let dry = home.run(&transfer(&rpc.url, &["--dry-run"])).ok();
    assert_eq!(dry["duplicateSends"][0]["txHash"], first_hash.as_str());
    // Another amount is another payment.
    let other = home.run(&transfer_amount(&rpc.url, "0.02", &[])).ok();
    assert_eq!(other["nonce"], PENDING_NONCE + 1);
    assert_eq!(other["duplicateSends"], json!([]));
    // A real second payment of the same amount, said so explicitly.
    let again = home.run(&transfer(&rpc.url, &["--allow-duplicate"])).ok();
    assert_eq!(again["nonce"], PENDING_NONCE + 2);
    assert_eq!(again["duplicateSends"][0]["txHash"], first_hash.as_str());
    // A --nonce send is an explicit retry or replacement at a fixed nonce: not checked.
    let fixed = home.run(&transfer(&rpc.url, &["--nonce", "10"])).ok();
    assert_eq!(fixed["nonce"], 10);
    assert!(fixed.get("duplicateSends").is_none(), "{fixed:#}");
    assert_eq!(rpc.sent().len(), 4);
}

#[test]
fn only_recent_well_formed_entries_count_as_duplicates() {
    let (home, me) = unlocked();
    let rpc = mining_node();
    let line = |ts: &str| {
        json!({ "ts": ts, "chain": "ethereum", "from": me, "nonce": 3,
                "txHash": format!("0x{}", "0a".repeat(32)), "to": OTHER.to_lowercase(),
                "value": "0.01 ETH", "kind": "transfer" })
        .to_string()
    };
    // An entry from long ago, and lines that are not entries.
    let old = line("2026-01-01T00:00:00Z");
    std::fs::write(
        journal_path(&home, &me),
        format!("{old}\nnot json\n[1,2]\n{{\"ts\":\"bad\"}}\n"),
    )
    .unwrap();
    let v = home.run(&transfer(&rpc.url, &[])).ok();
    assert_eq!(v["duplicateSends"], json!([]));
    // The same entry stamped 9 minutes ago is a duplicate (recipient case does not matter).
    let now = zwallet::core::time::unix_secs(std::time::SystemTime::now());
    let recent = line(&zwallet::core::time::rfc3339_utc(now - 540));
    std::fs::write(journal_path(&home, &me), format!("{recent}\n")).unwrap();
    home.run(&transfer(&rpc.url, &[])).err("VALIDATION_ERROR");
    // 11 minutes ago is not.
    let stale = line(&zwallet::core::time::rfc3339_utc(now - 660));
    std::fs::write(journal_path(&home, &me), format!("{stale}\n")).unwrap();
    home.run(&transfer(&rpc.url, &[])).ok();
}

#[test]
fn token_transfers_match_on_the_token_contract() {
    let (home, me) = unlocked();
    let rpc = mining_node();
    // Another contract that also calls itself USDC.
    let fake = "0x1111111111111111111111111111111111111111";
    let token = |t: &'static str| {
        let mut a = transfer(&rpc.url, &["--token", t]);
        a[6] = "0.5";
        a
    };
    let first = home.run(&token(USDC)).ok();
    assert_eq!(first["symbol"], "USDC");
    let text = std::fs::read_to_string(journal_path(&home, &me)).unwrap();
    let entry: Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
    assert_eq!(entry["kind"], "token-transfer");
    assert_eq!(entry["value"], "0.5 USDC");
    assert_eq!(entry["token"], USDC);
    assert_eq!(entry["to"], OTHER);
    // The same token again: a repeat.
    let v = home.run(&token(USDC)).json();
    assert_eq!(v["errorCode"], "VALIDATION_ERROR", "{v:#}");
    assert_eq!(v["duplicateSends"][0]["txHash"], first["txHash"]);
    // The same symbol and amount from another contract: another payment.
    home.run(&token(fake)).ok();
    // A native transfer of 0.5 is not a token transfer of 0.5.
    home.run(&transfer_amount(&rpc.url, "0.5", &[])).ok();
    assert_eq!(rpc.sent().len(), 3);
}
