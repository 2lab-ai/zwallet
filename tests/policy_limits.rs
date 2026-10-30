//! Optional per-transaction limits: authenticated edits and daemon enforcement without RPC.
mod common;

use alloy::{
    hex,
    primitives::{Address, U128, U256},
};
use common::{Home, PW};
use serde_json::{Value, json};
use std::time::Duration;
use zwallet::{
    chains::evm::{
        registry::WHITELIST_RULES,
        tx::{self, EvmFees, UnsignedEvmTx},
    },
    core::{
        protocol::{self, Request},
        whitelist::{self, TransactionLimits, Whitelist, WhitelistKey},
    },
};

const PAYEE: &str = "0x2222222222222222222222222222222222222222";
const ROUTER: &str = "0x3333333333333333333333333333333333333333";
const USDC: &str = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48";

fn edit(home: &Home, args: &[&str]) -> Value {
    let mut cmd = vec!["whitelist"];
    cmd.extend(args);
    cmd.extend(["--password-stdin", "--json"]);
    home.run_stdin(&cmd, &format!("{PW}\n")).ok()
}

async fn sign(
    home: &Home,
    to: &str,
    value: u64,
    input: Vec<u8>,
) -> zwallet::core::error::Result<Value> {
    let tx = UnsignedEvmTx {
        chain_id: 1,
        nonce: 0,
        gas_limit: 100_000,
        fees: EvmFees::Eip1559 {
            max_fee_per_gas: U128::from(1),
            max_priority_fee_per_gas: U128::from(1),
        },
        to: to.parse().unwrap(),
        value: U256::from(value),
        input: input.into(),
    };
    protocol::send_unlocked(
        home.path(),
        &Request::SignTx {
            chain: "ethereum".into(),
            index: 0,
            tx: json!(format!("0x{}", hex::encode(tx::encoded_for_signing(&tx)))),
        },
        Duration::from_secs(5),
    )
    .await
}

#[tokio::test]
async fn signed_limits_apply_to_the_daemons_payload_and_can_be_removed() {
    let _exclusive = common::exclusive();
    let home = Home::new();
    let created = home.create("main", &[]);
    let own = created["address"].as_str().unwrap();
    home.allow("alice", PAYEE, &["ethereum"]);
    home.allow_contract("router", ROUTER, &["ethereum"], &["0x12345678"]);
    for args in [
        vec!["whitelist", "native-cap", "unknown-chain", "1", "--json"],
        vec!["whitelist", "native-cap", "ethereum", "-1", "--json"],
        vec!["whitelist", "token-cap", "ethereum", PAYEE, "1", "--json"],
        vec!["whitelist", "token-cap", "base", USDC, "1", "--json"],
        vec!["whitelist", "token-cap", "ethereum", USDC, "1.5", "--json"],
        vec!["whitelist", "calldata-cap", "ethereum", "1.5", "--json"],
    ] {
        assert_ne!(home.run(&args).code, 0);
    }
    let native = edit(&home, &["native-cap", "ethereum", "0.000000000000000005"]);
    assert_eq!(
        native["policy"]["transactionLimits"]["ethereum"]["maxNativeWei"],
        "5"
    );
    edit(&home, &["token-cap", "ethereum", USDC, "100"]);
    edit(&home, &["calldata-cap", "ethereum", "68"]);
    let verified = home
        .run_stdin(
            &[
                "whitelist",
                "list",
                "--verify",
                "--password-stdin",
                "--json",
            ],
            &format!("{PW}\n"),
        )
        .ok();
    assert_eq!(verified["verified"], true);
    assert_eq!(
        verified["policy"]["transactionLimits"]["ethereum"]["maxTokenBaseUnits"]
            [USDC.to_ascii_lowercase()],
        "100"
    );
    let shown = home.run(&["whitelist", "list"]).stdout;
    assert!(shown.contains("native=wei, token=base units"), "{shown}");
    home.unlock("60s");
    for to in [PAYEE, own, ROUTER] {
        let input = if to == ROUTER {
            vec![0x12, 0x34, 0x56, 0x78]
        } else {
            vec![]
        };
        sign(&home, to, 5, input.clone()).await.unwrap();
        let denied = sign(&home, to, 6, input).await.unwrap_err();
        assert_eq!(
            denied.code,
            zwallet::core::error::ErrorCode::WhitelistDenied
        );
        assert!(denied.message.contains("native value cap"));
    }
    let token_data = |amount| {
        tx::erc20_transfer_calldata(PAYEE.parse::<Address>().unwrap(), U256::from(amount)).to_vec()
    };
    sign(&home, USDC, 0, token_data(100u64)).await.unwrap();
    assert!(
        sign(&home, USDC, 0, token_data(101u64))
            .await
            .unwrap_err()
            .message
            .contains("token amount cap")
    );
    let mut large = vec![0x12, 0x34, 0x56, 0x78];
    large.resize(69, 0);
    assert!(
        sign(&home, ROUTER, 0, large)
            .await
            .unwrap_err()
            .message
            .contains("calldata byte cap")
    );
    edit(&home, &["native-cap", "ethereum", "0"]);
    home.unlock("60s");
    sign(&home, own, 0, vec![]).await.unwrap();
    assert!(sign(&home, own, 1, vec![]).await.is_err());
    edit(&home, &["native-cap", "ethereum", "none"]);
    home.unlock("60s");
    sign(&home, own, 6, vec![]).await.unwrap();
    // Agent-side editing cannot raise an authenticated cap.
    let path = home.path().join("whitelist.json");
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, text.replace("\"100\"", "\"101\"")).unwrap();
    assert_eq!(
        sign(&home, USDC, 0, token_data(101u64))
            .await
            .unwrap_err()
            .code,
        zwallet::core::error::ErrorCode::WhitelistInvalid
    );
}

