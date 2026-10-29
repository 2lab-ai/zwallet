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
        &[
            "balance", "--chain", "tron", "--token", "usdt", "--rpc", &nile.url, "--json",
        ],
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
    assert!(
        !msg.contains("SECRETKEY99") && !msg.contains("rate limited"),
        "{msg}"
    );
    // The key in the path still reached the node: the URL is used as given.
    assert!(
        gateway.paths()[0].starts_with("/v1/SECRETKEY99/wallet/"),
        "{:?}",
        gateway.paths()
    );
    let garbage = MockTron::start(|_, _| Reply::Raw("{\"blockID\": \"\u{1b}[2J SECRET".into()));
    let msg = home
        .run(&[
            "balance",
            "--chain",
            "tron",
            "--rpc",
            &garbage.url,
            "--json",
        ])
        .err("RPC_ERROR");
    assert!(msg.contains("not JSON"), "{msg}");
    assert!(
        !msg.contains("SECRET") && common::ascii_printable(&msg),
        "{msg:?}"
    );
    // A block 0 id that is not the genesis, with an escape: shown escaped and cut.
    let evil =
        MockTron::start(|_, _| Reply::Json(tron::block0(&format!("\u{1b}[2J{}", "f".repeat(600)))));
    let msg = home
        .run(&["balance", "--chain", "tron", "--rpc", &evil.url, "--json"])
        .err("RPC_ERROR");
    assert!(common::ascii_printable(&msg), "{msg:?}");
    assert!(msg.len() < 400, "{msg}");
}

// ---- TRON-2: USDT, the owner-permission warning, the every-network row ----

/// Key 0 of the public test mnemonic, as M7 found it: someone holds its owner permission.
const TWER: &str = "TWer2Ygk5TEheHp3TPuYeqxmB6SsGZmaL6";

/// `home`'s cached Tron address replaced with `address` (read while locked: the cache).
fn cache_tron(home: &Home, address: &str) {
    home.edit_wallet(|v| v["keys"][0]["addresses"]["tron"] = json!(address));
}

#[test]
fn tr6_an_owner_permission_held_by_another_key_is_a_warning_naming_it() {
    let home = Home::new();
    home.create("main", &[]);
    cache_tron(&home, TWER);
    let node = MockTron::start(tron::node(tron::taken_over_account(), 0));
    let v = home
        .run(&["balance", "--chain", "tron", "--rpc", &node.url, "--json"])
        .ok();
    assert_eq!(
        (&v["address"], &v["raw"], &v["balance"], &v["activated"]),
        (
            &json!(TWER),
            &json!("8000010"),
            &json!("8.00001"),
            &json!(true)
        ),
        "{v}"
    );
    let w = v["warning"]
        .as_str()
        .unwrap_or_else(|| panic!("no warning: {v}"));
    assert!(w.contains(tron::TAKEOVER_KEY), "{w}");
    assert!(w.contains("do not fund it"), "{w}");
    // The same warning on the token's row and on `balance` without --chain.
    let v = home
        .run(&[
            "balance", "--chain", "tron", "--token", "usdt", "--rpc", &node.url, "--json",
        ])
        .ok();
    assert!(
        v["warning"]
            .as_str()
            .unwrap_or("")
            .contains(tron::TAKEOVER_KEY),
        "{v}"
    );
    let env = env_with_tron(&node.url);
    let every = home.run_env(&["balance", "--json"], &refs(&env)).ok();
    let row = tron_row(&every);
    assert!(
        row["warning"]
            .as_str()
            .unwrap_or("")
            .contains(tron::TAKEOVER_KEY),
        "{row}"
    );
    let human = home.run_env(&["balance"], &refs(&env));
    assert_eq!(human.code, 0, "{}", human.stderr);
    assert!(
        human
            .stdout
            .lines()
            .any(|l| l.starts_with("           warning: this Tron account")
                && l.contains(tron::TAKEOVER_KEY)),
        "{}",
        human.stdout
    );
    // An account owned by its own key alone carries none.
    let own = MockTron::start(tron::node(tron::account(TWER, 1), 0));
    let v = home
        .run(&["balance", "--chain", "tron", "--rpc", &own.url, "--json"])
        .ok();
    assert!(v.get("warning").is_none(), "{v}");
}

