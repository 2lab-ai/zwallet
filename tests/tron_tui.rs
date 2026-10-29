//! T40 App/TestBackend + loopback signer tests; no real funds or PTY.
mod common;
use alloy::{hex, signers::SignerSync};
use common::{
    FakeDaemon,
    tron::{MAINNET_GENESIS, MockTron, Reply},
};
use serde_json::{Value, json};
use std::{
    os::unix::fs::PermissionsExt,
    path::Path,
    sync::{
        Arc,
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
        seed::Seed,
        whitelist::{Entry, WHITELIST_FILE, Whitelist, WhitelistKey},
    },
    wallet::{self, CheckedList},
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
        "/wallet/gettransactionbyid"
        | "/wallet/gettransactioninfobyid"
        | "/wallet/gettransactionfrompending" => json!({}),
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
        Self::with_balance(format!("{:064x}", 100_000_000u64))
    }
    fn with_balance(balance_hex: String) -> Self {
        let price = Arc::new(AtomicU64::new(2000));
        let energy = Arc::new(AtomicU64::new(100000));
        let head = Arc::new(AtomicU64::new(send::now_ms() - 2000));
        let (p, e, h) = (price.clone(), energy.clone(), head.clone());
        let node = MockTron::start(move |path, b| {
            let mut value = fixture(
                path,
                b,
                p.load(Ordering::SeqCst),
                e.load(Ordering::SeqCst),
                h.load(Ordering::SeqCst),
            );
            if path == "/wallet/triggerconstantcontract" {
                value["constant_result"] = json!([balance_hex]);
            }
            Reply::Json(value)
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

use std::time::Duration;
use zwallet::tui::{
    app::{
        App, Effect, Key, Msg,
        transfer::{ReceiptState, Stage, TransferJob, TransferMsg},
    },
    term::FilteredBackend,
    ui,
};
const EVM: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
fn app(f: &Fixture, n: &Node) -> App {
    let mut app = App::new(&n.net(), Duration::from_secs(3600));
    app.configure(f.dir.path(), false);
    let row = wallet::KeyRow {
        name: "main".into(),
        index: 0,
        address: Some(EVM.into()),
        active: true,
    };
    let opened = wallet::Opened {
        keys: vec![row.clone()],
        active: "main".into(),
        warning: None,
        anchor: Ok(None),
        salted: true,
    };
    let addresses = vec![
        wallet::VerifiedAddress {
            key: "main".into(),
            chain: "ethereum",
            family: "evm",
            address: EVM.into(),
        },
        wallet::VerifiedAddress {
            key: "main".into(),
            chain: "tron",
            family: "tron",
            address: FROM.into(),
        },
    ];
    app.update(Msg::Verified(Ok((
        wallet::PasswordCheck {
            opened,
            addresses,
            repairs: vec![],
        },
        Ok(f.checked.clone()),
    ))));
    let generation = app.generation();
    app.update(Msg::Keys(
        generation,
        Ok(wallet::Keys {
            active: "main".into(),
            rows: vec![row],
            trust: wallet::Trust::Locked,
        }),
    ));
    app.update(Msg::Status(
        generation,
        Ok(wallet::StatusView {
            configured: true,
            locked: false,
            daemon_pids: vec![1],
            other_daemon_pids: vec![],
            serving: None,
            active: Some("main".into()),
            address: Some((Some(EVM.into()), wallet::Trust::Session)),
        }),
    ));
    app
}
fn screen(app: &App) -> String {
    use ratatui::{Terminal, backend::TestBackend, buffer::Cell};
    use zwallet::tui::{style, text::Source};
    let mut t = Terminal::new(FilteredBackend::new(TestBackend::new(80, 24))).unwrap();
    let mut placed = Vec::new();
    t.draw(|f| placed = ui::draw(f, app)).unwrap();
    assert_eq!(t.backend().replaced(), 0);
    let b = t.backend().inner().buffer();
    let rows: Vec<String> = (0..24)
        .map(|y| (0..80).map(|x| b[(x, y)].symbol()).collect())
        .collect();
    let styles: Vec<_> = style::ALL
        .iter()
        .map(|s| {
            let mut c = Cell::default();
            c.set_style(*s);
            c.style()
        })
        .collect();
    for c in &b.content {
        assert!(styles.contains(&c.style()));
        assert!(
            c.symbol()
                .chars()
                .all(|c| c.is_ascii_graphic() || c == ' ' || c == '₮')
        );
    }
    for p in placed {
        let max = match p.outside.source() {
            Source::WhitelistName | Source::KeyName | Source::RpcHost => 32,
            Source::Quantity => 78,
            Source::TransactionHash => 66,
            Source::Detail => 72,
            Source::Home | Source::Expiry => 64,
            Source::Address => 42,
            Source::Input | Source::Error | Source::Warning => 512,
        };
        assert!(p.outside.text().len() <= max);
        assert!(p.area.x + p.area.width <= 80 && p.area.y + p.area.height <= 24);
        if p.area.height == 1 {
            assert_eq!(
                &rows[p.area.y as usize][p.area.x as usize..(p.area.x + p.area.width) as usize],
                p.outside.text()
            );
        }
    }
    rows.join("\n")
}
fn key(app: &mut App, k: Key) -> Vec<Effect> {
    let work = app.update(Msg::Key(k));
    screen(app);
    work
}
fn text(app: &mut App, s: &str) {
    for c in s.chars() {
        key(app, Key::Char(c));
    }
}
async fn drive(app: &mut App, f: &Fixture, effects: Vec<Effect>) -> Vec<(u64, TransferJob)> {
    let mut queue = std::collections::VecDeque::from(effects);
    let mut waits = Vec::new();
    while let Some(effect) = queue.pop_front() {
        let Effect::Transfer(id) = effect else {
            continue;
        };
        let Some(job) = app.take_transfer_job(id) else {
            continue;
        };
        if matches!(job, TransferJob::Receipt { .. }) {
            waits.push((id, job));
            continue;
        }
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        job.run(id, f.dir.path(), &tx).await;
        while let Ok(msg) = rx.try_recv() {
            queue.extend(app.update(msg));
            screen(app);
        }
    }
    waits
}
async fn review(app: &mut App, f: &Fixture, usdt: bool) {
    let work = key(app, Key::Char('3'));
    drive(app, f, work).await;
    assert_eq!(app.transfer().stage, Stage::Assets, "{}", screen(app));
    if usdt {
        key(app, Key::Down);
    }
    let work = key(app, Key::Enter);
    drive(app, f, work).await;
    assert_eq!(app.transfer().stage, Stage::Payees);
    assert_eq!(app.transfer().tron.payees.len(), 1);
    key(app, Key::Enter);
    assert_eq!(app.transfer().stage, Stage::Amount);
    text(app, "1");
    let work = key(app, Key::Enter);
    drive(app, f, work).await;
    assert_eq!(app.transfer().stage, Stage::Review, "{}", screen(app));
    assert_eq!(f.count(), 0);
}
fn confirm(app: &mut App) -> Vec<Effect> {
    key(app, Key::Enter);
    assert_eq!(app.transfer().stage, Stage::Confirm);
    assert!(screen(app).contains("Type payee nickname:"));
    text(app, "alice");
    key(app, Key::Enter)
}
#[tokio::test]
async fn t40_trx_and_usdt_review_send_and_status_have_exact_tron_fields() {
    for usdt in [false, true] {
        let f = Fixture::new(0);
        let n = Node::new();
        let mut app = app(&f, &n);
        review(&mut app, &f, usdt).await;
        let view = screen(&app);
        for expected in [
            FROM,
            TO,
            "1000000",
            "Bandwidth:",
            "activation",
            "Energy:",
            "feeLimit:",
            "Maximum cost TRX:",
            "TRX left at least:",
            "At prices of block:",
            "Expires:",
        ] {
            assert!(view.contains(expected), "missing {expected}:\n{view}");
        }
        assert!(!view.contains("Nonce:") && !view.contains("g fee"));
        let original_expiration = app
            .transfer()
            .tron
            .review
            .as_ref()
            .unwrap()
            .intent()
            .expiration;
        n.head.fetch_add(1000, Ordering::SeqCst);
        let work = confirm(&mut app);
        let waits = drive(&mut app, &f, work).await;
        assert_eq!(f.count(), 1);
        assert_eq!(n.node.calls("/wallet/broadcasthex").len(), 1);
        assert_eq!(waits.len(), 1);
        let status = app.transfer().status.as_ref().unwrap();
        assert!(status.nonce.is_none());
        assert!(
            status
                .expiration
                .is_some_and(|value| value > original_expiration)
        );
        assert_eq!(status.hash.len(), 64);
        assert!(app.transfer().retained.is_none());
        let frame = screen(&app);
        assert!(frame.contains(&status.hash));
        assert!(!frame.contains("Nonce:") && !frame.contains("later version"));
        app.update(Msg::Transfer(waits[0].0,TransferMsg::Receipt(Ok(Some(json!({"status":"mined","success":null,"hint":"execution result is unknown; this payment may already have been included"}))))));
        assert_eq!(
            app.transfer().status.as_ref().unwrap().state,
            ReceiptState::MinedUnknown
        );
        assert!(screen(&app).contains("mined: execution result unknown"));
    }
}
#[tokio::test]
async fn t40_fee_rise_reconfirms_and_evm_overrides_never_start_work() {
    let f = Fixture::new(0);
    let n = Node::new();
    let mut app = app(&f, &n);
    review(&mut app, &f, true).await;
    for c in ['p', 'g'] {
        assert!(key(&mut app, Key::Char(c)).is_empty());
        assert_eq!(app.transfer().stage, Stage::Review);
    }
    n.price.store(3000, Ordering::SeqCst);
    n.energy.store(100001, Ordering::SeqCst);
    let work = confirm(&mut app);
    drive(&mut app, &f, work).await;
    assert_eq!(f.count(), 0);
    assert_eq!(app.transfer().stage, Stage::Review);
    assert!(screen(&app).contains("nothing signed"));
    let work = confirm(&mut app);
    drive(&mut app, &f, work).await;
    assert_eq!(f.count(), 1);
}
#[tokio::test]
async fn t40_fingerprint_key_index_and_lock_refuse_before_signing() {
    for case in 0..3 {
        let f = Fixture::new(0);
        let n = Node::new();
        let mut app = app(&f, &n);
        review(&mut app, &f, false).await;
        if case == 0 {
            std::fs::write(f.dir.path().join(WHITELIST_FILE), b"replaced").unwrap();
        }
        if case == 1 {
            app.update(Msg::Keys(
                app.generation(),
                Ok(wallet::Keys {
                    active: "main".into(),
                    rows: vec![wallet::KeyRow {
                        name: "main".into(),
                        index: 9,
                        address: Some(EVM.into()),
                        active: true,
                    }],
                    trust: wallet::Trust::Locked,
                }),
            ));
        }
        if case == 2 {
            key(&mut app, Key::Char('L'));
        }
        let work = key(&mut app, Key::Enter);
        drive(&mut app, &f, work).await;
        if app.transfer().stage == Stage::Confirm {
            text(&mut app, "alice");
            let work = key(&mut app, Key::Enter);
            drive(&mut app, &f, work).await;
        }
        assert_eq!(f.count(), 0);
        assert!(n.node.calls("/wallet/broadcasthex").is_empty());
        if case == 0 {
            assert!(app.checked_list().is_none());
        }
    }
}
#[tokio::test]
async fn t40_expired_unseen_keeps_warning_and_offers_no_resend() {
    let f = Fixture::new(0);
    let n = Node::new();
    let mut app = app(&f, &n);
    review(&mut app, &f, false).await;
    let work = confirm(&mut app);
    let waits = drive(&mut app, &f, work).await;
    let hint = format!(
        "this node returned no record of it as of block 9000000 on mock; another node may have it. {}",
        tron::lookup::SECOND_PAYMENT
    );
    app.update(Msg::Transfer(
        waits[0].0,
        TransferMsg::Receipt(Ok(Some(
            json!({"status":"expired-unseen","found":false,"hint":hint}),
        ))),
    ));
    assert_eq!(
        app.transfer().status.as_ref().unwrap().state,
        ReceiptState::ExpiredUnseen
    );
    let frame = screen(&app);
    assert!(frame.contains("expired-unseen"));
    assert!(frame.contains("second payment"), "{frame}");
    for c in ['n', 's', 'x', 'R'] {
        assert!(key(&mut app, Key::Char(c)).is_empty());
        assert_eq!(app.transfer().stage, Stage::Status);
    }
    assert_eq!(f.count(), 1);
}

#[test]
fn t38_tron_before_open_never_displays_primary_evm_and_receive_keeps_the_pin() {
    let f = Fixture::new(0);
    let n = Node::new();
    let mut preview = App::new(&n.net(), Duration::from_secs(3600));
    preview.configure(f.dir.path(), false);
    let generation = preview.generation();
    preview.update(Msg::Keys(
        generation,
        Ok(wallet::Keys {
            active: "main".into(),
            rows: vec![wallet::KeyRow {
                name: "main".into(),
                index: 0,
                address: Some(EVM.into()),
                active: true,
            }],
            trust: wallet::Trust::Locked,
        }),
    ));
    preview.update(Msg::Status(
        generation,
        Ok(wallet::StatusView {
            configured: true,
            locked: true,
            daemon_pids: vec![],
            other_daemon_pids: vec![],
            serving: None,
            active: Some("main".into()),
            address: Some((Some(EVM.into()), wallet::Trust::Locked)),
        }),
    ));
    assert!(!screen(&preview).contains("0xf39"));
    preview.update(Msg::Address(
        generation,
        "main".into(),
        Ok((FROM.into(), wallet::Trust::Locked)),
    ));
    assert!(screen(&preview).contains("TWer"));
    assert!(!screen(&preview).contains("0xf39"));
    let mut app = app(&f, &n);
    key(&mut app, Key::Char('4'));
    app.update(Msg::Addresses(
        app.generation(),
        "main".into(),
        Ok(wallet::Addresses {
            key: "main".into(),
            trust: wallet::Trust::Locked,
            rows: vec![wallet::AddressRow {
                chain: chains::find("tron").unwrap(),
                address: FROM.into(),
                error: None,
            }],
        }),
    ));
    let view = screen(&app);
    assert!(view.contains(FROM));
    assert!(view.contains("verified by password"));
    app.update(Msg::Address(
        app.generation(),
        "main".into(),
        Ok((TO.into(), wallet::Trust::Session)),
    ));
    assert!(screen(&app).contains("TWer"));
    assert!(!screen(&app).contains("verified by session"));
}

#[tokio::test]
async fn t39_tron_balance_keeps_activation_and_complete_owner_warning() {
    for active in [false, true] {
        let f = Fixture::new(0);
        let n = Node::new();
        let mut app = app(&f, &n);
        key(&mut app, Key::Char('2'));
        let mock = MockTron::start(move |p, b| {
            Reply::Json(if p == "/wallet/getaccount" {
                if active {
                    let mut value = common::tron::account(FROM, 1000000);
                    value["owner_permission"] =
                        json!({"threshold":2,"keys":[{"address":FROM,"weight":1}]});
                    value
                } else {
                    json!({})
                }
            } else {
                fixture(p, b, 2000, 100000, send::now_ms() - 1000)
            })
        });
        let network = Net {
            chain: n.net().chain,
            rpc_url: mock.url.clone(),
        };
        let read = wallet::read::read_network_details(network, FROM.into())
            .await
            .unwrap();
        assert_eq!(read.activated, Some(active));
        app.update(Msg::Balances(
            app.generation(),
            "main".into(),
            Ok(wallet::ChainBalances {
                chain: n.net().chain,
                key: "main".into(),
                address: FROM.into(),
                trust: wallet::Trust::Session,
                native: read.native,
                tokens: read.tokens,
                activated: read.activated,
                warning: read.warning,
            }),
        ));
        let mut view = screen(&app);
        assert!(view.contains("USDT"));
        if active {
            assert!(view.contains("Account warning:"));
            key(&mut app, Key::Tab);
            for _ in 0..5 {
                key(&mut app, Key::Down);
                view.push_str(&screen(&app));
            }
            assert!(view.contains("do not fund"), "{view}");
        } else {
            assert!(view.contains("not activated"));
        }
        assert_eq!(f.count(), 0);
    }
}

#[tokio::test]
async fn t40_fractional_trx_bounds_are_visible_exactly() {
    let f = Fixture::new(0);
    let n = Node::new();
    n.price.store(2001, Ordering::SeqCst);
    n.energy.store(100003, Ordering::SeqCst);
    let mut app = app(&f, &n);
    review(&mut app, &f, true).await;
    let held = app.transfer().tron.review.as_ref().unwrap();
    let frame = screen(&app);
    for (label, value) in [
        ("Estimated cost TRX: ", held.estimated_cost_sun()),
        ("Maximum cost TRX: ", held.max_cost_sun()),
        ("Maximum fee TRX: ", held.max_fee_sun()),
        ("TRX left at least: ", held.trx_left_sun()),
    ] {
        let exact = zwallet::core::amount::format_units(value, 6);
        assert!(exact.contains('.'), "fixture must exercise fractional TRX");
        assert!(frame.contains(&format!("{label}{exact}")), "{frame}");
    }
    assert!(!frame.contains("<invalid>"));
    assert_eq!(f.count(), 0);
}

async fn run_messages(app: &mut App, f: &Fixture, id: u64, job: TransferJob) -> Vec<Effect> {
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    job.run(id, f.dir.path(), &sender).await;
    let mut effects = Vec::new();
    while let Ok(message) = receiver.try_recv() {
        effects.extend(app.update(message));
        screen(app);
    }
    effects
}
#[tokio::test]
async fn t40_queued_quit_and_lock_drain_changed_success_and_bad_signature() {
    for case in 0..3 {
        let f = Fixture::new(if case == 2 { 3 } else { 0 });
        let n = Node::new();
        let mut app = app(&f, &n);
        review(&mut app, &f, true).await;
        let effects = confirm(&mut app);
        let Effect::Transfer(id) = effects[0] else {
            panic!("send job")
        };
        let job = app.take_transfer_job(id).unwrap();
        if case == 0 {
            n.price.store(3000, Ordering::SeqCst);
        }
        assert!(key(&mut app, Key::Char('L')).is_empty());
        assert!(key(&mut app, Key::Char('q')).is_empty());
        assert!(app.quit_pending() && !app.quitting());
        let effects = run_messages(&mut app, &f, id, job).await;
        assert!(effects.contains(&Effect::LockAll));
        assert!(app.quit_pending() && !app.quitting());
        let failed = app.take_quit_error();
        if case == 0 {
            assert_eq!(app.transfer().stage, Stage::Review);
            assert_eq!(f.count(), 0);
        } else if case == 1 {
            assert!(app.transfer().status.is_some());
            assert_eq!(f.count(), 1);
        } else {
            assert!(failed.is_some());
            assert_eq!(app.transfer().stage, Stage::Failed);
            assert_eq!(f.count(), 1);
        }
        assert_eq!(
            n.node.calls("/wallet/broadcasthex").len(),
            usize::from(case == 1)
        );
        app.update(Msg::Locked(Ok(vec![1])));
        assert!(app.quitting());
    }
}
#[tokio::test]
async fn t40_actual_receipt_worker_uses_checked_raw_and_three_execution_states() {
    for (execution, expected) in [
        (Some("SUCCESS"), ReceiptState::Success),
        (Some("REVERT"), ReceiptState::Reverted),
        (None, ReceiptState::MinedUnknown),
    ] {
        let f = Fixture::new(0);
        let n = Node::new();
        let mut app = app(&f, &n);
        review(&mut app, &f, true).await;
        let work = confirm(&mut app);
        let mut waits = drive(&mut app, &f, work).await;
        let (id, job) = waits.pop().unwrap();
        let request: Value = serde_json::from_slice(&f.signer.received.lock().unwrap()[0]).unwrap();
        let raw = request["tx"]
            .as_str()
            .unwrap()
            .trim_start_matches("0x")
            .to_string();
        let hash = hex::encode(tx::txid(&hex::decode(&raw).unwrap()));
        let mock = MockTron::start(move |p, b| {
            Reply::Json(match p {
                "/wallet/gettransactionbyid" => json!({"txID":hash,"raw_data_hex":raw}),
                "/wallet/gettransactioninfobyid" => {
                    json!({"id":hash,"blockNumber":9,"fee":42,"receipt":{"result":execution}})
                }
                _ => fixture(p, b, 2000, 100000, send::now_ms()),
            })
        });
        let TransferJob::Receipt {
            hash,
            expiration,
            timeout,
            cancel,
            ..
        } = job
        else {
            panic!("receipt")
        };
        let job = TransferJob::Receipt {
            net: Net {
                chain: n.net().chain,
                rpc_url: mock.url.clone(),
            },
            hash,
            expiration,
            timeout,
            cancel,
        };
        run_messages(&mut app, &f, id, job).await;
        assert_eq!(app.transfer().status.as_ref().unwrap().state, expected);
        assert_eq!(mock.calls("/wallet/gettransactionbyid").len(), 1);
        if execution.is_none() {
            assert!(screen(&app).contains("second payment"));
        }
        assert_eq!(f.count(), 1);
    }
}

#[tokio::test]
async fn t40_first_observation_deadline_stays_unknown_with_second_payment_warning() {
    let f = Fixture::new(0);
    let n = Node::new();
    let mut app = app(&f, &n);
    review(&mut app, &f, false).await;
    let work = confirm(&mut app);
    let (id, job) = drive(&mut app, &f, work).await.pop().unwrap();
    let TransferJob::Receipt {
        net,
        hash,
        expiration,
        cancel,
        ..
    } = job
    else {
        panic!("receipt")
    };
    run_messages(
        &mut app,
        &f,
        id,
        TransferJob::Receipt {
            net,
            hash,
            expiration,
            cancel,
            timeout: Duration::ZERO,
        },
    )
    .await;
    assert_eq!(
        app.transfer().status.as_ref().unwrap().state,
        ReceiptState::Unknown
    );
    assert!(screen(&app).contains("second payment"));
    // Tron has no nonce replacement: the EVM speed-up, cancel and retry keys sign nothing.
    for k in ['s', 'x', 'R'] {
        assert!(key(&mut app, Key::Char(k)).is_empty());
        assert!(screen(&app).contains("second payment"));
    }
    assert_eq!(f.count(), 1);
}

#[tokio::test]
async fn t40_repeat_override_warns_in_review_and_typed_confirmation() {
    let f = Fixture::new(0);
    let n = Node::new();
    let mut app = app(&f, &n);
    review(&mut app, &f, false).await;
    let effects = key(&mut app, Key::Char('d'));
    drive(&mut app, &f, effects).await;
    assert!(screen(&app).contains("a new send may pay twice"));
    key(&mut app, Key::Enter);
    assert!(screen(&app).contains("a new send may pay twice"));
    assert_eq!(app.transfer().stage, Stage::Confirm);
    assert_eq!(f.count(), 0);
}

#[tokio::test]
async fn t40_maximum_usdt_exact_base_units_keep_asset_and_decimals_visible() {
    use alloy::primitives::U256;
    for amount in [U256::MAX, U256::MAX - U256::from(1)] {
        let f = Fixture::new(0);
        let n = Node::with_balance("f".repeat(64));
        let mut app = app(&f, &n);
        let work = key(&mut app, Key::Char('3'));
        drive(&mut app, &f, work).await;
        key(&mut app, Key::Down);
        let work = key(&mut app, Key::Enter);
        drive(&mut app, &f, work).await;
        key(&mut app, Key::Enter);
        let decimal = zwallet::core::amount::format_units(amount, 6);
        assert_eq!(amount.to_string().len(), 78);
        text(&mut app, &decimal);
        let work = key(&mut app, Key::Enter);
        drive(&mut app, &f, work).await;
        assert_eq!(app.transfer().stage, Stage::Review);
        let held = app.transfer().tron.review.as_ref().unwrap();
        assert_eq!(held.intent().transfer_call().unwrap().1, amount);
        let shown = screen(&app);
        assert!(
            shown.contains("USDT") && shown.contains("6 decimals"),
            "{shown}"
        );
        assert!(shown.contains("exact base units"), "{shown}");
        assert!(!shown.contains("(cut)"), "{shown}");
        let raw = amount.to_string();
        assert!(
            shown.lines().any(|line| line == format!("|{raw}|")),
            "{shown}"
        );
        key(&mut app, Key::Enter);
        let confirmation = screen(&app);
        assert!(confirmation.contains("USDT") && confirmation.contains("6 decimals"));
        assert!(confirmation.lines().any(|line| line == format!("|{raw}|")));
        assert_eq!(f.count(), 0);
    }
}

#[tokio::test]
async fn t38_t41_tron_menu_aliases_reach_scoped_handlers_without_signing() {
    use zwallet::tui::actions::{ActionId, Availability, Tab, by_id};
    let actions = [
        ("tron address", ActionId::TronAddress, Tab::Receive),
        ("tron balance", ActionId::TronBalance, Tab::Wallet),
        ("tron transfer", ActionId::TronTransfer, Tab::Send),
        ("tron tx", ActionId::TronTx, Tab::Wallet),
    ];
    for (command, action, tab) in actions {
        let f = Fixture::new(0);
        let n = Node::new();
        let mut app = app(&f, &n);
        assert_eq!(
            by_id(action).availability(&app.menu_context()),
            Availability::Ready
        );
        key(&mut app, Key::Char('?'));
        text(&mut app, command);
        let effects = key(&mut app, Key::Enter);
        assert_eq!(app.navigation().tab, tab);
        assert_eq!(app.chain().key, "tron");
        assert_eq!(app.net().rpc_url, n.node.url);
        let frame = screen(&app);
        let short_from = format!("{}...{}", &FROM[..6], &FROM[FROM.len() - 4..]);
        assert!(frame.lines().nth(22).unwrap().contains(&short_from));
        assert!(!frame.contains(EVM));
        match action {
            ActionId::TronAddress => assert!(
                effects
                    .iter()
                    .any(|e| matches!(e,Effect::LoadAddresses(_,key) if key=="main"))
            ),
            ActionId::TronBalance => {
                assert!(
                    effects
                        .iter()
                        .any(|e| matches!(e,Effect::LoadBalances(_,key) if key=="main"))
                );
                assert!(effects.iter().any(|e|matches!(e,Effect::HistoryOwner(t) if t.endpoint.chain=="tron" && t.key=="main")));
            }
            ActionId::TronTransfer => {
                assert!(effects.iter().any(|e| matches!(e, Effect::Transfer(_))));
                drive(&mut app, &f, effects).await;
                assert_eq!(app.transfer().stage, Stage::Assets);
                assert_eq!(app.transfer().assets[0].symbol, "TRX");
            }
            ActionId::TronTx => {
                let target = effects
                    .into_iter()
                    .find_map(|e| match e {
                        Effect::HistoryOwner(t) => Some(t),
                        _ => None,
                    })
                    .unwrap();
                app.update(Msg::HistoryOwner(
                    target,
                    Ok((FROM.into(), wallet::Trust::Session)),
                ));
                text(&mut app, &"1".repeat(64));
                let work = key(&mut app, Key::Enter);
                assert!(work.iter().any(|e|matches!(e,Effect::LoadTransaction(r) if r.hash=="1".repeat(64) && r.scope.owner==FROM && r.scope.target.endpoint.chain=="tron")));
            }
            _ => unreachable!(),
        }
        assert_eq!(f.count(), 0);
        assert!(n.node.calls("/wallet/broadcasthex").is_empty());
    }
}
#[test]
fn tron_menu_aliases_require_explicit_chain_selection() {
    use zwallet::tui::actions::{ActionId, Availability, by_id};
    for (command, action) in [
        ("tron address", ActionId::TronAddress),
        ("tron balance", ActionId::TronBalance),
        ("tron transfer", ActionId::TronTransfer),
        ("tron tx", ActionId::TronTx),
    ] {
        let f = Fixture::new(0);
        let n = Node::new();
        let mut app = app(&f, &n);
        key(&mut app, Key::Char('c'));
        for _ in chains::CHAINS {
            key(&mut app, Key::Up);
        }
        key(&mut app, Key::Enter);
        assert_ne!(app.chain().key, "tron");
        let before = app.chain().key;
        assert_eq!(
            by_id(action).availability(&app.menu_context()),
            Availability::Disabled("select Tron with c")
        );
        key(&mut app, Key::Char('?'));
        text(&mut app, command);
        assert!(key(&mut app, Key::Enter).is_empty());
        assert_eq!(app.chain().key, before);
        assert!(screen(&app).contains("select Tron with c"));
        assert_eq!(f.count(), 0);
    }
}
