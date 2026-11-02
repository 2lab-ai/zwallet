//! Native TON public CLI acceptance with an isolated wallet, real key daemon and loopback-only
//! TON Center compatible replies. No owner wallet, live RPC or real assets.
mod common;
use common::{
    Home, PW,
    tron::{MockTron, Reply},
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use zwallet::chains::{self, ChainFamily, ton::Ton};
const TO: &str = "0:1111111111111111111111111111111111111111111111111111111111111111";
fn has_sensitive_payload(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.iter().any(|(key, value)| {
            matches!(
                key.to_ascii_lowercase().as_str(),
                "rawtx"
                    | "signature"
                    | "signedboc"
                    | "privatekey"
                    | "secretkey"
                    | "mnemonic"
                    | "seed"
            ) || has_sensitive_payload(value)
        }),
        Value::Array(values) => values.iter().any(has_sensitive_payload),
        _ => false,
    }
}
fn genesis() -> Value {
    json!({"init":{"root_hash":"F6OpKZKqvqeFp6CQmFomXNMfMj2EnaUSOXN+Mh+wVWk=","file_hash":"XplPz01CXAps5qeSWUtxcyBfdAo5zVb1N979KLSKD24=","seqno":0,"workchain":-1}})
}
#[test]
fn cli_same_seed_ton_receive_policy_dryrun_broadcast_and_body_lookup() {
    let _exclusive = common::exclusive();
    let home = Home::new();
    home.create("main", &[]);
    home.run_stdin(
        &[
            "whitelist",
            "add",
            "bob",
            TO,
            "--chains",
            "ton",
            "--password-stdin",
            "--json",
        ],
        &format!("{PW}\n"),
    )
    .ok();
    home.unlock("10m");
    let address = home.run(&["address", "--chain", "ton", "--json"]).ok();
    let from = address["address"].as_str().unwrap().to_string();
    assert!(from.starts_with("0:"));
    assert_eq!(address["walletVersion"], "v4r2");
    assert_eq!(address["derivationPath"], "m/44'/607'/0'");
    let captures = Arc::new(Mutex::new(Vec::<String>::new()));
    let captured = captures.clone();
    let node = MockTron::start(move |path, body| {
        if path.starts_with("/api/v3/transactionsByMessage?") {
            assert!(path.contains("body_hash="));
            return Reply::Json(json!({"transactions":[]}));
        }
        let result = match body["method"].as_str().unwrap() {
            "getMasterchainInfo" => genesis(),
            "getAddressInformation" => json!({"state":"uninitialized","balance":"10000000000"}),
            "sendBoc" => {
                captured
                    .lock()
                    .unwrap()
                    .push(body["params"]["boc"].as_str().unwrap().to_string());
                json!({"@type":"ok"})
            }
            other => panic!("unexpected TON RPC method {other}"),
        };
        Reply::Json(json!({"jsonrpc":"2.0","id":body["id"],"ok":true,"result":result}))
    });
    let rpc = format!("{}/api/v2/jsonRPC", node.url);
    let balance = home
        .run(&["balance", "--chain", "ton", "--rpc", &rpc, "--json"])
        .ok();
    assert_eq!(balance["balance"], "10");
    assert_eq!(balance["symbol"], "TON");
    let args = [
        "transfer",
        "--chain",
        "ton",
        "--to",
        "@bob",
        "--amount",
        "1.23456789",
        "--rpc",
        &rpc,
        "--json",
    ];
    let mut dry = args.to_vec();
    dry.push("--dry-run");
    let output = home.run(&dry);
    let v = output.ok();
    assert_eq!(v["dryRun"], true);
    assert_eq!(v["to"], TO);
    assert_eq!(v["amountRaw"], "1234567890");
    assert_eq!(v["hashKind"], "ton-signed-body");
    assert!(v.get("rawTx").is_none());
    assert!(!has_sensitive_payload(&v));
    assert!(captures.lock().unwrap().is_empty());
    let sent = home.run(&args).ok();
    assert_eq!(sent["dryRun"], false);
    assert_eq!(sent["hashKind"], "ton-signed-body");
    assert!(sent.get("rawTx").is_none());
    let hash = sent["txHash"].as_str().unwrap();
    assert_eq!(hash.len(), 64);
    let journal = sent["journal"].as_str().unwrap();
    assert!(journal.starts_with(home.path().to_str().unwrap()));
    let records = std::fs::read_to_string(journal).unwrap();
    assert!(records.contains(hash));
    use base64::Engine;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(&captures.lock().unwrap()[0])
        .unwrap();
    let inspected = Ton
        .inspect_signed(
            chains::find("ton").unwrap(),
            &json!({"rawTx":format!("0x{}",alloy::hex::encode(raw))}),
        )
        .unwrap();
    assert!(!output.stdout.contains(&inspected.raw));
    assert!(!sent.to_string().contains(&inspected.raw));
    assert!(!has_sensitive_payload(&sent));
    assert_eq!(inspected.signer, from);
    assert_eq!(inspected.tx_hash, hash);
    assert!(inspected.nonce.is_none());
    let lookup = home
        .run(&["tx", "--chain", "ton", hash, "--rpc", &rpc, "--json"])
        .ok();
    assert_eq!(lookup["found"], false);
    assert_eq!(lookup["hashKind"], "ton-signed-body");
    assert_eq!(lookup["status"], "not found");
    let duplicate = home.run(&args);
    assert_ne!(duplicate.code, 0);
    assert_eq!(captures.lock().unwrap().len(), 1, "duplicate was broadcast");
    let blocked = home.run(&[
        "transfer",
        "--chain",
        "ton",
        "--to",
        "0:2222222222222222222222222222222222222222222222222222222222222222",
        "--amount",
        "1",
        "--rpc",
        &rpc,
        "--json",
    ]);
    assert_ne!(blocked.code, 0);
    assert_eq!(
        captures.lock().unwrap().len(),
        1,
        "unlisted recipient was broadcast"
    );
}

