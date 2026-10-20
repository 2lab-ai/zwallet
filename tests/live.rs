//! Live tests against public mainnet RPC. Run with `cargo test -- --ignored`.
//! Every wallet here is a fresh temp wallet with zero balance; nothing is broadcast.

mod common;

use alloy::primitives::{Address, U256, keccak256};
use alloy::providers::Provider;
use common::Home;
use serde_json::Value;
use zwallet::chains::{self, Net, evm};
use zwallet::core::amount::parse_units;
use zwallet::core::whitelist::{Entry, Whitelist};

const USDC: &str = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48";
const WETH: &str = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2";
/// Multicall3, the same address on every chain: a contract that is no registry token.
const MULTICALL3: &str = "0xca11bde05977b3631167028862be2a173976ca11";

#[tokio::test(flavor = "current_thread")]
#[ignore = "live: public RPC"]
async fn every_default_rpc_serves_its_chain_id() {
    let mut failures = Vec::new();
    for chain in chains::CHAINS {
        let net = Net {
            chain,
            rpc_url: chain.default_rpc().to_string(),
        };
        match evm::rpc::connect(&net).await {
            Ok(_) => println!("ok   {:<10} {}", chain.key, net.rpc_url),
            Err(e) => {
                println!("FAIL {:<10} {} {}", chain.key, net.rpc_url, e.message);
                failures.push(chain.key);
            }
        }
    }
    assert!(failures.is_empty(), "failing RPCs: {failures:?}");
}

/// Unlocked for an hour: long enough for slow public RPCs; the guard locks it at the end.
fn unlocked_wallet() -> (Home, String) {
    let home = Home::new();
    let address = home.create("live", &["--unlock", "1h"])["address"]
        .as_str()
        .unwrap()
        .to_string();
    (home, address)
}

/// Like [`unlocked_wallet`], with a signed whitelist made by `change` (given the key's address)
/// before the unlock.
fn unlocked_wallet_with(change: impl FnOnce(&str, &mut Whitelist)) -> (Home, String) {
    let home = Home::new();
    let address = home.create("live", &[])["address"]
        .as_str()
        .unwrap()
        .to_string();
    home.edit_whitelist(|l| change(&address, l));
    home.unlock("1h");
    (home, address)
}

/// The account's pending nonce, read here over RPC rather than from zwallet's output.
async fn pending_nonce(chain: &str, address: &str) -> u64 {
    let chain = chains::find(chain).unwrap();
    let net = Net {
        chain,
        rpc_url: chain.default_rpc().to_string(),
    };
    let p = evm::rpc::connect(&net).await.unwrap();
    let a: Address = address.parse().unwrap();
    p.get_transaction_count(a).pending().await.unwrap()
}

fn wei(decimal: &Value) -> U256 {
    parse_units(decimal.as_str().unwrap(), 18).unwrap()
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "live: public RPC"]
async fn transfer_dry_run_self_zero_value_on_ethereum() {
    let (home, me) = unlocked_wallet();
    let v = home
        .run(&[
            "transfer",
            "--chain",
            "ethereum",
            "--to",
            &me,
            "--amount",
            "0",
            "--dry-run",
            "--json",
        ])
        .ok();
    println!("{v:#}");
    assert_eq!(v["dryRun"], true);
    assert_eq!(v["from"], me.as_str());
    assert_eq!(v["recoveredFrom"], me.as_str());
    let nonce = pending_nonce("ethereum", &me).await;
    assert_eq!(v["nonce"], nonce);
    assert_eq!(v["signerDecoded"]["nonce"], nonce);
    assert_eq!(v["gasLimit"], 21_000);
    assert_eq!(v["type"], "eip1559");
    assert!(v["maxFeePerGas"].as_str().unwrap().parse::<u128>().unwrap() > 0);
    // A dry-run never prints the broadcastable signed bytes.
    assert!(v.get("rawTx").is_none());
    assert!(v["txHash"].as_str().unwrap().starts_with("0x"));
    // Zero balance cannot pay gas: reported, not fatal, in a dry-run.
    assert_eq!(v["sufficientFunds"], false);
}

#[test]
#[ignore = "live: public RPC"]
fn invoke_dry_run_through_the_daemon() {
    // A registry token takes no other call (WETH's deposit() included), so the contract here is
    // Multicall3, listed with the one selector called.
    let selector = format!(
        "0x{}",
        alloy::hex::encode(&keccak256("getBlockNumber()")[..4])
    );
    let (home, me) = unlocked_wallet_with(|_, l| {
        l.add(Entry::new_contract(
            "multicall3",
            MULTICALL3,
            &["ethereum"],
            &[&selector],
        ))
        .unwrap()
    });
    let v = home
        .run(&[
            "invoke",
            "--chain",
            "ethereum",
            "--to",
            MULTICALL3,
            "--sig",
            "getBlockNumber()",
            "--dry-run",
            "--json",
        ])
        .ok();
    println!("{v:#}");
    assert_eq!(v["recoveredFrom"], me.as_str());
    assert_eq!(v["data"], selector.as_str());
    assert_eq!(v["signerDecoded"]["action"]["kind"], "contract-call");
    assert!(v["gasLimit"].as_u64().unwrap() > 21_000);
    assert!(v.get("rawTx").is_none());
}

