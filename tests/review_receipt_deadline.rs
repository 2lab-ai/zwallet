//! Receipt polling deadlines against a local RPC server. No wallet, keys or broadcasts.

use alloy::primitives::B256;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use zwallet::chains::{self, Net, evm::rpc};
use zwallet::core::error::ErrorCode;

struct MockRpc {
    net: Net,
    receipts: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for MockRpc {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl MockRpc {
    async fn start(delay: Duration, receipt: Value, error: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let net = Net {
            chain: chains::find("ethereum").unwrap(),
            rpc_url: format!("http://{}", listener.local_addr().unwrap()),
        };
        let receipts = Arc::new(AtomicUsize::new(0));
        let calls = receipts.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut data = Vec::new();
                let end = loop {
                    let mut byte = [0];
                    if stream.read(&mut byte).await.unwrap() == 0 {
                        return;
                    }
                    data.push(byte[0]);
                    if data.ends_with(b"\r\n\r\n") {
                        break data.len();
                    }
                };
                let headers = String::from_utf8_lossy(&data);
                let len: usize = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse().unwrap())
                    })
                    .unwrap();
                data.resize(end + len, 0);
                stream.read_exact(&mut data[end..]).await.unwrap();
                let request: Value = serde_json::from_slice(&data[end..]).unwrap();
                let mut response = json!({"jsonrpc": "2.0", "id": request["id"]});
                match request["method"].as_str().unwrap() {
                    "eth_chainId" => response["result"] = json!("0x1"),
                    "eth_getTransactionReceipt" => {
                        calls.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(delay).await;
                        if error {
                            response["error"] =
                                json!({"code": -32000, "message": "mock receipt failure"});
                        } else {
                            response["result"] = receipt.clone();
                        }
                    }
                    method => panic!("unexpected RPC method: {method}"),
                }
                let body = response.to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                // A timed-out client may already have disconnected.
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            }
        });
        Self {
            net,
            receipts,
            task,
        }
    }
}

fn receipt(status: &str) -> Value {
    json!({"transactionHash": format!("{:#x}", B256::ZERO), "status": status,
        "blockNumber": "0x2", "gasUsed": "0x5208"})
}

#[tokio::test]
async fn deadline_cancels_slow_receipt_rpc_without_accepting_a_late_success() {
    let mock = MockRpc::start(Duration::from_secs(2), receipt("0x1"), false).await;
    let provider = rpc::connect(&mock.net).await.unwrap();
    let start = Instant::now();
    let result = rpc::wait_receipt(&provider, B256::ZERO, Duration::from_millis(100))
        .await
        .unwrap();
    assert!(result.is_none(), "late receipt must remain unconfirmed");
    assert!(
        start.elapsed() < Duration::from_millis(500),
        "100ms deadline took {:?}",
        start.elapsed()
    );
    assert_eq!(mock.receipts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn deadline_bounds_the_sleep_before_another_poll() {
    let mock = MockRpc::start(Duration::ZERO, Value::Null, false).await;
    let provider = rpc::connect(&mock.net).await.unwrap();
    let start = Instant::now();
    let result = rpc::wait_receipt(&provider, B256::ZERO, Duration::from_millis(100))
        .await
        .unwrap();
    assert!(result.is_none());
    assert!(start.elapsed() < Duration::from_millis(500));
    assert_eq!(mock.receipts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn receipts_before_deadline_keep_their_execution_outcome() {
    for (status, expected) in [("0x1", Some(true)), ("0x0", Some(false)), ("0x2", None)] {
        let mock = MockRpc::start(Duration::ZERO, receipt(status), false).await;
        let provider = rpc::connect(&mock.net).await.unwrap();
        let result = rpc::wait_receipt(&provider, B256::ZERO, Duration::from_secs(2))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result["success"].as_bool(), expected);
        assert_eq!(result["blockNumber"], 2);
    }
}

#[tokio::test]
async fn rpc_error_before_deadline_remains_an_rpc_error() {
    let mock = MockRpc::start(Duration::ZERO, Value::Null, true).await;
    let provider = rpc::connect(&mock.net).await.unwrap();
    let result = rpc::wait_receipt(&provider, B256::ZERO, Duration::from_secs(2))
        .await
        .unwrap_err();
    assert_eq!(result.code, ErrorCode::RpcError);
}