#[test]
fn an_owner_permission_at_threshold_2_or_with_two_keys_is_a_warning_with_the_keys_escaped() {
    let home = Home::new();
    home.create("main", &[]);
    cache_tron(&home, TWER);
    let evil_key = "T\u{1b}[2J\nverified: true";
    for permission in [
        json!({ "threshold": 2, "keys": [{ "address": TWER, "weight": 1 }] }),
        json!({ "threshold": 1, "keys": [
            { "address": TWER, "weight": 1 },
            { "address": tron::TAKEOVER_KEY, "weight": 1 },
        ] }),
        json!({ "threshold": 1, "keys": [{ "address": evil_key, "weight": 1 }] }),
        json!({ "threshold": 1, "keys": (0..400).map(|_| json!({ "address": evil_key, "weight": 1 })).collect::<Vec<_>>() }),
        json!("not an object"),
    ] {
        let mut account = tron::account(TWER, 5);
        account["owner_permission"] = permission.clone();
        let node = MockTron::start(tron::node(account, 0));
        let out = home.run(&["balance", "--chain", "tron", "--rpc", &node.url, "--json"]);
        let v = out.ok();
        let w = v["warning"]
            .as_str()
            .unwrap_or_else(|| panic!("{permission}: no warning: {v}"));
        assert!(w.contains("do not fund it"), "{w}");
        assert!(common::ascii_printable(w), "{permission}: {w:?}");
        assert!(w.len() < 800, "{permission}: {} characters", w.len());
        let human = home.run(&["balance", "--chain", "tron", "--rpc", &node.url]);
        assert!(
            human.stdout.split('\n').all(common::ascii_printable),
            "{:?}",
            human.stdout
        );
    }
}

#[test]
fn tr7_usdt_is_the_registry_row_read_by_balance_of() {
    let home = Home::new();
    home.create("main", &[]);
    let me = cached_tron(&home);
    let node = MockTron::start(tron::node(tron::account(&me, 3_000_000), 12_345_678));
    let v = home
        .run(&[
            "balance", "--chain", "tron", "--token", "usdt", "--rpc", &node.url, "--json",
        ])
        .ok();
    assert_eq!(
        (&v["symbol"], &v["decimals"], &v["balance"], &v["raw"]),
        (
            &json!("USDT"),
            &json!(6),
            &json!("12.345678"),
            &json!("12345678")
        ),
        "{v}"
    );
    assert_eq!(
        (&v["token"], &v["symbolSource"], &v["activated"]),
        (&json!(tron::USDT), &json!("registry"), &json!(true)),
        "{v}"
    );
    // One balanceOf of this address at the registry contract; symbol and decimals are the
    // registry's, never asked.
    let calls = node.calls("/wallet/triggerconstantcontract");
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert_eq!(
        calls[0],
        json!({
            "owner_address": me,
            "contract_address": tron::USDT,
            "function_selector": "balanceOf(address)",
            "parameter": tron::address_word(&me),
            "visible": true,
        })
    );
    // `USDT` in any case; anything else that is no T... address is refused before any request.
    let seen = node.seen().len();
    home.run(&[
        "balance", "--chain", "tron", "--token", "USDT", "--rpc", &node.url, "--json",
    ])
    .ok();
    for bad in [
        "usdc",
        "0xdAC17F958D2ee523a2206206994597C13D831ec7",
        "tR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t",
    ] {
        home.run(&[
            "balance", "--chain", "tron", "--token", bad, "--rpc", &node.url, "--json",
        ])
        .err("INVALID_ADDRESS");
    }
    assert_eq!(node.seen().len(), seen + 3, "{:?}", node.paths());
}