#[test]
fn cli_ton_fee_estimate_above_cap_refuses_and_at_cap_can_dry_run() {
    use base64::Engine;
    use std::sync::atomic::{AtomicU64, Ordering};
    let home = Home::new();
    home.create("main", &[]);
    home.allow("bob", &chains::ton::policy_address(TO).unwrap(), &["ton"]);
    home.edit_whitelist(|list| {
        list.policy
            .max_fee_wei
            .insert("ton".into(), "50000000".into());
    });
    let (unsealed, _) =
        zwallet::core::store::unlock(home.path(), PW, &chains::derive_addresses).unwrap();
    let public = chains::ed25519::signer_at(&unsealed.seed, 607, 0, None)
        .unwrap()
        .verifying_key()
        .to_bytes();
    drop(unsealed);
    // V4R2 current data: seqno7, wallet-id, 256-bit public key, empty plugin dict; top-up bit.
    let mut data = vec![0xb5, 0xee, 0x9c, 0x72, 1, 1, 1, 1, 0, 43, 0, 0, 81];
    data.extend(7u32.to_be_bytes());
    data.extend(chains::ton::TON_WALLET_ID.to_be_bytes());
    data.extend(public);
    data.push(0x40);
    let data = base64::engine::general_purpose::STANDARD.encode(data);
    let code = base64::engine::general_purpose::STANDARD
        .encode(include_bytes!("../src/chains/ton/wallet-v4r2.boc"));
    home.unlock("10m");
    let estimate = Arc::new(AtomicU64::new(50_000_000));
    let amount = estimate.clone();
    let broadcasts = Arc::new(AtomicU64::new(0));
    let sends = broadcasts.clone();
    let node = MockTron::start(move |_, body| {
        let result = match body["method"].as_str().unwrap() {
            "getMasterchainInfo" => genesis(),
            "getAddressInformation" => {
                json!({"state":"active","balance":"10000000000","code":code,"data":data})
            }
            "estimateFee" => {
                json!({"source_fees":{"in_fwd_fee":0,"storage_fee":0,"gas_fee":amount.load(Ordering::SeqCst),"fwd_fee":0}})
            }
            "sendBoc" => {
                sends.fetch_add(1, Ordering::SeqCst);
                json!({"@type":"ok"})
            }
            _ => panic!("unexpected TON fee RPC"),
        };
        Reply::Json(json!({"jsonrpc":"2.0","id":body["id"],"ok":true,"result":result}))
    });
    let rpc = format!("{}/api/v2/jsonRPC", node.url);
    let args = [
        "transfer",
        "--chain",
        "ton",
        "--to",
        "@bob",
        "--amount",
        "1",
        "--rpc",
        &rpc,
        "--dry-run",
        "--json",
    ];
    let above = home.run(&args);
    assert_ne!(above.code, 0, "fee estimate above signed cap was accepted");
    let v = above.json();
    assert_eq!(v["errorCode"], "FEE_CAP_EXCEEDED");
    assert!(v.get("txHash").is_none());
    assert!(v.get("rawTx").is_none());
    assert_eq!(broadcasts.load(Ordering::SeqCst), 0);
    // ceil(41,666,666 * 1.2) = 50,000,000, exactly the configured cap/reserve.
    estimate.store(41_666_666, Ordering::SeqCst);
    let at = home.run(&args).ok();
    assert_eq!(at["feeEstimateRaw"], "50000000");
    assert_eq!(at["dryRun"], true);
    assert!(at["txHash"].as_str().is_some());
    assert_eq!(broadcasts.load(Ordering::SeqCst), 0);
}
