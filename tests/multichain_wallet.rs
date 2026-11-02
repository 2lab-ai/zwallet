//! Real CLI lifecycle in isolated homes; no live RPC or funds.
mod common;
use common::{Home, PW};
use serde_json::Value;
use zwallet::{chains, core::store};

fn snapshot(home: &Home) -> Value {
    serde_json::from_slice(&std::fs::read(home.wallet_json()).unwrap()).unwrap()
}
#[test]
fn native_addresses_share_the_existing_seed_and_key_indices_with_recovery_metadata() {
    let home = Home::new();
    home.create("first", &[]);
    let first = snapshot(&home);
    home.create("second", &[]);
    let second = snapshot(&home);
    assert_eq!(
        first["crypto"], second["crypto"],
        "adding a key must not replace sealed seed"
    );
    let (opened, file) = store::unlock(home.path(), PW, &chains::derive_addresses).unwrap();
    for (index, name) in [(0, "first"), (1, "second")] {
        let expected = chains::derive_addresses(&opened.seed, index).unwrap();
        assert_eq!(file.key(Some(name)).unwrap().addresses, expected);
        for chain in ["ethereum", "tron", "solana", "ton"] {
            let row = home
                .run(&["address", "--key", name, "--chain", chain, "--json"])
                .ok();
            assert_eq!(row["address"], expected[chain]);
            assert_eq!(row["verified"], false);
            if ["solana", "ton"].contains(&chain) {
                let path = chains::find(chain)
                    .unwrap()
                    .family
                    .keys()
                    .derivation_path(index);
                assert_eq!(row["derivationPath"], path);
                assert_eq!(row["mnemonicScheme"], "bip39-slip0010-ed25519");
                if chain == "ton" {
                    assert_eq!(row["walletVersion"], "v4r2");
                    assert_eq!(row["subwalletId"], 698983191u32);
                    assert_eq!(row["workchain"], 0);
                }
            }
        }
    }
    for chain in ["ethereum", "tron", "solana", "ton"] {
        assert_ne!(
            second["keys"][0]["addresses"][chain],
            second["keys"][1]["addresses"][chain]
        );
    }
    assert_eq!(
        first["keys"][0]["addresses"],
        second["keys"][0]["addresses"]
    );
}
#[test]
fn old_wallet_new_addresses_are_not_invented_while_locked_and_password_repair_only_adds_caches() {
    let home = Home::new();
    home.create("main", &[]);
    let original = snapshot(&home);
    home.edit_wallet(|v| {
        for chain in ["solana", "ton"] {
            v["keys"][0]["addresses"]
                .as_object_mut()
                .unwrap()
                .remove(chain);
        }
    });
    let legacy = std::fs::read(home.wallet_json()).unwrap();
    for chain in ["solana", "ton"] {
        home.run(&["address", "--chain", chain, "--json"])
            .err("WALLET_LOCKED");
        assert_eq!(std::fs::read(home.wallet_json()).unwrap(), legacy);
    }
    let rows = home.run(&["addresses", "--json"]).ok();
    for chain in ["solana", "ton"] {
        let row = rows["addresses"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["chainId"] == chain)
            .unwrap();
        assert_eq!(row["errorCode"], "WALLET_LOCKED");
        assert!(row.get("address").is_none());
    }
    let repair = home
        .run_stdin(
            &[
                "address",
                "--chain",
                "ton",
                "--verify",
                "--password-stdin",
                "--json",
            ],
            &format!("{PW}\n"),
        )
        .ok();
    assert_eq!(repair["verified"], true);
    assert_eq!(repair["verifiedBy"], "password");
    assert_eq!(repair["added"].as_array().unwrap().len(), 2);
    let after = snapshot(&home);
    assert_eq!(after["crypto"], original["crypto"]);
    assert_eq!(after["keys"], original["keys"]);
    home.unlock("10m");
    for chain in ["solana", "ton"] {
        let actual = home.run(&["address", "--chain", chain, "--json"]).ok();
        assert_eq!(actual["address"], original["keys"][0]["addresses"][chain]);
    }
}
#[test]
fn unsupported_native_operations_fail_before_wallet_or_network_access() {
    let home = Home::new();
    for chain in ["solana", "ton"] {
        home.run(&[
            "sign-message",
            "--chain",
            chain,
            "--message",
            "hello",
            "--json",
        ])
        .err("CAPABILITY_UNSUPPORTED");
        home.run(&[
            "transfer", "--chain", chain, "--to", "@payee", "--amount", "1", "--token", "usdt",
            "--json",
        ])
        .err("CAPABILITY_UNSUPPORTED");
        home.run(&[
            "transfer", "--chain", chain, "--to", "@payee", "--amount", "1", "--nonce", "2",
            "--json",
        ])
        .err("CAPABILITY_UNSUPPORTED");
        home.run(&[
            "transfer", "--chain", chain, "--to", "@payee", "--amount", "1", "--task", "a",
            "--json",
        ])
        .err("CAPABILITY_UNSUPPORTED");
        home.run(&["task", "create", "new-task", "--chain", chain, "--json"])
            .err("CAPABILITY_UNSUPPORTED");
    }
    assert!(!home.wallet_json().exists());
    assert_eq!(
        home.run(&["chains", "--json"])
            .ok()
            .as_array()
            .unwrap()
            .len(),
        chains::CHAINS.len()
    );
    let no_files = std::fs::read_dir(home.path()).unwrap().collect::<Vec<_>>();
    assert!(
        no_files.is_empty(),
        "unsupported operations must not create wallet files"
    );
}

