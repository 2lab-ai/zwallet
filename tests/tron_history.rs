//! H18B: pure history state and isolated HTTP nodes; no daemon, CLI, signing or public traffic.
use serde_json::{Value, json};
use std::time::Duration;
use zwallet::{
    chains::{self, Net, SendJournal},
    tui::app::{App, Effect, Key, Msg, UserText, history::HistoryScope},
    wallet::{KeyRow, Keys, StatusView, Trust},
};
const OWNER: &str = "TWer2Ygk5TEheHp3TPuYeqxmB6SsGZmaL6";
const TO: &str = "TPjjvMwjPoDC32V2dGDYTkLH4E5LAtBZ6C";
const EVM: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
const USDT: &str = "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t";
fn hash(n: u64) -> String {
    format!("{n:064x}")
}
fn entry(n: u64) -> Value {
    json!({"chain":"tron","txHash":hash(n),"from":OWNER,"to":TO,"kind":"transfer",
      "ts":"2026-10-28T03:04:05Z","expiresAt":"2026-10-28T03:05:05.123Z",
      "amountRaw":"1250000","value":"999 FORGED"})
}
#[test]
fn tron_journal_rows_preserve_expiration_and_registry_units_without_a_nonce() {
    let home = tempfile::Builder::new()
        .prefix("zwt-tron-history-")
        .tempdir_in("/tmp")
        .unwrap();
    let journal = SendJournal::new(home.path(), chains::find("tron").unwrap(), OWNER);
    journal.append(&entry(1)).unwrap();
    let mut token = entry(2);
    token["kind"] = json!("token-transfer");
    token["token"] = json!(USDT);
    journal.append(&token).unwrap();
    let rows = journal.recent().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].value, "1.25 USDT");
    assert_eq!(rows[0].token.as_deref(), Some(USDT));
    assert_eq!(rows[1].value, "1.25 TRX");
    for row in rows {
        assert_eq!(row.from, OWNER);
        assert_eq!(row.to, TO);
        assert_eq!(row.nonce, None);
        assert_eq!(row.expiration_ms, Some(1_793_156_705_123));
    }
}
#[test]
fn tron_journal_drops_wrong_owner_family_hash_token_and_expiration() {
    let home = tempfile::Builder::new()
        .prefix("zwt-tron-history-")
        .tempdir_in("/tmp")
        .unwrap();
    let journal = SendJournal::new(home.path(), chains::find("tron").unwrap(), OWNER);
    journal.append(&entry(1)).unwrap();
    for (field, value) in [
        ("from", json!(TO)),
        ("from", json!(OWNER.to_ascii_lowercase())),
        ("to", json!(EVM)),
        ("txHash", json!(format!("0x{}", hash(9)))),
        ("chain", json!("ethereum")),
        ("expiresAt", json!("soon")),
        ("amountRaw", json!("-1")),
        ("kind", json!("cancel")),
    ] {
        let mut bad = entry(9);
        bad[field] = value;
        journal.append(&bad).unwrap();
    }
    let mut bad = entry(9);
    bad["kind"] = json!("token-transfer");
    bad["token"] = json!(TO);
    journal.append(&bad).unwrap();
    let rows = journal.recent().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].tx_hash, hash(1));
}
fn starting() -> App {
    let net = Net {
        chain: chains::find("tron").unwrap(),
        rpc_url: "http://127.0.0.1:1".into(),
    };
    starting_with(net)
}
fn starting_with(net: Net) -> App {
    let mut app = App::new(&net, Duration::from_secs(60));
    app.update(Msg::Keys(
        0,
        Ok(Keys {
            active: "first".into(),
            rows: vec![KeyRow {
                name: "first".into(),
                index: 0,
                address: Some(EVM.into()),
                active: true,
            }],
            trust: Trust::Locked,
        }),
    ));
    app.update(Msg::Status(
        0,
        Ok(StatusView {
            configured: true,
            locked: false,
            daemon_pids: vec![],
            other_daemon_pids: vec![],
            serving: None,
            active: Some("first".into()),
            address: Some((Some(EVM.into()), Trust::Locked)),
        }),
    ));
    app
}
fn app(owner: &str) -> (App, Vec<Effect>) {
    let mut app = starting();
    let target = app
        .update(Msg::Key(Key::Enter))
        .into_iter()
        .find_map(|e| match e {
            Effect::HistoryOwner(t) => Some(t),
            _ => None,
        })
        .unwrap();
    let work = app.update(Msg::HistoryOwner(target, Ok((owner.into(), Trust::Locked))));
    (app, work)
}
fn opened() -> (App, HistoryScope) {
    let (app, work) = app(OWNER);
    let scope = work
        .into_iter()
        .find_map(|e| match e {
            Effect::LoadHistory(s) => Some(s),
            _ => None,
        })
        .unwrap();
    (app, scope)
}
#[test]
fn tron_history_rejects_stale_evm_owner_before_scheduling_a_read() {
    let (app, work) = app(EVM);
    assert!(work.is_empty());
    assert!(app.history().scope.is_none());
}
#[test]
fn slash_accepts_bare_and_prefixed_tron_txids_and_preserves_scope() {
    for input in [hash(42), format!("0x{}", hash(42).to_ascii_uppercase())] {
        let (mut app, scope) = opened();
        app.update(Msg::Key(Key::Char('/')));
        app.update(Msg::Paste(UserText(zeroize::Zeroizing::new(input))));
        let request = app
            .update(Msg::Key(Key::Enter))
            .into_iter()
            .find_map(|e| match e {
                Effect::LoadTransaction(r) => Some(r),
                _ => None,
            })
            .expect("Tron hash should start lookup");
        assert_eq!(request.hash, hash(42));
        assert_eq!(request.scope, scope);
    }
}