#[test]
#[ignore = "live: public RPC"]
fn invoke_from_an_empty_wallet_is_insufficient_funds_before_signing() {
    let (home, _) = unlocked_wallet();
    // Unlocked (so not WALLET_LOCKED); a live send from an empty wallet stops at the balance
    // pre-check, before anything is signed or broadcast.
    let msg = home
        .run(&[
            "invoke",
            "--chain",
            "ethereum",
            "--to",
            WETH,
            "--sig",
            "deposit()",
            "--value",
            "0.001",
            "--json",
        ])
        .err("INSUFFICIENT_FUNDS");
    println!("{msg}");
    assert!(msg.contains("need 0.001 ETH but have 0"), "{msg}");
}

#[test]
#[ignore = "live: public RPC"]
fn token_transfer_dry_run_from_empty_wallet() {
    // A token transfer to the key's own address needs it listed as a payee (only a native
    // self-transfer is allowed without one).
    let (home, me) =
        unlocked_wallet_with(|me, l| l.add(Entry::new_address("me", me, &["ethereum"])).unwrap());
    let v = home
        .run(&[
            "transfer",
            "--chain",
            "ethereum",
            "--token",
            USDC,
            "--to",
            &me,
            "--amount",
            "1.5",
            "--dry-run",
            "--json",
        ])
        .ok();
    println!("{v:#}");
    assert_eq!(v["recoveredFrom"], me.as_str());
    assert_eq!(v["symbol"], "USDC");
    assert_eq!(v["amountRaw"], "1500000");
    // The daemon's own ABI decoding of the calldata it signed.
    let action = &v["signerDecoded"]["action"];
    assert_eq!(action["kind"], "transfer-call");
    assert_eq!(action["token"], USDC);
    assert_eq!(action["to"], me.as_str());
    assert_eq!(action["amount"], "1500000");
    assert_eq!(action["assetType"], "unverified");
    assert_eq!(v["sufficientFunds"], false);
    assert!(!v["shortfall"].as_array().unwrap().is_empty());
    home.run(&[
        "transfer",
        "--chain",
        "ethereum",
        "--token",
        USDC,
        "--to",
        &me,
        "--amount",
        "1.1234567",
        "--dry-run",
        "--json",
    ])
    .err("VALIDATION_ERROR");
}

#[test]
#[ignore = "live: public RPC"]
fn call_and_balance_while_locked() {
    let home = Home::new();
    let me = home.create("live", &[])["address"]
        .as_str()
        .unwrap()
        .to_string();
    let d = home
        .run(&[
            "call",
            "--chain",
            "ethereum",
            "--to",
            USDC,
            "--sig",
            "decimals()(uint8)",
            "--json",
        ])
        .ok();
    println!("{d:#}");
    assert_eq!(d["result"], serde_json::json!(["6"]));
    let b = home
        .run(&[
            "call",
            "--chain",
            "ethereum",
            "--to",
            USDC,
            "--sig",
            "balanceOf(address)(uint256)",
            &me,
            "--json",
        ])
        .ok();
    assert_eq!(b["result"], serde_json::json!(["0"]));
    let raw = home
        .run(&[
            "call",
            "--chain",
            "ethereum",
            "--to",
            USDC,
            "--sig",
            "decimals()",
            "--json",
        ])
        .ok();
    assert!(raw["result"].is_null());
    assert!(raw["raw"].as_str().unwrap().ends_with("06"));

    let native = home.run(&["balance", "--chain", "ethereum", "--json"]).ok();
    assert_eq!(native["balance"], "0");
    assert_eq!(native["symbol"], "ETH");
    let token = home
        .run(&["balance", "--chain", "ethereum", "--token", USDC, "--json"])
        .ok();
    println!("{token:#}");
    assert_eq!(token["symbol"], "USDC");
    assert_eq!(token["decimals"], 6);
    assert_eq!(token["raw"], "0");
    // Not a token contract.
    home.run(&["balance", "--chain", "ethereum", "--token", &me, "--json"])
        .err("INVALID_ADDRESS");
}

