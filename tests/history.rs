//! Read-only history: untrusted explorer and journal input never grants a signing permission.
use alloy::primitives::{Address, U256};
use serde_json::json;
use zwallet::chains::evm::explorer::{self, Unavailable};
use zwallet::chains::{self, SendJournal};
use zwallet::history::{self, Source, State};

const OWNER: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
const OTHER: &str = "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC";
const USDC: &str = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48";
fn hash(n: u64) -> String {
    format!("0x{n:064x}")
}
fn tx(n: u64) -> serde_json::Value {
    json!({"hash": hash(n), "block_number": 900+n, "position": 2, "nonce": n,
      "from":{"hash":OWNER, "name":"zwallet: verified"},"to":{"hash":OTHER},
      "value":"1000000000000000000","fee":{"value":"21000"},"status":"ok",
      "timestamp":"2026-10-28T03:04:05Z", "method":"\u{1b}[2J██"})
}
fn token(n: u64) -> serde_json::Value {
    json!({"transaction_hash":hash(n),"block_number":900+n,"log_index":1,
    "from":{"hash":OTHER},"to":{"hash":OWNER},"token":{"address_hash":USDC,"symbol":"EVIL","decimals":"fake"},
    "total":{"value":"1234567"}})
}
fn parse(txs: Vec<serde_json::Value>, tokens: Vec<serde_json::Value>) -> explorer::Page {
    explorer::parse(
        chains::find("ethereum").unwrap(),
        OWNER.parse().unwrap(),
        &serde_json::to_vec(&json!({"items":txs})).unwrap(),
        &serde_json::to_vec(&json!({"items":tokens})).unwrap(),
    )
    .unwrap()
}

#[test]
fn explorer_shapes_and_poisoning_are_filtered_before_any_display() {
    let mut invalid_hash = tx(2);
    invalid_hash["hash"] = json!(format!("0x{}", "a".repeat(65)));
    let mut poison = tx(3);
    poison["from"]["hash"] = json!(OTHER);
    poison["to"]["hash"] = json!(OWNER);
    poison["value"] = json!("0");
    let mut unrelated = tx(4);
    unrelated["from"]["hash"] = json!(OTHER);
    let mut unknown_token = token(5);
    unknown_token["token"]["address_hash"] = json!(OTHER);
    let mut zero = token(6);
    zero["total"]["value"] = json!("0");
    let page = parse(
        vec![tx(1), invalid_hash, poison, unrelated],
        vec![token(1), unknown_token, zero],
    );
    assert_eq!(page.dropped, 5);
    let merged = history::merge(chains::find("ethereum").unwrap(), vec![], Ok(page));
    assert_eq!(merged.rows.len(), 1);
    assert_eq!(merged.rows[0].movements[0].symbol, "USDC");
    assert_eq!(merged.rows[0].movements[0].amount, U256::from(1234567));
    let shown = format!("{merged:?}");
    for forbidden in ["EVIL", "zwallet: verified", "fake", "\u{1b}", "██"] {
        assert!(!shown.contains(forbidden));
    }
}

#[test]
fn merge_deduplicates_sources_keeps_pending_first_and_only_ten_rows() {
    let chain = chains::find("ethereum").unwrap();
    let home = tempfile::Builder::new()
        .prefix("zwt-history-")
        .tempdir_in("/tmp")
        .unwrap();
    let journal = SendJournal::new(home.path(), chain, OWNER);
    journal.append(&json!({"ts":"2026-10-28T03:04:05Z","chain":"ethereum","from":OWNER,"nonce":1,
      "txHash":hash(1),"to":OTHER,"value":"1 ETH","amountRaw":"1000000000000000000","kind":"transfer"})).unwrap();
    let mut txs = (1..=14).map(tx).collect::<Vec<_>>();
    txs[0]["block_number"] = json!(null);
    txs[0]["status"] = json!(null);
    txs[0]["position"] = json!(null);
    txs[0]["timestamp"] = json!(null);
    txs[1]["block_number"] = json!(null);
    txs[1]["status"] = json!(null);
    txs[1]["position"] = json!(null);
    txs[1]["timestamp"] = json!(null);
    let merged = history::merge(chain, journal.recent().unwrap(), Ok(parse(txs, vec![])));
    assert_eq!(merged.rows.len(), 10);
    assert_eq!(merged.rows[0].state, State::Pending);
    assert_eq!(merged.rows[1].state, State::Pending);
    let first = merged.rows.iter().find(|r| r.tx_hash == hash(1)).unwrap();
    assert_eq!(first.sources, vec![Source::Journal, Source::Explorer]);
    assert!(merged.coverage.contains("explorer"));
}