#[allow(dead_code)]
#[path = "common/tron.rs"]
mod mock;
use mock::{MockTron, Reply};
use zwallet::{
    chains::tron::{self, send, tx},
    history::State,
    tui::app::history::{self as view, DetailRead, DetailRequest, TransactionDetail},
};
fn block(timestamp: u64) -> Value {
    let number = 9_000_000u64;
    let mut id = [0x77u8; 32];
    id[..8].copy_from_slice(&number.to_be_bytes());
    json!({"blockID":alloy::hex::encode(id),"block_header":{"raw_data":{"number":number,"timestamp":timestamp}}})
}
fn home() -> tempfile::TempDir {
    let home = tempfile::Builder::new()
        .prefix("zwt-tron-history-")
        .tempdir_in("/tmp")
        .unwrap();
    // Only public metadata is read; no password, decryption or socket is involved.
    std::fs::write(home.path().join("wallet.json"),serde_json::to_vec(&json!({
      "version":1,"createdAt":"2026-10-28T03:04:05Z","active":"first","nextIndex":1,
      "keys":[{"name":"first","index":0,"createdAt":"2026-10-28T03:04:05Z","addresses":{"tron":OWNER,"ethereum":EVM}}],
      "crypto":{"cipher":"aes-256-gcm","kdf":"scrypt","kdfparams":{"logN":14,"r":8,"p":1,"dklen":32,"salt":""},"nonce":"","ciphertext":""}
    })).unwrap()).unwrap();
    home
}
fn scope(node: &MockTron) -> HistoryScope {
    let (_, mut scope) = opened();
    scope.target.endpoint.url = node.url.clone();
    scope
}
fn request(scope: HistoryScope, hash: &str) -> DetailRequest {
    DetailRequest {
        scope,
        request: 1,
        hash: hash.into(),
        journal: None,
        explorer_listed: false,
    }
}
fn native(token: bool) -> Vec<u8> {
    let owner = tron::address::parse(OWNER).unwrap();
    let to = tron::address::parse(TO).unwrap();
    let contract = if token {
        let mut data = alloy::hex::decode("a9059cbb").unwrap();
        data.extend_from_slice(&[0; 12]);
        data.extend_from_slice(&to[1..]);
        data.extend_from_slice(&alloy::primitives::U256::from(1_250_000).to_be_bytes::<32>());
        tx::Contract::TriggerSmart {
            contract_address: tron::address::parse(USDT).unwrap(),
            data,
        }
    } else {
        tx::Contract::Transfer {
            to,
            amount_sun: 1_250_000,
        }
    };
    tx::encode_raw(&tx::DecodedTron {
        ref_block_bytes: [1, 2],
        ref_block_hash: [3; 8],
        expiration: send::now_ms() + 60_000,
        timestamp: send::now_ms(),
        owner,
        contract,
        fee_limit: token.then_some(10_000_000),
    })
    .unwrap()
}
fn observing(raw: Vec<u8>, mined: bool, result: Option<&'static str>) -> (MockTron, String) {
    let hash = alloy::hex::encode(tx::txid(&raw));
    let id = hash.clone();
    let node = MockTron::start(move |path, _| {
        Reply::Json(match path {
            "/wallet/getblockbynum" => mock::block0(mock::MAINNET_GENESIS),
            "/wallet/gettransactioninfobyid" if mined => {
                json!({"id":id,"blockNumber":42,"fee":12345,"receipt":{"result":result,"energy_usage_total":100,"net_usage":250}})
            }
            "/wallet/gettransactionbyid" => {
                json!({"txID":id,"raw_data_hex":alloy::hex::encode(&raw),"ret":[{"contractRet":result}],"from":"forged","to":"forged","symbol":"FAKE"})
            }
            "/wallet/gettransactioninfobyid" => json!({}),
            _ => panic!("unexpected path {path}"),
        })
    });
    (node, hash)
}
#[tokio::test]
async fn tron_history_is_ten_journal_rows_only_without_node_or_explorer_calls() {
    let home = home();
    let node = MockTron::start(|path, _| panic!("history must not fetch: {path}"));
    let scope = scope(&node);
    let journal = SendJournal::new(home.path(), chains::find("tron").unwrap(), OWNER);
    for n in 0..14 {
        let mut e = entry(n);
        e["ts"] = json!(format!("2026-10-28T03:04:{n:02}Z"));
        journal.append(&e).unwrap();
    }
    let read = view::read_history(home.path(), &scope).await.unwrap();
    assert_eq!(read.history.rows.len(), 10);
    assert_eq!(read.history.rows[0].tx_hash, hash(13));
    assert_eq!(
        read.history.coverage,
        "tron: this machine's sends only; incoming transfers not indexed"
    );
    assert!(
        read.history
            .rows
            .iter()
            .all(|r| r.nonce.is_none() && r.sources == [zwallet::history::Source::Journal])
    );
    assert!(node.seen().is_empty());
}
#[tokio::test]
async fn detail_uses_hash_checked_native_and_registry_intents_fee_sun_and_unknown_execution() {
    for (token, status) in [
        (false, Some("SUCCESS")),
        (true, Some("SUCCESS")),
        (true, Some("REVERT")),
        (true, Some("FUTURE_STATUS")),
        (true, None),
    ] {
        let home = home();
        let (node, hash) = observing(native(token), true, status);
        let read = view::read_transaction(home.path(), &request(scope(&node), &hash))
            .await
            .unwrap();
        let TransactionDetail::Tron(d) = read.detail else {
            panic!("wrong family")
        };
        assert_eq!(d.state, State::Mined);
        assert_eq!(d.block, Some(42));
        assert_eq!(d.fee_sun, Some(12345));
        assert_eq!(d.energy_used, Some(100));
        assert_eq!(d.net_usage, Some(250));
        assert_eq!(d.from.as_deref(), Some(OWNER));
        assert_eq!(d.to.as_deref(), Some(TO));
        assert_eq!(
            d.value.as_deref(),
            Some(if token { "1.25 USDT" } else { "1.25 TRX" })
        );
        assert_eq!(d.token, token.then_some(USDT));
        assert_eq!(
            d.success,
            match status {
                Some("SUCCESS") => Some(true),
                Some("REVERT") => Some(false),
                _ => None,
            }
        );
        if d.success.is_none() {
            assert!(d.hint.contains("second payment"));
        }
        assert!(!format!("{d:?}").contains("FAKE"));
        assert!(node.paths().iter().all(|p| p.starts_with("/wallet/")));
    }
}
#[tokio::test]
async fn unsupported_historical_bytes_and_unknown_token_never_get_transfer_semantics() {
    for mode in 0..2 {
        let mut raw = native(mode == 1);
        if mode == 0 {
            raw.extend_from_slice(&[0x80, 0x01, 0x01]);
        } else {
            let mut d = tx::decode_raw(&raw).unwrap();
            if let tx::Contract::TriggerSmart {
                contract_address, ..
            } = &mut d.contract
            {
                *contract_address = tron::address::parse(TO).unwrap();
            }
            raw = tx::encode_raw(&d).unwrap();
        }
        let (node, hash) = observing(raw, true, Some("SUCCESS"));
        let home = home();
        let read = view::read_transaction(home.path(), &request(scope(&node), &hash))
            .await
            .unwrap();
        let TransactionDetail::Tron(d) = read.detail else {
            panic!()
        };
        assert!(d.value.is_none() && d.token.is_none() && d.to.is_none());
    }
}
#[tokio::test]
async fn pending_not_found_and_expired_unseen_preserve_observation_limits() {
    let (node, hash) = observing(native(false), false, None);
    let home = home();
    let read = view::read_transaction(home.path(), &request(scope(&node), &hash))
        .await
        .unwrap();
    let TransactionDetail::Tron(d) = read.detail else {
        panic!()
    };
    assert_eq!(d.state, State::Pending);
    assert_eq!(d.fee_sun, None);
    assert_eq!(d.success, None);
    for stale_head in [false, true] {
        let head = send::now_ms() - if stale_head { 120_000 } else { 1000 };
        let node = MockTron::start(move |path, _| {
            Reply::Json(match path {
                "/wallet/getblockbynum" => mock::block0(mock::MAINNET_GENESIS),
                "/wallet/getblock" => block(head),
                _ => json!({}),
            })
        });
        let scope = scope(&node);
        let mut req = request(scope.clone(), &format!("0x{}", crate::hash(7)));
        // Caller-provided rows must not supply expiry. Only the fresh, owner-scoped journal does.
        let journal = SendJournal::new(home.path(), chains::find("tron").unwrap(), OWNER);
        let mut old = entry(7);
        old["expiresAt"] = json!(send::expires_at(send::now_ms() - 180_000));
        let forged = entry(8);
        journal.append(&forged).unwrap();
        req.journal = journal.recent().unwrap().first().cloned();
        let read = view::read_transaction(home.path(), &req).await.unwrap();
        let TransactionDetail::Tron(d) = read.detail else {
            panic!()
        };
        assert_eq!(d.state, State::NotFound);
        assert!(d.as_of.is_none());
        journal.append(&old).unwrap();
        let read = view::read_transaction(home.path(), &req).await.unwrap();
        let TransactionDetail::Tron(d) = read.detail else {
            panic!()
        };
        assert_eq!(
            d.state,
            if stale_head {
                State::NotFound
            } else {
                State::ExpiredUnseen
            }
        );
        if stale_head {
            assert!(d.node_behind_secs.unwrap() >= 120);
        } else {
            let at = d.as_of.unwrap();
            assert_eq!(at.block, 9_000_000);
            assert_eq!(at.block_time_ms, head);
            assert_eq!(at.host, chains::url_origin(&node.url));
            assert!(d.hint.contains("second payment"));
        }
        // The following case must begin without the just-written matching journal row.
        std::fs::remove_file(journal.path()).unwrap();
    }
}
#[tokio::test]
async fn wrong_genesis_receipt_hash_raw_hash_or_changed_owner_are_refused() {
    for case in 0..4 {
        let raw = native(false);
        let hash = alloy::hex::encode(tx::txid(&raw));
        let id = hash.clone();
        let node = MockTron::start(move |path, _| {
            Reply::Json(match path {
                "/wallet/getblockbynum" => mock::block0(if case == 0 {
                    mock::NILE_GENESIS
                } else {
                    mock::MAINNET_GENESIS
                }),
                "/wallet/gettransactioninfobyid" => {
                    json!({"id":if case==1{crate::hash(99)}else{id.clone()},"blockNumber":42})
                }
                "/wallet/gettransactionbyid" => {
                    json!({"txID":id,"raw_data_hex":if case==2{"00".into()}else{alloy::hex::encode(&raw)}})
                }
                _ => panic!("unexpected path {path}"),
            })
        });
        let home = home();
        let mut scope = scope(&node);
        if case == 3 {
            scope.owner = TO.into();
        }
        assert!(
            view::read_transaction(home.path(), &request(scope, &hash))
                .await
                .is_err()
        );
        if case == 0 {
            assert_eq!(node.paths(), ["/wallet/getblockbynum"]);
        }
        if case == 3 {
            assert!(node.seen().is_empty());
        }
    }
}
#[tokio::test]
async fn backend_rejects_an_evm_net_or_owner_without_any_rpc() {
    let node = MockTron::start(|path, _| panic!("wrong family must not fetch: {path}"));
    let mut net = Net {
        chain: chains::find("ethereum").unwrap(),
        rpc_url: node.url.clone(),
    };
    assert!(
        zwallet::history::tron::lookup(&net, &hash(1), OWNER, None)
            .await
            .is_err()
    );
    net.chain = chains::find("tron").unwrap();
    assert!(
        zwallet::history::tron::lookup(&net, &hash(1), EVM, None)
            .await
            .is_err()
    );
    assert!(node.seen().is_empty());
}