#[test]
fn native_tx_wait_timeout_does_not_start_another_lookup_after_its_deadline() {
    use common::tron::{MockTron, Reply};
    use serde_json::json;
    use std::time::{Duration, Instant};
    let home = Home::new();
    for chain in ["solana", "ton"] {
        let node = MockTron::start(move |_, body| {
            // Delay the network identity response beyond the complete wait budget. This
            // creates one in-flight lookup, rather than an immediate connection error.
            std::thread::sleep(Duration::from_secs(3));
            let result = if chain == "solana" {
                json!(chains::solana::MAINNET_GENESIS)
            } else {
                json!({"init":{"root_hash":"F6OpKZKqvqeFp6CQmFomXNMfMj2EnaUSOXN+Mh+wVWk=","file_hash":"XplPz01CXAps5qeSWUtxcyBfdAo5zVb1N979KLSKD24=","seqno":0,"workchain":-1}})
            };
            Reply::Json(json!({"jsonrpc":"2.0","id":body["id"],"ok":true,"result":result}))
        });
        let id = if chain == "solana" {
            bs58::encode([1u8; 64]).into_string()
        } else {
            "01".repeat(32)
        };
        let started = Instant::now();
        let value = home
            .run(&[
                "tx",
                "--chain",
                chain,
                &id,
                "--rpc",
                &node.url,
                "--wait",
                "--wait-timeout",
                "1s",
                "--json",
            ])
            .ok();
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "a receipt timeout started extra network work"
        );
        assert_eq!(value["status"], "unknown");
        assert_eq!(value["timedOut"], true);
        assert_eq!(value["txHash"], id);
        assert!(
            value.get("found").is_none(),
            "a timeout is not proof of absence"
        );
        assert_eq!(node.seen().len(), 1, "no second lookup after the deadline");
        // Zero is rejected by the public duration parser, without contacting a node.
        let before = node.seen().len();
        let zero = home.run(&[
            "tx",
            "--chain",
            chain,
            &id,
            "--rpc",
            &node.url,
            "--wait",
            "--wait-timeout",
            "0s",
            "--json",
        ]);
        assert_ne!(zero.code, 0);
        assert_eq!(node.seen().len(), before);
    }
}
