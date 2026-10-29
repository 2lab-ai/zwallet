//! T38–T41 process acceptance. Parent runs serially: real isolated daemon/PTy, loopback only.
mod common;
use alloy::hex;
use common::{
    Frame, Home, PW, PtyChild,
    tron::{MAINNET_GENESIS, MockTron, Reply},
};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use zwallet::chains::{
    self, SendJournal,
    tron::{self, send, tx},
};
const TO: &str = "TPjjvMwjPoDC32V2dGDYTkLH4E5LAtBZ6C";
const WAIT: Duration = Duration::from_secs(60);
fn block() -> Value {
    let n = 9_000_000u64;
    let mut id = [0x77u8; 32];
    id[..8].copy_from_slice(&n.to_be_bytes());
    json!({"blockID":hex::encode(id),"block_header":{"raw_data":{"number":n,"timestamp":send::now_ms()-1000}}})
}
fn raw(s: &str) -> Vec<u8> {
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
    assert_eq!(&bytes[at + n..at + n + 2], &[18, 65]);
    assert_eq!(bytes.len(), at + n + 67);
    let raw = bytes[at..at + n].to_vec();
    assert_eq!(
        tx::recover(&raw, &bytes[at + n + 2..]).unwrap(),
        tx::decode_raw(&raw).unwrap().owner
    );
    raw
}
struct Node {
    node: MockTron,
    saved: Arc<Mutex<Option<Value>>>,
    journal_first: Arc<AtomicBool>,
}
impl Node {
    fn new(home: &Home, owner: &str) -> Self {
        let saved = Arc::new(Mutex::new(None::<Value>));
        let state = saved.clone();
        let journal_first = Arc::new(AtomicBool::new(false));
        let first = journal_first.clone();
        let path = SendJournal::new(home.path(), chains::find("tron").unwrap(), owner)
            .path()
            .to_owned();
        let owner = owner.to_owned();
        let node = MockTron::start(move |p, b| {
            Reply::Json(match p {
            "/wallet/getblockbynum"=>common::tron::block0(MAINNET_GENESIS),
            "/wallet/getblock"=>block(),
            "/wallet/getaccount"=>common::tron::account(b["address"].as_str().unwrap(),1_000_000_000),
            "/wallet/getaccountresource"=>json!({}),
            "/wallet/getchainparameters"=>json!({"chainParameter":[{"key":"getTransactionFee","value":2001},{"key":"getEnergyFee","value":100},{"key":"getCreateNewAccountFeeInSystemContract","value":2000000},{"key":"getCreateAccountFee","value":300000},{"key":"getCreateNewAccountBandwidthRate","value":3},{"key":"getMaxFeeLimit","value":15000000000u64}]}),
            "/wallet/triggerconstantcontract"=>json!({"result":{"result":true},"constant_result":[format!("{:064x}",100_000_000u64)],"energy_used":100003,"transaction":{"ret":[{"ret":"SUCESS"}]}}),
            "/wallet/broadcasthex"=>{
                let raw=raw(b["transaction"].as_str().unwrap());
                assert_eq!(tron::address::encode(&tx::decode_raw(&raw).unwrap().owner),owner);
                let hash=hex::encode(tx::txid(&raw));
                let recorded=std::fs::read_to_string(&path).unwrap_or_default().lines().any(|l|
                    serde_json::from_str::<Value>(l).is_ok_and(|e| e["txHash"]==hash && e.get("nonce").is_none() && e["expiresAt"].is_string()));
                first.store(recorded,Ordering::SeqCst);
                *state.lock().unwrap()=Some(json!({"txID":hash,"raw_data_hex":hex::encode(raw),"ret":[{"contractRet":"SUCCESS"}]}));
                json!({"result":true})
            },
            "/wallet/gettransactionbyid"=>state.lock().unwrap().clone().unwrap_or(json!({})),
            "/wallet/gettransactioninfobyid"=>state.lock().unwrap().as_ref().map_or(json!({}),|t|
                json!({"id":t["txID"],"blockNumber":123,"fee":1000,"receipt":{"result":"SUCCESS"}})),
            "/wallet/gettransactionfrompending"=>json!({}),
            _=>panic!("unexpected mock RPC {p}"),
        })
        });
        Self {
            node,
            saved,
            journal_first,
        }
    }
}
fn has(frame: &Frame, text: &str) -> bool {
    frame.rows.iter().any(|s| s.contains(text))
}
fn wait(tui: &mut PtyChild, text: &str) -> Frame {
    tui.wait_frame(WAIT, |f| has(f, text))
}
#[test]
fn t38_t41_real_tron_receive_balance_send_and_hash_lookup_for_both_assets() {
    let _exclusive = common::exclusive();
    for usdt in [false, true] {
        let home = Home::new();
        home.create("main", &[]);
        let owner = home.run(&["address", "--chain", "tron", "--json"]).ok()["address"]
            .as_str()
            .unwrap()
            .to_owned();
        home.allow(
            "alice",
            &format!("0x{}", hex::encode(&tron::address::parse(TO).unwrap()[1..])),
            &["tron"],
        );
        let node = Node::new(&home, &owner);
        let mut cmd = home.command(&["tui", "--chain", "tron", "--allow-ptrace-scope-0"]);
        for chain in chains::CHAINS {
            cmd.env(chain.rpc_env_var(), "http://127.0.0.1:1");
        }
        cmd.env("ZWALLET_RPC_TRON", &node.node.url);
        let mut tui = PtyChild::spawn_full_screen(cmd, (80, 24));
        tui.wait_frame(WAIT, |f| has(f, "* main ") && has(f, "locked |"));
        tui.send(b"V");
        wait(&mut tui, "Wallet password:");
        tui.send(format!("{PW}\r").as_bytes());
        tui.wait_frame(WAIT, |f| has(f, &owner) && has(f, "verified"));
        tui.send(b"\r");
        wait(&mut tui, "Wallet password:");
        tui.send(format!("{PW}\r").as_bytes());
        tui.wait_frame(WAIT, |f| {
            has(f, "verified by password") && has(f, "signing (agent too) until ")
        });
        tui.send(b"4");
        let receive = wait(&mut tui, &owner);
        assert!(has(&receive, "verified by password"));
        tui.send(b"2");
        tui.wait_frame(WAIT, |f| has(f, "TRX") && has(f, "USDT"));
        tui.send(b"3");
        wait(&mut tui, "Select an asset");
        if usdt {
            tui.send(b"\x1b[B");
        }
        tui.send(b"\r");
        tui.wait_frame(WAIT, |f| {
            has(f, "Choose a password-checked Tron-only payee") && has(f, TO)
        });
        tui.send(b"\r");
        wait(&mut tui, "Amount:");
        tui.send(b"1\r");
        let review = tui.wait_frame(WAIT, |f| {
            has(f, "Whitelist allowed") && has(f, "Maximum cost TRX:")
        });
        assert!(has(&review, &owner) && has(&review, TO) && !has(&review, "<invalid>"));
        assert!(!has(&review, "Nonce:"));
        assert!(node.saved.lock().unwrap().is_none());
        tui.send(b"\r");
        wait(&mut tui, "Type payee nickname:");
        tui.send(b"alice\r");
        let mined = wait(&mut tui, "mined: success");
        assert!(has(&mined, "Expires:") && !has(&mined, "Nonce:"));
        assert!(node.journal_first.load(Ordering::SeqCst));
        let saved = node.saved.lock().unwrap().clone().unwrap();
        let hash = saved["txID"].as_str().unwrap();
        let decoded =
            tx::decode_raw(&hex::decode(saved["raw_data_hex"].as_str().unwrap()).unwrap()).unwrap();
        assert_eq!(decoded.transfer_call().is_some(), usdt);
        assert!(has(&mined, hash));
        assert_eq!(node.node.calls("/wallet/broadcasthex").len(), 1);
        tui.send(b"2");
        wait(&mut tui, "this machine's sends only");
        tui.send(format!("/{hash}\r").as_bytes());
        let detail = wait(&mut tui, "mined (success)");
        assert!(has(&detail, "1000 sun") && !has(&detail, "Gas used:") && !has(&detail, "Nonce:"));
        tui.send(b"q");
        let (status, stderr) = tui.wait_exit(WAIT);
        assert!(status.success(), "{status}: {stderr}");
        assert!(stderr.is_empty());
        let frame = tui.final_frame(WAIT);
        assert!(
            frame.stray.is_empty() && frame.pending.is_empty(),
            "{frame}"
        );
        assert_ne!(tui.local_modes() & libc::ICANON, 0);
        assert_ne!(tui.local_modes() & libc::ECHO, 0);
        let output = tui.output();
        let output = String::from_utf8_lossy(&output);
        assert!(!output.contains(PW));
        assert!(!output.contains("broadcasting "));
    }
}
