//! History/detail state and TestBackend only: no wallet, daemon or real terminal.
use ratatui::{Terminal, backend::TestBackend};
use std::time::Duration;
use zwallet::tui::term::FilteredBackend;
use zwallet::tui::{
    app::history::HistoryRead,
    app::{App, Effect, Key, Msg},
    ui,
};
use zwallet::wallet::{KeyRow, Keys, StatusView, Trust};
use zwallet::{
    chains::{self, Net},
    history::{self, Row, Source, State},
};
const OWNER: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
fn hash(n: u64) -> String {
    format!("0x{n:064x}")
}
fn opened() -> (App, zwallet::tui::app::history::HistoryScope) {
    let net = Net {
        chain: chains::find("ethereum").unwrap(),
        rpc_url: "http://127.0.0.1:1".into(),
    };
    let mut app = App::new(&net, Duration::from_secs(60));
    app.update(Msg::Keys(
        0,
        Ok(Keys {
            active: "first".into(),
            rows: vec![KeyRow {
                name: "first".into(),
                index: 0,
                address: Some(OWNER.into()),
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
            address: Some((Some(OWNER.into()), Trust::Locked)),
        }),
    ));
    let work = app.update(Msg::Key(Key::Enter));
    let target = work
        .into_iter()
        .find_map(|e| match e {
            Effect::HistoryOwner(t) => Some(t),
            _ => None,
        })
        .unwrap();
    let work = app.update(Msg::HistoryOwner(target, Ok((OWNER.into(), Trust::Locked))));
    let scope = work
        .into_iter()
        .find_map(|e| match e {
            Effect::LoadHistory(s) => Some(s),
            _ => None,
        })
        .unwrap();
    (app, scope)
}
fn frame(app: &App, width: u16, height: u16) -> String {
    use ratatui::buffer::Cell;
    use zwallet::tui::style;
    let mut terminal =
        Terminal::new(FilteredBackend::new(TestBackend::new(width, height))).unwrap();
    let mut places = vec![];
    terminal.draw(|f| places = ui::draw(f, app)).unwrap();
    assert_eq!(terminal.backend().replaced(), 0);
    for p in &places {
        assert!(p.area.right() <= width && p.area.bottom() <= height);
        assert!(p.outside.text().is_ascii());
        assert!(p.outside.text().len() <= p.outside.source().budget());
        if p.outside.source() == zwallet::tui::text::Source::Error {
            assert!(p.area.height <= 7);
        } else {
            assert_eq!(p.area.height, 1);
        }
    }
    let b = terminal.backend().inner().buffer();
    let styles: Vec<_> = style::ALL
        .iter()
        .map(|style| {
            let mut cell = Cell::default();
            cell.set_style(*style);
            cell.style()
        })
        .collect();
    for cell in &b.content {
        assert_eq!(cell.symbol().len(), 1, "not a single ASCII cell");
        assert!(cell.symbol().bytes().all(|b| (0x20..=0x7e).contains(&b)));
        assert!(styles.contains(&cell.style()), "style: {:?}", cell.style());
    }
    for p in &places {
        let shown = (p.area.y..p.area.bottom())
            .flat_map(|y| (p.area.x..p.area.right()).map(move |x| b[(x, y)].symbol()))
            .collect::<String>();
        assert!(
            shown.starts_with(p.outside.text()),
            "outside placement: {:?}",
            p.area
        );
    }
    (0..height)
        .map(|y| (0..width).map(|x| b[(x, y)].symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}
#[test]
fn ten_history_rows_keep_pending_first_sources_and_coverage_visible() {
    let (mut app, scope) = opened();
    let chain = chains::find("ethereum").unwrap();
    let rows = (1..=14)
        .map(|n| {
            let mut row = Row::new(chain, hash(n), Source::Explorer);
            row.state = if n <= 2 { State::Pending } else { State::Mined };
            row.block_number = (n > 2).then_some(n);
            row
        })
        .collect();
    let merged = history::merge(
        chain,
        vec![chains::JournalRow {
            chain: chain.key,
            tx_hash: hash(1),
            from: OWNER.into(),
            to: OWNER.into(),
            nonce: Some(1),
            expiration_ms: None,
            timestamp: 1,
            kind: "transfer",
            amount_raw: Some(alloy::primitives::U256::from(1)),
            value: "0.000000000000000001 ETH".into(),
            token: None,
        }],
        Ok(chains::evm::explorer::Page { rows, dropped: 3 }),
    );
    app.update(Msg::History(
        scope,
        Ok(HistoryRead {
            history: merged,
            list_unchanged: false,
        }),
    ));
    let text = frame(&app, 80, 24);
    assert!(text.contains("Recent transactions"), "{text}");
    assert_eq!(text.matches("explorer]").count(), 10, "{text}");
    assert!(text.contains("[journal,explorer]"));
    assert!(text.find("pending").unwrap() < text.find("mined").unwrap());
    assert!(text.contains("3 explorer rows not shown"), "{text}");
}

fn lookup(app: &mut App, n: u64) -> zwallet::tui::app::history::DetailRequest {
    app.update(Msg::Key(Key::Char('/')));
    app.update(Msg::Paste(zwallet::tui::app::UserText(
        zeroize::Zeroizing::new(hash(n)),
    )));
    app.update(Msg::Key(Key::Enter))
        .into_iter()
        .find_map(|e| match e {
            Effect::LoadTransaction(r) => Some(*r),
            _ => None,
        })
        .unwrap()
}
fn detail(n: u64, state: State) -> zwallet::tui::app::history::DetailRead {
    use alloy::primitives::U256;
    use zwallet::chains::evm::rpc::{MovementsState, TxDetail};
    use zwallet::tui::app::history::{DetailRead, TransactionDetail};
    DetailRead {
        list_unchanged: false,
        detail: TransactionDetail::Evm(TxDetail {
            chain: "ethereum",
            tx_hash: hash(n),
            state,
            success: Some(true),
            from: Some(OWNER.into()),
            to: Some("0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC".into()),
            contract_creation: false,
            nonce: Some(7),
            journal_nonce: None,
            value: Some(U256::from(1_000_000_000_000_000_000u64)),
            block_number: Some(90),
            gas_used: Some(U256::from(21000)),
            effective_gas_price: Some(U256::from(2)),
            l1_fee: Some(U256::from(1_000_000_000_000_000u64)),
            fee: Some(U256::from(2_000_000_000_000_000u64)),
            confirmations: Some(5),
            movements: vec![history::TokenMovement {
                token: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".into(),
                symbol: "USDC",
                decimals: 6,
                from: "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC".into(),
                to: OWNER.into(),
                amount: U256::from(1234567),
                log_index: 1,
            }],
            movements_state: MovementsState::Confirmed,
            hidden_movements: 2,
            hint: "Look up the original hash before another payment",
        }),
    }
}
#[test]
fn hash_field_requires_all_64_hex_digits_and_consumes_shortcuts() {
    let (mut app, _) = opened();
    app.update(Msg::Key(Key::Char('/')));
    app.update(Msg::Paste(zwallet::tui::app::UserText(
        zeroize::Zeroizing::new(format!("0x{}", "a".repeat(63))),
    )));
    assert!(app.update(Msg::Key(Key::Enter)).is_empty());
    assert!(app.navigation().popup.is_some());
    app.update(Msg::Key(Key::Char('q')));
    assert!(!app.quitting());
    app.update(Msg::Key(Key::Char('a')));
    let work = app.update(Msg::Key(Key::Enter));
    assert!(work.iter().any(|e| matches!(e, Effect::LoadTransaction(_))));
}
#[test]
fn detail_shows_node_fields_registry_movements_and_read_only_controls() {
    let (mut app, _) = opened();
    let request = lookup(&mut app, 4);
    app.update(transaction(request, Ok(detail(4, State::Mined))));
    let text = frame(&app, 80, 24);
    for expected in [
        "Fee: 0.002 ETH",
        "Included L1 fee: 0.001 ETH",
        "Confirmations: 5",
        "Nonce: 7",
        "needs: V   r refresh   Esc history",
    ] {
        assert!(text.contains(expected), "{expected}: {text}");
    }
    for _ in 0..8 {
        app.update(Msg::Key(Key::Down));
    }
    let text = frame(&app, 80, 24);
    assert!(text.contains("1.234567 USDC"), "{text}");
    assert!(text.contains("2 token movements not shown"), "{text}");
    for key in ['s', 'x', 'R'] {
        assert!(app.update(Msg::Key(Key::Char(key))).is_empty());
    }
    let text = frame(&app, 120, 30);
    assert!(!text.contains("s speed up"), "{text}");
    assert!(!text.contains("x cancel"), "{text}");
}
#[test]
fn previous_hash_chain_key_and_endpoint_results_never_replace_the_current_view() {
    let (mut app, scope) = opened();
    let old = lookup(&mut app, 4);
    let new = lookup(&mut app, 5);
    app.update(transaction(old, Ok(detail(4, State::Mined))));
    assert!(frame(&app, 80, 24).contains("reading transaction"));
    let mut wrong_endpoint = new.clone();
    wrong_endpoint.scope.target.endpoint.url = "http://127.0.0.1:2".into();
    app.update(transaction(wrong_endpoint, Ok(detail(5, State::Mined))));
    let mut wrong_key = new.clone();
    wrong_key.scope.target.key = "another".into();
    app.update(transaction(wrong_key, Ok(detail(5, State::Mined))));
    assert!(frame(&app, 80, 24).contains("reading transaction"));
    app.update(Msg::Key(Key::Char('c')));
    app.update(Msg::Key(Key::Down));
    app.update(Msg::Key(Key::Enter));
    app.update(transaction(new, Ok(detail(5, State::Mined))));
    app.update(Msg::History(
        scope,
        Ok(HistoryRead {
            history: history::History {
                rows: vec![],
                coverage: "STALE_EXPLORER_MARKER".into(),
                dropped: 0,
            },
            list_unchanged: false,
        }),
    ));
    let text = frame(&app, 80, 24);
    assert!(!text.contains("STALE_EXPLORER_MARKER"));
    assert!(!text.contains("Confirmations: 5"));
    assert_eq!(app.chain().key, "base");
}
#[test]
fn hostile_detail_error_is_bounded_and_dropped_state_names_the_original_hash() {
    let (mut app, _) = opened();
    let request = lookup(&mut app, 6);
    app.update(transaction(
        request.clone(),
        Err(zwallet::core::error::WalletError::rpc(
            "\u{1b}[2J█".repeat(600),
        )),
    ));
    let text = frame(&app, 80, 24);
    assert!(!text.contains('█'));
    assert!(!text.contains('\u{1b}'));
    app.update(transaction(request, Ok(detail(6, State::DroppedReplaced))));
    let text = frame(&app, 80, 24);
    assert!(text.contains("dropped/replaced"));
    assert!(text.contains(&hash(6)));
    assert!(text.contains("Nonce: 7"));
}
#[test]
fn unavailable_explorer_has_coverage_without_a_body_or_banner() {
    let (mut app, scope) = opened();
    let chain = chains::find("ethereum").unwrap();
    app.update(Msg::History(
        scope,
        Ok(HistoryRead {
            history: history::merge(
                chain,
                vec![],
                Err(chains::evm::explorer::Unavailable::Http(403)),
            ),
            list_unchanged: false,
        }),
    ));
    let text = frame(&app, 80, 24);
    assert!(text.contains("explorer unavailable (HTTP 403)"));
    assert!(!text.contains("history unavailable:"));
    assert!(frame(&app, 79, 24).contains("terminal too small"));
}

fn transaction(
    request: zwallet::tui::app::history::DetailRequest,
    result: zwallet::core::error::Result<zwallet::tui::app::history::DetailRead>,
) -> Msg {
    Msg::Transaction(Box::new(request), result)
}

#[test]
fn a_hash_this_node_does_not_know_keeps_the_lookup_hint() {
    let (mut app, _) = opened();
    let request = lookup(&mut app, 77);
    let mut read = detail(77, State::NotFound);
    if let zwallet::tui::app::history::TransactionDetail::Evm(d) = &mut read.detail {
        d.hint = "this node does not know the transaction; look up the original hash before another payment";
        d.from = None;
        d.to = None;
        d.nonce = None;
        d.value = None;
        d.fee = None;
        d.l1_fee = None;
        d.gas_used = None;
        d.effective_gas_price = None;
        d.block_number = None;
        d.confirmations = None;
        d.movements.clear();
        d.movements_state = zwallet::chains::evm::rpc::MovementsState::Unknown;
    }
    app.update(transaction(request, Ok(read)));
    let text = frame(&app, 80, 24);
    assert!(text.contains("Status: not found"), "{text}");
    assert!(
        text.contains("this node does not know the transaction"),
        "{text}"
    );
    assert!(!text.contains("contract creation"));
}

#[test]
fn changed_owner_invalidates_history_before_an_old_reply_can_arrive() {
    let (mut app, scope) = opened();
    app.update(Msg::Address(
        app.generation(),
        "first".into(),
        Ok((
            "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC".into(),
            Trust::Locked,
        )),
    ));
    app.update(Msg::History(
        scope,
        Ok(HistoryRead {
            history: history::History {
                rows: vec![],
                coverage: "OLD_OWNER_RESULT".into(),
                dropped: 0,
            },
            list_unchanged: false,
        }),
    ));
    let text = frame(&app, 80, 24);
    assert!(!text.contains("OLD_OWNER_RESULT"));
    assert!(text.contains("selected key's address changed"), "{text}");
}
#[test]
fn forged_explorer_names_and_symbols_never_reach_a_cell() {
    use serde_json::json;
    let (mut app, scope) = opened();
    let chain = chains::find("ethereum").unwrap();
    let body = json!({"items":[{"hash":hash(8),"block_number":8,"position":1,"nonce":7,
        "from":{"hash":OWNER,"name":"zwallet: verified"},"to":{"hash":OWNER},
        "value":"1","fee":{"value":"21000"},"status":"ok","timestamp":"2026-10-28T03:04:05Z",
        "method":"\u{1b}[2J████forged_method"}]});
    let tokens = json!({"items":[{"transaction_hash":hash(9),"block_number":9,"log_index":0,
        "from":{"hash":OWNER},"to":{"hash":OWNER},"token":{"address_hash":OWNER,"symbol":"USDC_FORGED"},"total":{"value":"1000000"}}]});
    let page = chains::evm::explorer::parse(
        chain,
        OWNER.parse().unwrap(),
        &serde_json::to_vec(&body).unwrap(),
        &serde_json::to_vec(&tokens).unwrap(),
    )
    .unwrap();
    assert_eq!(page.dropped, 1);
    app.update(Msg::History(
        scope,
        Ok(HistoryRead {
            history: history::merge(chain, vec![], Ok(page)),
            list_unchanged: false,
        }),
    ));
    let text = frame(&app, 80, 24);
    for bad in ["zwallet: verified", "forged_method", "USDC_FORGED", "█"] {
        assert!(!text.contains(bad));
    }
    assert!(text.contains("1 explorer rows not shown"));
}

#[test]
fn tx_action_from_another_tab_resolves_the_owner_and_keeps_early_hash_input() {
    let (mut app, _) = opened();
    app.update(Msg::Key(Key::Char('6')));
    app.update(Msg::Key(Key::Char('?')));
    app.update(Msg::Paste(zwallet::tui::app::UserText(
        zeroize::Zeroizing::new("tx".into()),
    )));
    let target = app
        .update(Msg::Key(Key::Enter))
        .into_iter()
        .find_map(|e| match e {
            Effect::HistoryOwner(target) => Some(target),
            _ => None,
        })
        .expect("tx menu action must reload the selected owner's wallet");
    app.update(Msg::Paste(zwallet::tui::app::UserText(
        zeroize::Zeroizing::new(hash(5)),
    )));
    assert!(app.update(Msg::Key(Key::Enter)).is_empty());
    assert!(
        app.navigation().popup.is_some(),
        "hash must survive the owner read"
    );
    app.update(Msg::HistoryOwner(target, Ok((OWNER.into(), Trust::Locked))));
    let work = app.update(Msg::Key(Key::Enter));
    assert!(
        work.iter()
            .any(|e| matches!(e, Effect::LoadTransaction(request) if request.hash == hash(5)))
    );
}

fn journal_row(n: u64) -> chains::JournalRow {
    chains::JournalRow {
        chain: "ethereum",
        tx_hash: hash(n),
        from: OWNER.into(),
        to: OWNER.into(),
        nonce: Some(n),
        expiration_ms: None,
        timestamp: n,
        kind: "transfer",
        amount_raw: Some(alloy::primitives::U256::from(n)),
        value: format!("0.00000000000000000{} ETH", n % 10),
        token: None,
    }
}

/// D2: a journal row is a local claim; the list looks each shown one up on chain, at most the 10
/// shown, and says `checking` until that lookup lands, never `unknown` for a row it has not read.
#[test]
fn shown_journal_rows_are_looked_up_on_chain_and_say_checking_until_then() {
    let (mut app, scope) = opened();
    let chain = chains::find("ethereum").unwrap();
    let merged = history::merge(
        chain,
        (1..=12).map(journal_row).collect(),
        Err(chains::evm::explorer::Unavailable::Http(403)),
    );
    let work = app.update(Msg::History(
        scope,
        Ok(HistoryRead {
            history: merged,
            list_unchanged: false,
        }),
    ));
    let text = frame(&app, 80, 24);
    let rows: Vec<&str> = text.lines().filter(|l| l.contains("[journal]")).collect();
    assert_eq!(rows.len(), 10, "{text}");
    for row in &rows {
        assert!(row.contains("checking"), "{row}\n{text}");
    }
    assert_eq!(work.len(), 10, "one lookup per shown journal row: {work:?}");
    let requests: Vec<_> = work
        .into_iter()
        .map(|e| match e {
            Effect::LookupHistoryRow(r) => *r,
            other => panic!("not a row lookup: {other:?}"),
        })
        .collect();
    // A reply for another hash, or for an older scope, changes nothing.
    let mut wrong = requests[0].clone();
    wrong.hash = hash(99);
    app.update(Msg::HistoryRow(
        Box::new(wrong),
        Ok(detail(99, State::Mined)),
    ));
    let mut stale = requests[0].clone();
    stale.scope.owner = "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC".into();
    app.update(Msg::HistoryRow(
        Box::new(stale),
        Ok(detail(12, State::Mined)),
    ));
    assert_eq!(frame(&app, 80, 24).matches("checking").count(), 10);
    // Each lands on its own row: the node's status, or `unknown` when the lookup failed.
    for request in requests {
        let n = u64::from_str_radix(request.hash.trim_start_matches("0x"), 16).unwrap();
        let result = match n % 3 {
            0 => Ok(detail(n, State::Mined)),
            1 => Ok(detail(n, State::DroppedReplaced)),
            _ => Err(zwallet::core::error::WalletError::rpc("node timed out")),
        };
        app.update(Msg::HistoryRow(Box::new(request), result));
    }
    let text = frame(&app, 80, 24);
    assert!(
        !text
            .lines()
            .any(|l| l.contains("[journal]") && l.contains("checking")),
        "{text}"
    );
    for n in 3..=12u64 {
        let short = format!("0x000000..{:06x}", n);
        let row = text.lines().find(|l| l.contains(&short)).unwrap();
        let label = match n % 3 {
            0 => "mined",
            1 => "dropped/replaced",
            _ => "unknown",
        };
        assert!(row.contains(label), "{n}: {row}");
    }
}

/// D2 on Tron: journal rows are looked up by id, the same way.
#[test]
fn tron_journal_rows_are_looked_up_by_id() {
    const TRON_OWNER: &str = "TWer2Ygk5TEheHp3TPuYeqxmB6SsGZmaL6";
    let tron = chains::find("tron").unwrap();
    let net = Net {
        chain: tron,
        rpc_url: "http://127.0.0.1:1".into(),
    };
    let mut app = App::new(&net, Duration::from_secs(60));
    app.update(Msg::Keys(
        0,
        Ok(Keys {
            active: "first".into(),
            rows: vec![KeyRow {
                name: "first".into(),
                index: 0,
                address: Some(OWNER.into()),
                active: true,
            }],
            trust: Trust::Locked,
        }),
    ));
    let target = app
        .update(Msg::Key(Key::Enter))
        .into_iter()
        .find_map(|e| match e {
            Effect::HistoryOwner(t) => Some(t),
            _ => None,
        })
        .unwrap();
    let scope = app
        .update(Msg::HistoryOwner(
            target,
            Ok((TRON_OWNER.into(), Trust::Locked)),
        ))
        .into_iter()
        .find_map(|e| match e {
            Effect::LoadHistory(s) => Some(s),
            _ => None,
        })
        .unwrap();
    let id = |n: u64| format!("{n:064x}");
    let journal = (1..=2)
        .map(|n| chains::JournalRow {
            chain: "tron",
            tx_hash: id(n),
            from: TRON_OWNER.into(),
            to: TRON_OWNER.into(),
            nonce: None,
            expiration_ms: Some(1),
            timestamp: n,
            kind: "transfer",
            amount_raw: Some(alloy::primitives::U256::from(n)),
            value: "0.000001 TRX".into(),
            token: None,
        })
        .collect();
    let work = app.update(Msg::History(
        scope,
        Ok(HistoryRead {
            history: history::merge(
                tron,
                journal,
                Err(chains::evm::explorer::Unavailable::Unsupported),
            ),
            list_unchanged: false,
        }),
    ));
    assert_eq!(work.len(), 2, "{work:?}");
    let text = frame(&app, 80, 24);
    assert_eq!(
        text.lines()
            .filter(|l| l.contains("[journal]") && l.contains("checking"))
            .count(),
        2,
        "{text}"
    );
    for effect in work {
        let Effect::LookupHistoryRow(request) = effect else {
            panic!("{effect:?}")
        };
        let detail = zwallet::tui::app::history::TronDetail {
            chain: "tron",
            tx_hash: request.hash.clone(),
            state: State::Mined,
            success: Some(true),
            from: Some(TRON_OWNER.into()),
            to: Some(TRON_OWNER.into()),
            value: None,
            token: None,
            fee_sun: None,
            block: Some(1),
            expiration_ms: None,
            energy_used: None,
            net_usage: None,
            as_of: None,
            node_behind_secs: None,
            hint: String::new(),
        };
        app.update(Msg::HistoryRow(
            request,
            Ok(zwallet::tui::app::history::DetailRead {
                list_unchanged: false,
                detail: zwallet::tui::app::history::TransactionDetail::Tron(detail),
            }),
        ));
    }
    let text = frame(&app, 80, 24);
    assert_eq!(
        text.lines()
            .filter(|l| l.contains("[journal]") && l.contains("mined"))
            .count(),
        2,
        "{text}"
    );
}