#[test]
fn a_contract_named_by_address_answers_its_own_symbol_marked_as_its_own() {
    let home = Home::new();
    home.create("main", &[]);
    let me = cached_tron(&home);
    // TWer2... as a "token": it answers symbol() = USDT, but it is not the registry contract.
    let fake = MockTron::start(tron::node(tron::account(&me, 0), 7));
    let v = home
        .run(&[
            "balance", "--chain", "tron", "--token", TWER, "--rpc", &fake.url, "--json",
        ])
        .ok();
    assert_eq!(
        (&v["token"], &v["symbolSource"], &v["symbol"], &v["raw"]),
        (
            &json!(TWER),
            &json!("contract"),
            &json!("USDT"),
            &json!("7")
        ),
        "{v}"
    );
    // A symbol with an escape: shown escaped and cut, still the contract's.
    let evil =
        MockTron::start(
            move |path, body| match (path, body["function_selector"].as_str()) {
                ("/wallet/triggerconstantcontract", Some("symbol()")) => Reply::Json(
                    tron::constant(&tron::abi_string(&format!("\u{1b}[2J{}", "U".repeat(100)))),
                ),
                _ => tron::node(tron::account(&me, 0), 7)(path, body),
            },
        );
    let v = home
        .run(&[
            "balance", "--chain", "tron", "--token", TWER, "--rpc", &evil.url, "--json",
        ])
        .ok();
    let sym = v["symbol"].as_str().unwrap();
    assert!(common::ascii_printable(sym) && sym.len() <= 32, "{sym:?}");
    assert_eq!(v["symbolSource"], "contract");
    // The registry contract named by its address: the registry's symbol.
    let real = MockTron::start(tron::node(tron::account(&cached_tron(&home), 0), 1));
    let v = home
        .run(&[
            "balance",
            "--chain",
            "tron",
            "--token",
            tron::USDT,
            "--rpc",
            &real.url,
            "--json",
        ])
        .ok();
    assert_eq!(
        (&v["symbol"], &v["symbolSource"]),
        (&json!("USDT"), &json!("registry")),
        "{v}"
    );
}

#[test]
fn a_balance_of_that_is_not_one_word_or_that_failed_is_rpc_error_escaped_and_bounded() {
    let home = Home::new();
    home.create("main", &[]);
    let me = cached_tron(&home);
    let revert = "\u{1b}".repeat(600);
    let cases: Vec<Value> = vec![
        // 33 bytes.
        tron::constant(&format!("{}00", tron::word(5))),
        // The node says the call failed, with 600 ESC as its message (hex, as java-tron writes
        // it).
        json!({
            "result": { "result": false, "code": "CONTRACT_EXE_ERROR", "message": alloy::hex::encode(&revert) },
            "constant_result": [tron::word(5)],
        }),
        // The request was accepted, but the VM reports a failed execution. Its apparent
        // return word must not be presented as a balance.
        json!({
            "result": {"result": true},
            "transaction": {"ret": [{"contractRet": "REVERT"}]},
            "constant_result": [tron::word(5)],
        }),
        // No result at all.
        json!({ "constant_result": [tron::word(5)] }),
        // Not hex.
        tron::constant("zz"),
    ];
    for reply in cases {
        let account = tron::account(&me, 2);
        let node = MockTron::start(move |path, body| match path {
            "/wallet/triggerconstantcontract" => Reply::Json(reply.clone()),
            _ => tron::node(account.clone(), 0)(path, body),
        });
        let out = home.run(&[
            "balance", "--chain", "tron", "--token", "usdt", "--rpc", &node.url, "--json",
        ]);
        let msg = out.err("RPC_ERROR");
        assert!(common::ascii_printable(&msg), "{msg:?}");
        assert!(msg.len() <= 600, "{} characters: {msg}", msg.len());
        assert!(out.json().get("raw").is_none(), "{}", out.stdout);
    }
}

/// The `tron` row of `balance` without `--chain`.
fn tron_row(v: &Value) -> Value {
    v["chains"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["chain"] == "tron")
        .unwrap_or_else(|| panic!("no tron row: {v}"))
        .clone()
}

