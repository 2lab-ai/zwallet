//! Tron read paths through the built binary against a local mock node (`common::tron`): the
//! chain identity check, the TRX balance, and what zwallet makes of a node's answers. No network:
//! every Tron endpoint is the mock on 127.0.0.1 (`--rpc` or `ZWALLET_RPC_TRON`), and every EVM
//! one a closed port.

mod common;

use common::Home;
use common::tron::{self, MAINNET_GENESIS, MockTron, NILE_GENESIS, Reply};
use serde_json::{Value, json};
use zwallet::chains;

/// The derived Tron address `create` cached for key `name`'s wallet.json entry.
fn cached_tron(home: &Home) -> String {
    let v: Value = serde_json::from_slice(&std::fs::read(home.wallet_json()).unwrap()).unwrap();
    v["keys"][0]["addresses"]["tron"]
        .as_str()
        .unwrap_or_else(|| panic!("no tron cache in {v}"))
        .to_string()
}

/// `ZWALLET_RPC_<CHAIN>` for every chain: `tron` at `tron_url`, every other one at a closed
/// port, so nothing falls back to a public default.
fn env_with_tron(tron_url: &str) -> Vec<(String, String)> {
    chains::CHAINS
        .iter()
        .map(|c| {
            let url = if c.key == "tron" {
                tron_url.to_string()
            } else {
                "http://127.0.0.1:1".to_string()
            };
            (c.rpc_env_var(), url)
        })
        .collect()
}

fn refs(env: &[(String, String)]) -> Vec<(&str, &str)> {
    env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect()
}

#[test]
fn tr5_a_node_whose_block_0_is_nile_is_refused_before_any_read() {
    let home = Home::new();
    home.create("main", &[]);
    let nile = MockTron::start(|path, _| match path {
        "/wallet/getblockbynum" => Reply::Json(tron::block0(NILE_GENESIS)),
        _ => Reply::Json(json!({ "balance": 5 })),
    });
    for args in [
        &["balance", "--chain", "tron", "--rpc", &nile.url, "--json"][..],
        &["balance", "--chain", "tron", "--token", "usdt", "--rpc", &nile.url, "--json"],
    ] {
        let msg = home.run(args).err("RPC_ERROR");
        assert!(msg.contains("block 0"), "{msg}");
        assert!(msg.contains(NILE_GENESIS), "{msg}");
        assert!(msg.contains(MAINNET_GENESIS), "{msg}");
    }
    // Only block 0 was asked for, each time: no account, no token.
    assert_eq!(
        nile.paths(),
        ["/wallet/getblockbynum", "/wallet/getblockbynum"]
    );
    assert_eq!(nile.calls("/wallet/getblockbynum")[0], json!({ "num": 0 }));
}

#[test]
fn trx_balance_reads_the_derived_address_at_the_node() {
    let home = Home::new();
    home.create("main", &[]);
    let me = cached_tron(&home);
    assert!(me.starts_with('T') && me.len() == 34, "{me}");
    let node = MockTron::start(tron::node(tron::account(&me, 1_500_000), 0));
    // Through the environment, as without --rpc.
    let env = env_with_tron(&node.url);
    let v = home
        .run_env(&["balance", "--chain", "tron", "--json"], &refs(&env))
        .ok();
    assert_eq!(
        (&v["chain"], &v["key"], &v["address"]),
        (&json!("tron"), &json!("main"), &json!(me))
    );
    assert_eq!(
        (&v["symbol"], &v["decimals"], &v["balance"], &v["raw"]),
        (&json!("TRX"), &json!(6), &json!("1.5"), &json!("1500000"))
    );
    assert_eq!(v["activated"], true, "{v}");
    assert_eq!(v["verified"], false, "{v}");
    assert!(v.get("warning").is_none(), "{v}");
    // The genesis check first, then the account, by its T... address.
    assert_eq!(
        node.paths(),
        ["/wallet/getblockbynum", "/wallet/getaccount"]
    );
    assert_eq!(
        node.calls("/wallet/getaccount")[0],
        json!({ "address": me, "visible": true })
    );
    // Every request says what it is and nothing about who runs it.
    for s in node.seen() {
        let ua = s.user_agent.as_deref().unwrap_or("");
        assert_eq!(ua, concat!("zwallet/", env!("CARGO_PKG_VERSION")), "{s:?}");
        assert!(s.headers.iter().all(|h| !h.contains('@')), "{s:?}");
    }
    // Human: the fields.
    let human = home.run_env(&["balance", "--chain", "tron", "--no-qr"], &refs(&env));
    assert_eq!(human.code, 0, "{}", human.stderr);
    assert!(human.stdout.contains("balance: 1.5\n"), "{}", human.stdout);
    assert!(human.stdout.contains("symbol: TRX\n"), "{}", human.stdout);
}