#[test]
#[ignore = "live: public RPC"]
fn op_stack_dry_run_includes_l1_fee() {
    let (home, me) = unlocked_wallet();
    let v = home
        .run(&[
            "transfer",
            "--chain",
            "base",
            "--to",
            &me,
            "--amount",
            "0",
            "--dry-run",
            "--json",
        ])
        .ok();
    println!("{v:#}");
    assert_eq!(v["recoveredFrom"], me.as_str());
    assert_eq!(v["chainId"], 8453);
    assert!(v["l1Fee"].is_string(), "missing l1Fee: {v}");
    // The L1 data fee is part of what the account must hold: the native shortfall's
    // `required` is value (0) + maxGasCost (L2 gas) + l1Fee, to the wei.
    let l1 = wei(&v["l1Fee"]);
    assert!(l1 > U256::ZERO, "{v}");
    let native = v["shortfall"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["asset"] == "ETH")
        .unwrap_or_else(|| panic!("no ETH shortfall: {v}"));
    assert_eq!(
        wei(&native["required"]),
        wei(&v["maxGasCost"]) + l1,
        "{v:#}"
    );
    let eth = home
        .run(&[
            "transfer",
            "--chain",
            "ethereum",
            "--to",
            &me,
            "--amount",
            "0",
            "--dry-run",
            "--json",
        ])
        .ok();
    assert!(eth["l1Fee"].is_null());
}

#[test]
#[ignore = "live: public RPC"]
fn bsc_legacy_dry_run_is_inside_the_ceiling() {
    // BSC has no base fee: zwallet signs a legacy gasPrice, capped at 6 x 1 gwei without
    // --max-fee-gwei (eth_gasPrice is not its own reference).
    let (home, me) = unlocked_wallet();
    let v = home
        .run(&[
            "transfer",
            "--chain",
            "bsc",
            "--to",
            &me,
            "--amount",
            "0",
            "--dry-run",
            "--json",
        ])
        .ok();
    println!("{v:#}");
    assert_eq!(v["type"], "legacy");
    assert_eq!(v["chainId"], 56);
    let price: u128 = v["gasPrice"].as_str().unwrap().parse().unwrap();
    assert!(price > 0 && price <= 6_000_000_000, "{price}");
    assert_eq!(v["signerDecoded"]["action"]["kind"], "native-transfer");
}

/// A mined USDC `transfer` from a recent Ethereum block, found over RPC here: `(hash, tx)`.
async fn recent_usdc_transfer() -> (String, Value) {
    let chain = chains::find("ethereum").unwrap();
    let net = Net {
        chain,
        rpc_url: chain.default_rpc().to_string(),
    };
    let p = evm::rpc::connect(&net).await.unwrap();
    let latest = p.get_block_number().await.unwrap();
    // A few blocks back, so every node behind the endpoint has the receipts.
    for n in (latest - 30..=latest - 5).rev() {
        let block: Value = p
            .client()
            .request("eth_getBlockByNumber", (format!("{n:#x}"), true))
            .await
            .unwrap();
        let found = block["transactions"].as_array().unwrap().iter().find(|tx| {
            tx["to"]
                .as_str()
                .is_some_and(|to| to.eq_ignore_ascii_case(USDC))
                && tx["input"]
                    .as_str()
                    .is_some_and(|i| i.starts_with("0xa9059cbb"))
        });
        if let Some(tx) = found {
            return (tx["hash"].as_str().unwrap().to_string(), tx.clone());
        }
    }
    panic!(
        "no USDC transfer in blocks {} to {}",
        latest - 30,
        latest - 5
    );
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "live: public RPC"]
async fn tx_looks_up_a_mined_mainnet_transfer_without_a_wallet() {
    let (hash, tx) = recent_usdc_transfer().await;
    let home = Home::new();
    let v = home
        .run(&["tx", "--chain", "ethereum", &hash, "--json"])
        .ok();
    println!("{v:#}");
    assert_eq!(v["txHash"], hash.as_str());
    assert_eq!(v["found"], true);
    assert_eq!(v["status"], "mined");
    let from: Address = tx["from"].as_str().unwrap().parse().unwrap();
    assert_eq!(v["from"], from.to_checksum(None).as_str());
    assert_eq!(v["to"], USDC);
    let nonce =
        u64::from_str_radix(tx["nonce"].as_str().unwrap().trim_start_matches("0x"), 16).unwrap();
    assert_eq!(v["nonce"], nonce);
    let block = u64::from_str_radix(
        tx["blockNumber"].as_str().unwrap().trim_start_matches("0x"),
        16,
    )
    .unwrap();
    assert_eq!(v["blockNumber"], block);
    assert!(v["gasUsed"].as_u64().unwrap() > 21_000, "{v:#}");
    assert!(v["success"].is_boolean(), "{v:#}");
    assert_eq!(v["value"], "0");
    assert_eq!(
        v["explorerUrl"],
        format!("https://etherscan.io/tx/{hash}").as_str()
    );
    // A hash nobody sent: not found, with the hint that keeps a retry on its nonce.
    let none = format!("0x{}", "0".repeat(63) + "1");
    let v = home
        .run(&["tx", "--chain", "ethereum", &none, "--json"])
        .ok();
    println!("{v:#}");
    assert_eq!(v["status"], "not found");
    assert!(
        v["hint"].as_str().unwrap().contains("without --nonce"),
        "{v:#}"
    );
}