#[test]
fn tr7_tr8_the_tron_row_has_trx_usdt_and_its_own_address_and_a_failed_usdt_read_is_its_row() {
    let home = Home::new();
    let evm = home.create("main", &[])["address"]
        .as_str()
        .unwrap()
        .to_string();
    let me = cached_tron(&home);
    let node = MockTron::start(tron::node(tron::account(&me, 1_000_000), 2_500_000));
    let env = env_with_tron(&node.url);
    let v = home.run_env(&["balance", "--json"], &refs(&env)).ok();
    // The top-level address is the EVM one; the tron row names its own.
    assert_eq!(v["address"], evm.as_str());
    let keys: Vec<&str> = v["chains"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["chain"].as_str().unwrap())
        .collect();
    assert_eq!(keys.last(), Some(&"sepolia"));
    assert!(keys.contains(&"tron"));
    let row = tron_row(&v);
    assert_eq!(
        row,
        json!({
            "chain": "tron",
            "chainId": 728126428,
            "address": me,
            "activated": true,
            "native": { "symbol": "TRX", "decimals": 6, "balance": "1", "raw": "1000000" },
            "tokens": [{
                "symbol": "USDT", "address": tron::USDT, "decimals": 6,
                "balance": "2.5", "raw": "2500000",
            }],
        })
    );
    let human = home.run_env(&["balance"], &refs(&env));
    assert_eq!(human.code, 0, "{}", human.stderr);
    assert!(
        human
            .stdout
            .contains(&format!("\ntron       1 TRX ({me})\n           2.5 USDT\n")),
        "{}",
        human.stdout
    );
    // balanceOf fails: the token row says why, TRX is still shown.
    let account = tron::account(&me, 1_000_000);
    let broken = MockTron::start(move |path, body| match path {
        "/wallet/triggerconstantcontract" => Reply::Json(json!({
            "result": { "code": "CONTRACT_VALIDATE_ERROR", "message": alloy::hex::encode("no contract") },
        })),
        _ => tron::node(account.clone(), 0)(path, body),
    });
    let env = env_with_tron(&broken.url);
    let v = home.run_env(&["balance", "--json"], &refs(&env)).ok();
    let row = tron_row(&v);
    assert_eq!(row["native"]["raw"], "1000000", "{row}");
    let usdt = &row["tokens"][0];
    assert_eq!(
        (&usdt["symbol"], &usdt["errorCode"]),
        (&json!("USDT"), &json!("RPC_ERROR")),
        "{usdt}"
    );
    assert!(
        usdt["error"].as_str().unwrap().contains("no contract"),
        "{usdt}"
    );
    assert!(usdt.get("raw").is_none(), "{usdt}");
}

#[test]
fn reflected_rpc_credentials_never_reach_errors_warnings_or_token_symbols() {
    const SECRET: &str = "SECRETKEY99";
    let home = Home::new();
    home.create("main", &[]);
    let me = cached_tron(&home);
    for field in ["block", "address", "balance", "permission", "symbol"] {
        let me = me.clone();
        let node = MockTron::start(move |path, body| {
            let path = path.split('?').next().unwrap();
            if field == "block" && path == "/wallet/getblockbynum" {
                return Reply::Json(tron::block0(SECRET));
            }
            if field == "symbol" && body["function_selector"] == "symbol()" {
                return Reply::Json(tron::constant(&tron::abi_string(SECRET)));
            }
            let mut account = tron::account(&me, 1);
            match field {
                "address" => account["address"] = json!(SECRET),
                "balance" => account["balance"] = json!(SECRET),
                "permission" => {
                    account["owner_permission"] = json!({
                        "threshold": SECRET,
                        "keys": [{"address": SECRET, "weight": SECRET}]
                    })
                }
                _ => {}
            }
            tron::node(account, 0)(path, body)
        });
        let url = format!("{}?apikey={SECRET}", node.url);
        let mut args = vec!["balance", "--chain", "tron", "--rpc", &url, "--json"];
        if field == "symbol" {
            args.extend(["--token", TWER]);
        }
        let out = home.run(&args);
        assert!(
            !out.stdout.contains(SECRET) && !out.stderr.contains(SECRET),
            "credential leaked from {field}"
        );
        let value = out.json();
        if matches!(field, "permission" | "symbol") {
            assert_eq!(out.code, 0, "{field}: {value}");
            let key = if field == "symbol" {
                "symbol"
            } else {
                "warning"
            };
            assert!(
                value[key].as_str().unwrap().contains("<redacted>"),
                "{value}"
            );
        } else {
            assert_eq!(value["errorCode"], "RPC_ERROR");
        }
    }
}
