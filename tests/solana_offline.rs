//! Native SOL loopback RPC regressions. No wallet home, network credentials or real assets.
mod common;
use common::tron::{MockTron, Reply};
use serde_json::{Value, json};
use std::time::Duration;
use zwallet::chains::{
    self, ChainFamily, Net, SendJournal, SendRecord, SendSummary, TransferRequest,
    solana::{self, Solana, tx},
};
use zwallet::core::error::ErrorCode;

fn vector() -> Value {
    serde_json::from_str(include_str!("fixtures/solana/native-transfer.json")).unwrap()
}
fn request() -> TransferRequest {
    let f = vector();
    TransferRequest {
        from: f["addresses"][0].as_str().unwrap().into(),
        to: f["addresses"][1].as_str().unwrap().into(),
        amount: "0.001".into(),
        token: None,
        dry_run: false,
        nonce: None,
        bump: None,
        replace: None,
        max_fee_per_gas: None,
        force: false,
        allow_pending: false,
        journal: None,
    }
}
fn net(node: &MockTron) -> Net {
    Net {
        chain: chains::find("solana").unwrap(),
        rpc_url: node.url.clone(),
    }
}
fn rpc_result(result: Value) -> Reply {
    Reply::Json(json!({"jsonrpc":"2.0","id":1,"result":result}))
}
fn fixture(method: &str, params: &Value) -> Value {
    let f = vector();
    match method {
        "getGenesisHash" => json!(solana::MAINNET_GENESIS),
        "getBalance" => json!({"context":{"slot":99},"value":10_000_000_000u64}),
        "getLatestBlockhash" => {
            json!({"context":{"slot":99},"value":{"blockhash":f["blockhash"],"lastValidBlockHeight":150}})
        }
        "getFeeForMessage" => json!({"context":{"slot":99},"value":5000}),
        "simulateTransaction" => {
            json!({"context":{"slot":99},"value":{"err":null,"logs":[],"unitsConsumed":150}})
        }
        "isBlockhashValid" => json!({"context":{"slot":100},"value":true}),
        "sendTransaction" => {
            assert_eq!(params[1]["skipPreflight"], false);
            assert_eq!(params[1]["preflightCommitment"], "confirmed");
            json!(f["signatureBase58"])
        }
        "getTransaction" => {
            json!({"slot":101,"transaction":[f["rawBase64"],"base64"],"meta":{"fee":5000,"err":null}})
        }
        "getSignatureStatuses" => json!({"context":{"slot":101},"value":[null]}),
        _ => panic!("unexpected Solana method {method}"),
    }
}
fn node() -> MockTron {
    MockTron::start(|_, body| {
        rpc_result(fixture(body["method"].as_str().unwrap(), &body["params"]))
    })
}
fn signed() -> chains::SignedTx {
    tx::inspect(&json!({"rawTx":vector()["rawBase64"],"txHash":vector()["signatureBase58"]}))
        .unwrap()
}
fn record(home: &std::path::Path, signed: &chains::SignedTx) -> SendRecord {
    let req = request();
    SendRecord::new(
        SendJournal::new(home, chains::find("solana").unwrap(), &req.from),
        signed,
        &SendSummary {
            kind: "transfer".into(),
            to: req.to,
            value: "0.001 SOL".into(),
            amount_raw: Some("1000000".into()),
            token: None,
            calldata_hash: None,
        },
    )
}
#[tokio::test]
async fn native_prepare_balance_broadcast_and_verified_receipt() {
    let node = node();
    let net = net(&node);
    let req = request();
    assert_eq!(
        Solana
            .native_balance(&net, &req.from)
            .await
            .unwrap()
            .raw
            .to_string(),
        "10000000000"
    );
    let prepared = Solana.prepare_transfer(&net, &req).await.unwrap();
    assert_eq!(
        prepared.unsigned,
        json!(format!("0x{}", vector()["messageHex"].as_str().unwrap()))
    );
    assert_eq!(prepared.fields["amountRaw"], "1000000");
    assert_eq!(prepared.fields["maxFeeRaw"], "5000");
    assert_eq!(prepared.fields["sufficientFunds"], true);
    let home = tempfile::Builder::new()
        .prefix("zwt-sol-")
        .tempdir_in("/tmp")
        .unwrap();
    let signed = signed();
    let record = record(home.path(), &signed);
    assert_eq!(
        Solana
            .broadcast(&net, &signed, false, &record)
            .await
            .unwrap(),
        chains::Broadcast::Accepted
    );
    assert_eq!(record.journal.recent().unwrap().len(), 1);
    let receipt = Solana
        .wait_for_receipt(&net, &signed.tx_hash, Duration::from_secs(2))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receipt["status"], "success");
    assert_eq!(receipt["valueRaw"], "1000000");
    assert_eq!(receipt["feeRaw"], "5000");
    assert_eq!(receipt["from"], req.from);
    assert_eq!(receipt["to"], req.to);
    let calls = node.seen();
    let sim = calls
        .iter()
        .find(|c| c.body["method"] == "simulateTransaction")
        .unwrap();
    assert_eq!(sim.body["params"][1]["sigVerify"], false);
    assert_eq!(sim.body["params"][1]["replaceRecentBlockhash"], false);
    assert!(calls.iter().all(|c| {
        c.user_agent
            .as_deref()
            .is_some_and(|a| a.starts_with("zwallet/"))
    }));
}
#[tokio::test]
async fn reject_wrong_network_changed_fee_bad_simulation_and_shape() {
    for (method, bad) in [
        (
            "getGenesisHash",
            json!("EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG"),
        ),
        ("getFeeForMessage", json!({"value":5001})),
        ("getFeeForMessage", json!({"value":null})),
        ("getBalance", json!({"value":-1})),
        ("getBalance", json!({"value":"1000000000"})),
        (
            "getLatestBlockhash",
            json!({"value":{"blockhash":"bad","lastValidBlockHeight":5}}),
        ),
        (
            "simulateTransaction",
            json!({"value":{"err":{"InstructionError":[0,"InvalidArgument"]}}}),
        ),
        ("simulateTransaction", json!({"value":{}})),
    ] {
        let node = MockTron::start(move |_, b| {
            rpc_result(if b["method"] == method {
                bad.clone()
            } else {
                fixture(b["method"].as_str().unwrap(), &b["params"])
            })
        });
        assert!(
            Solana
                .prepare_transfer(&net(&node), &request())
                .await
                .is_err(),
            "{method}"
        );
        assert!(
            !node
                .seen()
                .iter()
                .any(|c| c.body["method"] == "sendTransaction")
        );
    }
}
#[tokio::test]
async fn insufficient_dry_run_reports_deficit_without_simulation() {
    let node = MockTron::start(|_, b| {
        rpc_result(if b["method"] == "getBalance" {
            json!({"value":1})
        } else {
            fixture(b["method"].as_str().unwrap(), &b["params"])
        })
    });
    let mut req = request();
    assert_eq!(
        Solana
            .prepare_transfer(&net(&node), &req)
            .await
            .unwrap_err()
            .code,
        ErrorCode::InsufficientFunds
    );
    req.dry_run = true;
    let result = Solana.prepare_transfer(&net(&node), &req).await.unwrap();
    assert_eq!(result.fields["sufficientFunds"], false);
    assert!(
        !node
            .seen()
            .iter()
            .any(|c| c.body["method"] == "simulateTransaction")
    );
}
#[tokio::test]
async fn unsupported_modes_fail_before_rpc() {
    let node = node();
    for mode in 0..8 {
        let mut req = request();
        match mode {
            0 => req.token = Some("usdc".into()),
            1 => req.nonce = Some(1),
            2 => req.bump = Some(12500),
            3 => req.replace = Some("x".into()),
            4 => req.max_fee_per_gas = Some(2),
            5 => req.allow_pending = true,
            6 => req.force = true,
            _ => req.amount = "0".into(),
        };
        assert!(Solana.prepare_transfer(&net(&node), &req).await.is_err());
    }
    assert!(node.seen().is_empty());
}
#[tokio::test]
async fn expired_blockhash_and_invalid_signature_never_submit_or_journal() {
    let node = MockTron::start(|_, b| {
        rpc_result(if b["method"] == "isBlockhashValid" {
            json!({"value":false})
        } else {
            fixture(b["method"].as_str().unwrap(), &b["params"])
        })
    });
    let home = tempfile::Builder::new()
        .prefix("zwt-sol-")
        .tempdir_in("/tmp")
        .unwrap();
    let signed = signed();
    let record = record(home.path(), &signed);
    let e = Solana
        .broadcast(&net(&node), &signed, false, &record)
        .await
        .unwrap_err();
    assert!(e.message.contains("nothing was broadcast"));
    assert!(record.journal.recent().unwrap().is_empty());
    let mut bad = signed.clone();
    bad.signer = request().to;
    assert!(
        Solana
            .broadcast(&net(&node), &bad, false, &record)
            .await
            .is_err()
    );
    assert!(
        !node
            .seen()
            .iter()
            .any(|c| c.body["method"] == "sendTransaction")
    );
}
#[tokio::test]
async fn send_errors_and_mismatched_ids_remain_uncertain() {
    for error in [true, false] {
        let node = MockTron::start(move |_, b| {
            if b["method"] == "sendTransaction" {
                if error {
                    Reply::Json(
                        json!({"jsonrpc":"2.0","id":1,"error":{"code":-32002,"message":"leaked-secret-value"}}),
                    )
                } else {
                    rpc_result(json!("another-signature"))
                }
            } else {
                rpc_result(fixture(b["method"].as_str().unwrap(), &b["params"]))
            }
        });
        let home = tempfile::Builder::new()
            .prefix("zwt-sol-")
            .tempdir_in("/tmp")
            .unwrap();
        let signed = signed();
        let record = record(home.path(), &signed);
        let e = Solana
            .broadcast(&net(&node), &signed, false, &record)
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::BroadcastUncertain);
        assert_eq!(e.details["txHash"], signed.tx_hash);
        assert!(e.details.contains_key("journal"));
        assert!(!e.message.contains("leaked-secret-value"));
        assert_eq!(record.journal.recent().unwrap().len(), 1);
    }
}
#[tokio::test]
async fn lookup_verifies_signature_hash_and_complete_outcome() {
    for mode in 0..5 {
        let node = MockTron::start(move |_, b| {
            let mut v = fixture(b["method"].as_str().unwrap(), &b["params"]);
            if b["method"] == "getTransaction" {
                match mode {
                    0 => v["transaction"][0] = json!("invalid"),
                    1 => v["meta"]["fee"] = json!("5000"),
                    2 => {
                        v["meta"].as_object_mut().unwrap().remove("err");
                    }
                    3 => v["slot"] = json!(-1),
                    _ => v["transaction"][1] = json!("base58"),
                }
            }
            rpc_result(v)
        });
        assert!(
            Solana
                .lookup_tx(&net(&node), vector()["signatureBase58"].as_str().unwrap())
                .await
                .is_err()
        );
    }
    // A well-shaped signature other than the embedded one still cannot label these bytes.
    let other = bs58::encode([1; 64]).into_string();
    assert!(
        Solana
            .lookup_tx(&net(&node()), &other)
            .await
            .unwrap_err()
            .message
            .contains("another transaction")
    );
}
#[tokio::test]
async fn missing_or_processed_evidence_never_becomes_confirmed() {
    for observed in [false, true] {
        let node = MockTron::start(move |_, b| {
            rpc_result(match b["method"].as_str().unwrap() {
                "getTransaction" => Value::Null,
                "getSignatureStatuses" => {
                    assert_eq!(b["params"][1]["searchTransactionHistory"], true);
                    json!({"value":[if observed{json!({"slot":5,"confirmationStatus":"processed","err":null})}else{Value::Null}]})
                }
                method => fixture(method, &b["params"]),
            })
        });
        let row = Solana
            .lookup_tx(&net(&node), vector()["signatureBase58"].as_str().unwrap())
            .await
            .unwrap();
        assert_eq!(
            row["status"],
            if observed { "pending" } else { "not found" }
        );
        assert!(row.get("success").is_none());
        assert!(
            Solana
                .wait_for_receipt(
                    &net(&node),
                    vector()["signatureBase58"].as_str().unwrap(),
                    Duration::from_millis(20)
                )
                .await
                .unwrap()
                .is_none()
        );
    }
}
#[tokio::test]
async fn rpc_bounds_reject_huge_malformed_and_wrong_envelopes_without_echo() {
    for mode in 0..4 {
        let node = MockTron::start(move |_, _| match mode {
            0 => Reply::Raw("x".repeat((1 << 20) + 1)),
            1 => Reply::Raw("not-json leaked-secret-value".into()),
            2 => Reply::Json(json!({"id":2,"jsonrpc":"2.0","result":solana::MAINNET_GENESIS})),
            _ => Reply::Json(
                json!({"id":1,"jsonrpc":"2.0","error":{"code":-1,"message":"leaked-secret-value"}}),
            ),
        });
        let e = Solana
            .native_balance(&net(&node), &request().from)
            .await
            .unwrap_err();
        assert!(!e.to_json().to_string().contains("leaked-secret-value"));
    }
}
#[tokio::test]
async fn balance_deadline_cancels_slow_endpoint() {
    let node = MockTron::start(|_, _| {
        std::thread::sleep(Duration::from_millis(250));
        rpc_result(json!(solana::MAINNET_GENESIS))
    });
    let start = std::time::Instant::now();
    assert!(
        Solana
            .wallet_balances(&net(&node), &request().from, Duration::from_millis(25))
            .await
            .is_err()
    );
    assert!(start.elapsed() < Duration::from_millis(200));
}

