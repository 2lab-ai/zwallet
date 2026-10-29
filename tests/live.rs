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
/// Key 0 of the public test mnemonic on Tron (`.prd/05-tron.md` §Derivation): a live account
/// whose owner permission another key holds since someone took it over (M7). Nothing here pins
/// its balance or its keys, which can change.
const TWER: &str = "TWer2Ygk5TEheHp3TPuYeqxmB6SsGZmaL6";

#[tokio::test(flavor = "current_thread")]
#[ignore = "live: public RPC"]
async fn every_default_rpc_serves_its_chain_id() {
    let mut failures = Vec::new();
    for chain in chains::CHAINS {
        let net = Net {
            chain,
            rpc_url: chain.default_rpc().to_string(),
        };
        // Tron's identity check is its genesis block id (`.prd/05-tron.md` §Network access).
        let checked = match chain.family {
            chains::Family::Evm(_) => evm::rpc::connect(&net).await.map(drop),
            chains::Family::Tron(_) => chains::tron::rpc::connect(&net).await.map(drop),
        };
        match checked {
            Ok(()) => println!("ok   {:<10} {}", chain.key, net.rpc_url),
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
    // self-transfer is allowed without one, and only while there is no whitelist yet).
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

#[tokio::test(flavor = "current_thread")]
#[ignore = "live: public RPC"]
async fn every_registry_token_answers_its_symbol_and_decimals() {
    // Every row of the built-in registry, on its own chain through the default RPC: `symbol()`
    // is the recorded on-chain symbol and `decimals()` the recorded decimals.
    let mut failures = Vec::new();
    for t in evm::registry::TOKENS {
        let chain = chains::CHAINS
            .iter()
            .find(|c| evm::config(c).is_ok_and(|e| e.chain_id == t.chain_id))
            .unwrap();
        let net = Net {
            chain,
            rpc_url: chain.default_rpc().to_string(),
        };
        let token: Address = t.address.parse().unwrap();
        let meta = match evm::rpc::connect(&net).await {
            Ok(p) => evm::rpc::erc20_meta(&p, &net, token).await,
            Err(e) => Err(e),
        };
        let row = format!("{:<9} {:<4} {}", chain.key, t.symbol, t.address);
        match meta {
            Ok(m) if m.symbol == t.onchain_symbol && m.decimals == t.decimals => {
                println!("ok   {row} {} {}", m.symbol, m.decimals);
            }
            Ok(m) => {
                println!("FAIL {row} {:?} {}", m.symbol, m.decimals);
                failures.push(format!(
                    "{row}: symbol {:?} decimals {}, expected {:?} {}",
                    m.symbol, m.decimals, t.onchain_symbol, t.decimals
                ));
            }
            Err(e) => {
                println!("FAIL {row} {}", e.message);
                failures.push(format!("{row}: {}", e.message));
            }
        }
    }
    // The Tron registry, through Tron's default endpoint.
    assert!(!chains::tron::registry::TOKENS.is_empty());
    let tron = chains::find("tron").unwrap();
    let net = Net {
        chain: tron,
        rpc_url: tron.default_rpc().to_string(),
    };
    for t in chains::tron::registry::TOKENS {
        let meta = match chains::tron::rpc::connect(&net).await {
            Ok(c) => chains::tron::rpc::trc20_meta(&c, t.address, TWER).await,
            Err(e) => Err(e),
        };
        let row = format!("{:<9} {:<4} {}", tron.key, t.symbol, t.address);
        match meta {
            Ok(m)
                if m.symbol == t.onchain_symbol && m.decimals == t.decimals && m.from_registry =>
            {
                println!("ok   {row} {} {}", m.symbol, m.decimals);
            }
            Ok(m) => failures.push(format!(
                "{row}: symbol {:?} decimals {} (registry's: {}), expected {:?} {}",
                m.symbol, m.decimals, m.from_registry, t.onchain_symbol, t.decimals
            )),
            Err(e) => failures.push(format!("{row}: {}", e.message)),
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// One POST to Tron's HTTP API at the default endpoint, read by the test itself (no zwallet
/// code), with a User-Agent that names nobody: the reply as JSON.
fn tron_post(call: &str, body: Value) -> Value {
    use alloy::transports::http::reqwest;
    let url = format!(
        "{}/wallet/{call}",
        chains::find("tron").unwrap().default_rpc()
    );
    let run = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    run.block_on(async {
        let reply = reqwest::Client::builder()
            .user_agent("zwallet-live-test")
            .build()
            .unwrap()
            .post(url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.to_string())
            .send()
            .await
            .unwrap();
        assert!(reply.status().is_success(), "{}", reply.status());
        serde_json::from_slice(&reply.bytes().await.unwrap()).unwrap()
    })
}

/// A fresh wallet whose cached Tron address is [`TWER`] (read while locked: the cache).
fn wallet_caching_twer() -> Home {
    let home = Home::new();
    home.create("live", &[]);
    home.edit_wallet(|v| v["keys"][0]["addresses"]["tron"] = serde_json::json!(TWER));
    home
}

#[test]
#[ignore = "live: public RPC"]
fn tr6_tron_balance_is_the_nodes_and_warns_exactly_when_another_key_owns_it() {
    let home = wallet_caching_twer();
    let account = tron_post(
        "getaccount",
        serde_json::json!({ "address": TWER, "visible": true }),
    );
    let v = home.run(&["balance", "--chain", "tron", "--json"]).ok();
    println!("{v:#}");
    assert_eq!(v["address"], TWER);
    assert_eq!(
        v["raw"],
        account["balance"].as_u64().unwrap_or(0).to_string(),
        "{account}"
    );
    // This address alone at threshold 1 (or no owner permission: the default one) is the only
    // owner that gives no warning.
    let p = &account["owner_permission"];
    let alone = p.is_null()
        || (p["threshold"] == 1
            && p["keys"]
                .as_array()
                .is_some_and(|k| k.len() == 1 && k[0]["address"] == TWER && k[0]["weight"] == 1));
    assert_eq!(v.get("warning").is_some(), !alone, "{v} / {p}");
}

#[test]
#[ignore = "live: public RPC"]
fn tr7_usdt_balance_is_the_contracts_with_the_registrys_symbol_and_decimals() {
    let home = wallet_caching_twer();
    let payload = bs58::decode(TWER).with_check(None).into_vec().unwrap();
    let parameter = format!("{}{}", "0".repeat(24), alloy::hex::encode(&payload[1..]));
    let reply = tron_post(
        "triggerconstantcontract",
        serde_json::json!({
            "owner_address": TWER,
            "contract_address": "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t",
            "function_selector": "balanceOf(address)",
            "parameter": parameter,
            "visible": true,
        }),
    );
    let word = reply["constant_result"][0].as_str().unwrap();
    let raw = U256::from_str_radix(word, 16).unwrap();
    let v = home
        .run(&["balance", "--chain", "tron", "--token", "usdt", "--json"])
        .ok();
    println!("{v:#}");
    assert_eq!(
        (&v["symbol"], &v["decimals"], &v["symbolSource"]),
        (
            &serde_json::json!("USDT"),
            &serde_json::json!(6),
            &serde_json::json!("registry")
        )
    );
    assert_eq!(v["raw"], raw.to_string(), "{reply}");
}

#[test]
#[ignore = "live: public RPC"]
fn balance_without_chain_reads_every_network() {
    let home = Home::new();
    home.create("live", &[]);
    // A known address in place of the fresh key's (an edit of the cache, which `balance`
    // reads): Hardhat's account #0, public test key, dust on many networks; on Tron the same
    // mnemonic's key 0, TWer2... (05-tron.md M7).
    const HARDHAT_0: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
    home.edit_wallet(|v| {
        for (chain, a) in v["keys"][0]["addresses"].as_object_mut().unwrap() {
            let known = if chain == "tron" { TWER } else { HARDHAT_0 };
            *a = serde_json::json!(known);
        }
    });
    let v = home.run(&["balance", "--json"]).ok();
    println!("{v:#}");
    assert_eq!(v["address"], HARDHAT_0);
    let rows = v["chains"].as_array().unwrap();
    let keys: Vec<&str> = rows.iter().map(|r| r["chain"].as_str().unwrap()).collect();
    // Every network, the testnet last.
    assert_eq!(
        keys,
        [
            "ethereum",
            "base",
            "arbitrum",
            "optimism",
            "polygon",
            "bsc",
            "avalanche",
            "tron",
            "sepolia"
        ]
    );
    // TR8: Tron among the mainnets, with its own address on its row.
    let tron = rows.iter().find(|r| r["chain"] == "tron").unwrap();
    assert_eq!(tron["address"], TWER, "{tron:#}");
    // Each answered through its default RPC, with every registry token of its chain read.
    for row in rows {
        assert!(row.get("error").is_none(), "{row:#}");
        let id = row["chainId"].as_u64().unwrap();
        let tokens = if row["chain"] == "tron" {
            chains::tron::registry::TOKENS.len()
        } else {
            evm::registry::TOKENS
                .iter()
                .filter(|t| t.chain_id == id)
                .count()
        };
        let rows = row["tokens"].as_array().unwrap();
        assert_eq!(rows.len(), tokens, "{row:#}");
        assert!(rows.iter().all(|t| t.get("error").is_none()), "{row:#}");
    }
    let human = home.run(&["balance"]);
    println!("{}", human.stdout);
    assert_eq!(human.code, 0, "{}", human.stderr);
    assert!(
        human.stdout.trim_end().ends_with("networks have a balance"),
        "{}",
        human.stdout
    );
}

/// Fixed historical transaction; no wallet, signature request or broadcast.
#[tokio::test]
#[ignore = "live: historical Tron read only"]
async fn tron_historical_usdt_lookup_checks_raw_hash_and_recipient() {
    let chain = chains::find("tron").unwrap();
    let net = Net {
        chain,
        rpc_url: chain.default_rpc().to_string(),
    };
    let hash = "21b776c7957ba1213b8616414912c9e81643e15e3e55b59fe143b991fa35a690";
    let v = chains::tron::lookup::lookup_tx(&net, hash, None)
        .await
        .unwrap();
    assert_eq!(v["txHash"], hash);
    assert_eq!(v["status"], "mined");
    assert_eq!(v["success"], true);
    assert_eq!(v["from"], "TPdMMFcLKgJiUSmYsViAFvf6Nsb2WV52vs");
    assert_eq!(v["symbol"], "USDT");
    assert!(chains::tron::address::is_address(v["to"].as_str().unwrap()));
    assert_ne!(v["to"], v["token"]);
    assert_eq!(v["blockNumber"], 87494658);
}