#[test]
fn an_account_the_node_does_not_know_is_not_activated() {
    let home = Home::new();
    home.create("main", &[]);
    let node = MockTron::start(tron::node(json!({}), 0));
    let v = home
        .run(&["balance", "--chain", "tron", "--rpc", &node.url, "--json"])
        .ok();
    assert_eq!(
        (&v["balance"], &v["raw"], &v["activated"]),
        (&json!("0"), &json!("0"), &json!(false))
    );
}

#[test]
fn a_balance_that_is_no_whole_number_of_sun_below_2_pow_63_is_rpc_error() {
    let home = Home::new();
    home.create("main", &[]);
    let me = cached_tron(&home);
    for bad in [
        json!("8000010"),
        json!(-1),
        json!(9_223_372_036_854_775_808u64),
        json!(18_446_744_073_709_551_615u64),
        json!(1.5),
        json!(null),
        json!("\u{1b}[2J"),
    ] {
        let mut account = tron::account(&me, 0);
        account["balance"] = bad.clone();
        let node = MockTron::start(tron::node(account, 0));
        let out = home.run(&["balance", "--chain", "tron", "--rpc", &node.url, "--json"]);
        let msg = out.err("RPC_ERROR");
        assert!(msg.contains("balance"), "{bad}: {msg}");
        assert!(common::ascii_printable(&msg), "{bad}: {msg:?}");
        // Nothing shown as a balance.
        let v = out.json();
        assert!(v.get("raw").is_none() && v.get("balance").is_none(), "{v}");
    }
}

#[test]
fn an_answer_about_another_account_is_rpc_error() {
    let home = Home::new();
    home.create("main", &[]);
    // TWer2..., not this wallet's key.
    let other = tron::account("TWer2Ygk5TEheHp3TPuYeqxmB6SsGZmaL6", 7);
    let node = MockTron::start(tron::node(other, 0));
    let msg = home
        .run(&["balance", "--chain", "tron", "--rpc", &node.url, "--json"])
        .err("RPC_ERROR");
    assert!(msg.contains("another") || msg.contains("not "), "{msg}");
}

#[test]
fn a_gateway_error_names_the_origin_only_and_a_non_json_answer_its_position() {
    let home = Home::new();
    home.create("main", &[]);
    let gateway = MockTron::start(|_, _| Reply::Http(429, "rate limited; your key SECRETKEY99"));
    let url = format!("{}/v1/SECRETKEY99", gateway.url);
    let msg = home
        .run(&["balance", "--chain", "tron", "--rpc", &url, "--json"])
        .err("RPC_ERROR");
    assert!(msg.contains("HTTP 429 from http://127.0.0.1:"), "{msg}");
    assert!(!msg.contains("SECRETKEY99") && !msg.contains("rate limited"), "{msg}");
    // The key in the path still reached the node: the URL is used as given.
    assert!(gateway.paths()[0].starts_with("/v1/SECRETKEY99/wallet/"), "{:?}", gateway.paths());
    let garbage = MockTron::start(|_, _| Reply::Raw("{\"blockID\": \"\u{1b}[2J SECRET".into()));
    let msg = home
        .run(&["balance", "--chain", "tron", "--rpc", &garbage.url, "--json"])
        .err("RPC_ERROR");
    assert!(msg.contains("not JSON"), "{msg}");
    assert!(!msg.contains("SECRET") && common::ascii_printable(&msg), "{msg:?}");
    // A block 0 id that is not the genesis, with an escape: shown escaped and cut.
    let evil = MockTron::start(|_, _| {
        Reply::Json(tron::block0(&format!("\u{1b}[2J{}", "f".repeat(600))))
    });
    let msg = home
        .run(&["balance", "--chain", "tron", "--rpc", &evil.url, "--json"])
        .err("RPC_ERROR");
    assert!(common::ascii_printable(&msg), "{msg:?}");
    assert!(msg.len() < 400, "{msg}");
}
