//! EVM send-path tests through the built binary against a local mock JSON-RPC node. No network:
//! the mock listens on 127.0.0.1 and answers only what each test scripts. Every wallet is a temp
//! wallet whose daemon is locked (or killed) when the test ends.

mod common;

use alloy::primitives::keccak256;
use common::Home;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// A second hardhat account, used as a plain recipient.
const OTHER: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
const USDC: &str = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48";
const GWEI: u128 = 1_000_000_000;
/// The mock account's nonces: confirmed (`latest`) and next free (`pending`).
const LATEST_NONCE: u64 = 5;
const PENDING_NONCE: u64 = 7;

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

/// A healthy, funded EIP-1559 node: base fee 1 gwei, tips 0.1 gwei, 1 ETH balance.
fn healthy(method: &str, params: &Value) -> Reply {
    fee_node(method, params, Some(GWEI), GWEI / 10, GWEI + GWEI / 10)
}

/// A node with the given base fee (`None` = legacy chain), feeHistory tip, and eth_gasPrice.
fn fee_node(method: &str, params: &Value, base: Option<u128>, tip: u128, gas_price: u128) -> Reply {
    Reply::Result(match method {
        "eth_chainId" => json!("0x1"),
        "eth_getBalance" => hex(10u128.pow(18)),
        "eth_getTransactionCount" => match params[1].as_str() {
            Some("latest") => hex(LATEST_NONCE.into()),
            _ => hex(PENDING_NONCE.into()),
        },
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
    assert!(msg.contains("--nonce 7"), "{msg}");
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
    // geth's wording (most clients share it), matched case-insensitively.
    let rejections = [
        "replacement transaction underpriced",
        "Nonce too low: address 0xf39F, tx: 7 state: 8",
        "INSUFFICIENT FUNDS for gas * price + value",
        "intrinsic gas too low",
        "exceeds block gas limit",
        "max fee per gas less than block base fee: maxFeePerGas: 1, baseFee: 2",
    ];
    for (i, why) in rejections.into_iter().enumerate() {
        *answer.lock().unwrap() = why;
        let r = home.run(&transfer(&rpc.url, &[]));
        let msg = r.err("RPC_ERROR");
        assert!(
            msg.contains("the node rejected the transaction; nothing was broadcast"),
            "{msg}"
        );
        assert!(msg.contains(why), "{msg}");
        let v = r.json();
        let sent = rpc.sent();
        assert_eq!(sent.len(), i + 1, "the CLI must send exactly once");
        assert_eq!(v["txHash"], tx_hash_of(&sent[i]).as_str(), "{v:#}");
        assert_eq!(v["nonce"], PENDING_NONCE);
        assert_eq!(v["from"], me.as_str());
    }
    // A rejection is definitive: no lookup by hash.
    assert!(rpc.calls("eth_getTransactionByHash").is_empty());
    // Any other error text still asks for the transaction and stays uncertain.
    *answer.lock().unwrap() = "internal error";
    home.run(&transfer(&rpc.url, &[]))
        .err("BROADCAST_UNCERTAIN");
    assert!(!rpc.calls("eth_getTransactionByHash").is_empty());
}

#[test]
fn replacing_a_pending_nonce_pays_a_quarter_more() {
    let (home, _) = unlocked();
    let wei = |v: &Value, k: &str| -> u128 {
        v[k].as_str()
            .unwrap_or_else(|| panic!("{k}: {v:#}"))
            .parse()
            .unwrap()
    };
    let rpc = MockRpc::start(healthy);
    // Nonce 7 is the next free one (no replacement), 6 is pending (replaced).
    let fresh = home
        .run(&transfer(&rpc.url, &["--nonce", "7", "--dry-run"]))
        .ok();
    let repl = home
        .run(&transfer(&rpc.url, &["--nonce", "6", "--dry-run"]))
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
    let legacy = MockRpc::start(|m, p| fee_node(m, p, None, 0, 3 * GWEI));
    let v = home
        .run(&transfer(&legacy.url, &["--nonce", "6", "--dry-run"]))
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
            &["--nonce", "6", "--dry-run", "--max-fee-gwei", "3.5"],
        ))
        .err("VALIDATION_ERROR");
    assert!(msg.contains("--max-fee-gwei"), "{msg}");
    assert!(rpc.sent().is_empty() && legacy.sent().is_empty());
}

#[test]
fn failure_before_the_send_is_rpc_error_and_says_nothing_was_sent() {
    let (home, _) = unlocked();
    // `prepare` connects once (eth_chainId); the broadcast's own connect is the second call.
    let n = Arc::new(AtomicUsize::new(0));
    let c = n.clone();
    let rpc = MockRpc::start(move |m, p| match m {
        "eth_chainId" if c.fetch_add(1, Ordering::SeqCst) >= 1 => Reply::Error("node is down"),
        _ => healthy(m, p),
    });
    let msg = home.run(&transfer(&rpc.url, &[])).err("RPC_ERROR");
    assert!(msg.contains("nothing was broadcast"), "{msg}");
    assert!(rpc.sent().is_empty());
}

#[test]
fn explicit_nonce_is_signed_and_must_be_replaceable_or_next() {
    let (home, _) = unlocked();
    let rpc = MockRpc::start(healthy);
    let v = home
        .run(&transfer(&rpc.url, &["--nonce", "6", "--dry-run"]))
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
    assert!(lo.contains("already"), "{lo}");
    let hi = home
        .run(&transfer(&rpc.url, &["--nonce", "8", "--dry-run"]))
        .err("VALIDATION_ERROR");
    assert!(hi.contains("gap"), "{hi}");
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
fn rpc_returning_another_hash_is_rpc_error_with_the_computed_hash() {
    let (home, me) = unlocked();
    let bogus = format!("0x{}", "ab".repeat(32));
    let b = bogus.clone();
    let rpc = MockRpc::start(move |m, p| match m {
        "eth_sendRawTransaction" => Reply::Result(json!(b)),
        _ => healthy(m, p),
    });
    let r = home.run(&transfer(&rpc.url, &[]));
    r.err("RPC_ERROR");
    let v = r.json();
    println!("{v:#}");
    let hash = tx_hash_of(&rpc.sent()[0]);
    assert_eq!(v["txHash"], hash.as_str(), "{v:#}");
    assert_eq!(v["rpcTxHash"], bogus.as_str());
    assert_eq!(v["nonce"], PENDING_NONCE);
    assert_eq!(v["from"], me.as_str());
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
    assert_eq!(v["txHash"], tx_hash_of(&rpc.sent()[0]).as_str());
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
    assert!(msg.contains("401"), "{msg}");
    assert!(!leaks(&r), "{}", r.stdout);
}
