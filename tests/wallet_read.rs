//! Shared read/repair APIs: local fixture wallets and loopback RPC only; no CLI or daemon.
mod common;
use common::{PW, tiny_rpc, tiny_rpc_reply};
use serde_json::json;
use zwallet::core::{error::ErrorCode, store};
use zwallet::{chains, wallet};

fn run<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(f)
}

#[test]
fn session_rpc_requires_matching_chain_identity_and_redacts_errors() {
    let chain = chains::find("base").unwrap();
    let wrong = tiny_rpc(|_| json!("0x1"));
    let error = run(wallet::validate_rpc(&wallet::net_with(chain, &wrong))).unwrap_err();
    assert_eq!(error.code, ErrorCode::RpcError);
    assert!(error.message.contains("chain"));
    let right = tiny_rpc(|_| json!("0x2105"));
    run(wallet::validate_rpc(&wallet::net_with(chain, &right))).unwrap();
    let root =
        tiny_rpc_reply(|_, _| Err(json!({"code":-32000,"message":"denied fixture-path-token"})));
    let url = format!("{root}/fixture-path-token");
    let error = run(wallet::validate_rpc(&wallet::net_with(chain, &url))).unwrap_err();
    assert!(error.message.contains("<redacted>"));
    assert!(!error.message.contains("fixture-path-token"));
}

#[test]
fn password_read_is_immutable_and_repair_changes_only_bad_public_cache() {
    let temp = tempfile::Builder::new()
        .prefix("zwt-")
        .tempdir_in("/tmp")
        .unwrap();
    let home = temp.path().join("wallet");
    let file = store::create_master(
        &home,
        PW,
        "first",
        store::MasterOptions::default(),
        &chains::derive_addresses,
    )
    .unwrap();
    let path = home.join("wallet.json");
    let before = std::fs::read(&path).unwrap();
    let checked = wallet::check_password(&home, PW, None).unwrap();
    assert_eq!(checked.addresses.len(), chains::CHAINS.len());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    for row in &checked.addresses {
        let chain = chains::find(row.chain).unwrap();
        assert_eq!(row.family, chain.family.name());
        assert_eq!(&row.address, &file.keys[0].addresses[row.chain]);
    }
    let mut changed: serde_json::Value = serde_json::from_slice(&before).unwrap();
    changed["keys"][0]["addresses"]["ethereum"] =
        json!("\u{1b}[2J".to_string() + &"\u{2588}".repeat(600));
    std::fs::write(&path, serde_json::to_vec(&changed).unwrap()).unwrap();
    let bad_bytes = std::fs::read(&path).unwrap();
    let error = wallet::check_password(&home, PW, None).unwrap_err();
    assert_eq!(error.code, ErrorCode::ValidationError);
    assert_eq!(std::fs::read(&path).unwrap(), bad_bytes);
    let repaired = wallet::repair(&home, PW).unwrap();
    assert_eq!(repaired.checked.repairs.len(), 1);
    assert_eq!(repaired.checked.addresses.len(), chains::CHAINS.len());
    let after: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(after["crypto"], changed["crypto"]);
    assert_eq!(after["active"], changed["active"]);
    assert_eq!(after["keys"][0]["index"], changed["keys"][0]["index"]);
    assert_eq!(
        after["keys"][0]["addresses"]["ethereum"],
        json!(file.keys[0].addresses["ethereum"])
    );
    wallet::check_password(&home, PW, None).unwrap();
    assert!(!home.join("d.sock").exists());
}

#[test]
fn every_network_uses_supplied_endpoints_preserves_partial_failures_and_testnet_last() {
    let temp = tempfile::Builder::new()
        .prefix("zwt-")
        .tempdir_in("/tmp")
        .unwrap();
    let home = temp.path().join("wallet");
    store::create_master(
        &home,
        PW,
        "first",
        store::MasterOptions::default(),
        &chains::derive_addresses,
    )
    .unwrap();
    let ethereum = tiny_rpc(|method| match method {
        "eth_chainId" => json!("0x1"),
        "eth_getBalance" => json!("0xde0b6b3a7640000"),
        "eth_call" => json!(format!("0x{:064x}", 0)),
        _ => json!(null),
    });
    let base = tiny_rpc(|method| match method {
        "eth_chainId" => json!("0x2105"),
        "eth_getBalance" => json!("0x1"),
        "eth_call" => json!(format!("0x{:064x}", 0)),
        _ => json!(null),
    });
    let nets = chains::CHAINS
        .iter()
        .rev()
        .map(|chain| {
            wallet::net_with(
                chain,
                match chain.key {
                    "ethereum" => &ethereum,
                    "base" => &base,
                    _ => "http://127.0.0.1:1",
                },
            )
        })
        .collect();
    let result = run(wallet::every_balance_with(&home, "first", nets)).unwrap();
    assert_eq!(result.networks.len(), chains::CHAINS.len());
    assert_eq!(result.networks.last().unwrap().chain.key, "sepolia");
    assert_eq!(
        result
            .networks
            .iter()
            .filter(|row| row.balances.is_ok())
            .count(),
        2
    );
    assert!(result.networks.iter().all(|row| row.address.is_some()));
    assert!(matches!(result.trust, wallet::Trust::Locked));
    let one = run(wallet::balances(
        &home,
        "first",
        wallet::net_with(chains::find("ethereum").unwrap(), &ethereum),
    ))
    .unwrap();
    assert_eq!(one.native.raw.to_string(), "1000000000000000000");
    assert!(!one.tokens.is_empty());
    assert!(
        one.tokens
            .iter()
            .all(|token| token.raw.as_ref().is_ok_and(|raw| raw.is_zero()))
    );
    assert!(!home.join("d.sock").exists());
}