#[test]
fn limit_encoding_roundtrips_and_rejects_duplicate_or_malformed_fields() {
    let key = WhitelistKey::from_secret(&[7; 32]);
    let mut list = Whitelist::empty(&WHITELIST_RULES);
    list.generation = 1;
    let legacy = whitelist::canonical_bytes(&list);
    assert!(
        !String::from_utf8(legacy.clone())
            .unwrap()
            .contains("transactionLimits")
    );
    list.policy.transaction_limits.insert(
        "ethereum".into(),
        TransactionLimits {
            max_native_wei: Some("5".into()),
            max_calldata_bytes: Some(68),
            max_token_base_units: [(USDC.to_ascii_lowercase(), "100".into())].into(),
        },
    );
    list.validate(&WHITELIST_RULES).unwrap();
    let signed = key.sign(&list);
    let bytes = signed.to_file_bytes().unwrap();
    let decoded = whitelist::SignedList::from_file_bytes(&bytes).unwrap();
    whitelist::verify(&decoded, &key.anchor_for(&signed)).unwrap();
    assert_eq!(list, decoded.list);
    let text = String::from_utf8(bytes).unwrap();
    for corrupt in [
        text.replace(
            "\"maxNativeWei\": \"5\"",
            "\"maxNativeWei\": \"5\", \"maxNativeWei\": \"6\"",
        ),
        text.replace(
            "\"maxNativeWei\": \"5\"",
            "\"maxNativeWei\": \"escape\\u001b[31m\"",
        ),
        text.replace("\"maxCalldataBytes\": 68", "\"maxCalldataBytes\": -1"),
        text.replace(
            "\"maxCalldataBytes\": 68",
            "\"maxCalldataBytes\": 4294967296",
        ),
    ] {
        assert!(whitelist::SignedList::from_file_bytes(corrupt.as_bytes()).is_err());
    }
    let duplicate_chain = text.replace(
        "\"transactionLimits\": {",
        "\"transactionLimits\": { \"ethereum\": { \"maxNativeWei\": \"4\" },",
    );
    assert!(whitelist::SignedList::from_file_bytes(duplicate_chain.as_bytes()).is_err());
    let duplicate_token = text.replace(
        "\"maxTokenBaseUnits\": {",
        &format!(
            "\"maxTokenBaseUnits\": {{ \"{}\": \"99\",",
            USDC.to_ascii_lowercase()
        ),
    );
    assert!(whitelist::SignedList::from_file_bytes(duplicate_token.as_bytes()).is_err());
    let limits = list.policy.transaction_limits.remove("ethereum").unwrap();
    list.policy
        .transaction_limits
        .insert("unknown-chain".into(), limits.clone());
    assert!(list.validate(&WHITELIST_RULES).is_err());
    list.policy.transaction_limits.clear();
    assert_eq!(whitelist::canonical_bytes(&list), legacy);
    list.policy
        .transaction_limits
        .insert("base".into(), limits.clone());
    assert!(
        list.validate(&WHITELIST_RULES).is_err(),
        "Ethereum USDC is not a registry token on Base"
    );
    list.policy.transaction_limits.clear();
    let mut bad = limits;
    bad.max_token_base_units.insert(PAYEE.into(), "100".into());
    list.policy
        .transaction_limits
        .insert("ethereum".into(), bad);
    assert!(list.validate(&WHITELIST_RULES).is_err());
}

