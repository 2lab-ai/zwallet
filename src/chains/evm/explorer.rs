//! The first page from three explicitly supported Blockscout deployments. All fields are
//! outside claims. Labels, names, methods, symbols and decoded input never enter the model.
use crate::chains::{Chain, EXPLORERS};
use crate::core::time::parse_rfc3339_utc;
use crate::history::{Row, Source, State, TokenMovement};
use alloy::primitives::{Address, B256, U256};
use alloy::transports::http::reqwest::{Client, redirect};
use serde_json::Value;
use std::time::Duration;

pub const MAX_BODY: usize = 1 << 20;
pub const TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unavailable {
    Unsupported,
    Http(u16),
    NotJson,
    TooLarge,
    Timeout,
    Network,
}
impl Unavailable {
    /// Fixed text and a numeric HTTP status only: never any server-provided body or URL.
    pub fn reason(&self) -> String {
        match self {
            Self::Unsupported => "incoming transfers not indexed".into(),
            Self::Http(n) => format!("HTTP {n}"),
            Self::NotJson => "not JSON".into(),
            Self::TooLarge => "response exceeds 1 MiB".into(),
            Self::Timeout => "timed out after 8s".into(),
            Self::Network => "network error".into(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page {
    pub rows: Vec<Row>,
    pub dropped: usize,
}

/// No environment, flag, file, cookie, login, or redirect changes the destination.
pub async fn fetch(chain: &Chain, wallet: Address) -> Result<Page, Unavailable> {
    let base = EXPLORERS
        .iter()
        .find(|(key, _)| *key == chain.key)
        .map(|(_, base)| *base)
        .ok_or(Unavailable::Unsupported)?;
    fetch_from(chain, wallet, base, TIMEOUT).await
}
async fn fetch_from(
    chain: &Chain,
    wallet: Address,
    base: &str,
    timeout: Duration,
) -> Result<Page, Unavailable> {
    let client = Client::builder()
        .redirect(redirect::Policy::none())
        .referer(false)
        .build()
        .map_err(|_| Unavailable::Network)?;
    let path = format!("{base}/api/v2/addresses/{}/", wallet.to_checksum(None));
    let transactions = format!("{path}transactions");
    let transfers = format!("{path}token-transfers?type=ERC-20");
    let read = async {
        let (txs, tokens) = tokio::join!(body(&client, &transactions), body(&client, &transfers));
        parse(chain, wallet, &txs?, &tokens?)
    };
    tokio::time::timeout(timeout, read)
        .await
        .map_err(|_| Unavailable::Timeout)?
}
async fn body(client: &Client, url: &str) -> Result<Vec<u8>, Unavailable> {
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|_| Unavailable::Network)?;
    if response.status().as_u16() != 200 {
        return Err(Unavailable::Http(response.status().as_u16()));
    }
    if response
        .content_length()
        .is_some_and(|n| n > MAX_BODY as u64)
    {
        return Err(Unavailable::TooLarge);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| Unavailable::Network)? {
        if chunk.len() > MAX_BODY.saturating_sub(bytes.len()) {
            return Err(Unavailable::TooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
fn items(bytes: &[u8]) -> Result<Vec<Value>, Unavailable> {
    if bytes.len() > MAX_BODY {
        return Err(Unavailable::TooLarge);
    }
    let mut value: Value = serde_json::from_slice(bytes).map_err(|_| Unavailable::NotJson)?;
    value
        .as_object_mut()
        .and_then(|v| v.remove("items"))
        .and_then(|v| v.as_array().cloned())
        .ok_or(Unavailable::NotJson)
}
/// Parse already bounded response bodies; exported for deterministic replay/fixture testing.
/// Invalid items are counted and dropped whole. A malformed page makes the refresh unavailable.
pub fn parse(
    chain: &Chain,
    wallet: Address,
    transactions: &[u8],
    tokens: &[u8],
) -> Result<Page, Unavailable> {
    let (txs, tokens) = (items(transactions)?, items(tokens)?);
    let mut page = Page {
        rows: Vec::new(),
        dropped: 0,
    };
    for item in txs {
        match transaction(chain, wallet, &item) {
            Some(row) => page.rows.push(row),
            None => page.dropped += 1,
        }
    }
    for item in tokens {
        match transfer(chain, wallet, &item) {
            Some(row) => page.rows.push(row),
            None => page.dropped += 1,
        }
    }
    Ok(page)
}
fn hex<const N: usize>(v: &Value) -> Option<[u8; N]> {
    let s = v.as_str()?;
    let raw = s.strip_prefix("0x")?;
    if raw.len() != 2 * N || !raw.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    alloy::hex::decode_to_array(raw).ok()
}
fn address(v: &Value) -> Option<Address> {
    hex::<20>(v).map(Address::from)
}
fn hash(v: &Value) -> Option<String> {
    hex::<32>(v).map(|b| format!("{:#x}", B256::from(b)))
}
fn decimal(v: &Value, max: usize) -> Option<U256> {
    let text = match v {
        Value::String(s) => s.clone(),
        Value::Number(n) if n.is_u64() => n.to_string(),
        _ => return None,
    };
    if text.is_empty() || text.len() > max || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    U256::from_str_radix(&text, 10).ok()
}
fn small(v: &Value, max: usize) -> Option<u64> {
    u64::try_from(decimal(v, max)?).ok()
}
fn block(v: &Value) -> Option<Option<u64>> {
    if v.is_null() {
        Some(None)
    } else {
        Some(Some(small(v, 20)?))
    }
}
fn related(wallet: Address, from: Address, to: Option<Address>) -> bool {
    from == wallet || to == Some(wallet)
}

/// Blockscout uses RFC 3339, including fractional seconds. Keep only its normalized instant,
/// at the journal's second precision, and never render the original timestamp text.
fn timestamp(s: &str) -> Option<u64> {
    if !(20..=35).contains(&s.len()) || !s.is_ascii() {
        return None;
    }
    let (zone_at, offset) = if s.ends_with('Z') {
        (s.len() - 1, 0i64)
    } else {
        let at = s.len().checked_sub(6)?;
        let zone = &s[at..];
        if !matches!(zone.as_bytes()[0], b'+' | b'-') || zone.as_bytes()[3] != b':' {
            return None;
        }
        let (hours, minutes) = (&zone[1..3], &zone[4..]);
        if !hours
            .bytes()
            .chain(minutes.bytes())
            .all(|b| b.is_ascii_digit())
        {
            return None;
        }
        let (hours, minutes) = (hours.parse::<i64>().ok()?, minutes.parse::<i64>().ok()?);
        if hours > 23 || minutes > 59 {
            return None;
        }
        let offset = (hours * 60 + minutes) * 60;
        (
            at,
            if zone.starts_with('+') {
                offset
            } else {
                -offset
            },
        )
    };
    let fraction = s.get(19..zone_at)?;
    if !(fraction.is_empty()
        || (fraction.starts_with('.')
            && (2..=10).contains(&fraction.len())
            && fraction[1..].bytes().all(|b| b.is_ascii_digit())))
    {
        return None;
    }
    let local = parse_rfc3339_utc(&format!("{}Z", &s[..19]))?;
    let utc = i64::try_from(local).ok()?.checked_sub(offset)?;
    u64::try_from(utc).ok().filter(|&n| n <= 253_402_300_799)
}
fn transaction(chain: &Chain, wallet: Address, v: &Value) -> Option<Row> {
    let from = address(v.get("from")?.get("hash")?)?;
    let to = match v.get("to")? {
        Value::Null => None,
        v => Some(address(v.get("hash")?)?),
    };
    let value = decimal(v.get("value")?, 78)?;
    if !related(wallet, from, to) || (from != wallet && value.is_zero()) {
        return None;
    }
    let mut row = Row::new(chain, hash(v.get("hash")?)?, Source::Explorer);
    row.block_number = block(v.get("block_number")?)?;
    // A pending transaction has not acquired its block index or block timestamp yet. Null
    // means unknown only in that state; mined rows and malformed non-null values stay strict.
    row.position = match v.get("position")? {
        Value::Null if row.block_number.is_none() => None,
        position => Some(u32::try_from(small(position, 10)?).ok()?),
    };
    row.nonce = Some(small(v.get("nonce")?, 20)?);
    row.from = Some(from.to_checksum(None));
    row.to = to.map(|a| a.to_checksum(None));
    row.value = Some(value);
    row.fee = match v.get("fee")?.get("value")? {
        Value::Null if row.block_number.is_none() => None,
        value => Some(decimal(value, 78)?),
    };
    row.success = match v.get("status")? {
        Value::Null => None,
        Value::String(s) if s == "ok" => Some(true),
        Value::String(s) if s == "error" => Some(false),
        _ => return None,
    };
    row.timestamp = match v.get("timestamp")? {
        Value::Null if row.block_number.is_none() => None,
        value => Some(timestamp(value.as_str()?)?),
    };
    row.state = if row.block_number.is_some() {
        State::Mined
    } else {
        State::Pending
    };
    Some(row)
}
fn transfer(chain: &Chain, wallet: Address, v: &Value) -> Option<Row> {
    if v.get("token")?
        .get("type")
        .is_some_and(|kind| kind != "ERC-20")
        || v.get("total")?.get("token_id").is_some()
    {
        return None;
    }
    let from = address(v.get("from")?.get("hash")?)?;
    let to = address(v.get("to")?.get("hash")?)?;
    if !related(wallet, from, Some(to)) {
        return None;
    }
    let token = address(v.get("token")?.get("address_hash")?)?;
    let known = super::registry::lookup(chain.chain_id_json().as_u64()?, &token)?;
    let amount = decimal(v.get("total")?.get("value")?, 78)?;
    if amount.is_zero() {
        return None;
    }
    let mut row = Row::new(chain, hash(v.get("transaction_hash")?)?, Source::Explorer);
    row.block_number = block(v.get("block_number")?)?;
    row.state = if row.block_number.is_some() {
        State::Mined
    } else {
        State::Pending
    };
    row.movements.push(TokenMovement {
        token: token.to_checksum(None),
        symbol: known.symbol,
        decimals: known.decimals,
        from: from.to_checksum(None),
        to: to.to_checksum(None),
        amount,
        log_index: u32::try_from(small(v.get("log_index")?, 10)?).ok()?,
    });
    Some(row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    struct Server {
        url: String,
        stop: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }
    impl Server {
        fn new(status: u16, body: String, delay: Duration) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let stop = Arc::new(AtomicBool::new(false));
            let end = stop.clone();
            let thread = std::thread::spawn(move || {
                while !end.load(Ordering::SeqCst) {
                    let (mut stream, _) = match listener.accept() {
                        Ok(v) => v,
                        Err(_) => {
                            std::thread::sleep(Duration::from_millis(2));
                            continue;
                        }
                    };
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_millis(200)))
                        .unwrap();
                    // A bounded client rejects an oversized body before draining it. Drop
                    // joins this thread on the single-thread async test runtime, so socket
                    // cleanup cannot be relied on to wake a blocked server write.
                    stream
                        .set_write_timeout(Some(Duration::from_millis(200)))
                        .unwrap();
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                            break;
                        }
                    }
                    std::thread::sleep(delay);
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status} Status\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
                        body.len()
                    );
                }
            });
            Self {
                url,
                stop,
                thread: Some(thread),
            }
        }
    }
    impl Drop for Server {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            let result = self.thread.take().unwrap().join();
            if !std::thread::panicking() {
                result.unwrap();
            }
        }
    }
    #[tokio::test]
    async fn transport_obeys_http_body_and_refresh_limits() {
        let chain = crate::chains::find("ethereum").unwrap();
        let owner = Address::ZERO;
        for (code, body, want) in [
            (403, "secret-key \u{1b}[2J".into(), Unavailable::Http(403)),
            (200, "<html>secret</html>".into(), Unavailable::NotJson),
            (200, "x".repeat(MAX_BODY + 1), Unavailable::TooLarge),
        ] {
            let server = Server::new(code, body, Duration::ZERO);
            assert_eq!(
                fetch_from(chain, owner, &server.url, TIMEOUT).await,
                Err(want)
            );
        }
        let server = Server::new(200, "{\"items\":[]}".into(), Duration::from_millis(100));
        assert_eq!(
            fetch_from(chain, owner, &server.url, Duration::from_millis(20)).await,
            Err(Unavailable::Timeout)
        );
    }
    #[tokio::test]
    async fn local_fixture_is_read_through_http_without_production_override() {
        let body = include_str!("../../../tests/fixtures/explorer/transactions.json").to_string();
        let server = Server::new(200, body, Duration::ZERO);
        let page = fetch_from(
            crate::chains::find("ethereum").unwrap(),
            "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
                .parse()
                .unwrap(),
            &server.url,
            TIMEOUT,
        )
        .await
        .unwrap();
        // This fixture is the transaction endpoint's shape. The same body on the token
        // endpoint has no transaction_hash and is counted rather than treated as a movement.
        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.dropped, 1);
    }
}