#[test]
fn unavailable_explorer_keeps_the_journal_and_does_not_show_the_http_body() {
    let h = history::merge(
        chains::find("ethereum").unwrap(),
        vec![],
        Err(Unavailable::Http(403)),
    );
    assert_eq!(
        h.coverage,
        "ethereum: explorer unavailable (HTTP 403); this machine's sends only"
    );
    let h = history::merge(
        chains::find("base").unwrap(),
        vec![],
        Err(Unavailable::Unsupported),
    );
    assert!(h.coverage.contains("incoming transfers not indexed"));
}

#[test]
fn journal_history_rechecks_chain_owner_hash_and_never_uses_a_forged_symbol() {
    let chain = chains::find("ethereum").unwrap();
    let home = tempfile::Builder::new()
        .prefix("zwt-history-")
        .tempdir_in("/tmp")
        .unwrap();
    let journal = SendJournal::new(home.path(), chain, OWNER);
    let good = json!({"ts":"2026-10-28T03:04:05Z","chain":"ethereum","from":OWNER,"nonce":1,
      "txHash":hash(1),"to":OTHER,"value":"1 FAKE","amountRaw":"1000000000000000000","kind":"transfer"});
    journal.append(&good).unwrap();
    let mut bare_hash = good.clone();
    bare_hash["txHash"] = json!("a".repeat(64));
    journal.append(&bare_hash).unwrap();
    for field in ["from", "txHash", "chain", "to", "kind"] {
        let mut bad = good.clone();
        bad[field] = json!("\u{1b}[2J");
        journal.append(&bad).unwrap();
    }
    let rows = journal.recent().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].value, "1 ETH");
    assert_eq!(
        rows[0].from,
        OWNER.parse::<Address>().unwrap().to_checksum(None)
    );
}

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use zwallet::chains::Net;
use zwallet::chains::evm::rpc::{self, MovementsState};