fn model(hash: &str) -> DetailRead {
    DetailRead{list_unchanged:false,detail:TransactionDetail::Tron(zwallet::history::tron::Detail {
        chain:"tron",tx_hash:hash.into(),state:State::Mined,success:None,from:Some(OWNER.into()),to:Some(TO.into()),value:Some("1.25 USDT".into()),token:Some(USDT),fee_sun:Some(12345),block:Some(42),expiration_ms:Some(1_793_156_705_123),energy_used:Some(100),net_usage:Some(250),as_of:None,node_behind_secs:None,
        hint:"execution result is unknown; Sending this payment again is a second payment if this one was included".into(),
    })}
}
fn start(app: &mut App, hash: &str) -> DetailRequest {
    app.update(Msg::Key(Key::Char('/')));
    app.update(Msg::Paste(UserText(zeroize::Zeroizing::new(hash.into()))));
    *app.update(Msg::Key(Key::Enter))
        .into_iter()
        .find_map(|e| match e {
            Effect::LoadTransaction(r) => Some(r),
            _ => None,
        })
        .unwrap()
}
fn frame(app: &App, width: u16, height: u16) -> String {
    use ratatui::{Terminal, backend::TestBackend, buffer::Cell};
    use zwallet::tui::{style, term::FilteredBackend, text::Source, ui};
    let mut terminal =
        Terminal::new(FilteredBackend::new(TestBackend::new(width, height))).unwrap();
    let mut placed = vec![];
    terminal.draw(|f| placed = ui::draw(f, app)).unwrap();
    assert_eq!(terminal.backend().replaced(), 0);
    let buffer = terminal.backend().inner().buffer();
    let styles: Vec<_> = style::ALL
        .iter()
        .map(|style| {
            let mut cell = Cell::default();
            cell.set_style(*style);
            cell.style()
        })
        .collect();
    for cell in &buffer.content {
        assert!(styles.contains(&cell.style()));
        assert!(
            cell.symbol()
                .chars()
                .all(|c| c.is_ascii_graphic() || c == ' ' || c == '₮')
        );
    }
    for p in placed {
        assert!(p.area.right() <= width && p.area.bottom() <= height);
        assert!(p.outside.text().is_ascii());
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
        if p.area.height == 1 {
            let actual: String = (p.area.x..p.area.right())
                .map(|x| buffer[(x, p.area.y)].symbol())
                .collect();
            assert_eq!(actual, p.outside.text());
        }
    }
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
#[test]
fn tron_detail_frame_names_unknown_execution_fee_and_expiration_without_evm_controls() {
    let (mut app, _) = opened();
    let req = start(&mut app, &hash(7));
    app.update(Msg::Transaction(Box::new(req), Ok(model(&hash(7)))));
    let text = frame(&app, 80, 24);
    std::fs::write("/tmp/h18b-tron-detail-80x24.txt", &text).unwrap();
    for expected in [
        "mined (outcome unknown)",
        "12345 sun",
        "Decoded transfer intent: 1.25 USDT",
        "2026-10-28T03:05:05.123Z",
        "s/x/R unavailable",
        "second payment",
    ] {
        assert!(text.contains(expected), "{expected}: {text}");
    }
    for forbidden in [
        "Nonce:",
        "Gas used:",
        "wei",
        "Fee: 0",
        "verified by password",
    ] {
        assert!(!text.contains(forbidden), "{text}");
    }
    assert!(frame(&app, 79, 24).contains("terminal too small"));
    for k in ['s', 'x', 'R'] {
        assert!(app.update(Msg::Key(Key::Char(k))).is_empty());
    }
}
#[test]
fn late_tron_results_cannot_cross_hash_chain_owner_key_endpoint_or_generation() {
    let (mut app, scope) = opened();
    let old = start(&mut app, &hash(1));
    let current = start(&mut app, &hash(2));
    app.update(Msg::Transaction(Box::new(old), Ok(model(&hash(1)))));
    for part in 0..5 {
        let mut stale = current.clone();
        match part {
            0 => stale.scope.target.endpoint.chain = "ethereum",
            1 => stale.scope.owner = TO.into(),
            2 => stale.scope.target.key = "other".into(),
            3 => stale.scope.target.endpoint.url = "http://127.0.0.1:2".into(),
            _ => stale.scope.target.generation += 1,
        }
        app.update(Msg::Transaction(Box::new(stale), Ok(model(&hash(2)))));
        assert!(frame(&app, 80, 24).contains("reading transaction"));
    }
    // A response object with the right request envelope but a wrong detail hash fails closed.
    app.update(Msg::Transaction(
        Box::new(current.clone()),
        Ok(model(&hash(3))),
    ));
    assert!(frame(&app, 80, 24).contains("another chain or hash"));
    let current = start(&mut app, &hash(2));
    app.update(Msg::Key(Key::Char('c')));
    app.update(Msg::Key(Key::Up));
    app.update(Msg::Key(Key::Enter));
    assert_ne!(app.chain().key, "tron");
    app.update(Msg::Transaction(Box::new(current), Ok(model(&hash(2)))));
    app.update(Msg::History(
        scope,
        Ok(view::HistoryRead {
            history: zwallet::history::History {
                rows: vec![],
                coverage: "STALE_TRON".into(),
                dropped: 0,
            },
            list_unchanged: false,
        }),
    ));
    assert!(!frame(&app, 80, 24).contains("STALE_TRON"));
    assert!(app.history().detail.is_none());
}
#[test]
fn tron_hash_field_is_bounded_atomic_and_does_not_relax_evm_hashes() {
    use zwallet::tui::form::{Edit, Field, Input, Kind};
    for bad in [
        "a".repeat(65),
        format!("0x{}", "a".repeat(65)),
        format!("{}g", "a".repeat(63)),
        format!("{}\n", "a".repeat(64)),
        "TWer2Ygk5TEheHp3TPuYeqxmB6SsGZmaL6".into(),
    ] {
        let mut field = Field::new(Kind::TronTxHash);
        assert!(matches!(field.edit(Input::Paste(&bad)), Edit::Rejected(_)));
        assert_eq!(field.value(), "");
    }
    assert!(Field::with_value(Kind::TxHash, &hash(1)).is_err());
    let mut short = Field::with_value(Kind::TronTxHash, &"a".repeat(63)).unwrap();
    assert!(matches!(short.edit(Input::Enter), Edit::Rejected(_)));
    assert_eq!(short.edit(Input::Char('A')), Edit::Changed);
    assert_eq!(short.edit(Input::Enter), Edit::Submit);
}

#[test]
fn selected_family_header_and_key_rows_never_show_the_primary_evm_address_on_tron() {
    let mut before = starting();
    assert!(!frame(&before, 80, 24).contains("0x7099"));
    before.update(Msg::Address(
        before.generation(),
        "first".into(),
        Ok((OWNER.into(), Trust::Locked)),
    ));
    let text = frame(&before, 80, 24);
    assert!(text.contains("TWer2"));
    assert!(!text.contains("0x7099"));
    assert!(text.contains("not verified"));
    let (mut app, _) = opened();
    assert!(!frame(&app, 80, 24).contains("0x7099"));
    app.update(Msg::Key(Key::Char('1')));
    assert!(!frame(&app, 80, 24).contains("0x7099"));
}

#[test]
fn family_display_trust_tracks_cached_session_password_lock_and_chain_changes() {
    let mut app = starting();
    app.update(Msg::Address(
        app.generation(),
        "first".into(),
        Ok((OWNER.into(), Trust::Session)),
    ));
    let text = frame(&app, 80, 24);
    assert!(text.contains("verified by session"));
    assert!(text.contains("TWer2"));
    assert!(!text.contains("0x7099"));
    app.update(Msg::Status(
        app.generation(),
        Ok(StatusView {
            configured: true,
            locked: true,
            daemon_pids: vec![],
            other_daemon_pids: vec![],
            serving: None,
            active: Some("first".into()),
            address: Some((Some(EVM.into()), Trust::Locked)),
        }),
    ));
    assert!(!frame(&app, 80, 24).contains("verified by session"));
    pin_addresses(&mut app);
    let text = frame(&app, 80, 24);
    assert!(text.contains("verified by password"), "{text}");
    assert!(!text.contains("0x7099"));
    app.update(Msg::Address(
        app.generation(),
        "first".into(),
        Ok((
            OWNER.into(),
            Trust::Unverified(Some("fixture impostor".into())),
        )),
    ));
    let text = frame(&app, 80, 24);
    assert!(text.contains("NOT this build"));
    assert!(!text.contains("verified by password"));
    app.update(Msg::Key(Key::Char('c')));
    app.update(Msg::Key(Key::Up));
    app.update(Msg::Key(Key::Enter));
    assert_ne!(app.chain().key, "tron");
    let text = frame(&app, 80, 24);
    assert!(!text.contains("TWer2"), "{text}");
    assert!(text.contains("0x7099"));
}

#[test]
fn a_fresh_impostor_status_revokes_tron_session_display_trust() {
    let (mut app, _) = opened();
    app.update(Msg::Address(
        app.generation(),
        "first".into(),
        Ok((OWNER.into(), Trust::Session)),
    ));
    assert!(frame(&app, 80, 24).contains("verified by session"));
    app.update(Msg::Status(
        app.generation(),
        Ok(StatusView {
            configured: true,
            locked: false,
            daemon_pids: vec![],
            other_daemon_pids: vec![],
            serving: None,
            active: Some("first".into()),
            address: Some((
                Some(EVM.into()),
                Trust::Unverified(Some("fixture impostor".into())),
            )),
        }),
    ));
    let text = frame(&app, 80, 24);
    assert!(text.contains("NOT this build"));
    assert!(!text.contains("verified by session"));
    assert!(!text.contains("0x7099"));
}

fn pin_addresses(app: &mut App) {
    use zwallet::wallet::{Opened, PasswordCheck, VerifiedAddress};
    app.update(Msg::Verified(Ok((
        PasswordCheck {
            opened: Opened {
                keys: vec![KeyRow {
                    name: "first".into(),
                    index: 0,
                    address: Some(EVM.into()),
                    active: true,
                }],
                active: "first".into(),
                warning: None,
                anchor: Ok(None),
                salted: false,
            },
            addresses: vec![
                VerifiedAddress {
                    key: "first".into(),
                    chain: "ethereum",
                    family: "evm",
                    address: EVM.into(),
                },
                VerifiedAddress {
                    key: "first".into(),
                    chain: "tron",
                    family: "tron",
                    address: OWNER.into(),
                },
            ],
            repairs: vec![],
        },
        Err("no list".into()),
    ))));
}

fn assert_status_mismatch_keeps_fresh_lock(new_locked: bool) {
    use zwallet::{
        tui::app::{LockLine, Pane},
        wallet::Serving,
    };
    let status = |locked, address: &str, expiry: &str| StatusView {
        configured: true,
        locked,
        daemon_pids: if locked { vec![] } else { vec![41] },
        other_daemon_pids: vec![],
        serving: (!locked).then(|| Serving {
            pid: 41,
            expires_at: Some(expiry.into()),
            whitelist: json!({}),
        }),
        active: Some("first".into()),
        address: Some((Some(address.into()), Trust::Locked)),
    };
    for changed in [
        "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC",
        "invalid primary address",
    ] {
        let mut app = starting();
        pin_addresses(&mut app);
        app.update(Msg::Status(
            app.generation(),
            Ok(status(!new_locked, EVM, "2098-01-02T03:04:05Z")),
        ));
        app.update(Msg::Status(
            app.generation(),
            Ok(status(new_locked, changed, "2099-01-02T03:04:05Z")),
        ));
        let Pane::Ready(bar) = app.status() else {
            panic!("fresh lock evidence was discarded")
        };
        if new_locked {
            assert_eq!(bar.lock, LockLine::Locked);
        } else {
            let LockLine::Open(Some(expiry)) = &bar.lock else {
                panic!("newly open session was hidden: {:?}", bar.lock)
            };
            assert!(expiry.text().contains("2099"), "{}", expiry.text());
        }
        let latest = app.navigation().session.as_ref().unwrap();
        assert_eq!(latest.locked, new_locked);
        if !new_locked {
            assert_eq!(
                latest.serving.as_ref().unwrap().expires_at.as_deref(),
                Some("2099-01-02T03:04:05Z")
            );
        }
        assert!(matches!(app.keys(), Pane::Failed(_)));
        assert!(app.family_pinned("first", chains::find("tron").unwrap()));
        let shown = frame(&app, 80, 24);
        assert!(
            !shown
                .lines()
                .rev()
                .take(2)
                .any(|line| line.contains("0x7099"))
        );
        if new_locked {
            assert!(shown.contains("locked"));
            assert!(!shown.contains("signing (agent too)"));
        } else {
            assert!(shown.contains("signing (agent too) until"));
        }
    }
}
#[test]
fn primary_address_mismatch_does_not_hide_a_freshly_locked_session() {
    assert_status_mismatch_keeps_fresh_lock(true);
}
#[test]
fn primary_address_mismatch_does_not_hide_a_new_session_and_expiry() {
    assert_status_mismatch_keeps_fresh_lock(false);
}

#[test]
fn negative_session_status_downgrades_cached_tron_proof_but_preserves_password_pin() {
    use zwallet::tui::app::TrustLine;
    for pinned in [false, true] {
        let (mut app, _) = opened();
        if pinned {
            pin_addresses(&mut app);
        }
        app.update(Msg::Address(
            app.generation(),
            "first".into(),
            Ok((OWNER.into(), Trust::Session)),
        ));
        assert_eq!(
            app.displayed_address("first").unwrap().1,
            if pinned {
                TrustLine::Password
            } else {
                TrustLine::Session
            }
        );
        app.update(Msg::Status(
            app.generation(),
            Ok(StatusView {
                configured: true,
                locked: false,
                daemon_pids: vec![41],
                other_daemon_pids: vec![],
                serving: None,
                active: Some("first".into()),
                address: Some((Some(EVM.into()), Trust::Unverified(None))),
            }),
        ));
        assert_eq!(
            app.displayed_address("first").unwrap().1,
            if pinned {
                TrustLine::Password
            } else {
                TrustLine::NotVerified
            }
        );
        let shown = frame(&app, 80, 24);
        assert!(!shown.contains("verified by session"));
        if pinned {
            assert!(shown.contains("verified by password"));
        } else {
            assert!(shown.contains("not verified"));
        }
        assert!(!shown.contains("0x7099"));
        assert!(shown.contains("signing (agent too)"));
    }
}

#[tokio::test]
async fn t41_app_slash_drives_real_tron_lookup_and_keeps_scope_and_execution_uncertainty() {
    for token in [false, true] {
        for result in [Some("SUCCESS"), Some("REVERT"), None] {
            let home = home();
            let (node, hash) = observing(native(token), true, result);
            let net = Net {
                chain: chains::find("tron").unwrap(),
                rpc_url: node.url.clone(),
            };
            let mut app = starting_with(net);
            app.configure(home.path(), false);
            let target = app
                .update(Msg::Key(Key::Enter))
                .into_iter()
                .find_map(|e| match e {
                    Effect::HistoryOwner(t) => Some(t),
                    _ => None,
                })
                .unwrap();
            let owner =
                zwallet::wallet::address(home.path(), &target.key, target.net().unwrap().chain)
                    .await
                    .unwrap();
            let scope = app
                .update(Msg::HistoryOwner(target, Ok(owner)))
                .into_iter()
                .find_map(|e| match e {
                    Effect::LoadHistory(s) => Some(s),
                    _ => None,
                })
                .unwrap();
            let read = view::read_history(home.path(), &scope).await.unwrap();
            app.update(Msg::History(scope, Ok(read)));
            let before = frame(&app, 80, 24);
            assert!(before.contains("this machine's sends only"));
            assert!(!before.contains(EVM));
            let request = start(&mut app, &hash);
            assert_eq!(request.hash, hash);
            let read = view::read_transaction(home.path(), &request).await.unwrap();
            app.update(Msg::Transaction(Box::new(request), Ok(read)));
            let mut shown = frame(&app, 80, 24);
            for _ in 0..5 {
                app.update(Msg::Key(Key::Down));
                shown.push_str(&frame(&app, 80, 24));
            }
            assert!(shown.contains(&hash));
            assert!(shown.contains("sun"));
            assert!(shown.contains(if token { "USDT" } else { "TRX" }));
            assert_eq!(shown.contains(&format!("Token: {USDT}")), token, "{shown}");
            assert!(!shown.contains("Nonce:") && !shown.contains("Gas used:"));
            if result.is_none() {
                assert!(shown.contains("second payment"), "{shown}");
            }
            for action in ['s', 'x', 'R'] {
                assert!(app.update(Msg::Key(Key::Char(action))).is_empty());
                let shown = frame(&app, 80, 24);
                assert!(shown.contains("no nonce replacement"), "{shown}");
            }
            assert_eq!(node.calls("/wallet/gettransactionbyid").len(), 1);
        }
    }
}