#[tokio::test]
async fn malformed_execution_evidence_stays_unknown() {
    for err in [
        json!(false),
        json!(42),
        json!([]),
        json!({}),
        json!({"InstructionError":[0,false]}),
        json!({"InstructionError":[4,"InvalidArgument"]}),
        json!({"Custom":7}),
        json!({"InsufficientFundsForRent":{"account_index":4}}),
    ] {
        let node = MockTron::start(move |_, b| {
            let mut v = fixture(b["method"].as_str().unwrap(), &b["params"]);
            if b["method"] == "getTransaction" {
                v["meta"]["err"] = err.clone();
            }
            rpc_result(v)
        });
        assert!(
            Solana
                .lookup_tx(&net(&node), vector()["signatureBase58"].as_str().unwrap())
                .await
                .is_err()
        );
    }
    for err in [
        json!("InsufficientFundsForFee"),
        json!({"InstructionError":[0,"InvalidArgument"]}),
        json!({"InstructionError":[0,{"Custom":1}]}),
    ] {
        let node = MockTron::start(move |_, b| {
            let mut v = fixture(b["method"].as_str().unwrap(), &b["params"]);
            if b["method"] == "getTransaction" {
                v["meta"]["err"] = err.clone();
            }
            rpc_result(v)
        });
        let row = Solana
            .lookup_tx(&net(&node), vector()["signatureBase58"].as_str().unwrap())
            .await
            .unwrap();
        assert_eq!(row["success"], false);
    }
}

