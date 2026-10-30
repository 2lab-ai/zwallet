//! Tron C regression tests: isolated local nodes, no public traffic and no funded sends.
mod common;
use alloy::hex;
use common::tron::{MAINNET_GENESIS, MockTron, Reply, USDT};
use common::{FakeDaemon, Home};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use zeroize::Zeroizing;
use zwallet::{
    chains::{
        self, Net, SendJournal, SendRecord, SendSummary, TransferRequest,
        tron::{self, lookup, send, tx},
    },
    core::{error::ErrorCode, seed::Seed},
};
const TEST_MNEMONIC: &str = "test test test test test test test test test test test junk";
const FROM: &str = "TWer2Ygk5TEheHp3TPuYeqxmB6SsGZmaL6";
const TO: &str = "TPjjvMwjPoDC32V2dGDYTkLH4E5LAtBZ6C";
const OTHER: &str = "TPdMMFcLKgJiUSmYsViAFvf6Nsb2WV52vs";
fn net(node: &MockTron) -> Net {
    Net {
        chain: chains::find("tron").unwrap(),
        rpc_url: node.url.clone(),
    }
}
fn request(token: bool) -> TransferRequest {
    TransferRequest {
        from: FROM.into(),
        to: TO.into(),
        amount: "1".into(),
        token: token.then(|| "usdt".into()),
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
fn block(timestamp: u64) -> Value {
    let number = 9_000_000u64;
    let mut id = [0x77u8; 32];
    id[..8].copy_from_slice(&number.to_be_bytes());
    json!({"blockID":hex::encode(id),"block_header":{"raw_data":{"number":number,"timestamp":timestamp}}})
}
fn parameters() -> Value {
    json!({"chainParameter":[
    {"key":"getTransactionFee","value":2000},{"key":"getEnergyFee","value":100},
    {"key":"getCreateNewAccountFeeInSystemContract","value":2000000},{"key":"getCreateAccountFee","value":300000},
    {"key":"getCreateNewAccountBandwidthRate","value":3},{"key":"getMaxFeeLimit","value":15000000000u64}]})
}
fn fixture(path: &str, body: &Value) -> Value {
    match path {
        "/wallet/getblockbynum" => common::tron::block0(MAINNET_GENESIS),
        "/wallet/getblock" => block(send::now_ms() - 1000),
        "/wallet/getaccount" => {
            common::tron::account(body["address"].as_str().unwrap(), 100_000_000)
        }
        "/wallet/getaccountresource" => json!({}),
        "/wallet/getchainparameters" => parameters(),
        "/wallet/triggerconstantcontract" => {
            json!({"result":{"result":true},"constant_result":[format!("{:064x}",100_000_000u64)],"energy_used":100000,"transaction":{"ret":[{"ret":"SUCESS"}]}})
        }
        "/wallet/broadcasthex" => json!({"result":true}),
        "/wallet/gettransactionbyid"
        | "/wallet/gettransactionfrompending"
        | "/wallet/gettransactioninfobyid" => json!({}),
        _ => panic!("unexpected path {path}"),
    }
}
fn seed() -> Seed {
    Seed::from_phrase(Zeroizing::new(TEST_MNEMONIC.into())).unwrap()
}
fn signed(raw: &Value) -> Value {
    let bytes = hex::decode(raw.as_str().unwrap().trim_start_matches("0x")).unwrap();
    let d = tx::decode_raw(&bytes).unwrap();
    let s = tron::signer_at(&seed(), 0).unwrap();
    json!({"rawData":raw,"signature":format!("0x{}",hex::encode(tx::sign(&s,&d).unwrap())),"txHash":hex::encode(tx::txid(&bytes)),"decoded":{"to":"untrusted daemon text"}})
}
fn raw_from_envelope(s: &str) -> Vec<u8> {
    let bytes = hex::decode(s).unwrap();
    assert_eq!(bytes[0], 10);
    let (mut n, mut shift, mut at) = (0usize, 0, 1);
    loop {
        let b = bytes[at];
        at += 1;
        n |= ((b & 127) as usize) << shift;
        if b < 128 {
            break;
        }
        shift += 7;
    }
    assert_eq!(bytes[at + n], 18);
    assert_eq!(bytes[at + n + 1], 65);
    assert_eq!(bytes.len(), at + n + 67);
    let raw = bytes[at..at + n].to_vec();
    assert_eq!(
        tx::recover(&raw, &bytes[at + n + 2..]).unwrap(),
        tx::decode_raw(&raw).unwrap().owner
    );
    raw
}

#[tokio::test]
async fn tr17_dynamic_new_account_and_contract_maximum_does_not_depend_on_resources() {
    for (missing, contract, staked) in [
        (true, false, 0),
        (true, false, 10000),
        (false, true, 0),
        (false, false, 0),
    ] {
        let node = MockTron::start(move |path, body| {
            Reply::Json(match path {
                "/wallet/getaccount" if body["address"] == TO => {
                    if missing {
                        json!({})
                    } else {
                        let mut a = common::tron::account(TO, 0);
                        if contract {
                            a["type"] = json!("Contract");
                        }
                        a
                    }
                }
                "/wallet/getaccountresource" => json!({"NetLimit":staked,"freeNetLimit":600}),
                _ => fixture(path, body),
            })
        });
        let p = send::prepare_transfer(&net(&node), &request(false))
            .await
            .unwrap();
        let f = &p.fields["fees"];
        let raw = hex::decode(p.unsigned.as_str().unwrap().trim_start_matches("0x")).unwrap();
        let d = tx::decode_raw(&raw).unwrap();
        assert_eq!(d.expiration, d.timestamp + 60000);
        assert_eq!(
            send::parse_expiration(p.fields["expiresAt"].as_str().unwrap()),
            Some(d.expiration)
        );
        let bytes = f["bandwidthBytes"].as_u64().unwrap();
        let max = 1_000_000
            + if missing || contract {
                2_300_000u64.max(bytes * 2000)
            } else {
                bytes * 2000
            };
        assert_eq!(f["maxCostSun"], max.to_string());
        if missing {
            assert_eq!(
                f["estimatedCostSun"],
                if staked > 0 { "3000000" } else { "3300000" }
            );
        }
    }
}

#[tokio::test]
async fn tr18_preflight_refuses_unfunded_stale_reverted_and_malformed_node_state() {
    for case in 0..8 {
        let node = MockTron::start(move |path, body| {
            Reply::Json(match (path, case) {
                ("/wallet/getaccount", 0) => json!({}),
                ("/wallet/getaccount", 1) if body["address"] == FROM => {
                    common::tron::account(FROM, 2_200_000)
                }
                ("/wallet/getaccount", 1) if body["address"] == TO => json!({}),
                ("/wallet/getaccount", 2) if body["address"] == FROM => {
                    common::tron::account(FROM, 11_000_000)
                }
                ("/wallet/triggerconstantcontract", 3)
                    if body["function_selector"] == "transfer(address,uint256)" =>
                {
                    json!({"result":{"result":true},"energy_used":1,"transaction":{"ret":[{"ret":"FAILED"}]}})
                }
                ("/wallet/getblock", 4) => block(send::now_ms() - 36_000_000),
                ("/wallet/triggerconstantcontract", 5)
                    if body["function_selector"] == "transfer(address,uint256)" =>
                {
                    json!({"result":{"result":true},"energy_used":"9".repeat(600)})
                }
                ("/wallet/getaccount", 6) if body["address"] == FROM => {
                    let mut a = common::tron::account(FROM, 100_000_000);
                    a["owner_permission"] =
                        json!({"threshold":2,"keys":[{"address":FROM,"weight":1}]});
                    a
                }
                ("/wallet/triggerconstantcontract", 7) => {
                    json!({"result":{"result":true},"constant_result":["0".repeat(64)],"energy_used":100000})
                }
                _ => fixture(path, body),
            })
        });
        let e = send::prepare_transfer(&net(&node), &request(matches!(case, 2 | 3 | 5 | 7)))
            .await
            .unwrap_err();
        assert_eq!(
            e.code,
            if matches!(case, 0 | 1 | 2 | 7) {
                ErrorCode::InsufficientFunds
            } else if case == 6 {
                ErrorCode::ValidationError
            } else {
                ErrorCode::RpcError
            },
            "case {case}: {e:?}"
        );
        if case == 2 {
            assert!(e.message.contains("maxCostSun") && e.message.contains("shortfall"));
        }
        assert!(node.calls("/wallet/broadcasthex").is_empty());
    }
}

#[tokio::test]
async fn tr16_signed_reply_hash_raw_and_recovered_owner_are_checked() {
    let node = MockTron::start(|p, b| Reply::Json(fixture(p, b)));
    let p = send::prepare_transfer(&net(&node), &request(false))
        .await
        .unwrap();
    let good = signed(&p.unsigned);
    let expected = send::inspect_signed(&good, Some(&p.unsigned)).unwrap();
    assert_eq!(
        raw_from_envelope(&expected.raw),
        hex::decode(p.unsigned.as_str().unwrap().trim_start_matches("0x")).unwrap()
    );
    let mut bad = good.clone();
    bad["txHash"] = json!("11".repeat(32));
    assert_eq!(
        send::inspect_signed(&bad, Some(&p.unsigned))
            .unwrap_err()
            .code,
        ErrorCode::ValidationError
    );
    let mut raw = tx::decode_raw(
        &hex::decode(p.unsigned.as_str().unwrap().trim_start_matches("0x")).unwrap(),
    )
    .unwrap();
    raw.expiration += 1;
    let other = json!(format!("0x{}", hex::encode(tx::encode_raw(&raw).unwrap())));
    let bad = signed(&other);
    assert!(send::inspect_signed(&bad, Some(&p.unsigned)).is_err());
    let other_signer = tron::signer_at(&seed(), 1).unwrap();
    raw.owner = tron::address::parse(TO).unwrap();
    let signature = tx::sign(&other_signer, &raw).unwrap();
    let mut bad = good;
    bad["signature"] = json!(format!("0x{}", hex::encode(signature)));
    assert!(send::inspect_signed(&bad, Some(&p.unsigned)).is_err());
}

fn stored_tx(raw: &Value, ret: &str) -> Value {
    let raw = raw.as_str().unwrap().trim_start_matches("0x");
    json!({"txID":hex::encode(tx::txid(&hex::decode(raw).unwrap())),"raw_data_hex":raw,"ret":[{"contractRet":ret}]})
}
#[tokio::test]
async fn tr20_receipts_use_vm_status_not_usdt_return_word_and_actual_burn() {
    for (token, status, success) in [
        (false, "SUCCESS", true),
        (true, "SUCCESS", true),
        (true, "REVERT", false),
        (true, "OUT_OF_ENERGY", false),
    ] {
        let prep = MockTron::start(|p, b| Reply::Json(fixture(p, b)));
        let p = send::prepare_transfer(&net(&prep), &request(token))
            .await
            .unwrap();
        let t = stored_tx(&p.unsigned, status);
        let hash = t["txID"].as_str().unwrap().to_string();
        let id = hash.clone();
        let node = MockTron::start(move |path, body| {
            Reply::Json(match path {
                "/wallet/gettransactionbyid" => t.clone(),
                "/wallet/gettransactioninfobyid" => {
                    json!({"id":id,"blockNumber":9,"fee":11500000,"contractResult":["0".repeat(64)],"receipt":{"result":status,"energy_usage_total":110000,"net_usage":345}})
                }
                _ => fixture(path, body),
            })
        });
        let result = lookup::wait(&net(&node), &hash, None, Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(result["status"], "mined");
        assert_eq!(result["success"], success);
        assert_eq!(result["feeSun"], "11500000");
        assert_eq!(result["to"], TO);
        if token {
            let maximum = p.fields["fees"]["maxCostSun"]
                .as_str()
                .unwrap()
                .parse::<u64>()
                .unwrap();
            assert!(maximum >= 11500000);
            assert!(
                p.fields["fees"]["estimatedCostSun"]
                    .as_str()
                    .unwrap()
                    .parse::<u64>()
                    .unwrap()
                    < 11500000
            );
        }
        if !success {
            assert!(
                result["hint"]
                    .as_str()
                    .unwrap()
                    .contains("fees were burned")
            );
        }
    }
}

#[tokio::test]
async fn tr25_expired_unseen_requires_journal_expiration_and_a_fresh_head() {
    let hash = "11".repeat(32);
    let now = send::now_ms();
    for (head, expiry, want, lag) in [
        (now - 1000, Some(now - 31000), "not found", false),
        (now - 90000, Some(now - 120000), "not found", true),
        (now - 300000, Some(now - 360000), "not found", true),
        (now - 1000, Some(now - 121000), "expired-unseen", false),
        (now - 1000, None, "not found", false),
    ] {
        let node = MockTron::start(move |p, b| {
            Reply::Json(if p == "/wallet/getblock" {
                block(head)
            } else {
                fixture(p, b)
            })
        });
        let v = lookup::lookup_tx(&net(&node), &hash, expiry).await.unwrap();
        assert_eq!(v["status"], want, "{v:?}");
        assert_eq!(v.contains_key("nodeBehindSecs"), lag);
        if want == "expired-unseen" {
            assert!(
                v["hint"]
                    .as_str()
                    .unwrap()
                    .contains("this node returned no record")
            );
            assert!(v["hint"].as_str().unwrap().contains(lookup::SECOND_PAYMENT));
            assert!(v["asOf"].is_object());
        }
    }
}
#[tokio::test]
async fn lookup_rejects_wrong_raw_and_wait_has_one_deadline() {
    let node = MockTron::start(|p, b| {
        Reply::Json(if p == "/wallet/gettransactionbyid" {
            json!({"raw_data_hex":"0001"})
        } else {
            fixture(p, b)
        })
    });
    assert_eq!(
        lookup::lookup_tx(&net(&node), &"11".repeat(32), None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RpcError
    );
    let slow = MockTron::start(|p, b| {
        std::thread::sleep(Duration::from_millis(500));
        Reply::Json(fixture(p, b))
    });
    let now = Instant::now();
    let v = lookup::wait(
        &net(&slow),
        &"11".repeat(32),
        None,
        Duration::from_millis(30),
    )
    .await
    .unwrap();
    assert!(now.elapsed() < Duration::from_millis(300));
    assert_eq!(v["status"], "unknown");
    assert!(v["found"].is_null());
}

#[tokio::test]
async fn wait_distinguishes_incomplete_observation_from_completed_not_found() {
    let (release, blocked) = std::sync::mpsc::channel();
    let blocked = Mutex::new(blocked);
    let incomplete = MockTron::start(move |p, b| {
        if p == "/wallet/gettransactioninfobyid" {
            // The test owns completion. A timeout must not invent a completed observation.
            blocked.lock().unwrap().recv().unwrap();
        }
        Reply::Json(fixture(p, b))
    });
    let network = net(&incomplete);
    let hash = "11".repeat(32);
    let deadline = lookup::wait(&network, &hash, None, Duration::from_millis(150));
    // This real-I/O check also permits the deadline to expire during connection. The private
    // polling tests expire at known awaits to prove both sides without an RPC speed assumption.
    let result = deadline.await;
    // Release even when connecting used the deadline, so a late handler cannot be stranded.
    release.send(()).unwrap();
    let v = result.unwrap();
    assert_eq!(v["status"], "unknown");
    assert!(v["found"].is_null());

    let complete = MockTron::start(|p, b| Reply::Json(fixture(p, b)));
    let v = lookup::lookup_tx(&net(&complete), &"11".repeat(32), None)
        .await
        .unwrap();
    assert_eq!(v["status"], "not found");
    assert_eq!(v["found"], false);
    assert_eq!(complete.calls("/wallet/getblock").len(), 1);
}

#[tokio::test]
async fn tr21_broadcast_outcomes_preserve_computed_hash_and_journal() {
    let prep = MockTron::start(|p, b| Reply::Json(fixture(p, b)));
    let p = send::prepare_transfer(&net(&prep), &request(false))
        .await
        .unwrap();
    let signed = send::inspect_signed(&signed(&p.unsigned), Some(&p.unsigned)).unwrap();
    for case in 0..6 {
        let t = stored_tx(&p.unsigned, "SUCCESS");
        let node = MockTron::start(move |path, body| {
            if path == "/wallet/broadcasthex" {
                assert!(!raw_from_envelope(body["transaction"].as_str().unwrap()).is_empty());
                return match case {
                    0 => Reply::Json(json!({"result":true})),
                    1 => Reply::Json(json!({"result":false,"code":"DUP_TRANSACTION_ERROR"})),
                    2 => Reply::Json(json!({"result":false,"code":"CONTRACT_VALIDATE_ERROR"})),
                    3 | 4 => Reply::Http(503, "unavailable"),
                    _ => Reply::Json(json!({"result":true,"txid":"22".repeat(32)})),
                };
            }
            Reply::Json(if path == "/wallet/gettransactionbyid" && case == 4 {
                t.clone()
            } else {
                fixture(path, body)
            })
        });
        let home = tempfile::Builder::new()
            .prefix("zwt-")
            .tempdir_in("/tmp")
            .unwrap();
        let j = SendJournal::new(home.path(), chains::find("tron").unwrap(), FROM);
        let summary = SendSummary {
            kind: "transfer".into(),
            to: TO.into(),
            value: "1 TRX".into(),
            amount_raw: Some("1000000".into()),
            token: None,
            calldata_hash: None,
        };
        let mut record = SendRecord::new(j.clone(), &signed, &summary);
        record.entry["expiresAt"] = p.fields["expiresAt"].clone();
        let result = lookup::broadcast(&net(&node), &signed, &record).await;
        match case {
            2 => assert_eq!(result.unwrap_err().code, ErrorCode::RpcError),
            3 => {
                let e = result.unwrap_err();
                assert_eq!(e.code, ErrorCode::BroadcastUncertain);
                assert!(e.message.contains(lookup::SECOND_PAYMENT));
                assert!(e.details.contains_key("expiresAt"));
            }
            _ => assert!(result.is_ok(), "{result:?}"),
        };
        let text = std::fs::read_to_string(j.path()).unwrap();
        assert_eq!(text.lines().count(), 1);
        let entry: Value = serde_json::from_str(text.trim()).unwrap();
        assert!(entry.get("nonce").is_none());
        assert_eq!(entry["expiresAt"], p.fields["expiresAt"]);
        assert!(
            j.path()
                .to_string_lossy()
                .contains(&tron::address::hex41(&tron::address::parse(FROM).unwrap()))
        );
        assert_eq!(
            j.expiration(&signed.tx_hash),
            send::parse_expiration(p.fields["expiresAt"].as_str().unwrap())
        );
        let dupe = j
            .recent_duplicates(&summary, send::now_ms() / 1000)
            .unwrap();
        let e = j.duplicate_error(dupe);
        assert!(e.message.contains("expiresAt") && e.message.contains("a second payment"));
        assert!(e.message.contains(&signed.tx_hash));
    }
}

fn fake_home() -> Home {
    let home = Home::new();
    home.create("main", &[]);
    home.edit_wallet(|v| v["keys"][0]["addresses"]["tron"] = json!(FROM));
    home.allow(
        "bob",
        &format!("0x{}", hex::encode(&tron::address::parse(TO).unwrap()[1..])),
        &["tron"],
    );
    home
}
fn fake(home: &Home, bad: u8) -> FakeDaemon {
    FakeDaemon::bind_with(&home.sock(), move |bytes| {
        let req: Value = serde_json::from_slice(bytes).unwrap();
        let mut result = if req["op"] == "sign_tx" {
            signed(&req["tx"])
        } else {
            json!({"locked":false,"pid":1,"expiresAt":"2099-01-01T00:00:00Z"})
        };
        if req["op"] == "sign_tx" {
            match bad {
                1 => result["txHash"] = json!("00".repeat(32)),
                2 => result["rawData"] = json!("0x00"),
                3 => result["signature"] = json!(format!("0x{}", "00".repeat(65))),
                _ => {}
            }
        }
        let mut out = serde_json::to_vec(&json!({"v":2,"ok":true,"result":result})).unwrap();
        out.push(b'\n');
        out
    })
}
fn sign_count(fake: &FakeDaemon) -> usize {
    fake.received
        .lock()
        .unwrap()
        .iter()
        .filter(|b| serde_json::from_slice::<Value>(b).unwrap()["op"] == "sign_tx")
        .count()
}
fn pay<'a>(node: &'a MockTron, to: &'a str, extra: &[&'a str]) -> Vec<&'a str> {
    let mut a = vec![
        "transfer", "--chain", "tron", "--to", to, "--amount", "1", "--rpc", &node.url, "--json",
    ];
    a.extend_from_slice(extra);
    a
}

#[test]
fn tron_native_task_reports_reconcile_checked_raw_data_and_receipt_without_nonce() {
    let home = fake_home();
    let published = Arc::new(Mutex::new(None::<Value>));
    let captured = published.clone();
    let node = MockTron::start(move |path, body| {
        Reply::Json(match path {
            "/wallet/broadcasthex" => {
                let raw = raw_from_envelope(body["transaction"].as_str().unwrap());
                let id = hex::encode(tx::txid(&raw));
                *captured.lock().unwrap() = Some(
                    json!({"txID":id,"raw_data_hex":hex::encode(raw),"ret":[{"contractRet":"SUCCESS"}]}),
                );
                json!({"result":true})
            }
            "/wallet/gettransactionbyid" => captured.lock().unwrap().clone().unwrap_or(json!({})),
            "/wallet/gettransactioninfobyid" => captured
                .lock()
                .unwrap()
                .as_ref()
                .map(|v| json!({"id":v["txID"],"blockNumber":9000001,"fee":1234}))
                .unwrap_or(json!({})),
            _ => fixture(path, body),
        })
    });
    home.run(&[
        "task",
        "create",
        "tron-dev",
        "--chain",
        "tron",
        "--allocation",
        "native=2000000",
        "--rpc",
        &node.url,
        "--json",
    ])
    .ok();
    let daemon = fake(&home, 0);
    home.run(&pay(&node, "@bob", &["--task", "tron-dev"])).ok();
    let count = sign_count(&daemon);
    let r = home
        .run(&["task", "report", "tron-dev", "--rpc", &node.url, "--json"])
        .ok();
    assert_eq!(r["records"][0]["outcome"], "confirmed", "{r}");
    assert_eq!(r["records"][0]["actualFeeRaw"], "1234");
    assert!(r["records"][0]["nonce"].is_null());
    assert_eq!(sign_count(&daemon), count, "read-only report did not sign");
    let native = r["assets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["asset"] == "native")
        .unwrap();
    assert_eq!(native["remainingAllocationRaw"], "998766");
}
#[test]
fn tr19_cli_flags_and_nonregistry_tokens_fail_before_any_rpc() {
    let home = Home::new();
    let node = MockTron::start(|p, b| Reply::Json(fixture(p, b)));
    for extra in [
        vec!["--nonce", "1"],
        vec!["--nonce", "1", "--replace", "anything"],
        vec!["--nonce", "1", "--bump", "2"],
        vec!["--allow-pending"],
        vec!["--max-fee-gwei", "1"],
        vec!["--token", USDT],
    ] {
        home.run(&pay(&node, TO, &extra)).err("VALIDATION_ERROR");
    }
    assert!(node.seen().is_empty());
}
#[test]
fn tr16_tr17_cli_dry_run_and_bad_daemon_replies_never_publish_bytes() {
    for bad in 0..4 {
        let home = fake_home();
        let daemon = fake(&home, bad);
        let node = MockTron::start(|p, b| Reply::Json(fixture(p, b)));
        let run = home.run(&pay(&node, "@bob", &["--dry-run"]));
        if bad == 0 {
            let v = run.ok();
            assert_eq!(v["whitelist"]["allowed"], true);
            assert_eq!(v["recoveredFrom"], FROM);
            assert_eq!(v["signerDecoded"]["to"], TO);
            assert!(
                v.get("rawTx").is_none()
                    && v.get("rawData").is_none()
                    && v.get("signature").is_none()
            );
            let before = sign_count(&daemon);
            let v = home.run(&pay(&node, OTHER, &["--dry-run"])).ok();
            assert_eq!(v["whitelist"]["allowed"], false);
            assert_eq!(sign_count(&daemon), before);
        } else {
            run.err("VALIDATION_ERROR");
        }
        assert!(node.calls("/wallet/broadcasthex").is_empty());
        assert!(
            !SendJournal::new(home.path(), chains::find("tron").unwrap(), FROM)
                .path()
                .exists()
        );
    }
}
#[test]
fn tr18_cli_preflight_refusals_send_zero_signature_requests() {
    let home = fake_home();
    let daemon = fake(&home, 0);
    let node = MockTron::start(|p, b| {
        Reply::Json(if p == "/wallet/getaccount" {
            common::tron::account(b["address"].as_str().unwrap(), 1)
        } else {
            fixture(p, b)
        })
    });
    home.run(&pay(&node, "@bob", &[])).err("INSUFFICIENT_FUNDS");
    assert_eq!(sign_count(&daemon), 0);
}
#[test]
fn tr20_cli_send_wait_and_duplicate_are_one_journaled_payment() {
    for token in [false, true] {
        let home = fake_home();
        home.edit_whitelist(|list| list.policy.duplicate_window_secs = Some(1800));
        let daemon = fake(&home, 0);
        let state = Arc::new(Mutex::new(None::<Value>));
        let saved = state.clone();
        let node = MockTron::start(move |p, b| {
            if p == "/wallet/broadcasthex" {
                let raw = raw_from_envelope(b["transaction"].as_str().unwrap());
                *saved.lock().unwrap() = Some(stored_tx(
                    &json!(format!("0x{}", hex::encode(raw))),
                    "SUCCESS",
                ));
                return Reply::Json(json!({"result":true}));
            }
            if let Some(t) = saved.lock().unwrap().as_ref() {
                if p == "/wallet/gettransactionbyid" {
                    return Reply::Json(t.clone());
                }
                if p == "/wallet/gettransactioninfobyid" {
                    return Reply::Json(
                        json!({"id":t["txID"],"blockNumber":123,"fee":11500000,"receipt":{"result":"SUCCESS","energy_usage_total":110000},"contractResult":["0".repeat(64)]}),
                    );
                }
            }
            Reply::Json(fixture(p, b))
        });
        let mut extra = vec!["--wait", "--wait-timeout", "1s"];
        if token {
            extra.extend(["--token", "usdt"]);
        }
        let v = home.run(&pay(&node, "@bob", &extra)).ok();
        assert_eq!(v["status"], "mined");
        assert_eq!(v["success"], true);
        assert_eq!(v["duplicateGuard"]["windowSecs"], 1800);
        let path = v["journal"].as_str().unwrap();
        let mut entry: Value =
            serde_json::from_str(std::fs::read_to_string(path).unwrap().trim()).unwrap();
        entry["ts"] = json!(zwallet::core::time::rfc3339_utc(
            zwallet::core::time::unix_secs(std::time::SystemTime::now()) - 1200
        ));
        std::fs::write(path, format!("{entry}\n")).unwrap();
        assert!(v.get("nonce").is_none());
        assert_eq!(sign_count(&daemon), 1);
        let refused = home.run(&pay(&node, "@bob", &extra));
        let e = refused.err("VALIDATION_ERROR");
        assert!(e.contains("expiresAt") && e.contains("second payment"));
        assert!(e.starts_with("advisory duplicate guard (policy: local-file-unverified; window 1800s; last 1 MiB of local journal): "));
        assert_eq!(refused.json()["duplicateGuard"], v["duplicateGuard"]);
        let mut human_args = pay(&node, "@bob", &extra);
        human_args.retain(|arg| *arg != "--json");
        let human = home.run(&human_args);
        assert_ne!(human.code, 0);
        assert!(human.stderr.starts_with("error: advisory duplicate guard (policy: local-file-unverified; window 1800s; last 1 MiB of local journal): "), "{}", human.stderr);
        assert_eq!(sign_count(&daemon), 1);
        assert_eq!(node.calls("/wallet/broadcasthex").len(), 1);
    }
}

#[test]
fn tr25_cli_journal_expiration_is_observed_without_resend_language() {
    let home = fake_home();
    let hash = "33".repeat(32);
    let now = send::now_ms();
    let journal = SendJournal::new(home.path(), chains::find("tron").unwrap(), FROM);
    journal
        .append(&json!({"chain":"tron","txHash":hash,"expiresAt":send::expires_at(now-120_000)}))
        .unwrap();
    let node = MockTron::start(move |p, b| {
        Reply::Json(if p == "/wallet/getblock" {
            block(now - 1000)
        } else {
            fixture(p, b)
        })
    });
    for extra in [vec![], vec!["--wait", "--wait-timeout", "1s"]] {
        let mut args = vec!["tx", "--chain", "tron", &hash, "--rpc", &node.url, "--json"];
        args.extend(extra);
        let value = home.run(&args).ok();
        assert_eq!(value["status"], "expired-unseen");
        assert!(
            value["hint"]
                .as_str()
                .unwrap()
                .contains(lookup::SECOND_PAYMENT)
        );
    }
    let bad = json!({"chain":"tron","txHash":format!("{hash}\u{1b}[2J"),"expiresAt":send::expires_at(now),"kind":"transfer","value":"1 TRX","to":TO});
    let text = journal.entries_text(&[bad]);
    assert!(text.contains("<invalid>"), "{text}");
    assert!(!text.contains('\u{1b}'));
}

#[test]
fn tr17_fee_cap_and_every_preflight_failure_skip_the_signer() {
    let home = fake_home();
    let daemon = fake(&home, 0);
    for case in 0..7 {
        let node = MockTron::start(move |p, b| {
            Reply::Json(match (p, case) {
                ("/wallet/getaccount", 0) => json!({}),
                ("/wallet/getaccount", 1) if b["address"] == FROM => {
                    common::tron::account(FROM, 11_000_000)
                }
                ("/wallet/triggerconstantcontract", 2)
                    if b["function_selector"] == "transfer(address,uint256)" =>
                {
                    json!({"result":{"result":false,"message":hex::encode("reverted")}})
                }
                ("/wallet/triggerconstantcontract", 3)
                    if b["function_selector"] == "balanceOf(address)" =>
                {
                    json!({"result":{"result":true},"constant_result":["0".repeat(64)]})
                }
                ("/wallet/getblock", 4) => block(send::now_ms() - 36_000_000),
                ("/wallet/triggerconstantcontract", 5)
                    if b["function_selector"] == "transfer(address,uint256)" =>
                {
                    json!({"result":{"result":true},"energy_used":"9".repeat(600)})
                }
                ("/wallet/triggerconstantcontract", 6)
                    if b["function_selector"] == "transfer(address,uint256)" =>
                {
                    json!({"result":{"result":true},"energy_used":300000})
                }
                _ => fixture(p, b),
            })
        });
        let result = home.run(&pay(&node, "@bob", &["--dry-run", "--token", "usdt"]));
        if case == 6 {
            let v = result.ok();
            assert_eq!(v["whitelist"]["allowed"], false);
            assert_eq!(v["whitelist"]["code"], "FEE_CAP_EXCEEDED");
        } else {
            result.err(if matches!(case, 0 | 1 | 3) {
                "INSUFFICIENT_FUNDS"
            } else {
                "RPC_ERROR"
            });
        }
        assert_eq!(sign_count(&daemon), 0, "case {case}");
    }
}

#[tokio::test]
async fn tr20_activation_burn_fits_the_maximum_when_staked_bandwidth_disappears() {
    let node = MockTron::start(|p, b| {
        Reply::Json(match p {
            "/wallet/getaccount" if b["address"] == TO => json!({}),
            "/wallet/getaccountresource" => json!({"NetLimit":10_000}),
            _ => fixture(p, b),
        })
    });
    let prepared = send::prepare_transfer(&net(&node), &request(false))
        .await
        .unwrap();
    assert_eq!(prepared.fields["fees"]["estimatedCostSun"], "3000000");
    assert_eq!(prepared.fields["fees"]["maxCostSun"], "3300000");
    let transaction = stored_tx(&prepared.unsigned, "SUCCESS");
    let hash = transaction["txID"].as_str().unwrap().to_string();
    let id = hash.clone();
    let receipt = MockTron::start(move |p, b| {
        Reply::Json(match p {
            "/wallet/gettransactionbyid" => transaction.clone(),
            "/wallet/gettransactioninfobyid" => {
                json!({"id":id,"blockNumber":9,"fee":2300000,"receipt":{"net_fee":300000}})
            }
            _ => fixture(p, b),
        })
    });
    let observed = lookup::lookup_tx(&net(&receipt), &hash, None)
        .await
        .unwrap();
    assert_eq!(observed["feeSun"], "2300000");
    assert_eq!(observed["success"], true);
    assert_eq!(
        observed["feeSun"].as_str().unwrap().parse::<u64>().unwrap() + 1_000_000,
        3_300_000
    );
}

#[test]
fn tr25_source_and_readme_never_claim_that_repeating_an_unknown_send_is_safe() {
    fn paths(root: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(root).unwrap().flatten() {
            let p = entry.path();
            if p.is_dir() {
                paths(&p, out);
            } else if p.extension().is_some_and(|e| e == "rs") {
                out.push(p);
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = vec![root.join("README.md")];
    paths(&root.join("src"), &mut files);
    for p in files {
        let text = std::fs::read_to_string(&p).unwrap().to_lowercase();
        for forbidden in [
            "cannot double-pay",
            "can't double-pay",
            "no double payment",
            "cannot be paid twice",
            "safe to send again",
            "safe to resend",
            "safe to retry",
            "you can resend",
            "until then do not send",
            "after that, send",
            "can never be included",
            "was never included",
            "was not paid",
            "definitively",
        ] {
            assert!(!text.contains(forbidden), "{}: {forbidden}", p.display());
        }
    }
}

#[tokio::test]
async fn historical_bytes_outside_signer_subset_keep_the_nodes_execution_result() {
    // A memo is refused for new signatures, but should not turn a successful historical
    // native transfer into a failed lookup. Its opaque bytes must still hash to this txID.
    let prep = MockTron::start(|p, b| Reply::Json(fixture(p, b)));
    let prepared = send::prepare_transfer(&net(&prep), &request(false))
        .await
        .unwrap();
    let mut raw =
        hex::decode(prepared.unsigned.as_str().unwrap().trim_start_matches("0x")).unwrap();
    raw.extend_from_slice(&[0x52, 1, b'x']);
    assert!(tx::decode_raw(&raw).is_err());
    let hash = hex::encode(tx::txid(&raw));
    let id = hash.clone();
    let node = MockTron::start(move |p, b| {
        Reply::Json(match p {
            "/wallet/gettransactionbyid" => {
                json!({"raw_data_hex":hex::encode(&raw),"txID":id,"ret":[{"contractRet":"SUCCESS"}]})
            }
            "/wallet/gettransactioninfobyid" => {
                json!({"id":id,"blockNumber":9,"receipt":{},"fee":1})
            }
            _ => fixture(p, b),
        })
    });
    let v = lookup::lookup_tx(&net(&node), &hash, None).await.unwrap();
    assert_eq!(v["status"], "mined");
    assert_eq!(v["success"], true);
    assert!(!v.contains_key("to"));
}

#[tokio::test]
async fn missing_and_unknown_execution_results_are_not_a_failed_payment() {
    for token in [false, true] {
        for status in [
            None,
            Some(Value::Null),
            Some(json!("NEW_FUTURE_STATUS")),
            Some(json!("DEFAULT")),
            Some(json!("UNKNOWN")),
        ] {
            let prep = MockTron::start(|p, b| Reply::Json(fixture(p, b)));
            let prepared = send::prepare_transfer(&net(&prep), &request(token))
                .await
                .unwrap();
            let mut transaction = stored_tx(&prepared.unsigned, "SUCCESS");
            if !token {
                transaction["ret"] = json!([{}]);
                if let Some(s) = &status {
                    transaction["ret"][0]["contractRet"] = s.clone();
                }
            }
            let hash = transaction["txID"].as_str().unwrap().to_string();
            let id = hash.clone();
            let node = MockTron::start(move |p, b| {
                Reply::Json(match p {
                    "/wallet/gettransactionbyid" => transaction.clone(),
                    "/wallet/gettransactioninfobyid" => {
                        let mut info = json!({"id":id,"blockNumber":9,"receipt":{},"fee":1});
                        if token && let Some(s) = &status {
                            info["receipt"]["result"] = s.clone();
                        }
                        info
                    }
                    _ => fixture(p, b),
                })
            });
            let v = lookup::lookup_tx(&net(&node), &hash, None).await.unwrap();
            assert_eq!(v["status"], "mined");
            assert!(v["success"].is_null(), "{v:?}");
            let hint = v["hint"].as_str().unwrap();
            assert!(hint.contains("execution result is unknown"));
            assert!(hint.contains(lookup::SECOND_PAYMENT));
            assert!(!hint.contains("transaction failed"));
        }
    }
}

#[test]
fn real_daemon_and_cli_sign_both_tron_assets_for_the_mock_node() {
    for token in [false, true] {
        let home = Home::new();
        home.create("main", &[]);
        let from = home.run(&["address", "--chain", "tron", "--json"]).ok()["address"]
            .as_str()
            .unwrap()
            .to_owned();
        home.allow(
            "bob",
            &format!("0x{}", hex::encode(&tron::address::parse(TO).unwrap()[1..])),
            &["tron"],
        );
        home.unlock("1m");
        let state = Arc::new(Mutex::new(None::<Value>));
        let saved = state.clone();
        let expected_owner = from.clone();
        let node = MockTron::start(move |path, body| {
            if path == "/wallet/broadcasthex" {
                let raw = raw_from_envelope(body["transaction"].as_str().unwrap());
                assert_eq!(
                    tron::address::encode(&tx::decode_raw(&raw).unwrap().owner),
                    expected_owner
                );
                *saved.lock().unwrap() = Some(stored_tx(
                    &json!(format!("0x{}", hex::encode(raw))),
                    "SUCCESS",
                ));
                return Reply::Json(json!({"result":true}));
            }
            if let Some(transaction) = saved.lock().unwrap().as_ref() {
                if path == "/wallet/gettransactionbyid" {
                    return Reply::Json(transaction.clone());
                }
                if path == "/wallet/gettransactioninfobyid" {
                    return Reply::Json(
                        json!({"id":transaction["txID"],"blockNumber":123,"fee":1000,"receipt":{"result":"SUCCESS"}}),
                    );
                }
            }
            Reply::Json(fixture(path, body))
        });
        let mut options = vec!["--wait", "--wait-timeout", "1s"];
        if token {
            options.extend(["--token", "usdt"]);
        }
        let result = home.run(&pay(&node, "@bob", &options)).ok();
        assert_eq!(result["status"], "mined");
        assert_eq!(result["success"], true);
        assert!(result.get("nonce").is_none());
        assert_eq!(node.calls("/wallet/broadcasthex").len(), 1);
        let journal = SendJournal::new(home.path(), chains::find("tron").unwrap(), &from);
        assert!(
            journal
                .expiration(result["txHash"].as_str().unwrap())
                .is_some()
        );
    }
}
