//! Connect-inclusive receipt deadline regression from the independent TUI-5 review.
//! Only loopback HTTP is used: no signer, wallet, daemon or PTY. Initial sends and typed
//! wait extensions use the same Receipt worker, so neither may accept a late ready result.
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use zwallet::chains::{self, Net};
use zwallet::tui::app::{
    Msg,
    transfer::{TransferJob, TransferMsg},
};

#[tokio::test]
async fn receipt_result_resumed_after_the_connect_inclusive_deadline_is_unknown() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let net = Net {
        chain: chains::find("ethereum").unwrap(),
        rpc_url: format!("http://{}", listener.local_addr().unwrap()),
    };
    let receipt_seen = Arc::new(tokio::sync::Notify::new());
    let seen = receipt_seen.clone();
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut data = Vec::new();
            loop {
                let mut byte = [0];
                stream.read_exact(&mut byte).await.unwrap();
                data.push(byte[0]);
                if data.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let len: usize = String::from_utf8_lossy(&data)
                .lines()
                .find_map(|l| {
                    let (name, value) = l.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().unwrap())
                })
                .unwrap();
            let mut body = vec![0; len];
            stream.read_exact(&mut body).await.unwrap();
            let req: Value = serde_json::from_slice(&body).unwrap();
            let result = match req["method"].as_str().unwrap() {
                "eth_chainId" => {
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    json!("0x1")
                }
                "eth_getTransactionReceipt" => {
                    seen.notify_one();
                    json!({"transactionHash":req["params"][0],"status":"0x1","blockNumber":"0x1","gasUsed":"0x5208"})
                }
                m => panic!("unexpected method {m}"),
            };
            let reply = json!({"jsonrpc":"2.0","id":req["id"],"result":result}).to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            stream.shutdown().await.unwrap();
        }
    });
    let (cancel, receiver) = tokio::sync::watch::channel(false);
    let job = TransferJob::Receipt {
        net,
        expiration: None,
        hash: format!("0x{}", "0".repeat(64)),
        timeout: Duration::from_millis(500),
        cancel: receiver,
    };
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let started = tokio::time::Instant::now();
    let task = job.run(1, std::path::Path::new("/tmp/unused-no-wallet"), &tx);
    tokio::pin!(task);
    tokio::select! { biased;
        _ = receipt_seen.notified() => {},
        _ = &mut task => panic!("receipt unexpectedly completed before the server observed it"),
    }
    // Transport driver keeps running while this consumer is deliberately not polled. The
    // review deadline is 500ms from start, not another 500ms after the chain-id response.
    tokio::time::sleep_until(started + Duration::from_millis(550)).await;
    task.await;
    match rx.recv().await.unwrap() {
        Msg::Transfer(1, TransferMsg::Receipt(result)) => assert!(
            result.unwrap().is_none(),
            "late receipt was accepted past the original end-to-end deadline"
        ),
        _ => panic!("unexpected result"),
    }
    drop(cancel);
    server.await.unwrap();
}