#[test]
fn duplicate_window_human_edit_is_signed_verified_resettable_and_bounded() {
    let _exclusive = common::exclusive();
    let home = Home::new();
    home.create("main", &[]);
    for duration in ["0s", "2d", "invalid"] {
        let result = home.run(&["whitelist", "duplicate-window", duration, "--json"]);
        assert_ne!(result.code, 0);
        assert!(!result.stderr.contains("Wallet password"));
    }
    let change = edit(&home, &["duplicate-window", "30m"]);
    assert_eq!(change["duplicateWindowSecs"], 1800);
    assert_eq!(change["advisory"], true);
    assert_eq!(change["policy"]["duplicateWindowSecs"], 1800);
    let verified = home
        .run_stdin(
            &[
                "whitelist",
                "list",
                "--verify",
                "--password-stdin",
                "--json",
            ],
            &format!("{PW}\n"),
        )
        .ok();
    assert_eq!(verified["verified"], true);
    assert_eq!(verified["policy"]["duplicateWindowSecs"], 1800);
    let shown = home.run(&["whitelist", "list"]).stdout;
    assert!(shown.contains("duplicate window: 1800 seconds; advisory, last 1 MiB"));
    let path = home.path().join(whitelist::WHITELIST_FILE);
    let bytes = std::fs::read(&path).unwrap();
    let mut forged: Value = serde_json::from_slice(&bytes).unwrap();
    forged["policy"]["duplicateWindowSecs"] = json!(1);
    std::fs::write(&path, serde_json::to_vec(&forged).unwrap()).unwrap();
    let unverified = home.run_stdin(
        &[
            "whitelist",
            "list",
            "--verify",
            "--password-stdin",
            "--json",
        ],
        &format!("{PW}\n"),
    );
    assert_ne!(unverified.json()["verified"], true);
    std::fs::write(&path, bytes).unwrap();
    let restored = edit(&home, &["duplicate-window", "default"]);
    assert_eq!(restored["duplicateWindowSecs"], 600);
    assert!(restored["policy"].get("duplicateWindowSecs").is_none());
    edit(&home, &["duplicate-window", "1d"]);
    let reset = home.run_stdin(
        &["whitelist", "reset", "--password-stdin", "--json"],
        &format!("{PW}\n"),
    );
    reset.ok();
    assert!(
        reset
            .stderr
            .contains("advisory duplicate window returns to 600 seconds")
    );
    let list = home.run(&["whitelist", "list", "--json"]).ok();
    assert!(list["policy"].get("duplicateWindowSecs").is_none());
}
