use super::*;
use crate::core::{
    seed::TEST_MNEMONIC,
    whitelist::{Entry, Whitelist},
};
use zeroize::Zeroizing;
fn fixture() -> Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/ton/v4r2-sdk.json")).unwrap()
}
fn seed() -> Seed {
    Seed::from_phrase(Zeroizing::new(TEST_MNEMONIC.into())).unwrap()
}
fn chain() -> &'static Chain {
    chains::find("ton").unwrap()
}
fn listed(to: &str) -> Whitelist {
    let mut l = Whitelist::empty(&chains::WHITELIST_RULES);
    l.generation = 1;
    l.entries.push(Entry::new_address(
        "payee",
        &policy_address(to).unwrap(),
        &["ton"],
    ));
    l
}
fn live_intent() -> tx::Intent {
    tx::Intent {
        to: [0x11; 32],
        amount: 1_234_567_890,
        seqno: 7,
        valid_until: (now() + 120) as u32,
    }
}
fn value(i: &tx::Intent) -> Value {
    json!(format!(
        "0x{}",
        alloy::hex::encode(cell::encode(&tx::unsigned(i)))
    ))
}
#[test]
fn independent_sdk_address_stateinit_signing_and_external_vectors() {
    let f = fixture();
    let key = chains::ed25519::signer_at(&seed(), 607, 0, None).unwrap();
    assert_eq!(
        alloy::hex::encode(key.verifying_key().to_bytes()),
        f["publicKey"]
    );
    assert_eq!(Ton.derive_address(&seed(), 0).unwrap(), f["address"]);
    assert_eq!(alloy::hex::encode(tx::code().hash()), f["codeHash"]);
    assert_eq!(
        alloy::hex::encode(tx::state_init(&key.verifying_key().to_bytes()).hash()),
        f["stateInitHash"]
    );
    let i = tx::Intent {
        to: [0x11; 32],
        amount: 1_234_567_890,
        seqno: 7,
        valid_until: 1_900_000_000,
    };
    let u = tx::unsigned(&i);
    assert_eq!(alloy::hex::encode(u.hash()), f["unsignedHash"]);
    assert_eq!(alloy::hex::encode(cell::encode(&u)), f["unsignedBoc"]);
    let external = tx::sign(&key, &u);
    let signed = tx::inspect(&external).unwrap();
    assert_eq!(alloy::hex::encode(signed.body.hash()), f["signedBodyHash"]);
    assert_eq!(alloy::hex::encode(external.hash()), f["externalHash"]);
    let independent =
        cell::decode(&alloy::hex::decode(f["externalBoc"].as_str().unwrap()).unwrap()).unwrap();
    assert_eq!(tx::inspect(&independent).unwrap().intent, i);
    assert_eq!(independent.hash(), external.hash());
}
#[test]
fn address_crc_network_workchain_and_canonical_identity() {
    let f = fixture();
    let raw = f["address"].as_str().unwrap();
    assert_eq!(
        normalize_address(f["friendly"].as_str().unwrap()).unwrap(),
        raw
    );
    let h = address::parse(raw).unwrap();
    for tag in [0x11, 0x51] {
        assert_eq!(address::parse(&address::friendly(&h, tag)).unwrap(), h);
    }
    assert!(address::parse(&address::friendly(&h, 0x91)).is_err());
    let mut bad = address::friendly(&h, 0x51).into_bytes();
    bad[3] = b'a';
    assert!(address::parse(std::str::from_utf8(&bad).unwrap()).is_err());
    assert!(normalize_address(&raw.replacen("0:", "-1:", 1)).is_err());
    assert_eq!(policy_address(raw).unwrap(), format!("ton:{raw}"));
}
#[test]
fn policy_checks_actual_recipient_amount_and_expiry() {
    let from = Ton.derive_address(&seed(), 0).unwrap();
    let i = live_intent();
    let to = address::raw(&i.to);
    let mut list = listed(&to);
    let u = value(&i);
    check_unsigned(chain(), &from, &u, &ListView::Usable(&list)).unwrap();
    assert!(check_unsigned(chain(), &from, &u, &ListView::NoList).is_err());
    list.entries[0].chains = vec!["solana".into()];
    assert!(check_unsigned(chain(), &from, &u, &ListView::Usable(&list)).is_err());
    list = listed(&to);
    list.policy.max_fee_wei.insert("ton".into(), "1".into());
    assert!(check_unsigned(chain(), &from, &u, &ListView::Usable(&list)).is_err());
    let mut expired = i;
    expired.valid_until = now() as u32;
    assert!(
        check_unsigned(
            chain(),
            &from,
            &value(&expired),
            &ListView::Usable(&listed(&to))
        )
        .is_err()
    );
    expired.valid_until = (now() + 301) as u32;
    assert!(
        check_unsigned(
            chain(),
            &from,
            &value(&expired),
            &ListView::Usable(&listed(&to))
        )
        .is_err()
    );
}
#[test]
fn signed_bytes_verified_and_payload_change_rejected() {
    let i = live_intent();
    let u = value(&i);
    let signed = Ton
        .sign_tx(
            &seed(),
            0,
            chain(),
            &u,
            &ListView::Usable(&listed(&address::raw(&i.to))),
        )
        .unwrap();
    let s = Ton.inspect_signed(chain(), &signed).unwrap();
    assert!(s.nonce.is_none());
    verify_signed_payload(&s, &u).unwrap();
    let mut other = i;
    other.amount += 1;
    assert!(verify_signed_payload(&s, &value(&other)).is_err());
    let mut wrong = s;
    wrong.signer = address::raw(&[0x22; 32]);
    assert!(verify_signed_payload(&wrong, &u).is_err());
}
#[test]
fn unsupported_modes_ops_extra_refs_and_malformed_cells_are_rejected() {
    let i = live_intent();
    let u = tx::unsigned(&i);
    for (byte, v) in [(12, 1), (13, 128), (13, 32), (13, 1)] {
        let mut c = (*u).clone();
        c.data[byte] = v;
        assert!(tx::decode_unsigned(&c).is_err());
    }
    let mut extra = (*u).clone();
    extra.refs.push(u.refs[0].clone());
    assert!(tx::decode_unsigned(&extra).is_err());
    let bytes = cell::encode(&u);
    for n in 0..bytes.len() {
        assert!(cell::decode(&bytes[..n]).is_err());
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(cell::decode(&trailing).is_err());
    let mut exotic = bytes;
    exotic[11] |= 8;
    assert!(cell::decode(&exotic).is_err());
}
#[test]
fn shared_dag_hash_work_is_bounded_by_unique_cells() {
    let mut c = cell::Builder::default().end();
    for _ in 0..30 {
        let mut b = cell::Builder::default();
        for _ in 0..4 {
            b.reference(c.clone());
        }
        c = b.end();
    }
    let bytes = cell::encode(&c);
    assert!(bytes.len() < 256);
    let read = cell::decode(&bytes).unwrap();
    assert_eq!(read.hash(), c.hash());
    assert!(tx::decode_unsigned(&read).is_err());
}

async fn server(replies: Vec<(&'static str, Value)>) -> (Net, tokio::task::JoinHandle<Vec<Value>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/api/v2/jsonRPC", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for (want, result) in replies {
            let (mut stream, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut bytes = Vec::new();
            let mut buf = [0; 4096];
            let (header, end) = loop {
                let n = stream.read(&mut buf).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
                if let Some(p) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                    let h = std::str::from_utf8(&bytes[..p]).unwrap().to_string();
                    let len = h
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(str::trim)
                                .and_then(|s| s.parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= p + 4 + len {
                        break (h, p + 4);
                    }
                }
            };
            let get = header.starts_with("GET ");
            let req: Value = if get {
                assert!(header.contains("/api/v3/transactionsByMessage?"));
                assert!(header.contains("body_hash="));
                assert!(header.contains("direction=in"));
                json!({"method":"lookup"})
            } else {
                serde_json::from_slice(&bytes[end..]).unwrap()
            };
            assert_eq!(req["method"], want);
            requests.push(req);
            let status = result
                .get("__httpStatus")
                .and_then(Value::as_u64)
                .unwrap_or(200);
            let response = if get {
                result
            } else {
                json!({"jsonrpc":"2.0","id":1,"ok":true,"result":result})
            }
            .to_string();
            let header = format!(
                "HTTP/1.1 {status} Reply\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response.len()
            );
            stream.write_all(header.as_bytes()).await.unwrap();
            stream.write_all(response.as_bytes()).await.unwrap();
        }
        requests
    });
    (
        Net {
            chain: chain(),
            rpc_url: url,
        },
        task,
    )
}
fn genesis() -> Value {
    json!({"init":{"root_hash":rpc::ZERO_ROOT,"file_hash":rpc::ZERO_FILE,"seqno":0,"workchain":-1}})
}
fn uninit() -> Value {
    json!({"balance":"10000000000","state":"uninitialized"})
}
fn active() -> Value {
    let public = chains::ed25519::signer_at(&seed(), 607, 0, None)
        .unwrap()
        .verifying_key()
        .to_bytes();
    json!({"balance":"10000000000","state":"active","code":STANDARD.encode(cell::encode(&tx::code())),"data":STANDARD.encode(cell::encode(&tx::data(&public,8)))})
}
fn request(from: String) -> TransferRequest {
    TransferRequest {
        from,
        to: address::raw(&[0x11; 32]),
        amount: "1.23456789".into(),
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
#[tokio::test]
async fn native_balance_and_unsigned_prepare_do_not_sign_or_leak_secret() {
    let from = Ton.derive_address(&seed(), 0).unwrap();
    let (net, task) = server(vec![
        ("getMasterchainInfo", genesis()),
        ("getAddressInformation", uninit()),
        ("getMasterchainInfo", genesis()),
        ("getAddressInformation", uninit()),
    ])
    .await;
    assert_eq!(
        Ton.native_balance(&net, &from).await.unwrap().raw,
        U256::from(10_000_000_000u64)
    );
    let p = Ton.prepare_transfer(&net, &request(from)).await.unwrap();
    assert_eq!(p.fields["amountRaw"], "1234567890");
    assert_eq!(p.fields["feeEstimateRaw"], "50000000");
    assert_eq!(p.fields["feeEstimateSource"], "fixed-deployment-reserve");
    assert_eq!(p.fields["bounce"], false);
    let decoded = inspect_unsigned(chain(), &p.unsigned).unwrap();
    assert_eq!(decoded["amountRaw"], "1234567890");
    assert!(decoded.get("from").is_none());
    assert!(decoded.get("feeRaw").is_none());
    assert_eq!(task.await.unwrap().len(), 4);
}
#[tokio::test]
async fn wrong_zerostate_refuses_before_balance_or_signing() {
    let mut g = genesis();
    g["init"]["root_hash"] = json!("testnet");
    let (net, task) = server(vec![("getMasterchainInfo", g)]).await;
    assert!(
        Ton.native_balance(&net, &address::raw(&[1; 32]))
            .await
            .is_err()
    );
    assert_eq!(task.await.unwrap().len(), 1);
}
#[tokio::test]
async fn offline_signed_deployment_broadcast_checks_seqno_and_body() {
    let mut i = live_intent();
    i.seqno = 0;
    let to = address::raw(&i.to);
    let v = Ton
        .sign_tx(
            &seed(),
            0,
            chain(),
            &value(&i),
            &ListView::Usable(&listed(&to)),
        )
        .unwrap();
    let s = Ton.inspect_signed(chain(), &v).unwrap();
    let (net, task) = server(vec![
        ("getMasterchainInfo", genesis()),
        ("getAddressInformation", uninit()),
        ("sendBoc", json!({"@type":"ok"})),
    ])
    .await;
    let home = tempfile::Builder::new()
        .prefix("zwt-")
        .tempdir_in("/tmp")
        .unwrap();
    let summary = chains::SendSummary {
        kind: "transfer".into(),
        to,
        value: "1.23456789 TON".into(),
        amount_raw: Some(i.amount.to_string()),
        token: None,
        calldata_hash: None,
    };
    let record = SendRecord::new(
        chains::SendJournal::new(home.path(), chain(), &s.signer),
        &s,
        &summary,
    );
    assert_eq!(
        Ton.broadcast(&net, &s, false, &record).await.unwrap(),
        Broadcast::Accepted
    );
    let requests = task.await.unwrap();
    let bytes = STANDARD
        .decode(requests[2]["params"]["boc"].as_str().unwrap())
        .unwrap();
    assert_eq!(
        tx::inspect(&cell::decode(&bytes).unwrap()).unwrap().intent,
        i
    );
    assert!(record.journal.path().exists());
}
fn indexed() -> Value {
    let f = fixture();
    let body = STANDARD.encode(alloy::hex::decode(f["signedBodyBoc"].as_str().unwrap()).unwrap());
    json!({"transactions":[{"hash":alloy::hex::encode([0x88;32]),"account":f["address"],"mc_block_seqno":100,"emulated":false,"total_fees":"3000000","in_msg":{"source":null,"destination":f["address"],"message_content":{"hash":f["signedBodyHash"],"body":body}},"out_msgs":[{"source":f["address"],"destination":f["to"],"value":f["amountRaw"]}],"description":{"aborted":false,"compute_ph":{"success":true},"action":{"success":true}}}]})
}
#[tokio::test]
async fn receipt_requires_exact_body_signature_and_actual_outgoing_payment() {
    let f = fixture();
    let (net, task) = server(vec![
        ("getMasterchainInfo", genesis()),
        ("lookup", indexed()),
        ("getAddressInformation", active()),
    ])
    .await;
    let v = Ton
        .lookup_tx(&net, f["signedBodyHash"].as_str().unwrap())
        .await
        .unwrap();
    assert_eq!(v["success"], true);
    assert_eq!(v["hashKind"], "ton-signed-body");
    assert_ne!(v["txHash"], v["transactionHash"]);
    assert_eq!(v["executionEvidence"], "outgoing-message-emitted");
    task.await.unwrap();
    for mutation in [
        "missing-output",
        "emulated",
        "missing-block",
        "compute-failed",
        "wrong-value",
    ] {
        let mut index = indexed();
        let r = &mut index["transactions"][0];
        match mutation {
            "missing-output" => r["out_msgs"] = json!([]),
            "emulated" => r["emulated"] = json!(true),
            "missing-block" => r["mc_block_seqno"] = Value::Null,
            "compute-failed" => r["description"]["compute_ph"]["success"] = json!(false),
            _ => r["out_msgs"][0]["value"] = json!("1"),
        };
        let (net, task) = server(vec![
            ("getMasterchainInfo", genesis()),
            ("lookup", index),
            ("getAddressInformation", active()),
        ])
        .await;
        let v = Ton
            .lookup_tx(&net, f["signedBodyHash"].as_str().unwrap())
            .await
            .unwrap();
        assert_ne!(v["success"], true, "{mutation}");
        task.await.unwrap();
    }
}
#[tokio::test]
async fn foreign_body_is_not_reported_as_payment() {
    let mut index = indexed();
    index["transactions"][0]["in_msg"]["message_content"]["hash"] =
        json!(alloy::hex::encode([0x55; 32]));
    let (net, task) = server(vec![("getMasterchainInfo", genesis()), ("lookup", index)]).await;
    assert!(
        Ton.lookup_tx(&net, fixture()["signedBodyHash"].as_str().unwrap())
            .await
            .is_err()
    );
    task.await.unwrap();
}
#[tokio::test]
async fn wait_does_not_return_an_emulated_receipt_as_confirmed() {
    let f = fixture();
    let mut index = indexed();
    index["transactions"][0]["emulated"] = json!(true);
    let (net, task) = server(vec![
        ("getMasterchainInfo", genesis()),
        ("lookup", index),
        ("getAddressInformation", active()),
    ])
    .await;
    assert!(
        Ton.wait_for_receipt(
            &net,
            f["signedBodyHash"].as_str().unwrap(),
            Duration::from_millis(100)
        )
        .await
        .unwrap()
        .is_none()
    );
    task.await.unwrap();
}

#[tokio::test]
async fn rate_limited_reads_retry_but_send_is_never_repeated() {
    let (net, task) = server(vec![
        ("getMasterchainInfo", genesis()),
        ("getAddressInformation", json!({"__httpStatus":429})),
        ("getAddressInformation", uninit()),
    ])
    .await;
    assert_eq!(
        Ton.native_balance(&net, fixture()["address"].as_str().unwrap())
            .await
            .unwrap()
            .raw,
        U256::from(10_000_000_000u64)
    );
    assert_eq!(task.await.unwrap().len(), 3);
    let (net, task) = server(vec![
        ("getMasterchainInfo", genesis()),
        ("sendBoc", json!({"__httpStatus":429})),
    ])
    .await;
    let c = rpc::connect(&net).await.unwrap();
    let err = c
        .call("sendBoc", json!({"boc":"public-test-placeholder"}))
        .await
        .unwrap_err();
    assert!(err.message.contains("429"));
    assert_eq!(task.await.unwrap().len(), 2);
}
#[tokio::test]
async fn malformed_source_evidence_is_not_external_message() {
    for source in [json!(true), json!(17), json!([]), json!({})] {
        let mut index = indexed();
        index["transactions"][0]["in_msg"]["source"] = source;
        let (net, task) = server(vec![("getMasterchainInfo", genesis()), ("lookup", index)]).await;
        assert!(
            Ton.lookup_tx(&net, fixture()["signedBodyHash"].as_str().unwrap())
                .await
                .is_err()
        );
        task.await.unwrap();
    }
}

#[tokio::test]
async fn default_provider_pacing_spaces_the_first_send_after_its_preflight() {
    let (net, task) = server(vec![
        ("getMasterchainInfo", genesis()),
        ("getAddressInformation", uninit()),
        ("sendBoc", json!({"@type":"ok"})),
    ])
    .await;
    let mut c = rpc::connect(&net).await.unwrap();
    c.enable_test_pacing();
    let started = std::time::Instant::now();
    c.call(
        "getAddressInformation",
        json!({"address":fixture()["address"]}),
    )
    .await
    .unwrap();
    c.call("sendBoc", json!({"boc":"public-test-placeholder"}))
        .await
        .unwrap();
    assert!(
        started.elapsed() >= Duration::from_millis(1100),
        "request start pacing was skipped"
    );
    assert_eq!(task.await.unwrap().len(), 3);
}