#[test]
fn public_cli_temp_home_real_daemon_native_lifecycle() {
    use std::sync::{Arc, Mutex};
    let observed = Arc::new(Mutex::new(None::<String>));
    let in_node = observed.clone();
    let node = MockTron::start(move |_, b| {
        let method = b["method"].as_str().unwrap();
        rpc_result(match method {
            "sendTransaction" => {
                let raw = b["params"][0].as_str().unwrap();
                let (signed, _) = tx::inspect_raw(raw).unwrap();
                assert_eq!(b["params"][1]["skipPreflight"], false);
                *in_node.lock().unwrap() = Some(raw.into());
                json!(signed.tx_hash)
            }
            "getTransaction" => match in_node.lock().unwrap().as_ref() {
                Some(raw) => {
                    let (signed, _) = tx::inspect_raw(raw).unwrap();
                    assert_eq!(b["params"][0], signed.tx_hash);
                    json!({"slot":101,"transaction":[raw,"base64"],"meta":{"fee":5000,"err":null}})
                }
                None => Value::Null,
            },
            _ => fixture(method, &b["params"]),
        })
    });
    let home = common::Home::new();
    home.create("main", &[]);
    let to = vector()["addresses"][1].as_str().unwrap().to_string();
    home.allow("bob", &solana::policy_address(&to).unwrap(), &["solana"]);
    home.unlock("2m");
    let address = home.run(&["address", "--chain", "solana", "--json"]).ok();
    let from = address["address"].as_str().unwrap();
    assert!(solana::normalize_address(from).is_ok());
    let balance = home
        .run(&["balance", "--chain", "solana", "--rpc", &node.url, "--json"])
        .ok();
    assert_eq!(balance["raw"], "10000000000");
    assert_eq!(balance["address"], from);
    let args = [
        "transfer", "--chain", "solana", "--rpc", &node.url, "--to", "@bob", "--amount", "0.001",
        "--json",
    ];
    let mut dry = args.to_vec();
    dry.push("--dry-run");
    let run = home.run(&dry);
    let output = run.ok();
    assert_eq!(output["dryRun"], true);
    assert!(output.get("rawTx").is_none());
    assert!(!run.stdout.contains("rawTx"));
    assert!(!run.stdout.contains(common::PW));
    assert!(observed.lock().unwrap().is_none());
    let journal = SendJournal::new(home.path(), chains::find("solana").unwrap(), from);
    assert!(journal.recent().unwrap().is_empty());
    let output = home.run(&args).ok();
    assert_eq!(output["dryRun"], false);
    assert_eq!(output["from"], from);
    assert_eq!(output["to"], to);
    assert!(output.get("rawTx").is_none());
    let hash = output["txHash"].as_str().unwrap();
    let wire = observed.lock().unwrap().clone().unwrap();
    let (signed, intent) = tx::inspect_raw(&wire).unwrap();
    assert_eq!(signed.tx_hash, hash);
    assert_eq!(signed.signer, from);
    assert_eq!(intent.lamports, 1_000_000);
    assert_eq!(intent.to, solana::parse_address(&to).unwrap());
    assert_eq!(journal.recent().unwrap().len(), 1);
    let lookup = home
        .run(&[
            "tx", "--chain", "solana", "--rpc", &node.url, hash, "--json",
        ])
        .ok();
    assert_eq!(lookup["status"], "mined");
    assert_eq!(lookup["success"], true);
    assert_eq!(lookup["valueRaw"], "1000000");
    assert_eq!(lookup["from"], from);
    assert_eq!(lookup["to"], to);
    let other = vector()["addresses"][0].as_str().unwrap().to_string();
    home.run(&[
        "transfer", "--chain", "solana", "--rpc", &node.url, "--to", &other, "--amount", "0.002",
        "--json",
    ])
    .err("WHITELIST_DENIED");
    assert_eq!(
        node.seen()
            .iter()
            .filter(|c| c.body["method"] == "sendTransaction")
            .count(),
        1
    );
    home.stop_daemons();
}
