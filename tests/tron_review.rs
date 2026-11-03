//! T40 review/confirm tests use loopback nodes, an isolated mock signer and no funded wallet.
mod common;
use alloy::{hex, primitives::U256, signers::SignerSync};
use common::{
    FakeDaemon,
    tron::{MAINNET_GENESIS, MockTron, Reply},
};
use serde_json::{Value, json};
use std::{
    os::unix::fs::PermissionsExt,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
use zeroize::Zeroizing;
use zwallet::{
    chains::{
        self, Net,
        tron::{self, send, tx},
    },
    core::{
        error::ErrorCode,
        seed::Seed,
        whitelist::{Entry, WHITELIST_FILE, Whitelist, WhitelistKey},
    },
    wallet::{self, CheckedList, tron_send::*},
};
const FROM: &str = "TWer2Ygk5TEheHp3TPuYeqxmB6SsGZmaL6";
const TO: &str = "TPjjvMwjPoDC32V2dGDYTkLH4E5LAtBZ6C";
const OTHER: &str = "TPdMMFcLKgJiUSmYsViAFvf6Nsb2WV52vs";
fn body(address: &str) -> String {
    format!(
        "0x{}",
        hex::encode(&tron::address::parse(address).unwrap()[1..])
    )
}
fn block(timestamp: u64) -> Value {
    let n = 9_000_000u64;
    let mut id = [0x77u8; 32];
    id[..8].copy_from_slice(&n.to_be_bytes());
    json!({"blockID":hex::encode(id),"block_header":{"raw_data":{"number":n,"timestamp":timestamp}}})
}
fn fixture(path: &str, b: &Value, price: u64, energy: u64, head: u64) -> Value {
    match path {
        "/wallet/getblockbynum" => common::tron::block0(MAINNET_GENESIS),
        "/wallet/getblock" => block(head),
        "/wallet/getaccount" => {
            common::tron::account(b["address"].as_str().unwrap(), 1_000_000_000)
        }
        "/wallet/getaccountresource" => json!({}),
        "/wallet/getchainparameters" => {
            json!({"chainParameter":[{"key":"getTransactionFee","value":price},{"key":"getEnergyFee","value":100},{"key":"getCreateNewAccountFeeInSystemContract","value":2000000},{"key":"getCreateAccountFee","value":300000},{"key":"getCreateNewAccountBandwidthRate","value":3},{"key":"getMaxFeeLimit","value":15000000000u64}]})
        }
        "/wallet/triggerconstantcontract" => {
            json!({"result":{"result":true},"constant_result":[format!("{:064x}",100_000_000u64)],"energy_used":energy,"transaction":{"ret":[{"ret":"SUCESS"}]}})
        }
        "/wallet/broadcasthex" => json!({"result":true}),
        _ => panic!("unexpected mock RPC {path}"),
    }
}
struct Node {
    node: MockTron,
    price: Arc<AtomicU64>,
    energy: Arc<AtomicU64>,
    head: Arc<AtomicU64>,
}
impl Node {
    fn new() -> Self {
        let price = Arc::new(AtomicU64::new(2000));
        let energy = Arc::new(AtomicU64::new(100000));
        let head = Arc::new(AtomicU64::new(send::now_ms() - 2000));
        let (p, e, h) = (price.clone(), energy.clone(), head.clone());
        let node = MockTron::start(move |path, b| {
            Reply::Json(fixture(
                path,
                b,
                p.load(Ordering::SeqCst),
                e.load(Ordering::SeqCst),
                h.load(Ordering::SeqCst),
            ))
        });
        Self {
            node,
            price,
            energy,
            head,
        }
    }
    fn net(&self) -> Net {
        Net {
            chain: chains::find("tron").unwrap(),
            rpc_url: self.node.url.clone(),
        }
    }
}
fn signed(raw: &Value, mode: u64) -> Value {
    let seed = Seed::from_phrase(Zeroizing::new(
        "test test test test test test test test test test test junk".into(),
    ))
    .unwrap();
    let bytes = hex::decode(raw.as_str().unwrap().trim_start_matches("0x")).unwrap();
    let mut d = tx::decode_raw(&bytes).unwrap();
    if mode == 1 {
        match &mut d.contract {
            tx::Contract::Transfer { to, .. } => *to = tron::address::parse(OTHER).unwrap(),
            tx::Contract::TriggerSmart { data, .. } => {
                data[16..36].copy_from_slice(&tron::address::parse(OTHER).unwrap()[1..])
            }
        }
    }
    let s = tron::signer_at(&seed, if mode == 2 { 1 } else { 0 }).unwrap();
    let bytes = tx::encode_raw(&d).unwrap();
    let signature = s.sign_hash_sync(&tx::txid(&bytes)).unwrap();
    let mut signature = signature.as_bytes().to_vec();
    signature[64] -= 27;
    if mode == 3 {
        signature.truncate(64);
    }
    json!({"rawData":format!("0x{}",hex::encode(&bytes)),"signature":format!("0x{}",hex::encode(signature)),"txHash":hex::encode(tx::txid(&bytes)),"decoded":{"to":"forged"}})
}
struct Fixture {
    signer: FakeDaemon,
    dir: tempfile::TempDir,
    checked: CheckedList,
}
impl Fixture {
    fn new(mode: u64) -> Self {
        let dir = tempfile::Builder::new()
            .prefix("zwt-tr-")
            .tempdir_in("/tmp")
            .unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut list = Whitelist::empty(&chains::WHITELIST_RULES);
        list.generation = 1;
        list.entries
            .push(Entry::new_address("alice", &body(TO), &["tron"]));
        let checked = write_list(dir.path(), list);
        let signer = FakeDaemon::bind_with(&dir.path().join("d.sock"), move |b| {
            let q: Value = serde_json::from_slice(b).unwrap();
            assert_eq!(q["op"], "sign_tx");
            assert_eq!(q["chain"], "tron");
            assert_eq!(q["index"], 0);
            let mut r =
                serde_json::to_vec(&json!({"v":2,"ok":true,"result":signed(&q["tx"],mode)}))
                    .unwrap();
            r.push(b'\n');
            r
        });
        Self {
            signer,
            dir,
            checked,
        }
    }
    fn input(&self, asset: TronAsset) -> TronTransferInput {
        TronTransferInput {
            task_id: None,
            key_name: "main".into(),
            index: 0,
            from: FROM.into(),
            payee: tron_transfer_payees(&self.checked).remove(0),
            asset,
            amount: "1".into(),
            allow_duplicate: false,
        }
    }
    async fn review(&self, node: &Node, asset: TronAsset) -> TronTransferReview {
        review_tron_transfer(
            self.dir.path(),
            &node.net(),
            &self.checked,
            self.input(asset),
        )
        .await
        .unwrap()
    }
    fn count(&self) -> usize {
        self.signer.received.lock().unwrap().len()
    }
}
fn write_list(home: &Path, list: Whitelist) -> CheckedList {
    let key = WhitelistKey::from_secret(&[7; 32]);
    let signed = key.sign(&list);
    std::fs::write(home.join(WHITELIST_FILE), signed.to_file_bytes().unwrap()).unwrap();
    wallet::check_list(home, &Ok(Some(key.anchor_for(&signed)))).unwrap()
}
#[tokio::test]
async fn fee_rise_never_signs_and_returns_a_new_review() {
    for asset in [TronAsset::Trx, TronAsset::Usdt] {
        let f = Fixture::new(0);
        let n = Node::new();
        let old = f.review(&n, asset).await;
        let max = old.max_fee_sun();
        n.price.store(3000, Ordering::SeqCst);
        n.head.fetch_add(1000, Ordering::SeqCst);
        let out = send_tron_transfer(old, "alice", None).await.unwrap();
        assert!(
            matches!(&out,SendTronTransferOutcome::ReviewChanged(r) if r.max_fee_sun()>max),
            "fee rise signed: {out:?}"
        );
        assert_eq!(f.count(), 0);
        assert!(n.node.calls("/wallet/broadcasthex").is_empty());
    }
}
#[tokio::test]
async fn fee_drop_and_fresh_reference_sign_exact_payment_once_and_journal_before_progress() {
    for asset in [TronAsset::Trx, TronAsset::Usdt] {
        let f = Fixture::new(0);
        let n = Node::new();
        let old = f.review(&n, asset).await;
        let held = old.intent().contract.clone();
        let expiry = old.intent().expiration;
        assert_eq!(f.count(), 0);
        assert_eq!(
            old.max_cost_sun(),
            old.max_fee_sun()
                + if asset == TronAsset::Trx {
                    U256::from(1_000_000)
                } else {
                    U256::ZERO
                }
        );
        n.price.store(1000, Ordering::SeqCst);
        n.energy.store(90000, Ordering::SeqCst);
        n.head.fetch_add(1000, Ordering::SeqCst);
        let journal = chains::SendJournal::new(f.dir.path(), n.net().chain, FROM);
        let path = journal.path().to_path_buf();
        let observed = Arc::new(Mutex::new(Vec::new()));
        let o = observed.clone();
        let progress: chains::SendProgress = Arc::new(move |line| {
            let entry: Value =
                serde_json::from_str(std::fs::read_to_string(&path).unwrap().trim()).unwrap();
            assert!(entry.get("nonce").is_none());
            assert!(entry.get("expiresAt").is_some());
            o.lock().unwrap().push(line.to_owned());
        });
        let SendTronTransferOutcome::Sent(out) = send_tron_transfer(old, "alice", Some(progress))
            .await
            .unwrap()
        else {
            panic!("fee drop should send")
        };
        assert_eq!(f.count(), 1);
        assert_eq!(n.node.calls("/wallet/broadcasthex").len(), 1);
        assert_eq!(observed.lock().unwrap().len(), 1);
        assert_eq!(out.fields["txHash"].as_str().unwrap().len(), 64);
        assert!(out.fields.get("nonce").is_none());
        assert!(out.fields.get("rawTx").is_none());
        assert_ne!(out.fields["signerDecoded"]["to"], "forged");
        let q: Value = serde_json::from_slice(&f.signer.received.lock().unwrap()[0]).unwrap();
        let raw = hex::decode(q["tx"].as_str().unwrap().trim_start_matches("0x")).unwrap();
        let d = tx::decode_raw(&raw).unwrap();
        assert_eq!(d.contract, held);
        assert!(d.expiration > expiry);
        assert_eq!(out.fields["expiresAt"], send::expires_at(d.expiration));
    }
}
#[tokio::test]
async fn immutable_picker_excludes_wildcards_contracts_and_mixed_scope() {
    let f = Fixture::new(0);
    let mut checked = f.checked.clone();
    checked.list.entries.extend([
        Entry::new_address("wild", &body(OTHER), &["*"]),
        Entry::new_address("mixed", &body(OTHER), &["tron", "ethereum"]),
        Entry::new_contract("contract", &body(OTHER), &["tron"], &[]),
    ]);
    assert_eq!(tron_transfer_payees(&checked).len(), 1);
    let n = Node::new();
    let input = f.input(TronAsset::Trx);
    let mut other = f.checked.clone();
    other.list.entries[0].address = body(OTHER);
    assert!(
        review_tron_transfer(f.dir.path(), &n.net(), &other, input)
            .await
            .is_err()
    );
    assert!(n.node.seen().is_empty());
    assert_eq!(f.count(), 0);
}
#[tokio::test]
async fn fingerprint_before_and_after_rpc_and_at_confirm_blocks_signing() {
    for phase in 0..4 {
        let f = Fixture::new(0);
        let n = Node::new();
        let old = if phase >= 2 {
            Some(f.review(&n, TronAsset::Trx).await)
        } else {
            None
        };
        let changed = f.dir.path().join(WHITELIST_FILE);
        let mutate = || std::fs::write(&changed, b"tampered").unwrap();
        if phase == 0 || phase == 2 {
            mutate();
        }
        let home = f.dir.path().to_path_buf();
        let rpc = MockTron::start(move |p, b| {
            if p == "/wallet/getblock" {
                std::fs::write(home.join(WHITELIST_FILE), b"changed during RPC").unwrap();
            }
            Reply::Json(fixture(p, b, 2000, 100000, send::now_ms() - 1000))
        });
        let testnet = Net {
            chain: n.net().chain,
            rpc_url: rpc.url.clone(),
        };
        let result = if phase < 2 {
            review_tron_transfer(f.dir.path(), &testnet, &f.checked, f.input(TronAsset::Trx))
                .await
                .map(|_| ())
        } else if phase == 2 {
            send_tron_transfer(old.unwrap(), "alice", None)
                .await
                .map(|_| ())
        } else {
            // Prepare normally, then arm a mutation in the held endpoint's confirm RPC.
            let home = f.dir.path().to_path_buf();
            let armed = Arc::new(AtomicU64::new(0));
            let a = armed.clone();
            let rpc = MockTron::start(move |p, b| {
                if p == "/wallet/getblock" && a.load(Ordering::SeqCst) == 1 {
                    std::fs::write(home.join(WHITELIST_FILE), b"after confirm RPC").unwrap();
                }
                Reply::Json(fixture(p, b, 2000, 100000, send::now_ms() - 1000))
            });
            let net = Net {
                chain: n.net().chain,
                rpc_url: rpc.url.clone(),
            };
            let old = review_tron_transfer(f.dir.path(), &net, &f.checked, f.input(TronAsset::Trx))
                .await
                .unwrap();
            armed.store(1, Ordering::SeqCst);
            send_tron_transfer(old, "alice", None).await.map(|_| ())
        };
        assert!(result.is_err(), "phase {phase}");
        assert_eq!(f.count(), 0);
    }
}
#[tokio::test]
async fn wrong_confirmation_policy_denial_lock_and_duplicate_all_refuse_before_signing() {
    for case in 0..4 {
        let mut f = Fixture::new(0);
        let n = Node::new();
        if case == 1 {
            f.checked.list.policy.set_max_fee_wei("tron", U256::from(1));
            f.checked = write_list(f.dir.path(), f.checked.list.clone());
        }
        let old = f.review(&n, TronAsset::Usdt).await;
        let lock = if case == 2 {
            Some(chains::SendLock::acquire(f.dir.path(), n.net().chain, FROM).unwrap())
        } else {
            None
        };
        if case == 3 {
            let journal = chains::SendJournal::new(f.dir.path(), n.net().chain, FROM);
            journal.append(&json!({"ts":zwallet::core::time::now_rfc3339(),"chain":"tron","from":FROM,"txHash":"ab".repeat(32),"to":TO,"value":"1 USDT","amountRaw":"1000000","kind":"token-transfer","token":tron::registry::TOKENS[0].address,"expiresAt":send::expires_at(old.intent().expiration)})).unwrap();
        }
        assert!(
            send_tron_transfer(old, if case == 0 { "Alice" } else { "alice" }, None)
                .await
                .is_err(),
            "case {case}"
        );
        assert_eq!(f.count(), 0);
        assert!(n.node.calls("/wallet/broadcasthex").is_empty());
        drop(lock);
    }
}
#[tokio::test]
async fn signer_substitution_wrong_key_and_malformed_signature_never_broadcast() {
    for mode in 1..=3 {
        for asset in [TronAsset::Trx, TronAsset::Usdt] {
            let f = Fixture::new(mode);
            let n = Node::new();
            let old = f.review(&n, asset).await;
            let e = send_tron_transfer(old, "alice", None).await.unwrap_err();
            assert_eq!(e.code, ErrorCode::ValidationError);
            assert_eq!(f.count(), 1);
            assert!(n.node.calls("/wallet/broadcasthex").is_empty());
        }
    }
}

#[tokio::test]
async fn duplicate_override_is_held_by_the_review_and_new_fund_shortage_never_signs() {
    for asset in [TronAsset::Trx, TronAsset::Usdt] {
        let f = Fixture::new(0);
        let n = Node::new();
        let first = f.review(&n, asset).await;
        send_tron_transfer(first, "alice", None).await.unwrap();
        let mut input = f.input(asset);
        input.allow_duplicate = true;
        let repeat = review_tron_transfer(f.dir.path(), &n.net(), &f.checked, input)
            .await
            .unwrap();
        assert_eq!(
            repeat.fields()["duplicateSends"].as_array().unwrap().len(),
            1
        );
        assert!(repeat.allow_duplicate());
        send_tron_transfer(repeat, "alice", None).await.unwrap();
        assert_eq!(f.count(), 2);
    }
    let f = Fixture::new(0);
    let funds = Arc::new(AtomicU64::new(1_000_000_000));
    let shared = funds.clone();
    let node = MockTron::start(move |p, b| {
        Reply::Json(if p == "/wallet/getaccount" && b["address"] == FROM {
            common::tron::account(FROM, shared.load(Ordering::SeqCst))
        } else {
            fixture(p, b, 2000, 100000, send::now_ms() - 1000)
        })
    });
    let net = Net {
        chain: chains::find("tron").unwrap(),
        rpc_url: node.url.clone(),
    };
    let old = review_tron_transfer(f.dir.path(), &net, &f.checked, f.input(TronAsset::Trx))
        .await
        .unwrap();
    funds.store(1, Ordering::SeqCst);
    assert_eq!(
        send_tron_transfer(old, "alice", None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::InsufficientFunds
    );
    assert_eq!(f.count(), 0);
    assert!(node.calls("/wallet/broadcasthex").is_empty());
}

#[tokio::test]
async fn foreign_chain_bad_sender_and_invalid_amount_are_rejected_before_rpc() {
    for case in 0..4 {
        let f = Fixture::new(0);
        let n = Node::new();
        let mut net = n.net();
        let mut input = f.input(TronAsset::Trx);
        match case {
            0 => net.chain = chains::find("ethereum").unwrap(),
            1 => input.from = FROM.to_lowercase(),
            2 => input.amount = "0.0000001".into(),
            _ => input.amount = "0".into(),
        }
        assert!(
            review_tron_transfer(f.dir.path(), &net, &f.checked, input)
                .await
                .is_err()
        );
        assert_eq!(f.count(), 0);
        assert!(n.node.seen().is_empty());
    }
}