struct Node {
    url: String,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Node {
    fn new(
        answer: impl Fn(&str, &serde_json::Value) -> serde_json::Value + Send + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let thread = std::thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                let (mut stream, _) = match listener.accept() {
                    Ok(s) => s,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut len = 0;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
                    }
                    if line == "\r\n" {
                        break;
                    }
                    if let Some((k, v)) = line.split_once(':')
                        && k.eq_ignore_ascii_case("content-length")
                    {
                        len = v.trim().parse::<usize>().unwrap();
                    }
                }
                if len == 0 {
                    continue;
                }
                let mut body = vec![0; len];
                if reader.read_exact(&mut body).is_err() {
                    continue;
                }
                let req: serde_json::Value = serde_json::from_slice(&body).unwrap();
                let response=json!({"jsonrpc":"2.0","id":req["id"],"result":answer(req["method"].as_str().unwrap(),&req["params"])}).to_string();
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
                    response.len(),
                    response
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
impl Drop for Node {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let result = self.thread.take().unwrap().join();
        if !std::thread::panicking() {
            result.unwrap();
        }
    }
}
fn run<T>(f: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(f)
}
fn log(token: &str, from: &str, to: &str, amount: u64, index: u64) -> serde_json::Value {
    json!({"address":token,"topics":[format!("{:#x}",alloy::primitives::keccak256("Transfer(address,address,uint256)")),
        format!("0x{:0>64}",&from[2..]),format!("0x{:0>64}",&to[2..])],
        "data":format!("0x{amount:064x}"),"logIndex":format!("0x{index:x}"),"transactionHash":hash(42),"removed":false})
}
fn mined_tx() -> serde_json::Value {
    json!({"hash":hash(42),"blockNumber":"0x10","nonce":"0x7","from":OWNER,"to":OTHER,"value":"0x0"})
}
fn receipt(status: &str) -> serde_json::Value {
    json!({"transactionHash":hash(42),"blockNumber":"0x10","from":OWNER,"to":OTHER,
        "gasUsed":"0x2","effectiveGasPrice":"0x3","l1Fee":"0x5","status":status,"logs":[]})
}

#[test]
fn detail_uses_exact_receipt_fees_confirmations_and_registry_movements() {
    let base_usdc = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913";
    let mut rec = receipt("0x1");
    rec["logs"] = json!([
        log(base_usdc, OTHER, OWNER, 1234567, 0),
        log(base_usdc, OWNER, OTHER, 2, 1),
        log(OTHER, OTHER, OWNER, 1000000, 2),
        log(base_usdc, OTHER, OWNER, 0, 3)
    ]);
    let node = Node::new(move |m, _| match m {
        "eth_chainId" => json!("0x2105"),
        "eth_getTransactionByHash" => mined_tx(),
        "eth_getTransactionReceipt" => rec.clone(),
        "eth_blockNumber" => json!("0x15"),
        _ => panic!("unexpected {m}"),
    });
    let net = Net {
        chain: chains::find("base").unwrap(),
        rpc_url: node.url.clone(),
    };
    let detail = run(async {
        let p = rpc::connect(&net).await.unwrap();
        rpc::tx_detail(
            &p,
            &net,
            hash(42).parse().unwrap(),
            OWNER.parse().unwrap(),
            None,
        )
        .await
        .unwrap()
    });
    assert_eq!(detail.state, State::Mined);
    assert_eq!(detail.fee, Some(U256::from(11)));
    assert_eq!(detail.confirmations, Some(6));
    assert_eq!(detail.to.as_deref(), Some(OTHER));
    assert_eq!(detail.movements.len(), 2);
    assert_eq!(detail.movements[0].amount, U256::from(1234567));
    assert_eq!(detail.movements[0].symbol, "USDC");
    assert_eq!(detail.hidden_movements, 2);
}

#[test]
fn reverted_unknown_and_pending_never_claim_token_movements() {
    for status in ["0x0", "0x9", "pending"] {
        let node = Node::new(move |m, _| match m {
            "eth_chainId" => json!("0x1"),
            "eth_getTransactionByHash" => {
                let mut tx = mined_tx();
                if status == "pending" {
                    tx["blockNumber"] = json!(null);
                }
                tx
            }
            "eth_getTransactionReceipt" => {
                if status == "pending" {
                    json!(null)
                } else {
                    let mut r = receipt(status);
                    r["logs"] = json!([log(USDC, OTHER, OWNER, 123, 0)]);
                    r
                }
            }
            "eth_blockNumber" => json!("0x15"),
            "eth_getTransactionCount" => json!("0x7"),
            _ => panic!("unexpected {m}"),
        });
        let net = Net {
            chain: chains::find("ethereum").unwrap(),
            rpc_url: node.url.clone(),
        };
        let d = run(async {
            let p = rpc::connect(&net).await.unwrap();
            rpc::tx_detail(
                &p,
                &net,
                hash(42).parse().unwrap(),
                OWNER.parse().unwrap(),
                None,
            )
            .await
            .unwrap()
        });
        assert!(d.movements.is_empty());
        assert_eq!(
            d.movements_state,
            match status {
                "0x0" => MovementsState::Reverted,
                "pending" => MovementsState::Pending,
                _ => MovementsState::Unknown,
            }
        );
    }
}

#[test]
fn a_consumed_journal_nonce_reports_dropped_replaced_without_claiming_no_payment() {
    let chain = chains::find("ethereum").unwrap();
    let home = tempfile::Builder::new()
        .prefix("zwt-history-")
        .tempdir_in("/tmp")
        .unwrap();
    let journal = SendJournal::new(home.path(), chain, OWNER);
    journal
        .append(
            &json!({"ts":"2026-10-28T03:04:05Z","chain":"ethereum","from":OWNER,"nonce":7,
      "txHash":hash(42),"to":OTHER,"value":"1 ETH","amountRaw":"1","kind":"transfer"}),
        )
        .unwrap();
    let rows = journal.recent().unwrap();
    let node = Node::new(|m, p| match m {
        "eth_chainId" => json!("0x1"),
        "eth_getTransactionByHash" | "eth_getTransactionReceipt" => json!(null),
        "eth_getTransactionCount" => {
            assert_eq!(p[1], "latest");
            json!("0x8")
        }
        _ => panic!("unexpected {m}"),
    });
    let net = Net {
        chain,
        rpc_url: node.url.clone(),
    };
    let d = run(async {
        let p = rpc::connect(&net).await.unwrap();
        rpc::tx_detail(
            &p,
            &net,
            hash(42).parse().unwrap(),
            OWNER.parse().unwrap(),
            Some(&rows[0]),
        )
        .await
        .unwrap()
    });
    assert_eq!(d.state, State::DroppedReplaced);
    assert!(d.hint.contains("does not prove"));
}

#[test]
fn wrong_hash_and_receipt_fee_overflow_are_errors_not_other_transaction_details() {
    for wrong_hash in [true, false] {
        let node = Node::new(move |m, _| match m {
            "eth_chainId" => json!("0x1"),
            "eth_getTransactionByHash" => mined_tx(),
            "eth_getTransactionReceipt" => {
                let mut r = receipt("0x1");
                if wrong_hash {
                    r["transactionHash"] = json!(hash(43));
                } else {
                    r["gasUsed"] = json!(format!("0x{}", "f".repeat(64)));
                    r["effectiveGasPrice"] = json!("0x2");
                }
                r
            }
            _ => panic!("unexpected {m}"),
        });
        let net = Net {
            chain: chains::find("ethereum").unwrap(),
            rpc_url: node.url.clone(),
        };
        let err = run(async {
            let p = rpc::connect(&net).await.unwrap();
            rpc::tx_detail(
                &p,
                &net,
                hash(42).parse().unwrap(),
                OWNER.parse().unwrap(),
                None,
            )
            .await
            .unwrap_err()
        });
        assert_eq!(err.code, zwallet::core::error::ErrorCode::RpcError);
    }
}

#[test]
fn explorer_numeric_hash_and_nft_boundaries_drop_whole_rows() {
    let mut large = tx(2);
    large["value"] = json!("1".repeat(79));
    let mut overflow = tx(3);
    overflow["value"] = json!("9".repeat(78));
    let mut short = tx(4);
    short["hash"] = json!(format!("0x{}", "a".repeat(63)));
    let mut block_over = tx(5);
    block_over["block_number"] = json!("1".repeat(21));
    let mut max = tx(6);
    max["value"] = json!(U256::MAX.to_string());
    max["block_number"] = json!(u64::MAX.to_string());
    let mut nft = token(7);
    nft["token"]["type"] = json!("ERC-721");
    let mut token_id = token(8);
    token_id["total"]["token_id"] = json!("1");
    let page = parse(
        vec![large, overflow, short, block_over, max],
        vec![nft, token_id],
    );
    assert_eq!(page.rows.len(), 1);
    assert_eq!(page.rows[0].value, Some(U256::MAX));
    assert_eq!(page.dropped, 6);
}

#[test]
fn a_reply_from_a_different_chain_cannot_be_merged_into_the_selected_chain() {
    let chain = chains::find("ethereum").unwrap();
    let home = tempfile::Builder::new()
        .prefix("zwt-history-")
        .tempdir_in("/tmp")
        .unwrap();
    let journal = SendJournal::new(home.path(), chain, OWNER);
    journal
        .append(
            &json!({"ts":"2026-10-28T03:04:05Z","chain":"ethereum","from":OWNER,"nonce":7,
      "txHash":hash(42),"to":OTHER,"value":"1 ETH","amountRaw":"1","kind":"transfer"}),
        )
        .unwrap();
    let result = history::merge(
        chains::find("base").unwrap(),
        journal.recent().unwrap(),
        Ok(parse(vec![tx(1)], vec![])),
    );
    assert!(result.rows.is_empty());
}

#[test]
fn explorer_rfc3339_timestamps_are_normalized_and_invalid_dates_are_dropped() {
    let mut fractional = tx(1);
    fractional["timestamp"] = json!("2026-10-28T03:04:05.123456Z");
    let mut zone = tx(2);
    zone["timestamp"] = json!("2026-10-28T12:04:05+09:00");
    let mut invalid = tx(3);
    invalid["timestamp"] = json!("2026-02-31T00:00:00Z");
    let mut control = tx(4);
    control["timestamp"] = json!("2026-10-28T03:04:05Z\u{1b}");
    let page = parse(vec![fractional, zone, invalid, control], vec![]);
    assert_eq!(page.dropped, 2);
    assert_eq!(page.rows.len(), 2);
    assert_eq!(page.rows[0].timestamp, page.rows[1].timestamp);
    assert_eq!(
        zwallet::core::time::rfc3339_utc(page.rows[0].timestamp.unwrap()),
        "2026-10-28T03:04:05Z"
    );
}

#[test]
fn explorer_pending_null_position_and_timestamp_are_unknown_not_invalid() {
    let mut pending = tx(1);
    pending["block_number"] = json!(null);
    pending["status"] = json!(null);
    pending["position"] = json!(null);
    pending["timestamp"] = json!(null);
    let page = parse(vec![tx(2), pending], vec![]);
    assert_eq!(page.dropped, 0);
    let merged = history::merge(chains::find("ethereum").unwrap(), vec![], Ok(page));
    assert_eq!(merged.rows.len(), 2);
    let first = &merged.rows[0];
    assert_eq!(first.tx_hash, hash(1));
    assert_eq!(first.state, State::Pending);
    assert_eq!(first.position, None);
    assert_eq!(first.timestamp, None);
}

#[test]
fn explorer_only_pending_rows_allow_null_position_or_timestamp() {
    let mut pending = tx(1);
    pending["block_number"] = json!(null);
    pending["status"] = json!(null);
    for field in ["position", "timestamp"] {
        let mut allowed = pending.clone();
        allowed[field] = json!(null);
        let mut mined = tx(2);
        mined[field] = json!(null);
        let page = parse(vec![allowed, mined], vec![]);
        assert_eq!(page.rows.len(), 1, "{field}");
        assert_eq!(page.dropped, 1, "{field}");
        for bad in [
            json!(false),
            json!("invalid"),
            json!(-1),
            json!({}),
            json!([]),
        ] {
            let mut malformed = pending.clone();
            malformed[field] = bad;
            let page = parse(vec![malformed], vec![]);
            assert!(page.rows.is_empty(), "{field}");
            assert_eq!(page.dropped, 1, "{field}");
        }
        let mut missing = pending.clone();
        missing.as_object_mut().unwrap().remove(field);
        assert_eq!(parse(vec![missing], vec![]).dropped, 1, "{field}");
    }
}

#[test]
fn explorer_pending_null_fee_is_unknown_but_invalid_fee_is_rejected() {
    let mut pending = tx(1);
    pending["block_number"] = json!(null);
    pending["status"] = json!(null);
    pending["position"] = json!(null);
    pending["timestamp"] = json!(null);
    pending["fee"]["value"] = json!(null);
    let page = parse(vec![pending.clone()], vec![]);
    assert_eq!(page.dropped, 0);
    assert_eq!(page.rows[0].fee, None);
    let mut mined = tx(2);
    mined["fee"]["value"] = json!(null);
    assert_eq!(parse(vec![mined], vec![]).dropped, 1);
    for value in [
        json!(false),
        json!("invalid"),
        json!(-1),
        json!({}),
        json!([]),
    ] {
        let mut malformed = pending.clone();
        malformed["fee"]["value"] = value;
        assert_eq!(parse(vec![malformed], vec![]).dropped, 1);
    }
    pending["fee"].as_object_mut().unwrap().remove("value");
    assert_eq!(parse(vec![pending], vec![]).dropped, 1);
}

#[test]
fn merge_transaction_metadata_wins_over_conflicting_token_rows_in_either_order() {
    let chain = chains::find("ethereum").unwrap();
    for pending in [false, true] {
        let mut transaction = tx(1);
        let mut transfer = token(1);
        if pending {
            transaction["block_number"] = json!(null);
            transaction["status"] = json!(null);
            transaction["position"] = json!(null);
            transaction["timestamp"] = json!(null);
        } else {
            transfer["block_number"] = json!(null);
        }
        let expected = parse(vec![transaction.clone()], vec![]).rows.remove(0);
        for reversed in [false, true] {
            for with_journal in [false, true] {
                let mut page = parse(vec![transaction.clone()], vec![transfer.clone()]);
                if reversed {
                    page.rows.reverse();
                }
                let journal = if with_journal {
                    vec![chains::JournalRow {
                        chain: chain.key,
                        tx_hash: hash(1),
                        from: OWNER.to_owned(),
                        to: OTHER.to_owned(),
                        nonce: Some(1),
                        expiration_ms: None,
                        timestamp: 100,
                        kind: "transfer",
                        amount_raw: Some(U256::from(1)),
                        value: "0.000000000000000001 ETH".to_owned(),
                        token: None,
                    }]
                } else {
                    vec![]
                };
                let merged = history::merge(chain, journal, Ok(page));
                assert_eq!(merged.rows.len(), 1);
                let actual = &merged.rows[0];
                assert_eq!(
                    actual.state, expected.state,
                    "pending={pending}, reversed={reversed}, journal={with_journal}"
                );
                assert_eq!(actual.block_number, expected.block_number);
                assert_eq!(actual.position, expected.position);
                assert_eq!(actual.success, expected.success);
                assert_eq!(actual.fee, expected.fee);
                assert_eq!(actual.value, expected.value);
                assert_eq!(actual.from, expected.from);
                assert_eq!(actual.to, expected.to);
                assert_eq!(actual.nonce, expected.nonce);
                assert_eq!(actual.movements.len(), 1);
                assert_eq!(actual.journal.is_some(), with_journal);
            }
        }
    }
}

#[test]
fn a_node_hash_mismatch_never_echoes_the_rpc_url_credential() {
    const SECRET: &str = "fixture_api_key_abcdef123456";
    let node = Node::new(|m, _| match m {
        "eth_chainId" => json!("0x1"),
        "eth_getTransactionByHash" => {
            let mut tx = mined_tx();
            tx["hash"] = json!(SECRET);
            tx
        }
        _ => panic!("unexpected {m}"),
    });
    let net = Net {
        chain: chains::find("ethereum").unwrap(),
        rpc_url: format!("{}?apiKey={SECRET}", node.url),
    };
    let err = run(async {
        let p = rpc::connect(&net).await.unwrap();
        rpc::tx_state(&p, &net, hash(42).parse().unwrap(), None)
            .await
            .unwrap_err()
    });
    assert!(!err.message.contains(SECRET));
    assert!(err.message.contains("<redacted>"));
}

#[test]
fn missing_op_stack_l1_fee_is_unknown_not_a_false_complete_total() {
    let node = Node::new(|m, _| match m {
        "eth_chainId" => json!("0x2105"),
        "eth_getTransactionByHash" => mined_tx(),
        "eth_getTransactionReceipt" => {
            let mut r = receipt("0x1");
            r.as_object_mut().unwrap().remove("l1Fee");
            r
        }
        "eth_blockNumber" => json!("0xf"),
        _ => panic!("unexpected {m}"),
    });
    let net = Net {
        chain: chains::find("base").unwrap(),
        rpc_url: node.url.clone(),
    };
    let d = run(async {
        let p = rpc::connect(&net).await.unwrap();
        rpc::tx_detail(
            &p,
            &net,
            hash(42).parse().unwrap(),
            OWNER.parse().unwrap(),
            None,
        )
        .await
        .unwrap()
    });
    assert_eq!(d.fee, None);
    assert_eq!(d.l1_fee, None);
    assert_eq!(d.confirmations, None);
}

#[test]
#[ignore = "live: public Blockscout history, no key or transaction broadcast"]
fn live_blockscout_history_uses_the_fixed_ethereum_endpoint() {
    let address = "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045"
        .parse()
        .unwrap();
    let page = run(explorer::fetch(chains::find("ethereum").unwrap(), address)).unwrap();
    assert!(!page.rows.is_empty());
    assert!(page.rows.iter().all(|r| r.sources == [Source::Explorer]));
}

#[test]
fn malformed_or_unrelated_transfer_logs_are_counted_not_displayed() {
    let mut dirty = log(USDC, OTHER, OWNER, 1, 1);
    dirty["topics"][1] = json!(format!("0x1{}", "0".repeat(63)));
    let mut nft = log(USDC, OTHER, OWNER, 1, 2);
    nft["topics"].as_array_mut().unwrap().push(json!(hash(1)));
    let mut wrong_hash = log(USDC, OTHER, OWNER, 1, 3);
    wrong_hash["transactionHash"] = json!(hash(41));
    let mut huge = log(USDC, OTHER, OWNER, 1, 4);
    huge["data"] = json!(format!("0x{}", "a".repeat(66)));
    let mut removed = log(USDC, OTHER, OWNER, 1, 5);
    removed["removed"] = json!(true);
    let mut r = receipt("0x1");
    r["logs"] = json!([
        dirty,
        nft,
        wrong_hash,
        huge,
        removed,
        log(USDC, OTHER, OWNER, 9, 6)
    ]);
    let node = Node::new(move |m, _| match m {
        "eth_chainId" => json!("0x1"),
        "eth_getTransactionByHash" => mined_tx(),
        "eth_getTransactionReceipt" => r.clone(),
        "eth_blockNumber" => json!("0x15"),
        _ => panic!("unexpected {m}"),
    });
    let net = Net {
        chain: chains::find("ethereum").unwrap(),
        rpc_url: node.url.clone(),
    };
    let d = run(async {
        let p = rpc::connect(&net).await.unwrap();
        rpc::tx_detail(
            &p,
            &net,
            hash(42).parse().unwrap(),
            OWNER.parse().unwrap(),
            None,
        )
        .await
        .unwrap()
    });
    assert_eq!(d.movements.len(), 1);
    assert_eq!(d.movements[0].amount, U256::from(9));
    assert_eq!(d.hidden_movements, 5);
}

#[test]
fn transaction_detail_rejects_a_provider_for_another_chain_or_endpoint_before_rpc() {
    let node = Node::new(|m, _| match m {
        "eth_chainId" => json!("0x1"),
        _ => panic!("mismatched provider queried {m}"),
    });
    let original = Net {
        chain: chains::find("ethereum").unwrap(),
        rpc_url: node.url.clone(),
    };
    run(async {
        let provider = rpc::connect(&original).await.unwrap();
        for net in [
            Net {
                chain: chains::find("base").unwrap(),
                rpc_url: original.rpc_url.clone(),
            },
            Net {
                chain: original.chain,
                rpc_url: format!("{}/other", original.rpc_url),
            },
        ] {
            let error = rpc::tx_detail(
                &provider,
                &net,
                hash(42).parse().unwrap(),
                OWNER.parse().unwrap(),
                None,
            )
            .await
            .unwrap_err();
            assert!(
                error
                    .message
                    .contains("does not match the selected chain and endpoint")
            );
        }
    });
}
