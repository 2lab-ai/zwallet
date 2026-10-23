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
use zwallet::core::whitelist::{Entry, Whitelist};

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

/// A wallet unlocked for 10 minutes whose signed whitelist lets these tests pay OTHER on every
/// chain and call ROUTER's `deposit()` on Ethereum, under the default fee caps.
fn unlocked() -> (Home, String) {
    listed(|l| {
        payee(l, "other", OTHER, &["*"]);
        contract(l, "router", ROUTER, &["ethereum"], &[DEPOSIT]);
    })
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
    assert_eq!(v["pendingRange"], Value::Null);
    assert_eq!(v["pendingCount"], 0);
    // Nothing broadcast, nothing journaled.
    assert!(!journal_path(&home, &me).exists());
    let inv = home
        .run(&[
            "invoke",
            "--chain",
            "ethereum",
            "--to",
            ROUTER,
            "--data",
            DEPOSIT,
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
    // How to look it up: the exact command and the explorer page.
    assert!(
        msg.contains(&format!(
            "look it up first (`zwallet tx --chain ethereum {hash}` or \
             https://etherscan.io/tx/{hash})"
        )),
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
        // The journal's earlier send is named, with how to look it up; the refused retry is
        // not offered as it.
        assert!(msg.contains(&earlier), "{msg}");
        assert!(
            msg.contains(&format!(
                "`zwallet tx --chain ethereum {earlier}` or https://etherscan.io/tx/{earlier}"
            )),
            "{msg}"
        );
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
        // The journal names it, with the command that looks it up.
        assert!(msg.contains(&six), "{msg}");
        assert!(
            msg.contains(&format!("`zwallet tx --chain ethereum {six}`")),
            "{msg}"
        );
        // The pending transaction may be another payment: never replace that one.
        assert!(
            msg.ends_with(
                "If the transaction at nonce 6 is a different payment than the one you are \
                 retrying, do not replace it; that payment and yours are separate"
            ),
            "{msg}"
        );
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
    assert!(
        msg.ends_with(
            "If the transaction at nonce 6 is a different payment than the one you are \
             retrying, do not replace it; that payment and yours are separate"
        ),
        "{msg}"
    );
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
    for part in [
        "the transaction at nonce 4 is already mined (nonce consumed); whether the payment \
         succeeded is not known from this alone: look up its receipt with `zwallet tx --chain \
         ethereum <its txHash>` before sending anything new; nothing was signed.",
        "If the transaction at nonce 4 is a different payment than the one you are retrying, \
         do not replace it; that payment and yours are separate.",
        // Where to look: the journal has nothing at nonce 4, so the explorer.
        "has no entry at nonce 4; find it on the explorer: https://etherscan.io/address/",
    ] {
        assert!(lo.contains(part), "{part}: {lo}");
    }
    assert!(!lo.contains(WENT_THROUGH), "{lo}");
    // Never a pointer to another nonce: that is how a retry becomes a second send.
    assert!(!lo.contains("next free nonce"), "{lo}");
    let hi = home
        .run(&transfer(&rpc.url, &["--nonce", "8", "--dry-run"]))
        .err("VALIDATION_ERROR");
    assert!(hi.contains("gap"), "{hi}");
    assert!(hi.contains("has no entry at nonce 8"), "{hi}");
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
        let journal = format!(
            "once it is broadcast, in the send journal {} (look it up with `zwallet tx --chain \
             ethereum <txHash>` or https://etherscan.io/tx/<txHash>); for a different payment, \
             retry after it finishes",
            journal_path(&home, &me).display()
        );
        for part in [
            named.as_str(),
            "If it is an earlier attempt at this same payment, wait for it to finish and look up \
             its result before sending again: its txHash is in its output and, ",
            journal.as_str(),
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
             second payment; look it up with `zwallet tx --chain ethereum {hash}` or \
             https://etherscan.io/tx/{hash}"
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
        ("amountRaw", json!("10000000000000000")),
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
             this hash up before sending again (`zwallet tx --chain ethereum {hash}` or \
             https://etherscan.io/tx/{hash}); a retry must pass --nonce 7 --replace {hash}"
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
            "invoke", "--chain", "ethereum", "--to", ROUTER, "--data", DEPOSIT, "--value", "0.5",
            "--rpc", &rpc.url, "--json",
        ])
        .ok();
    let text = std::fs::read_to_string(&journal).unwrap();
    let last: Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
    assert_eq!(last["kind"], "invoke");
    assert_eq!(last["value"], "0.5 ETH");
    assert_eq!(last["amountRaw"], "500000000000000000");
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
    assert!(
        msg.contains(&format!("`zwallet tx --chain ethereum {first_hash}`")),
        "{msg}"
    );
    let v = r.json();
    assert_eq!(v["pendingNonces"], json!([7]));
    assert_eq!(v["pendingRange"], json!([7, 7]));
    assert_eq!(v["pendingCount"], 1);
    assert_eq!(v["pendingSends"][0]["txHash"], first_hash.as_str());
    assert_eq!(
        v["journal"],
        journal_path(&home, &me).display().to_string().as_str()
    );
    assert_eq!(rpc.sent().len(), 1, "the re-run was broadcast");
    // A dry-run reports the pending nonces instead of refusing.
    let dry = home.run(&transfer(&rpc.url, &["--dry-run"])).ok();
    assert_eq!(dry["pendingNonces"], json!([7]));
    assert_eq!(dry["pendingRange"], json!([7, 7]));
    assert_eq!(dry["pendingCount"], 1);
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
                 without --nonce is a second payment; look it up with `zwallet tx --chain \
                 ethereum {hash}` or https://etherscan.io/tx/{hash}"
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
    // A whitelist fee cap of 0.1 ETH, above every fee this test builds: the CLI's ceiling is
    // what is tested here (the cap is in `w9_…`).
    let (home, _) = listed(|l| {
        payee(l, "other", OTHER, &["*"]);
        l.policy
            .set_max_fee_wei("ethereum", alloy::primitives::U256::from(10u128.pow(17)));
    });
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

/// Arbitrum's USDC: the one registry token the [`wallet_node`]s of these tests hold (12.5).
const ARB_USDC: &str = "0xaf88d065e77c8cC2239327C5EDb3A432268e5831";

/// A node serving chain `id` that only reads balances: `native` wei for any account (answered
/// after `delay`), and 12.5 USDC of [`ARB_USDC`], zero of any other token.
fn wallet_node(id: u64, native: u128, delay: Duration) -> MockRpc {
    failing_wallet_node(id, native, delay, None)
}

/// How a [`failing_wallet_node`] treats `balanceOf` of one token.
#[derive(Clone, Copy)]
enum TokenFault {
    /// A JSON-RPC error.
    Fails,
    /// No answer for 40 s.
    Hangs,
}

/// A [`wallet_node`] whose `balanceOf` of token `broken.0` (any case) goes wrong as `broken.1`.
fn failing_wallet_node(
    id: u64,
    native: u128,
    delay: Duration,
    broken: Option<(&'static str, TokenFault)>,
) -> MockRpc {
    MockRpc::start(move |m, p| match m {
        "eth_chainId" => Reply::Result(hex(id.into())),
        "eth_getBalance" => {
            std::thread::sleep(delay);
            Reply::Result(hex(native))
        }
        "eth_call" => {
            let data = p[0]["input"]
                .as_str()
                .or(p[0]["data"].as_str())
                .unwrap_or("");
            assert!(
                data.starts_with("0x70a08231"),
                "mock: unexpected eth_call {data}"
            );
            let to = p[0]["to"].as_str().unwrap();
            match broken {
                Some((token, TokenFault::Fails)) if to.eq_ignore_ascii_case(token) => {
                    return Reply::Error("execution reverted");
                }
                Some((token, TokenFault::Hangs)) if to.eq_ignore_ascii_case(token) => {
                    std::thread::sleep(Duration::from_secs(40));
                    return Reply::Drop;
                }
                _ => {}
            }
            let held = if to.eq_ignore_ascii_case(ARB_USDC) {
                12_500_000
            } else {
                0
            };
            Reply::Result(json!(format!("0x{held:064x}")))
        }
        other => panic!("mock: unexpected {other}"),
    })
}

/// A [`wallet_node`] for every chain, keyed by chain key: 1.5 ETH on Base, nothing elsewhere.
fn every_chain_node(delay: Duration) -> Vec<(&'static str, MockRpc)> {
    zwallet::chains::CHAINS
        .iter()
        .map(|c| {
            let id = zwallet::chains::evm::config(c).chain_id;
            let native = if c.key == "base" {
                15 * 10u128.pow(17)
            } else {
                0
            };
            (c.key, wallet_node(id, native, delay))
        })
        .collect()
}

/// `ZWALLET_RPC_<CHAIN>` for every chain: its node, or the URL `instead` gives for that chain
/// key. Every chain is set, so none falls back to its public default RPC.
fn rpc_env(nodes: &[(&str, MockRpc)], instead: &[(&str, &str)]) -> Vec<(String, String)> {
    nodes
        .iter()
        .map(|(key, node)| {
            let url = instead
                .iter()
                .find(|(k, _)| k == key)
                .map_or(node.url.as_str(), |(_, u)| u);
            (
                format!("ZWALLET_RPC_{}", key.to_uppercase()),
                url.to_string(),
            )
        })
        .collect()
}

fn env_refs(env: &[(String, String)]) -> Vec<(&str, &str)> {
    env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect()
}

/// Every chain key in the order `balance` without `--chain` lists them: mainnets, then Sepolia.
const ALL_CHAINS_ORDER: [&str; 8] = [
    "ethereum",
    "base",
    "arbitrum",
    "optimism",
    "polygon",
    "bsc",
    "avalanche",
    "sepolia",
];

#[test]
fn balance_without_chain_reads_every_network_and_its_registry_tokens() {
    let home = Home::new();
    let me = home.create("t", &[])["address"]
        .as_str()
        .unwrap()
        .to_string();
    let nodes = every_chain_node(Duration::from_secs(1));
    let env = rpc_env(&nodes, &[]);
    let started = std::time::Instant::now();
    let v = home.run_env(&["balance", "--json"], &env_refs(&env)).ok();
    // The networks are read at once: eight 1 s balance reads take about 1 s, not the 8 s one
    // after another would (the bound leaves room for a loaded machine).
    let took = started.elapsed();
    assert!(took < Duration::from_secs(7), "{took:?}");
    assert_eq!(
        (&v["key"], &v["address"], &v["verified"]),
        (&json!("t"), &json!(me), &json!(false))
    );
    let rows = v["chains"].as_array().unwrap();
    let keys: Vec<&str> = rows.iter().map(|r| r["chain"].as_str().unwrap()).collect();
    assert_eq!(keys, ALL_CHAINS_ORDER);
    for row in rows {
        let key = row["chain"].as_str().unwrap();
        let chain = zwallet::chains::find(key).unwrap();
        let id = zwallet::chains::evm::config(chain).chain_id;
        assert_eq!(row["chainId"], id, "{row:#}");
        assert!(row.get("error").is_none(), "{row:#}");
        // The same address on every EVM chain: only the top-level one.
        assert!(row.get("address").is_none(), "{row:#}");
        let (balance, raw) = if key == "base" {
            ("1.5", "1500000000000000000")
        } else {
            ("0", "0")
        };
        assert_eq!(
            row["native"],
            json!({ "symbol": chain.symbol, "decimals": 18, "balance": balance, "raw": raw }),
            "{key}"
        );
        // Every registry row of this chain, in registry order, zero balances included.
        let want: Vec<Value> = zwallet::chains::evm::registry::TOKENS
            .iter()
            .filter(|t| t.chain_id == id)
            .map(|t| {
                let (balance, raw) = if t.address == ARB_USDC {
                    ("12.5", "12500000")
                } else {
                    ("0", "0")
                };
                json!({
                    "symbol": t.symbol,
                    "address": t.address,
                    "decimals": t.decimals,
                    "balance": balance,
                    "raw": raw,
                })
            })
            .collect();
        assert_eq!(row["tokens"], json!(want), "{key}");
        // One balance read of the key's address, and one balanceOf per registry token.
        let node = &nodes.iter().find(|(k, _)| *k == key).unwrap().1;
        let reads = node.calls("eth_getBalance");
        assert_eq!(reads.len(), 1, "{key}");
        assert!(
            reads[0][0].as_str().unwrap().eq_ignore_ascii_case(&me),
            "{key}"
        );
        let mut calls: Vec<String> = node
            .calls("eth_call")
            .iter()
            .map(|p| p[0]["to"].as_str().unwrap().to_lowercase())
            .collect();
        let mut tokens: Vec<String> = want
            .iter()
            .map(|t| t["address"].as_str().unwrap().to_lowercase())
            .collect();
        calls.sort();
        tokens.sort();
        assert_eq!(calls, tokens, "{key}");
    }
    // BNB Smart Chain has no registry token.
    assert_eq!(rows[5]["tokens"], json!([]));

    // Human: one line per network, a held token under its network, then the count.
    let human = home.run_env(&["balance"], &env_refs(&env));
    assert_eq!(human.code, 0, "{}", human.stderr);
    assert_eq!(
        human.stdout,
        format!(
            "key: t\naddress: {me}\n\
             ethereum   0 ETH\n\
             base       1.5 ETH\n\
             arbitrum   0 ETH\n\
             \x20          12.5 USDC\n\
             optimism   0 ETH\n\
             polygon    0 POL\n\
             bsc        0 BNB\n\
             avalanche  0 AVAX\n\
             sepolia    0 ETH\n\
             2 of 8 networks have a balance\n"
        )
    );
}

#[test]
fn balance_without_chain_shows_the_networks_that_answer() {
    let home = Home::new();
    home.create("t", &[]);
    let nodes = every_chain_node(Duration::ZERO);
    // Polygon refuses the connection; BNB Smart Chain takes it and never answers.
    let silent = MockRpc::start(|_, _| {
        std::thread::sleep(Duration::from_secs(40));
        Reply::Drop
    });
    let env = rpc_env(
        &nodes,
        &[("polygon", "http://127.0.0.1:1"), ("bsc", &silent.url)],
    );
    let started = std::time::Instant::now();
    let v = home.run_env(&["balance", "--json"], &env_refs(&env)).ok();
    // BNB Smart Chain gets its 8 s, not the 30 s a single RPC call may take.
    let took = started.elapsed();
    assert!(
        took >= Duration::from_secs(7) && took < Duration::from_secs(20),
        "{took:?}"
    );
    let rows = v["chains"].as_array().unwrap();
    let keys: Vec<&str> = rows.iter().map(|r| r["chain"].as_str().unwrap()).collect();
    assert_eq!(keys, ALL_CHAINS_ORDER);
    for row in rows {
        match row["chain"].as_str().unwrap() {
            "polygon" | "bsc" => {
                assert_eq!(row["errorCode"], "RPC_ERROR", "{row:#}");
                assert!(row.get("native").is_none(), "{row:#}");
                assert!(row.get("tokens").is_none(), "{row:#}");
            }
            _ => {
                assert!(row.get("error").is_none(), "{row:#}");
                assert!(row["native"]["balance"].is_string(), "{row:#}");
            }
        }
    }
    let bsc = rows[5]["error"].as_str().unwrap();
    assert!(bsc.contains("timed out after 8s"), "{bsc}");
    let polygon = rows[4]["error"].as_str().unwrap();
    assert!(polygon.starts_with("eth_chainId failed"), "{polygon}");

    // Human (Polygon down only, its RPC URL carrying a key): its line says so briefly, and the
    // count says one did not answer; still exit 0. Neither form prints the key.
    let keyed = "http://127.0.0.1:1/v3/SECRETKEY9?apikey=SECRETKEY10";
    let env = rpc_env(&nodes, &[("polygon", keyed)]);
    let human = home.run_env(&["balance"], &env_refs(&env));
    assert_eq!(human.code, 0, "{}", human.stderr);
    let json = home.run_env(&["balance", "--json"], &env_refs(&env));
    for out in [&human.stdout, &human.stderr, &json.stdout, &json.stderr] {
        assert!(!out.contains("SECRETKEY"), "{out}");
    }
    assert_eq!(json.ok()["chains"][4]["errorCode"], "RPC_ERROR");
    let lines: Vec<&str> = human.stdout.lines().collect();
    let polygon = lines
        .iter()
        .find(|l| l.starts_with("polygon "))
        .unwrap_or_else(|| panic!("{}", human.stdout));
    assert!(polygon.starts_with("polygon    error: "), "{polygon}");
    assert!(polygon.ends_with(" [RPC_ERROR]"), "{polygon}");
    assert!(polygon.chars().count() <= 110, "{polygon}");
    assert_eq!(
        lines.last(),
        Some(&"2 of 8 networks have a balance; 1 did not answer"),
        "{}",
        human.stdout
    );
}

#[test]
fn balance_without_chain_shows_a_failed_token_read_on_its_row_only() {
    let home = Home::new();
    let me = home.create("t", &[])["address"]
        .as_str()
        .unwrap()
        .to_string();
    // Arbitrum's node refuses balanceOf of USDT; Optimism's never answers it for DAI. Both still
    // answer the chain id, the native balance and every other token.
    const ARB_USDT: &str = "0xFd086bC7CD5C481DCC9C85ebE478A1C0b69FCbb9";
    const OP_DAI: &str = "0xDA10009cBd5D07dd0CeCc66161FC93D7c9000da1";
    let mut nodes = every_chain_node(Duration::ZERO);
    for (key, id, broken) in [
        ("arbitrum", 42161, (ARB_USDT, TokenFault::Fails)),
        ("optimism", 10, (OP_DAI, TokenFault::Hangs)),
    ] {
        let slot = nodes.iter_mut().find(|(k, _)| *k == key).unwrap();
        slot.1 = failing_wallet_node(id, 0, Duration::ZERO, Some(broken));
    }
    let env = rpc_env(&nodes, &[]);
    let started = std::time::Instant::now();
    let v = home.run_env(&["balance", "--json"], &env_refs(&env)).ok();
    // The hanging read ends with the network's 8 s, and the others on Optimism do not wait.
    let took = started.elapsed();
    assert!(
        took >= Duration::from_secs(7) && took < Duration::from_secs(20),
        "{took:?}"
    );
    let rows = v["chains"].as_array().unwrap();
    for row in rows {
        assert!(row.get("error").is_none(), "{row:#}");
        assert!(row["native"]["balance"].is_string(), "{row:#}");
    }
    let tokens = |key: &str| {
        rows.iter().find(|r| r["chain"] == key).unwrap()["tokens"]
            .as_array()
            .unwrap()
            .clone()
    };
    let check = |key: &str, broken: &str, why: &str| {
        let tokens = tokens(key);
        let want = zwallet::chains::evm::registry::TOKENS
            .iter()
            .filter(|t| t.chain_id == if key == "arbitrum" { 42161 } else { 10 })
            .count();
        assert_eq!(tokens.len(), want, "{key}");
        for t in &tokens {
            if t["address"] == broken {
                // The failed read: symbol, address, decimals and why; no balance.
                assert_eq!(t["errorCode"], "RPC_ERROR", "{t:#}");
                let msg = t["error"].as_str().unwrap();
                assert!(msg.contains(why), "{msg}");
                assert!(
                    t.get("balance").is_none() && t.get("raw").is_none(),
                    "{t:#}"
                );
                assert_eq!(t["symbol"], if key == "arbitrum" { "USDT" } else { "DAI" });
                assert!(t["decimals"].is_u64(), "{t:#}");
            } else {
                assert!(t.get("error").is_none(), "{t:#}");
                assert!(t["balance"].is_string(), "{t:#}");
            }
        }
    };
    check("arbitrum", ARB_USDT, "execution reverted");
    check("optimism", OP_DAI, "timed out after 8s");
    let arb = tokens("arbitrum");
    let usdc = arb.iter().find(|t| t["address"] == ARB_USDC).unwrap();
    assert_eq!(usdc["balance"], "12.5");

    // Human: a failed token read is a line under its network; zero tokens stay hidden.
    let human = home.run_env(&["balance"], &env_refs(&env));
    assert_eq!(human.code, 0, "{}", human.stderr);
    let lines: Vec<&str> = human.stdout.lines().collect();
    assert_eq!(lines[..2], ["key: t".to_string(), format!("address: {me}")]);
    let at = |prefix: &str| {
        lines
            .iter()
            .position(|l| l.starts_with(prefix))
            .unwrap_or_else(|| panic!("{prefix}: {}", human.stdout))
    };
    let arb = at("arbitrum ");
    assert_eq!(lines[arb], "arbitrum   0 ETH");
    assert_eq!(lines[arb + 1], "           12.5 USDC");
    assert!(
        lines[arb + 2].starts_with("           USDT error: eth_call failed")
            && lines[arb + 2].ends_with(" [RPC_ERROR]"),
        "{}",
        human.stdout
    );
    let op = at("optimism ");
    assert_eq!(arb + 3, op, "{}", human.stdout);
    assert_eq!(lines[op], "optimism   0 ETH");
    assert!(
        lines[op + 1].starts_with("           DAI error: ")
            && lines[op + 1].contains("timed out after 8s"),
        "{}",
        human.stdout
    );
    assert!(lines[op + 2].starts_with("polygon "), "{}", human.stdout);
    assert_eq!(
        lines.last(),
        Some(&"2 of 8 networks have a balance; 2 token reads failed"),
        "{}",
        human.stdout
    );
}

#[test]
fn balance_without_chain_fails_when_no_network_answers() {
    let home = Home::new();
    home.create("t", &[]);
    let env: Vec<(String, String)> = zwallet::chains::CHAINS
        .iter()
        .map(|c| (c.rpc_env_var(), "http://127.0.0.1:1".to_string()))
        .collect();
    let msg = home
        .run_env(&["balance", "--json"], &env_refs(&env))
        .err("RPC_ERROR");
    assert!(msg.starts_with("no network answered"), "{msg}");
    for key in ALL_CHAINS_ORDER {
        assert!(msg.contains(&format!("{key}: ")), "{key}: {msg}");
    }
    let human = home.run_env(&["balance"], &env_refs(&env));
    assert_eq!(human.code, 1, "{}", human.stdout);
    assert!(
        human.stderr.starts_with("error: no network answered"),
        "{}",
        human.stderr
    );
    assert!(human.stderr.trim_end().ends_with("[RPC_ERROR]"));
}

#[test]
fn balance_with_chain_prints_what_it_always_did() {
    let home = Home::new();
    let me = home.create("t", &[])["address"]
        .as_str()
        .unwrap()
        .to_string();
    let base = wallet_node(8453, 15 * 10u128.pow(17), Duration::ZERO);
    let v = home
        .run(&["balance", "--chain", "base", "--rpc", &base.url, "--json"])
        .ok();
    assert_eq!(
        v,
        json!({
            "chain": "base",
            "key": "t",
            "address": me,
            "symbol": "ETH",
            "decimals": 18,
            "balance": "1.5",
            "raw": "1500000000000000000",
            "verified": false,
        })
    );
    let human = home.run(&["balance", "--chain", "base", "--rpc", &base.url]);
    assert_eq!(
        human.stdout,
        format!(
            "address: {me}\nbalance: 1.5\nchain: base\ndecimals: 18\nkey: t\nraw: \
             1500000000000000000\nsymbol: ETH\nverified: false\n"
        )
    );
    // A token: its own symbol and decimals, read from the contract, plus `token`.
    let node = mining_node();
    let v = home
        .run(&[
            "balance", "--chain", "ethereum", "--token", USDC, "--rpc", &node.url, "--json",
        ])
        .ok();
    assert_eq!(
        v,
        json!({
            "chain": "ethereum",
            "key": "t",
            "address": me,
            "symbol": "USDC",
            "decimals": 6,
            "balance": "1000000",
            "raw": "1000000000000",
            "verified": false,
            "token": USDC,
        })
    );
    // The env var still picks the RPC for one chain.
    let v = home
        .run_env(
            &["balance", "--chain", "base", "--json"],
            &[("ZWALLET_RPC_BASE", &base.url)],
        )
        .ok();
    assert_eq!(v["balance"], "1.5");
    // A token contract or an RPC URL without --chain: refused, nothing read.
    let reads = base.calls("eth_getBalance").len();
    for flag in [["--token", USDC], ["--rpc", base.url.as_str()]] {
        // The JSON error names the missing argument, as the human one does.
        let msg = home
            .run(&["balance", flag[0], flag[1], "--json"])
            .err("VALIDATION_ERROR");
        assert!(msg.contains("--chain <CHAIN>"), "{msg}");
        let human = home.run(&["balance", flag[0], flag[1]]);
        assert_eq!(human.code, 1, "{}", human.stdout);
        assert!(human.stderr.contains("--chain <CHAIN>"), "{}", human.stderr);
    }
    assert_eq!(base.calls("eth_getBalance").len(), reads);
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
    assert!(
        msg.contains(&format!(
            "look it up: `zwallet tx --chain ethereum {first_hash}` or \
             https://etherscan.io/tx/{first_hash})"
        )),
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
    // Another (registry) token contract that also calls itself USDC here.
    let fake = USDT;
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

#[test]
fn a_send_after_a_cut_off_journal_append_is_still_seen_as_a_repeat() {
    let (home, me) = unlocked();
    let rpc = mining_node();
    // A run killed in the middle of appending its journal line.
    std::fs::write(
        journal_path(&home, &me),
        r#"{"ts":"2026-10-20T00:00:00Z","chain":"ethereum","from":"0x"#,
    )
    .unwrap();
    let args = transfer(&rpc.url, &[]);
    let first = home.run(&args).ok();
    // Run again: the first send's line must be readable, so this is a repeat.
    let v = home.run(&args).json();
    assert_eq!(v["errorCode"], "VALIDATION_ERROR", "{v:#}");
    assert_eq!(v["duplicateSends"][0]["txHash"], first["txHash"], "{v:#}");
    assert_eq!(rpc.sent().len(), 1, "the repeat was broadcast");
}

/// Runs `args` like [`Home::run`], but kills the command if it has not finished within `limit`
/// (`None` then): a command stuck on a FIFO must fail the test, not hang it.
fn run_bounded(home: &Home, args: &[&str], limit: Duration) -> Option<common::Run> {
    use std::process::Stdio;
    let mut child = home
        .command(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + limit;
    while child.try_wait().unwrap().is_none() {
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let out = child.wait_with_output().unwrap();
    Some(common::Run {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

#[test]
fn a_fifo_in_place_of_the_journal_refuses_the_send_without_hanging() {
    let (home, me) = unlocked();
    let rpc = MockRpc::start(healthy);
    let path = journal_path(&home, &me);
    let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
    // SAFETY: mkfifo with a valid NUL-terminated path.
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    for extra in [&["--dry-run"][..], &[]] {
        let r = run_bounded(&home, &transfer(&rpc.url, extra), Duration::from_secs(20))
            .unwrap_or_else(|| panic!("{extra:?}: the send hung on the FIFO journal"));
        let msg = r.err("VALIDATION_ERROR");
        assert!(msg.contains(&path.display().to_string()), "{msg}");
        assert!(msg.contains("not a regular file"), "{msg}");
        assert!(msg.contains("nothing was signed"), "{msg}");
    }
    // A directory in its place: refused the same way.
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    let msg = home.run(&transfer(&rpc.url, &[])).err("VALIDATION_ERROR");
    assert!(msg.contains("not a regular file"), "{msg}");
    assert!(rpc.sent().is_empty(), "sent with an unreadable journal");
}

#[test]
fn a_token_repeat_is_refused_even_when_symbol_fails_the_second_time() {
    let (home, me) = unlocked();
    // `symbol()` answers once (the first run), then fails: the second run sees symbol "?".
    let symbol_ok = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let accepted = Arc::new(AtomicUsize::new(0));
    let (ok, a) = (symbol_ok.clone(), accepted.clone());
    let rpc =
        MockRpc::start(move |m, p| match m {
            "eth_getTransactionCount" => Reply::Result(hex(
                u128::from(PENDING_NONCE) + a.load(Ordering::SeqCst) as u128
            )),
            "eth_sendRawTransaction" => {
                a.fetch_add(1, Ordering::SeqCst);
                healthy(m, p)
            }
            "eth_call" => {
                let tx = &p[0];
                let data = tx["input"].as_str().or(tx["data"].as_str()).unwrap_or("");
                let word = |n: u128| format!("{n:064x}");
                match &data[..10.min(data.len())] {
                    "0x313ce567" => Reply::Result(json!(format!("0x{}", word(6)))),
                    "0x70a08231" => Reply::Result(json!(format!("0x{}", word(10u128.pow(12))))),
                    "0x95d89b41" if ok.swap(false, Ordering::SeqCst) => Reply::Result(json!(
                        format!("0x{}{}{:0<64}", word(32), word(4), "55534443")
                    )),
                    "0x95d89b41" => Reply::Error("execution reverted"),
                    other => panic!("mock: unexpected eth_call {other}"),
                }
            }
            _ => healthy(m, p),
        });
    let mut args = transfer(&rpc.url, &["--token", USDC]);
    args[6] = "0.5";
    let first = home.run(&args).ok();
    assert_eq!(first["symbol"], "USDC");
    let text = std::fs::read_to_string(journal_path(&home, &me)).unwrap();
    let entry: Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
    // The raw amount is journaled next to the display value.
    assert_eq!(entry["amountRaw"], "500000", "{entry}");
    assert_eq!(entry["value"], "0.5 USDC", "{entry}");
    // The same transfer again, but the node no longer answers symbol(): still a repeat.
    let r = home.run(&args);
    let v = r.json();
    assert_eq!(v["errorCode"], "VALIDATION_ERROR", "{v:#}");
    assert_eq!(v["duplicateSends"][0]["txHash"], first["txHash"], "{v:#}");
    assert_eq!(rpc.sent().len(), 1, "the repeat was broadcast");
}

#[test]
fn a_bogus_pending_count_is_reported_bounded_not_enumerated() {
    let (home, _) = unlocked();
    // A broken or hostile node: nothing mined, but half of u64 pending.
    let pending = u64::MAX / 2;
    let rpc = MockRpc::start(move |m, p| match m {
        "eth_getTransactionCount" if p[1] == "latest" => Reply::Result(hex(0)),
        "eth_getTransactionCount" => Reply::Result(hex(pending.into())),
        _ => healthy(m, p),
    });
    let check = |v: &Value| {
        let listed = v["pendingNonces"].as_array().unwrap();
        assert_eq!(listed.len(), 64, "{v:#}");
        assert_eq!(listed[0], 0);
        assert_eq!(listed[63], 63);
        assert_eq!(v["pendingCount"], pending, "{v:#}");
        assert_eq!(v["pendingRange"], json!([0, pending - 1]), "{v:#}");
    };
    let dry = home.run(&transfer(&rpc.url, &["--dry-run"])).ok();
    check(&dry);
    // Without --allow-pending the guard refuses, listing the same bounded fields.
    let r = home.run(&transfer(&rpc.url, &[]));
    let msg = r.err("VALIDATION_ERROR");
    assert!(
        msg.contains(&format!("at nonces 0 to {}", pending - 1)),
        "{msg}"
    );
    check(&r.json());
    // With it, the send goes out at the node's pending nonce.
    let v = home.run(&transfer(&rpc.url, &["--allow-pending"])).ok();
    check(&v);
    assert_eq!(v["nonce"], pending);
    assert_eq!(rpc.sent().len(), 1);
}

/// A mined receipt for `hash` in block 0x99 whose `status` is `status` (`None`: no status field,
/// as before Byzantium).
fn receipt(hash: &str, status: Option<&str>) -> Value {
    let mut r = json!({ "transactionHash": hash, "blockNumber": "0x99", "gasUsed": "0x5208" });
    if let Some(s) = status {
        r["status"] = json!(s);
    }
    r
}

/// A node that knows `pending` (unmined), `mined` (in block 0x99, with a [`receipt`] whose status
/// is `status`), and nothing else. Every transaction is from `from` to OTHER at nonce 7, 0.01 ETH.
fn lookup_node(from: &str, pending: &str, mined: &str, status: Option<&'static str>) -> MockRpc {
    let (from, pending, mined) = (from.to_lowercase(), pending.to_string(), mined.to_string());
    MockRpc::start(move |m, p| {
        let h = p[0].as_str().unwrap_or("");
        let tx = |block: Value| {
            json!({ "hash": h, "from": from, "to": OTHER.to_lowercase(), "nonce": "0x7",
                    "value": hex(10u128.pow(16)), "blockNumber": block, "gas": "0x5208",
                    "maxFeePerGas": "0x1", "maxPriorityFeePerGas": "0x1" })
        };
        match m {
            "eth_chainId" => Reply::Result(json!("0x1")),
            "eth_getTransactionByHash" if h == pending => Reply::Result(tx(Value::Null)),
            "eth_getTransactionByHash" if h == mined => Reply::Result(tx(json!("0x99"))),
            "eth_getTransactionReceipt" if h == mined => Reply::Result(receipt(h, status)),
            "eth_getTransactionByHash" | "eth_getTransactionReceipt" => Reply::Result(Value::Null),
            other => panic!("mock: unexpected {other}"),
        }
    })
}

#[test]
fn tx_looks_a_transaction_up_while_locked() {
    let home = Home::new();
    // A wallet that was never unlocked; `tx` needs no key at all.
    let me = home.create("t", &[])["address"]
        .as_str()
        .unwrap()
        .to_string();
    let (pending, mined, unknown) = (
        format!("0x{}", "a1".repeat(32)),
        format!("0x{}", "b2".repeat(32)),
        format!("0x{}", "c3".repeat(32)),
    );
    let rpc = lookup_node(&me, &pending, &mined, Some("0x1"));
    let tx = |hash: &str| {
        home.run(&[
            "tx", "--chain", "ethereum", hash, "--rpc", &rpc.url, "--json",
        ])
        .ok()
    };

    let v = tx(&mined);
    println!("{v:#}");
    assert_eq!(v["chain"], "ethereum");
    assert_eq!(v["txHash"], mined.as_str());
    assert_eq!(v["found"], true);
    assert_eq!(v["status"], "mined");
    assert_eq!(v["from"], me.as_str());
    assert_eq!(v["to"], OTHER);
    assert_eq!(v["nonce"], 7);
    assert_eq!(v["value"], "0.01");
    assert_eq!(v["valueRaw"], "10000000000000000");
    assert_eq!(v["blockNumber"], 0x99);
    assert_eq!(v["gasUsed"], 21000);
    assert_eq!(v["success"], true);
    assert_eq!(
        v["explorerUrl"],
        format!("https://etherscan.io/tx/{mined}").as_str()
    );
    assert!(v.get("hint").is_none(), "{v:#}");

    let v = tx(&pending);
    println!("{v:#}");
    assert_eq!(v["found"], true);
    assert_eq!(v["status"], "pending");
    assert_eq!(v["nonce"], 7);
    assert_eq!(v["from"], me.as_str());
    for k in ["blockNumber", "gasUsed", "success"] {
        assert!(v.get(k).is_none(), "{k}: {v:#}");
    }
    assert_eq!(
        v["hint"],
        format!(
            "still pending; to speed up or cancel use --nonce 7 --replace {pending}; a run \
             without --nonce is a second payment"
        )
        .as_str()
    );

    let v = tx(&unknown);
    println!("{v:#}");
    assert_eq!(v["found"], false);
    assert_eq!(v["status"], "not found");
    assert_eq!(v["txHash"], unknown.as_str());
    for k in ["from", "to", "nonce", "value"] {
        assert!(v[k].is_null(), "{k}: {v:#}");
    }
    assert_eq!(
        v["explorerUrl"],
        format!("https://etherscan.io/tx/{unknown}").as_str()
    );
    let hint = v["hint"].as_str().unwrap();
    for part in [
        "this node does not know the transaction",
        "that does not mean it was never sent",
        "Do not send this payment again without --nonce",
        "--nonce <the nonce it was signed with>",
    ] {
        assert!(hint.contains(part), "{part}: {hint}");
    }

    // A reverted transaction is mined, not successful.
    let reverted = lookup_node(&me, &pending, &mined, Some("0x0"));
    let v = home
        .run(&[
            "tx",
            "--chain",
            "ethereum",
            &mined,
            "--rpc",
            &reverted.url,
            "--json",
        ])
        .ok();
    assert_eq!(v["status"], "mined");
    assert_eq!(v["success"], false);

    // Human output, and no wallet at all.
    let bare = Home::new();
    let r = bare.run(&["tx", "--chain", "ethereum", &mined, "--rpc", &rpc.url]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(
        r.stdout.lines().any(|l| l == "status: mined"),
        "{}",
        r.stdout
    );
    // Nothing was ever sent.
    assert!(rpc.sent().is_empty() && reverted.sent().is_empty());
}

/// Receipt `status` values and the outcome they report: only `0x1` and `0x0` say anything. An
/// empty or unknown value, or none at all (pre-Byzantium), is unknown, never a failure.
const RECEIPT_STATUSES: [(Option<&str>, Option<bool>); 5] = [
    (Some("0x1"), Some(true)),
    (Some("0x0"), Some(false)),
    (Some(""), None),
    (Some("0x2"), None),
    (None, None),
];

/// `v` carries the hint for a mined transaction whose outcome is unknown.
fn assert_outcome_unknown_hint(v: &Value, hash: &str) {
    let hint = v["hint"]
        .as_str()
        .unwrap_or_else(|| panic!("no hint: {v:#}"));
    for part in [
        "the execution outcome is unknown",
        "do not send this payment again",
        "check the receipt on the explorer",
        &format!("https://etherscan.io/tx/{hash}"),
    ] {
        assert!(hint.contains(part), "{part}: {hint}");
    }
}

#[test]
fn tx_reads_the_outcome_only_from_receipt_status_0x0_or_0x1() {
    let home = Home::new();
    let (pending, mined) = (
        format!("0x{}", "a1".repeat(32)),
        format!("0x{}", "b2".repeat(32)),
    );
    for (status, outcome) in RECEIPT_STATUSES {
        let rpc = lookup_node(OTHER, &pending, &mined, status);
        let v = home
            .run(&[
                "tx", "--chain", "ethereum", &mined, "--rpc", &rpc.url, "--json",
            ])
            .ok();
        let case = format!("{status:?}: {v:#}");
        assert_eq!(v["status"], "mined", "{case}");
        // Present whenever there is a receipt; null when its status says nothing.
        assert_eq!(v.get("success"), Some(&json!(outcome)), "{case}");
        match outcome {
            Some(_) => assert!(v.get("hint").is_none(), "{case}"),
            None => assert_outcome_unknown_hint(&v, &mined),
        }
    }
}

#[test]
fn tx_mined_without_a_receipt_is_outcome_unknown() {
    let home = Home::new();
    let mined = format!("0x{}", "b2".repeat(32));
    // The transaction names its block, but this node has no receipt for it (yet).
    let m = mined.clone();
    let rpc = MockRpc::start(move |method, p| match method {
        "eth_chainId" => Reply::Result(json!("0x1")),
        "eth_getTransactionByHash" if p[0] == m.as_str() => Reply::Result(json!({
            "hash": m, "from": OTHER.to_lowercase(), "to": OTHER.to_lowercase(), "nonce": "0x7",
            "value": "0x0", "blockNumber": "0x99",
        })),
        "eth_getTransactionByHash" | "eth_getTransactionReceipt" => Reply::Result(Value::Null),
        other => panic!("mock: unexpected {other}"),
    });
    let v = home
        .run(&[
            "tx", "--chain", "ethereum", &mined, "--rpc", &rpc.url, "--json",
        ])
        .ok();
    assert_eq!(v["status"], "mined", "{v:#}");
    assert_eq!(v["blockNumber"], 0x99, "{v:#}");
    assert_eq!(v.get("success"), Some(&Value::Null), "{v:#}");
    assert_outcome_unknown_hint(&v, &mined);
    assert!(v["hint"].as_str().unwrap().contains("no receipt"), "{v:#}");
}

#[test]
fn wait_reads_the_outcome_only_from_receipt_status_0x0_or_0x1() {
    let (home, _) = unlocked();
    for (status, outcome) in RECEIPT_STATUSES {
        let rpc = MockRpc::start(move |m, p| match m {
            "eth_getTransactionReceipt" => Reply::Result(receipt(p[0].as_str().unwrap(), status)),
            _ => healthy(m, p),
        });
        // The same payment each time, on a fresh node.
        let v = home
            .run(&transfer(&rpc.url, &["--wait", "--allow-duplicate"]))
            .ok();
        let hash = tx_hash_of(&rpc.sent()[0]);
        let case = format!("{status:?}: {v:#}");
        assert_eq!(v["txHash"], hash.as_str(), "{case}");
        let shown = match outcome {
            Some(true) => "success",
            Some(false) => "reverted",
            None => "mined",
        };
        assert_eq!(v["status"], shown, "{case}");
        assert_eq!(v.get("success"), Some(&json!(outcome)), "{case}");
        assert_eq!(v["blockNumber"], 0x99, "{case}");
        assert_eq!(v["gasUsed"], 21000, "{case}");
        assert_eq!(v["nonce"], PENDING_NONCE, "{case}");
        match outcome {
            Some(_) => assert!(v.get("hint").is_none(), "{case}"),
            None => assert_outcome_unknown_hint(&v, &hash),
        }
    }
}

#[test]
fn wait_with_a_receipt_for_another_transaction_reports_no_outcome() {
    let (home, _) = unlocked();
    let other = format!("0x{}", "dd".repeat(32));
    // Success or reverted, a receipt about another transaction says nothing about this one.
    for status in ["0x1", "0x0"] {
        let o = other.clone();
        let rpc = MockRpc::start(move |m, p| match m {
            "eth_getTransactionReceipt" => Reply::Result(receipt(&o, Some(status))),
            _ => healthy(m, p),
        });
        let v = home
            .run(&transfer(&rpc.url, &["--wait", "--allow-duplicate"]))
            .ok();
        let hash = tx_hash_of(&rpc.sent()[0]);
        let case = format!("{status}: {v:#}");
        assert!(
            rpc.calls("eth_getTransactionReceipt")
                .contains(&json!([hash])),
            "{case}"
        );
        assert_eq!(v["txHash"], hash.as_str(), "{case}");
        assert_eq!(v["nonce"], PENDING_NONCE, "{case}");
        assert_eq!(v["status"], "unknown", "{case}");
        for k in ["success", "blockNumber", "gasUsed"] {
            assert!(v.get(k).is_none(), "{k}: {case}");
        }
        let why = v["receiptError"].as_str().unwrap();
        assert!(
            why.contains(&format!("answered about transaction {other}")),
            "{case}"
        );
        let hint = v["hint"].as_str().unwrap();
        assert!(
            hint.starts_with("receipt unknown, it may still be pending"),
            "{case}"
        );
    }
}

#[test]
fn tx_refuses_a_bad_hash_and_a_node_answering_another_transaction() {
    let home = Home::new();
    // Not a hash: refused before any RPC (the port is closed).
    for bad in ["0x1234", "abcd", "0xzz"] {
        home.run(&[
            "tx",
            "--chain",
            "ethereum",
            bad,
            "--rpc",
            "http://127.0.0.1:1",
            "--json",
        ])
        .err("VALIDATION_ERROR");
    }
    // A node that answers with another transaction cannot say anything about this one.
    let other = format!("0x{}", "dd".repeat(32));
    let o = other.clone();
    let rpc = MockRpc::start(move |m, _| match m {
        "eth_chainId" => Reply::Result(json!("0x1")),
        "eth_getTransactionByHash" => Reply::Result(json!({
            "hash": o, "from": OTHER, "to": OTHER, "nonce": "0x1", "value": "0x0",
            "blockNumber": "0x1",
        })),
        "eth_getTransactionReceipt" => Reply::Result(Value::Null),
        other => panic!("mock: unexpected {other}"),
    });
    let wanted = format!("0x{}", "ee".repeat(32));
    let msg = home
        .run(&[
            "tx", "--chain", "ethereum", &wanted, "--rpc", &rpc.url, "--json",
        ])
        .err("RPC_ERROR");
    assert!(msg.contains(&other), "{msg}");
}

/// The words that must never follow from a mined nonce alone: the transaction there may have
/// reverted and paid nothing.
const WENT_THROUGH: &str = "went through";

#[test]
fn replacing_a_mined_transaction_says_only_that_its_nonce_is_consumed() {
    let (home, me) = unlocked();
    let mined = format!("0x{}", "9a".repeat(32));
    // Reverted (0x0) is mined and paid nothing; unknown is mined with no outcome.
    for (status, outcome) in RECEIPT_STATUSES.iter() {
        let (m, from, status) = (mined.clone(), me.clone(), *status);
        let inner = pending_node(&me, healthy);
        let rpc = MockRpc::start(move |method, p| match method {
            "eth_getTransactionByHash" if p[0] == m.as_str() => Reply::Result(json!({
                "hash": m, "from": from.to_lowercase(), "nonce": "0x6", "blockNumber": "0x99",
                "maxFeePerGas": "0x1", "maxPriorityFeePerGas": "0x1",
            })),
            "eth_getTransactionReceipt" if p[0] == m.as_str() => Reply::Result(receipt(&m, status)),
            _ => inner(method, p),
        });
        let v = home
            .run(&transfer(
                &rpc.url,
                &["--nonce", "6", "--replace", &mined, "--dry-run"],
            ))
            .json();
        let case = format!("{status:?}: {v:#}");
        assert_eq!(v["errorCode"], "VALIDATION_ERROR", "{case}");
        let msg = v["error"].as_str().unwrap();
        assert_eq!(
            msg,
            format!(
                "--replace {mined}: the transaction at nonce 6 is already mined (nonce consumed); \
                 whether the payment succeeded is not known from this alone: look up its receipt \
                 with `zwallet tx --chain ethereum {mined}` or https://etherscan.io/tx/{mined} \
                 before sending anything new; nothing was signed"
            ),
            "{case}"
        );
        assert!(!msg.contains(WENT_THROUGH), "{case}");
        // The receipt says how it executed.
        assert_eq!(v["minedTxHash"], mined.as_str(), "{case}");
        assert_eq!(v.get("success"), Some(&json!(outcome)), "{case}");
        assert!(rpc.sent().is_empty());
    }
}

#[test]
fn replacing_a_mined_transaction_whose_receipt_cannot_be_read_names_no_outcome() {
    let (home, me) = unlocked();
    let mined = format!("0x{}", "9a".repeat(32));
    let other = format!("0x{}", "dd".repeat(32));
    // The receipt call fails, or answers about another transaction (`Some`): no receipt either way.
    for broken in [None, Some(other)] {
        let (m, from) = (mined.clone(), me.clone());
        let inner = pending_node(&me, healthy);
        let reply = broken.clone();
        let rpc = MockRpc::start(move |method, p| match method {
            "eth_getTransactionByHash" if p[0] == m.as_str() => Reply::Result(json!({
                "hash": m, "from": from.to_lowercase(), "nonce": "0x6", "blockNumber": "0x99",
                "maxFeePerGas": "0x1", "maxPriorityFeePerGas": "0x1",
            })),
            "eth_getTransactionReceipt" => match &reply {
                None => Reply::Error("receipts unavailable"),
                Some(o) => Reply::Result(receipt(o, Some("0x1"))),
            },
            _ => inner(method, p),
        });
        let v = home
            .run(&transfer(
                &rpc.url,
                &["--nonce", "6", "--replace", &mined, "--dry-run"],
            ))
            .json();
        let case = format!("{broken:?}: {v:#}");
        // The receipt was asked for, and its failure only leaves the outcome out.
        assert!(
            rpc.calls("eth_getTransactionReceipt")
                .contains(&json!([mined])),
            "{case}"
        );
        assert_eq!(v["errorCode"], "VALIDATION_ERROR", "{case}");
        assert_eq!(
            v["error"],
            format!(
                "--replace {mined}: the transaction at nonce 6 is already mined (nonce consumed); \
                 whether the payment succeeded is not known from this alone: look up its receipt \
                 with `zwallet tx --chain ethereum {mined}` or https://etherscan.io/tx/{mined} \
                 before sending anything new; nothing was signed"
            )
            .as_str(),
            "{case}"
        );
        for k in ["minedTxHash", "success"] {
            assert!(v.get(k).is_none(), "{k}: {case}");
        }
        assert!(rpc.sent().is_empty());
    }
}

/// A node that mines every transaction it accepts at once, as `from`'s at its nonce, with a
/// [`receipt`] whose status is the one in `status` at the time of the lookup.
fn instant_mining_node(from: &str, status: Arc<Mutex<Option<&'static str>>>) -> MockRpc {
    let from = from.to_lowercase();
    // The raw transactions accepted so far.
    let accepted: Arc<Mutex<Vec<String>>> = Arc::default();
    MockRpc::start(move |m, p| {
        let h = p[0].as_str().unwrap_or("").to_string();
        let raw = accepted
            .lock()
            .unwrap()
            .iter()
            .find(|r| tx_hash_of(r) == h)
            .cloned();
        match (m, raw) {
            ("eth_getTransactionCount", _) => Reply::Result(hex(
                u128::from(PENDING_NONCE) + accepted.lock().unwrap().len() as u128
            )),
            ("eth_sendRawTransaction", _) => {
                accepted.lock().unwrap().push(h);
                healthy(m, p)
            }
            ("eth_getTransactionByHash", Some(raw)) => Reply::Result(json!({
                "hash": h, "from": from, "nonce": hex(nonce_of(&raw).into()),
                "blockNumber": "0x99", "maxFeePerGas": "0x1", "maxPriorityFeePerGas": "0x1",
            })),
            ("eth_getTransactionReceipt", Some(_)) => {
                Reply::Result(receipt(&h, *status.lock().unwrap()))
            }
            _ => healthy(m, p),
        }
    })
}

#[test]
fn a_mined_nonce_reports_its_receipt_never_that_the_payment_went_through() {
    let (home, me) = unlocked();
    let status = Arc::new(Mutex::new(Some("0x0")));
    let rpc = instant_mining_node(&me, status.clone());
    let sent = home.run(&transfer(&rpc.url, &[])).ok();
    assert_eq!(sent["nonce"], PENDING_NONCE);
    let hash = sent["txHash"].as_str().unwrap().to_string();
    // Nonce 7 is mined now. Reverted (0x0): the nonce is used, the payment was not made.
    for (s, outcome) in RECEIPT_STATUSES.iter() {
        *status.lock().unwrap() = *s;
        // The hash comes from the journal's entry at 7, or from --replace.
        for extra in [vec![], vec!["--replace", hash.as_str()]] {
            let mut args = vec!["--nonce", "7", "--dry-run"];
            args.extend(extra);
            let v = home.run(&transfer(&rpc.url, &args)).json();
            let case = format!("{s:?} {args:?}: {v:#}");
            assert_eq!(v["errorCode"], "VALIDATION_ERROR", "{case}");
            let msg = v["error"].as_str().unwrap();
            assert!(
                msg.starts_with(&format!(
                    "the transaction at nonce 7 is already mined (nonce consumed); whether the \
                     payment succeeded is not known from this alone: look up its receipt with \
                     `zwallet tx --chain ethereum {hash}` or https://etherscan.io/tx/{hash} \
                     before sending anything new; nothing was signed. If the transaction at \
                     nonce 7 is a different payment"
                )),
                "{case}"
            );
            assert!(!msg.contains(WENT_THROUGH), "{case}");
            assert_eq!(v["nonce"], 7, "{case}");
            assert_eq!(v["minedTxHash"], hash.as_str(), "{case}");
            assert_eq!(v.get("success"), Some(&json!(outcome)), "{case}");
        }
    }
    // A mined nonce no known hash is at (the journal has nothing at 6; the --replace hash is at
    // 7, so its receipt is not this nonce's) says so too, and where to find the hash.
    for extra in [vec![], vec!["--replace", hash.as_str()]] {
        let mut args = vec!["--nonce", "6", "--dry-run"];
        args.extend(extra);
        let v = home.run(&transfer(&rpc.url, &args)).json();
        let msg = v["error"].as_str().unwrap();
        assert!(
            msg.starts_with(
                "the transaction at nonce 6 is already mined (nonce consumed); whether the \
                 payment succeeded is not known from this alone: look up its receipt with \
                 `zwallet tx --chain ethereum <its txHash>` before sending anything new; nothing \
                 was signed."
            ),
            "{args:?}: {v:#}"
        );
        assert!(msg.contains("has no entry at nonce 6"), "{msg}");
        assert!(
            v.get("minedTxHash").is_none() && v.get("success").is_none(),
            "{args:?}: {v:#}"
        );
    }
    assert_eq!(rpc.sent().len(), 1);
}

#[test]
fn a_mined_nonce_whose_receipt_cannot_be_read_names_no_outcome() {
    let (home, me) = unlocked();
    // A send at nonce 7, in the journal.
    let first = MockRpc::start(healthy);
    let sent = home.run(&transfer(&first.url, &[])).ok();
    assert_eq!(sent["nonce"], PENDING_NONCE);
    let hash = sent["txHash"].as_str().unwrap().to_string();
    // Nonce 7 is mined now, as this key's, but its receipt cannot be read.
    let (h, from) = (hash.clone(), me.to_lowercase());
    let rpc = MockRpc::start(move |m, p| match m {
        "eth_getTransactionCount" => Reply::Result(hex(u128::from(PENDING_NONCE) + 1)),
        "eth_getTransactionByHash" if p[0] == h.as_str() => Reply::Result(json!({
            "hash": h, "from": from, "nonce": hex(PENDING_NONCE.into()), "blockNumber": "0x99",
            "maxFeePerGas": "0x1", "maxPriorityFeePerGas": "0x1",
        })),
        "eth_getTransactionReceipt" => Reply::Error("receipts unavailable"),
        _ => healthy(m, p),
    });
    // The hash comes from the journal's entry at 7, or from --replace.
    for extra in [vec![], vec!["--replace", hash.as_str()]] {
        let before = rpc.calls("eth_getTransactionReceipt").len();
        let mut args = vec!["--nonce", "7", "--dry-run"];
        args.extend(extra);
        let v = home.run(&transfer(&rpc.url, &args)).json();
        let case = format!("{args:?}: {v:#}");
        // The receipt was asked for, and its failure only leaves the outcome out.
        let asked = rpc.calls("eth_getTransactionReceipt");
        assert!(asked[before..].contains(&json!([hash])), "{case}");
        assert_eq!(v["errorCode"], "VALIDATION_ERROR", "{case}");
        let msg = v["error"].as_str().unwrap();
        assert!(
            msg.starts_with(
                "the transaction at nonce 7 is already mined (nonce consumed); whether the \
                 payment succeeded is not known from this alone: look up its receipt with \
                 `zwallet tx --chain ethereum <its txHash>` before sending anything new; nothing \
                 was signed."
            ),
            "{case}"
        );
        assert!(!msg.contains(WENT_THROUGH), "{case}");
        // The journal still names the transaction there, to look up.
        assert!(
            msg.contains(&format!("has at nonce 7: {hash} (nonce 7")),
            "{case}"
        );
        assert_eq!(v["nonce"], 7, "{case}");
        for k in ["minedTxHash", "success"] {
            assert!(v.get(k).is_none(), "{k}: {case}");
        }
    }
    assert!(rpc.sent().is_empty());
}

// Whitelist enforcement (`.prd/04-whitelist.md`): the daemon decides every signature from its
// own decoding of the bytes against the list that verifies against the sealed anchor. Denials are
// tested on live sends: a dry-run may be stopped by the CLI before the daemon is asked.

/// The EIP-55 example addresses: payees in these tests.
const ALICE: &str = "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed";
const BOB: &str = "0xfB6916095ca1df60bB79Ce92cE3Ea74c37c5d359";
/// A contract that is no registry token, and a spender.
const ROUTER: &str = "0x2222222222222222222222222222222222222222";
const SPENDER: &str = "0x3333333333333333333333333333333333333333";
/// Registry tokens on Ethereum besides USDC.
const USDT: &str = "0xdAC17F958D2ee523a2206206994597C13D831ec7";
/// `deposit()`.
const DEPOSIT: &str = "0xd0e30db0";

/// A new wallet whose whitelist is one signed edit (`change` on an empty list), then unlocked for
/// 10 minutes: `(home, the active key's address)`.
fn listed(change: impl FnOnce(&mut Whitelist)) -> (Home, String) {
    let home = Home::new();
    home.create("t", &[]);
    home.edit_whitelist(change);
    let me = home.unlock("10m")["address"].as_str().unwrap().to_string();
    (home, me)
}

fn payee(list: &mut Whitelist, nickname: &str, address: &str, chains: &[&str]) {
    list.add(Entry::new_address(nickname, address, chains))
        .unwrap();
}

fn contract(list: &mut Whitelist, nickname: &str, address: &str, chains: &[&str], sels: &[&str]) {
    list.add(Entry::new_contract(nickname, address, chains, sels))
        .unwrap();
}

/// [`healthy`] serving chain `id`, with the OP-stack GasPriceOracle's `getL1Fee` answering 1 gwei.
fn healthy_on(id: u64) -> impl Fn(&str, &Value) -> Reply + Send + Sync + 'static {
    move |m, p| match m {
        "eth_chainId" => Reply::Result(hex(id.into())),
        "eth_call" => {
            let data = p[0]["input"]
                .as_str()
                .or(p[0]["data"].as_str())
                .unwrap_or("");
            assert!(
                data.starts_with("0x49948e0e"),
                "mock: unexpected eth_call {data}"
            );
            Reply::Result(json!(format!("0x{GWEI:064x}")))
        }
        _ => healthy(m, p),
    }
}

/// `transfer` of 0.01 native to `to` on `chain`.
fn pay<'a>(chain: &'a str, to: &'a str, rpc: &'a str, extra: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec![
        "transfer", "--chain", chain, "--to", to, "--amount", "0.01", "--rpc", rpc, "--json",
    ];
    args.extend_from_slice(extra);
    args
}

/// `invoke` on Ethereum: `to`, then either `--data <hex>` or `--sig <sig> <args…>`.
fn invoke<'a>(to: &'a str, call: &[&'a str], rpc: &'a str, extra: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec!["invoke", "--chain", "ethereum", "--to", to];
    args.extend_from_slice(call);
    args.extend_from_slice(&["--rpc", rpc, "--json"]);
    args.extend_from_slice(extra);
    args
}

/// Asserts a daemon refusal with `code` whose `command` detail (when `command` is `Some`) is
/// that exact command and appears in the message. Returns the error object.
fn refused(
    r: &common::Run,
    code: &str,
    counterparty: &str,
    chain: &str,
    command: Option<&str>,
) -> Value {
    let msg = r.err(code);
    let v = r.json();
    assert_eq!(v["counterparty"], counterparty, "{v:#}");
    assert_eq!(v["chain"], chain, "{v:#}");
    match command {
        Some(c) => {
            assert_eq!(v["command"], c, "{v:#}");
            assert!(msg.contains(c), "{msg}");
        }
        None => assert!(v.get("command").is_none(), "{v:#}"),
    }
    assert!(msg.starts_with("refusing to sign"), "{msg}");
    v
}

#[test]
fn w2_a_send_to_an_unlisted_address_is_denied_and_nothing_is_signed() {
    let (home, me) = listed(|l| payee(l, "alice", ALICE, &["ethereum"]));
    let rpc = MockRpc::start(healthy);
    let r = home.run(&pay("ethereum", OTHER, &rpc.url, &[]));
    let cmd = format!("zwallet whitelist add <nick> {OTHER} --chains ethereum");
    refused(&r, "WHITELIST_DENIED", OTHER, "ethereum", Some(&cmd));
    assert!(rpc.sent().is_empty(), "a denied send was broadcast");
    assert!(
        !journal_path(&home, &me).exists(),
        "a denied send was journaled"
    );
    // Without --json: the message, then what it is about and the command, one per line.
    let mut human = pay("ethereum", OTHER, &rpc.url, &[]);
    human.retain(|a| *a != "--json");
    let r = home.run(&human);
    assert_eq!(r.code, 1);
    let lines: Vec<&str> = r.stderr.lines().collect();
    assert!(
        lines[0].starts_with("error: refusing to sign: ")
            && lines[0].ends_with(" [WHITELIST_DENIED]"),
        "{}",
        r.stderr
    );
    assert_eq!(
        lines[1..],
        [
            format!("counterparty: {OTHER}"),
            "chain: ethereum".to_string(),
            format!("command: {cmd}"),
        ],
        "{}",
        r.stderr
    );
    // The listed payee on the same wallet goes through.
    home.run(&pay("ethereum", ALICE, &rpc.url, &[])).ok();
    assert_eq!(rpc.sent().len(), 1);
}

#[test]
fn w3_a_payee_is_payable_only_on_its_chains() {
    let (home, me) = listed(|l| payee(l, "alice", ALICE, &["base"]));
    let base = MockRpc::start(healthy_on(8453));
    let v = home.run(&pay("base", ALICE, &base.url, &[])).ok();
    assert_eq!(v["signerDecoded"]["action"]["to"], ALICE, "{v:#}");
    assert_eq!(base.sent().len(), 1, "signed and sent on base");
    let journal = home
        .path()
        .join(format!("sends-8453-{}.jsonl", me.to_lowercase()));
    assert!(journal.exists());
    // The same payee on Ethereum: listed, but not for this chain.
    let eth = MockRpc::start(healthy);
    let r = home.run(&pay("ethereum", ALICE, &eth.url, &[]));
    // Her entry and the chain it lacks are named; no command (an entry changes by a remove and
    // an add, which the human chooses).
    let v = refused(&r, "WHITELIST_DENIED", ALICE, "ethereum", None);
    let msg = v["error"].as_str().unwrap();
    assert!(
        msg.contains("'alice'") && msg.contains("not ethereum"),
        "{msg}"
    );
    assert!(eth.sent().is_empty());
}

#[test]
fn w5_a_list_that_does_not_verify_refuses_every_send() {
    let home = Home::new();
    home.create("t", &[]);
    home.allow("alice", ALICE, &["*"]);
    let gen1 = std::fs::read(home.path().join("whitelist.json")).unwrap();
    home.allow("bob", BOB, &["*"]);
    let gen2 = std::fs::read(home.path().join("whitelist.json")).unwrap();
    // whitelist.json alone swapped back to the older signed copy.
    std::fs::write(home.path().join("whitelist.json"), &gen1).unwrap();
    let me = home.unlock("10m")["address"].as_str().unwrap().to_string();
    let rpc = mining_node();
    for to in [ALICE, BOB] {
        let r = home.run(&pay("ethereum", to, &rpc.url, &[]));
        // No edit is suggested for a list that does not verify: the human checks it first.
        let v = refused(&r, "WHITELIST_INVALID", to, "ethereum", None);
        let msg = v["error"].as_str().unwrap();
        assert!(
            msg.contains("generation 1 is not the sealed generation 2"),
            "{msg}"
        );
        assert!(msg.contains("`zwallet whitelist list --verify`"), "{msg}");
    }
    // A hand edit of the current list (an entry added, the signature kept): the same.
    let current = home.path().join("whitelist.json");
    std::fs::write(&current, &gen2).unwrap(); // an edit never builds on the swapped file
    let mut v: Value = serde_json::from_slice(&std::fs::read(&current).unwrap()).unwrap();
    v["entries"].as_array_mut().unwrap().push(json!({
        "nickname": "mallory", "kind": "address", "address": SPENDER, "chains": ["*"],
    }));
    std::fs::write(&current, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    home.unlock("10m");
    let r = home.run(&pay("ethereum", SPENDER, &rpc.url, &[]));
    let v = refused(&r, "WHITELIST_INVALID", SPENDER, "ethereum", None);
    assert!(
        v["error"].as_str().unwrap().contains("does not verify"),
        "{v:#}"
    );
    assert!(rpc.sent().is_empty(), "a send went out on an invalid list");
    // Not even the signing key's own address is payable on a list that does not verify.
    let r = home.run(&pay("ethereum", &me, &rpc.url, &[]));
    refused(&r, "WHITELIST_INVALID", &me, "ethereum", None);
    assert!(rpc.sent().is_empty());
}

#[test]
fn w5b_an_older_wallet_and_list_restored_together_are_accepted_and_shown() {
    let home = Home::new();
    home.create("t", &[]);
    home.allow("alice", ALICE, &["*"]);
    // Two files: the key is derived from the password and the salt the older wallet.json seals.
    let files = ["wallet.json", "whitelist.json"];
    let old: Vec<Vec<u8>> = files
        .iter()
        .map(|f| std::fs::read(home.path().join(f)).unwrap())
        .collect();
    home.allow("bob", BOB, &["*"]);
    for (f, bytes) in files.iter().zip(&old) {
        std::fs::write(home.path().join(f), bytes).unwrap();
    }
    // Accepted, as documented; unlock shows the older generation and its entries.
    let unlocked = home.unlock("10m");
    assert_eq!(
        unlocked["whitelist"],
        json!({ "gen": 1, "verified": true, "entries": ["alice"] }),
        "{unlocked:#}"
    );
    let rpc = mining_node();
    home.run(&pay("ethereum", ALICE, &rpc.url, &[])).ok();
    let r = home.run(&pay("ethereum", BOB, &rpc.url, &[]));
    let cmd = format!("zwallet whitelist add <nick> {BOB} --chains ethereum");
    refused(&r, "WHITELIST_DENIED", BOB, "ethereum", Some(&cmd));
    assert_eq!(rpc.sent().len(), 1);
}

#[test]
fn w6_approvals_are_revoke_only_and_transfer_from_is_never_signed() {
    let (home, me) = listed(|l| payee(l, "alice", ALICE, &["*"]));
    let rpc = mining_node();
    let approve = |amount: &'static str| {
        invoke(
            USDC,
            &["--sig", "approve(address,uint256)", SPENDER, amount],
            &rpc.url,
            &[],
        )
    };
    let r = home.run(&approve("1"));
    refused(&r, "WHITELIST_DENIED", SPENDER, "ethereum", None);
    let r = home.run(&invoke(
        USDC,
        &[
            "--sig",
            "transferFrom(address,address,uint256)",
            ALICE,
            &me,
            "1",
        ],
        &rpc.url,
        &[],
    ));
    refused(&r, "WHITELIST_DENIED", USDC, "ethereum", None);
    // An approval on a token outside the registry, even of 0.
    let r = home.run(&invoke(
        ROUTER,
        &["--sig", "approve(address,uint256)", SPENDER, "0"],
        &rpc.url,
        &[],
    ));
    refused(&r, "WHITELIST_DENIED", ROUTER, "ethereum", None);
    assert!(rpc.sent().is_empty());
    // Revoking on a registry token: signed and sent, whatever the spender.
    let v = home.run(&approve("0")).ok();
    assert_eq!(
        v["signerDecoded"]["action"]["kind"], "approve-call",
        "{v:#}"
    );
    assert_eq!(rpc.sent().len(), 1);
}

#[test]
fn w6_contract_calls_need_a_listed_contract_and_selector() {
    let (home, _) = listed(|l| {
        payee(l, "alice", ALICE, &["*"]);
        contract(l, "router", ROUTER, &["ethereum"], &["0x12345678"]);
    });
    let rpc = mining_node();
    // An unlisted contract.
    let r = home.run(&invoke(BOB, &["--data", DEPOSIT], &rpc.url, &[]));
    let cmd = format!(
        "zwallet whitelist add <nick> {BOB} --chains ethereum --contract --selectors {DEPOSIT}"
    );
    refused(&r, "WHITELIST_DENIED", BOB, "ethereum", Some(&cmd));
    // A payee is no contract entry.
    let r = home.run(&invoke(ALICE, &["--data", DEPOSIT], &rpc.url, &[]));
    refused(&r, "WHITELIST_DENIED", ALICE, "ethereum", None);
    // A listed contract with a selector it does not list.
    let r = home.run(&invoke(ROUTER, &["--data", DEPOSIT], &rpc.url, &[]));
    let v = refused(&r, "WHITELIST_DENIED", ROUTER, "ethereum", None);
    let msg = v["error"].as_str().unwrap();
    assert!(
        msg.contains("'router'") && msg.contains(&format!("does not list selector {DEPOSIT}")),
        "{msg}"
    );
    // Any other call on a registry token, whatever the list says.
    let r = home.run(&invoke(USDC, &["--data", DEPOSIT], &rpc.url, &[]));
    refused(&r, "WHITELIST_DENIED", USDC, "ethereum", None);
    assert!(rpc.sent().is_empty());
    // The listed selector, with native value: signed and sent.
    let v = home
        .run(&invoke(
            ROUTER,
            &["--data", "0x12345678", "--value", "0.001"],
            &rpc.url,
            &[],
        ))
        .ok();
    assert_eq!(
        v["signerDecoded"]["action"]["kind"], "contract-call",
        "{v:#}"
    );
    assert_eq!(rpc.sent().len(), 1);
}

#[test]
fn w6_token_transfers_need_a_registry_token_and_a_payee_that_is_no_token() {
    // USDT listed as a payee (allowed in the list), and still never a token transfer's target.
    let (home, _) = listed(|l| {
        payee(l, "alice", ALICE, &["*"]);
        payee(l, "usdt", USDT, &["ethereum"]);
    });
    let rpc = mining_node();
    let token = |t: &'static str, to: &'static str| {
        let mut a = pay("ethereum", to, &rpc.url, &["--token", t]);
        a[6] = "0.5";
        a
    };
    // A contract outside the registry, to a listed payee.
    let r = home.run(&token(ROUTER, ALICE));
    refused(&r, "WHITELIST_DENIED", ROUTER, "ethereum", None);
    // A registry token to another token contract.
    let r = home.run(&token(USDC, USDT));
    refused(&r, "WHITELIST_DENIED", USDT, "ethereum", None);
    // A registry token to an unlisted address.
    let r = home.run(&token(USDC, BOB));
    let cmd = format!("zwallet whitelist add <nick> {BOB} --chains ethereum");
    refused(&r, "WHITELIST_DENIED", BOB, "ethereum", Some(&cmd));
    assert!(rpc.sent().is_empty());
    let v = home.run(&token(USDC, ALICE)).ok();
    assert_eq!(
        v["signerDecoded"]["action"]["kind"], "transfer-call",
        "{v:#}"
    );
    assert_eq!(rpc.sent().len(), 1);
}

#[test]
fn w9_fees_above_the_cap_or_without_a_cap_are_refused_by_the_daemon() {
    let (home, me) = listed(|l| {
        payee(l, "alice", ALICE, &["*"]);
        l.policy.max_fee_wei.remove("arbitrum");
    });
    // feeHistory claims 500 gwei tips; --max-fee-gwei lets the CLI build that.
    let greedy = MockRpc::start(|m, p| fee_node(m, p, Some(GWEI), 500 * GWEI, GWEI + GWEI / 10));
    let over = |to: &str| {
        let r = home.run(&pay(
            "ethereum",
            to,
            &greedy.url,
            &["--max-fee-gwei", "1000.5"],
        ));
        let v = r.json();
        let fee: u128 = v["feeWei"]
            .as_str()
            .unwrap_or_else(|| panic!("{v:#}"))
            .parse()
            .unwrap();
        assert!(fee > 3 * 10u128.pow(15), "{v:#}");
        assert_eq!(v["capWei"], "3000000000000000", "{v:#}");
        let amount = zwallet::core::amount::format_units(alloy::primitives::U256::from(fee), 18);
        let cmd = format!("zwallet whitelist fee-cap ethereum {amount}");
        refused(&r, "FEE_CAP_EXCEEDED", to, "ethereum", Some(&cmd));
    };
    over(ALICE);
    // Self-transfers are under the cap too while a list verifies.
    over(&me);
    // The action comes first: over the cap, an unlisted payee is denied with the add command
    // and a forbidden action with none; neither names a fee-cap.
    let cap = ["--max-fee-gwei", "1000.5"];
    let r = home.run(&pay("ethereum", OTHER, &greedy.url, &cap));
    let add = format!("zwallet whitelist add <nick> {OTHER} --chains ethereum");
    let v = refused(&r, "WHITELIST_DENIED", OTHER, "ethereum", Some(&add));
    assert!(v.get("feeWei").is_none(), "{v:#}");
    let r = home.run(&invoke(
        USDC,
        &["--sig", "approve(address,uint256)", SPENDER, "1"],
        &greedy.url,
        &cap,
    ));
    let v = refused(&r, "WHITELIST_DENIED", SPENDER, "ethereum", None);
    assert!(!v["error"].as_str().unwrap().contains("fee-cap"), "{v:#}");
    assert!(v.get("feeWei").is_none(), "{v:#}");
    assert!(greedy.sent().is_empty());
    // No cap for the chain: refused, naming the command that sets one.
    let arb = MockRpc::start(healthy_on(42161));
    let r = home.run(&pay("arbitrum", ALICE, &arb.url, &[]));
    let v = r.json();
    assert_eq!(v["errorCode"], "FEE_CAP_EXCEEDED", "{v:#}");
    let msg = v["error"].as_str().unwrap();
    assert!(msg.contains("no fee cap for arbitrum"), "{msg}");
    assert!(
        v["command"]
            .as_str()
            .unwrap()
            .starts_with("zwallet whitelist fee-cap arbitrum "),
        "{v:#}"
    );
    assert!(arb.sent().is_empty());
}

#[test]
fn w11_self_transfers_need_no_list() {
    let (home, me) = unlocked_without_list();
    let rpc = mining_node();
    let mut zero = pay("ethereum", &me, &rpc.url, &[]);
    zero[6] = "0";
    let dry = {
        let mut a = zero.clone();
        a.push("--dry-run");
        home.run(&a).ok()
    };
    assert_eq!(dry["recoveredFrom"], me.as_str());
    home.run(&zero).ok();
    assert_eq!(rpc.sent().len(), 1, "the self-transfer was not sent");
    // Anyone else: refused until a human signs a first list, with the command that does.
    let r = home.run(&pay("ethereum", OTHER, &rpc.url, &[]));
    let cmd = format!("zwallet whitelist add <nick> {OTHER} --chains ethereum");
    let v = refused(&r, "WHITELIST_INVALID", OTHER, "ethereum", Some(&cmd));
    assert!(
        v["error"].as_str().unwrap().contains("no whitelist yet"),
        "{v:#}"
    );
    assert_eq!(rpc.sent().len(), 1);
}

#[test]
fn a_malformed_sealed_anchor_unlocks_and_signs_nothing() {
    let home = Home::new();
    home.create("t", &[]);
    // A sealed policy that names a generation but no key or digest.
    let lock = zwallet::core::store::MetaLock::acquire(home.path()).unwrap();
    let opened = lock.open(common::PW).unwrap();
    lock.reseal(&opened, &json!({ "whitelistGen": 1 })).unwrap();
    drop(lock);
    let unlocked = home.unlock("10m");
    let w = &unlocked["whitelist"];
    assert_eq!(w["verified"], false, "{unlocked:#}");
    assert!(
        w["reason"].as_str().unwrap().contains("malformed"),
        "{unlocked:#}"
    );
    let me = unlocked["address"].as_str().unwrap().to_string();
    let rpc = mining_node();
    let r = home.run(&pay("ethereum", ALICE, &rpc.url, &[]));
    let v = refused(&r, "WHITELIST_INVALID", ALICE, "ethereum", None);
    assert!(
        v["error"]
            .as_str()
            .unwrap()
            .contains("`zwallet whitelist list --verify`"),
        "{v:#}"
    );
    // An anchor that cannot be read is an untrusted list: the key's own address too.
    let r = home.run(&pay("ethereum", &me, &rpc.url, &[]));
    refused(&r, "WHITELIST_INVALID", &me, "ethereum", None);
    assert!(rpc.sent().is_empty());
}

#[test]
fn a_whitelist_sealed_by_zwallet_0_1_0_unlocks_and_signs_nothing() {
    let home = Home::new();
    home.create("t", &[]);
    home.allow("alice", ALICE, &["*"]);
    // zwallet 0.1.0 sealed the anchor (key, generation, digest) with no salt: its key sat in a
    // file of its own.
    let lock = zwallet::core::store::MetaLock::acquire(home.path()).unwrap();
    let opened = lock.open(common::PW).unwrap();
    let mut policy = opened.unsealed().policy.clone();
    policy.as_object_mut().unwrap().remove("whitelistSalt");
    lock.reseal(&opened, &policy).unwrap();
    drop(lock);
    let unlocked = home.unlock("10m");
    let w = &unlocked["whitelist"];
    assert_eq!(w["verified"], false, "{unlocked:#}");
    assert!(
        w["reason"]
            .as_str()
            .unwrap()
            .contains("made by zwallet 0.1.0"),
        "{unlocked:#}"
    );
    // Untrusted, not "no whitelist yet": neither the listed payee nor the key's own address.
    let me = unlocked["address"].as_str().unwrap().to_string();
    let rpc = mining_node();
    for to in [ALICE, me.as_str()] {
        let r = home.run(&pay("ethereum", to, &rpc.url, &[]));
        let v = refused(&r, "WHITELIST_INVALID", to, "ethereum", None);
        assert!(
            v["error"]
                .as_str()
                .unwrap()
                .contains("made by zwallet 0.1.0"),
            "{v:#}"
        );
    }
    assert!(rpc.sent().is_empty());
}

/// A new wallet with no whitelist, unlocked for 10 minutes.
fn unlocked_without_list() -> (Home, String) {
    let home = Home::new();
    let me = home.create("t", &["--unlock", "10m"])["address"]
        .as_str()
        .unwrap()
        .to_string();
    (home, me)
}

#[test]
fn self_transfers_with_no_list_yet_are_under_the_built_in_fee_cap() {
    // B-M2: with no list yet, a native self-transfer must not burn the balance in fees.
    let (home, me) = unlocked_without_list();
    let greedy = MockRpc::start(|m, p| fee_node(m, p, Some(GWEI), 500 * GWEI, GWEI + GWEI / 10));
    let r = home.run(&pay(
        "ethereum",
        &me,
        &greedy.url,
        &["--max-fee-gwei", "1000.5"],
    ));
    let v = r.json();
    assert_eq!(v["errorCode"], "FEE_CAP_EXCEEDED", "{v:#}");
    assert_eq!(v["counterparty"], me.as_str(), "{v:#}");
    // The compiled-in default for Ethereum: 0.003 ETH. The first fee-cap edit is the way past.
    assert_eq!(v["capWei"], "3000000000000000", "{v:#}");
    assert!(
        v["command"]
            .as_str()
            .unwrap()
            .starts_with("zwallet whitelist fee-cap ethereum "),
        "{v:#}"
    );
    assert!(
        greedy.sent().is_empty(),
        "a fee-burning self-transfer was sent"
    );
    // At normal fees the self-transfer still goes through (A7).
    let rpc = mining_node();
    home.run(&pay("ethereum", &me, &rpc.url, &[])).ok();
    assert_eq!(rpc.sent().len(), 1);
    // A list that stops verifying mid-session (junk written over it) signs nothing at all, the
    // cheap self-transfer included (--allow-duplicate: the journal has the one sent above).
    home.stop_daemons();
    home.allow("other", OTHER, &["*"]);
    home.unlock("10m");
    std::fs::write(home.path().join("whitelist.json"), b"{}").unwrap();
    for (rpc_url, extra) in [
        (
            &greedy.url,
            &["--max-fee-gwei", "1000.5", "--allow-duplicate"][..],
        ),
        (&rpc.url, &["--allow-duplicate"]),
    ] {
        let r = home.run(&pay("ethereum", &me, rpc_url, extra));
        refused(&r, "WHITELIST_INVALID", &me, "ethereum", None);
    }
    assert!(greedy.sent().is_empty());
    assert_eq!(
        rpc.sent().len(),
        1,
        "a self-transfer went out on a broken list"
    );
}

#[test]
fn breaking_the_list_never_relaxes_its_signed_fee_cap() {
    // P-1: the signed list caps Ethereum fees at 0.0001 ETH, below the built-in 0.003 ETH. A
    // self-transfer costing more than the list allows is refused; corrupting whitelist.json must
    // not let the same request through under the looser built-in cap.
    let (home, me) = listed(|l| {
        payee(l, "other", OTHER, &["*"]);
        l.policy
            .set_max_fee_wei("ethereum", alloy::primitives::U256::from(10u128.pow(14)));
    });
    // A 20 gwei base fee: the CLI's own fee limit for 21000 gas lands between the two caps.
    let rpc = MockRpc::start(|m, p| fee_node(m, p, Some(20 * GWEI), GWEI, 21 * GWEI));
    let args = pay("ethereum", &me, &rpc.url, &[]);
    let r = home.run(&args);
    let v = r.json();
    assert_eq!(v["errorCode"], "FEE_CAP_EXCEEDED", "{v:#}");
    assert_eq!(v["capWei"], "100000000000000", "{v:#}");
    let fee: u128 = v["feeWei"].as_str().unwrap().parse().unwrap();
    assert!(fee > 10u128.pow(14) && fee <= 3 * 10u128.pow(15), "{v:#}");
    std::fs::write(home.path().join("whitelist.json"), b"{}").unwrap();
    let r = home.run(&args);
    let v = refused(&r, "WHITELIST_INVALID", &me, "ethereum", None);
    assert!(
        v["error"]
            .as_str()
            .unwrap()
            .contains("`zwallet whitelist list --verify`"),
        "{v:#}"
    );
    assert!(rpc.sent().is_empty(), "the broken list let a send through");
}

/// `(op, the request)` of every request a [`common::FakeDaemon`] received.
fn daemon_ops(fake: &common::FakeDaemon) -> Vec<String> {
    fake.received
        .lock()
        .unwrap()
        .iter()
        .map(|m| {
            let v: Value = serde_json::from_slice(m).unwrap_or_default();
            v["op"].as_str().unwrap_or("?").to_string()
        })
        .collect()
}

#[test]
fn f1_a_dry_run_the_list_denies_is_explained_and_never_reaches_the_signer() {
    let home = Home::new();
    home.create("t", &[]);
    home.allow("alice", ALICE, &["ethereum"]);
    // A listener answering "unlocked" in place of the key daemon records every request.
    let fake = common::FakeDaemon::answering(
        &home.sock(),
        json!({ "locked": false, "pid": 1, "expiresAt": "2099-01-01T00:00:00Z" }),
    );
    let rpc = MockRpc::start(healthy);
    let v = home
        .run(&pay("ethereum", OTHER, &rpc.url, &["--dry-run"]))
        .ok();
    assert_eq!(v["dryRun"], true, "{v:#}");
    let w = &v["whitelist"];
    assert_eq!(w["allowed"], false, "{v:#}");
    assert_eq!(w["code"], "WHITELIST_DENIED", "{v:#}");
    assert_eq!(
        w["command"],
        format!("zwallet whitelist add <nick> {OTHER} --chains ethereum"),
        "{v:#}"
    );
    assert!(
        w["reason"]
            .as_str()
            .unwrap()
            .starts_with("refusing to sign"),
        "{v:#}"
    );
    for k in ["recoveredFrom", "txHash", "signerDecoded", "rawTx"] {
        assert!(v.get(k).is_none(), "{k}: {v:#}");
    }
    // An invoke of an unlisted contract: the same.
    let v = home
        .run(&invoke(
            ROUTER,
            &["--data", DEPOSIT],
            &rpc.url,
            &["--dry-run"],
        ))
        .ok();
    assert_eq!(v["whitelist"]["allowed"], false, "{v:#}");
    assert!(
        v["whitelist"]["command"]
            .as_str()
            .unwrap()
            .contains("--contract --selectors 0xd0e30db0"),
        "{v:#}"
    );
    // A whitelist.json that cannot be read: nothing on it can be trusted, every value send is
    // refused here too.
    std::fs::write(home.path().join("whitelist.json"), b"{ not a list").unwrap();
    let v = home
        .run(&pay("ethereum", OTHER, &rpc.url, &["--dry-run"]))
        .ok();
    assert_eq!(v["whitelist"]["allowed"], false, "{v:#}");
    assert_eq!(v["whitelist"]["code"], "WHITELIST_INVALID", "{v:#}");
    // No edit is suggested from a list that cannot be trusted.
    assert!(v["whitelist"].get("command").is_none(), "{v:#}");
    let ops = daemon_ops(&fake);
    assert!(!ops.iter().any(|op| op == "sign_tx"), "{ops:?}");
    assert!(rpc.sent().is_empty());
}

#[test]
fn f1_a_dry_run_with_an_untrusted_list_names_no_edit_and_refuses_even_a_self_transfer() {
    let home = Home::new();
    let me = home.create("t", &[])["address"]
        .as_str()
        .unwrap()
        .to_string();
    home.allow("alice", ALICE, &["ethereum"]);
    std::fs::write(home.path().join("whitelist.json"), b"{ not a list").unwrap();
    let fake = common::FakeDaemon::answering(
        &home.sock(),
        json!({ "locked": false, "pid": 1, "expiresAt": "2099-01-01T00:00:00Z" }),
    );
    let rpc = MockRpc::start(healthy);
    let mut zero = pay("ethereum", &me, &rpc.url, &["--dry-run"]);
    zero[6] = "0";
    for args in [pay("ethereum", OTHER, &rpc.url, &["--dry-run"]), zero] {
        let v = home.run(&args).ok();
        let w = &v["whitelist"];
        assert_eq!(w["allowed"], false, "{v:#}");
        assert_eq!(w["code"], "WHITELIST_INVALID", "{v:#}");
        // No edit is suggested from a list that cannot be trusted.
        assert!(w.get("command").is_none(), "{v:#}");
        assert!(
            w["reason"]
                .as_str()
                .unwrap()
                .contains("zwallet whitelist list --verify"),
            "{v:#}"
        );
    }
    let ops = daemon_ops(&fake);
    assert!(!ops.iter().any(|op| op == "sign_tx"), "{ops:?}");
    assert!(rpc.sent().is_empty());
}

#[test]
fn f1_a_dry_run_without_a_whitelist_file_is_left_to_the_daemon() {
    // Without whitelist.json this process cannot tell a fresh wallet from a deleted list (the
    // anchor is sealed): it checks nothing and asks the daemon, as a live send would.
    let home = Home::new();
    home.create("t", &[]);
    home.allow("alice", ALICE, &["ethereum"]);
    std::fs::remove_file(home.path().join("whitelist.json")).unwrap();
    let fake = common::FakeDaemon::answering(
        &home.sock(),
        json!({ "locked": false, "pid": 1, "expiresAt": "2099-01-01T00:00:00Z" }),
    );
    let rpc = MockRpc::start(healthy);
    let r = home.run(&pay("ethereum", OTHER, &rpc.url, &["--dry-run"]));
    let ops = daemon_ops(&fake);
    assert_eq!(
        ops.iter().filter(|op| *op == "sign_tx").count(),
        1,
        "{ops:?}"
    );
    // The listener signs nothing real, so the CLI refuses its answer; there is no whitelist
    // verdict either way.
    let v = r.json();
    assert!(v.get("whitelist").is_none(), "{v:#}");
    assert!(rpc.sent().is_empty());

    // With the real daemon of a wallet that has no whitelist yet: its own refusal, the same as a
    // live send's, and a self-transfer it signs, with no `whitelist` field.
    let (home, me) = unlocked_without_list();
    let r = home.run(&pay("ethereum", OTHER, &rpc.url, &["--dry-run"]));
    r.err("WHITELIST_INVALID");
    assert!(r.json().get("whitelist").is_none(), "{}", r.stdout);
    let mut zero = pay("ethereum", &me, &rpc.url, &["--dry-run"]);
    zero[6] = "0";
    let v = home.run(&zero).ok();
    assert_eq!(v["recoveredFrom"], me.as_str(), "{v:#}");
    assert!(v.get("whitelist").is_none(), "{v:#}");
    assert!(rpc.sent().is_empty());
}

#[test]
fn f1_a_dry_run_the_list_allows_is_signed_by_the_daemon() {
    let (home, me) = listed(|l| payee(l, "alice", ALICE, &["ethereum"]));
    let rpc = MockRpc::start(healthy);
    let v = home
        .run(&pay("ethereum", ALICE, &rpc.url, &["--dry-run"]))
        .ok();
    assert_eq!(v["whitelist"], json!({ "allowed": true }), "{v:#}");
    assert_eq!(v["recoveredFrom"], me.as_str(), "{v:#}");
    assert_eq!(v["signerDecoded"]["action"]["to"], ALICE, "{v:#}");
    // A live send is unchanged: the daemon decides.
    let r = home.run(&pay("ethereum", OTHER, &rpc.url, &[]));
    r.err("WHITELIST_DENIED");
    assert!(rpc.sent().is_empty());
}
