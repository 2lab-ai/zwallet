//! A Tron node on 127.0.0.1 for the tests: java-tron's HTTP API (`POST /wallet/<call>` with a
//! JSON body) keyed by path, the `MockRpc` pattern of `tests/evm_offline.rs`. It records every
//! request (path, body and `User-Agent`) and answers what the test's handler says.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

/// Tron mainnet's block 0 id (`.prd/05-tron.md` §Network access, M6).
pub const MAINNET_GENESIS: &str =
    "00000000000000001ebf88508a03865c71d452e25f4d51194196a1d22b6653dc";
/// Nile testnet's block 0 id (M6).
pub const NILE_GENESIS: &str = "0000000000000000d698d4192c56cb6be724a558448e2684802de4d6cd8690dc";
/// Tether's USDT on Tron, the registry row.
pub const USDT: &str = "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t";
/// A key of the public test mnemonic that holds TWer2... since someone took it over (M7).
pub const TAKEOVER_KEY: &str = "TDRqzjYye3bGaBtg1bAPw5F4duDchNSbtk";

pub enum Reply {
    /// 200 with this JSON.
    Json(Value),
    /// 200 with this body as it is (not JSON, or JSON of another shape).
    Raw(String),
    /// A non-200 status with this plain-text body (an API gateway's error page).
    Http(u16, &'static str),
}

type Handler = dyn Fn(&str, &Value) -> Reply + Send + Sync;

/// One request as the node saw it.
#[derive(Debug, Clone)]
pub struct Seen {
    pub path: String,
    pub body: Value,
    pub user_agent: Option<String>,
    /// Every header line, `name: value`, as sent.
    pub headers: Vec<String>,
}

pub struct MockTron {
    pub url: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl MockTron {
    pub fn start(handler: impl Fn(&str, &Value) -> Reply + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let handler: Arc<Handler> = Arc::new(handler);
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (handler, log) = (handler.clone(), log.clone());
                std::thread::spawn(move || serve(stream, &*handler, &log));
            }
        });
        Self { url, seen }
    }

    /// Every request so far, in order.
    pub fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    /// The bodies of the requests to `path` (`/wallet/getaccount`).
    pub fn calls(&self, path: &str) -> Vec<Value> {
        self.seen()
            .into_iter()
            .filter(|s| s.path == path)
            .map(|s| s.body)
            .collect()
    }

    /// The paths asked, in order.
    pub fn paths(&self) -> Vec<String> {
        self.seen().into_iter().map(|s| s.path).collect()
    }
}

/// One keep-alive connection: HTTP/1.1 requests with a Content-Length body, until EOF.
fn serve(stream: TcpStream, handler: &Handler, log: &Mutex<Vec<Seen>>) {
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);
    loop {
        let mut request_line = String::new();
        if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
            return;
        }
        let path = request_line
            .split_whitespace()
            .nth(1)
            .unwrap_or("")
            .to_string();
        let (mut len, mut user_agent, mut headers) = (0usize, None, Vec::new());
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                return;
            }
            let l = line.trim_end();
            if l.is_empty() {
                break;
            }
            headers.push(l.to_string());
            if let Some((k, v)) = l.split_once(':') {
                if k.eq_ignore_ascii_case("content-length") {
                    len = v.trim().parse().unwrap();
                } else if k.eq_ignore_ascii_case("user-agent") {
                    user_agent = Some(v.trim().to_string());
                }
            }
        }
        let mut body = vec![0u8; len];
        if reader.read_exact(&mut body).is_err() {
            return;
        }
        let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        log.lock().unwrap().push(Seen {
            path: path.clone(),
            body: body.clone(),
            user_agent,
            headers,
        });
        let (status, text) = match handler(&path, &body) {
            Reply::Json(v) => ("200 OK".to_string(), v.to_string()),
            Reply::Raw(s) => ("200 OK".to_string(), s),
            Reply::Http(code, s) => (format!("{code} Error"), s.to_string()),
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

/// `getblockbynum {"num": 0}`'s answer for a network whose block 0 is `id`.
pub fn block0(id: &str) -> Value {
    json!({
        "blockID": id,
        "block_header": { "raw_data": { "timestamp": 1529891469000u64, "number": 0 } },
    })
}

/// A `getaccount` answer: an activated account holding `sun`, owned by itself alone.
pub fn account(address: &str, sun: u64) -> Value {
    json!({
        "address": address,
        "balance": sun,
        "create_time": 1753240515000u64,
        "owner_permission": {
            "permission_name": "owner",
            "threshold": 1,
            "keys": [{ "address": address, "weight": 1 }],
        },
        "active_permission": [{
            "type": "Active", "id": 2, "permission_name": "active", "threshold": 1,
            "operations": "7fff1fc0033ec30f000000000000000000000000000000000000000000000000",
            "keys": [{ "address": address, "weight": 1 }],
        }],
    })
}

/// A 32-byte ABI word holding `n`, as `constant_result` writes it (hex, no `0x`).
pub fn word(n: u128) -> String {
    format!("{n:064x}")
}

/// A successful `triggerconstantcontract` answer returning `hex` (no `0x`).
pub fn constant(hex: &str) -> Value {
    json!({
        "result": { "result": true },
        "energy_used": 943,
        "constant_result": [hex],
        "transaction": { "ret": [{}], "visible": true, "txID": "00".repeat(32) },
    })
}

/// ABI encoding of one `string` return value, as `symbol()` answers.
pub fn abi_string(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut padded = alloy::hex::encode(bytes);
    while padded.len() % 64 != 0 || padded.is_empty() {
        padded.push('0');
    }
    format!("{}{}{padded}", word(32), word(bytes.len() as u128))
}

/// A mainnet node: block 0 is mainnet's, `getaccount` answers `account` for every address,
/// and every TRC20 contract answers `balanceOf` = `usdt` (in its base units), `decimals()` = 6
/// and `symbol()` = `USDT`.
pub fn node(
    account: Value,
    usdt: u128,
) -> impl Fn(&str, &Value) -> Reply + Send + Sync + 'static {
    move |path, body| match path {
        "/wallet/getblockbynum" => Reply::Json(block0(MAINNET_GENESIS)),
        "/wallet/getaccount" => Reply::Json(account.clone()),
        "/wallet/triggerconstantcontract" => {
            match body["function_selector"].as_str().unwrap_or("") {
                "balanceOf(address)" => Reply::Json(constant(&word(usdt))),
                "decimals()" => Reply::Json(constant(&word(6))),
                "symbol()" => Reply::Json(constant(&abi_string("USDT"))),
                other => panic!("mock: unexpected function {other}"),
            }
        }
        other => panic!("mock: unexpected {other}"),
    }
}

/// The 32-byte ABI word of a Tron address (12 zero bytes, then its 20-byte body), as the
/// `parameter` of `balanceOf(address)` carries it.
pub fn address_word(t: &str) -> String {
    let payload = bs58::decode(t).with_check(None).into_vec().unwrap();
    format!("{}{}", "0".repeat(24), alloy::hex::encode(&payload[1..]))
}

/// The `getaccount` answer M7 recorded for TWer2... (key 0 of the public test mnemonic):
/// 8,000,010 sun, its owner permission held by another key.
pub fn taken_over_account() -> Value {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/tron-account-taken-over.json"
    ))
    .unwrap();
    serde_json::from_str(&text).unwrap()
}
