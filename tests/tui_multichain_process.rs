//! Real PTY/native-send acceptance. All key homes and endpoints are isolated test fixtures.
mod common;
use base64::{Engine, engine::general_purpose::STANDARD};
use common::{
    Home, PW, PtyChild,
    tron::{MockTron, Reply},
};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use zwallet::chains::{self, ChainFamily};
const WAIT: Duration = Duration::from_secs(60);
fn shown(f: &common::Frame, s: &str) -> bool {
    f.rows.iter().any(|r| r.contains(s))
}
fn node(chain: &str, sent: Arc<Mutex<Vec<String>>>) -> MockTron {
    let chain = chain.to_string();
    MockTron::start(move |path, body| {
        if path.contains("/v3/transactionsByMessage") {
            return Reply::Json(json!({"transactions":[]}));
        }
        let method = body["method"].as_str().unwrap_or("");
        let result = if chain == "solana" {
            match method {
                "getGenesisHash" => json!(chains::solana::MAINNET_GENESIS),
                "getBalance" => json!({"context":{"slot":99},"value":10_000_000_000u64}),
                "getLatestBlockhash" => {
                    json!({"context":{"slot":99},"value":{"blockhash":bs58::encode([9u8;32]).into_string(),"lastValidBlockHeight":150}})
                }
                "getFeeForMessage" => json!({"context":{"slot":99},"value":5000}),
                "simulateTransaction" => json!({"context":{"slot":99},"value":{"err":null}}),
                "isBlockhashValid" => json!({"context":{"slot":100},"value":true}),
                "sendTransaction" => {
                    let wire = body["params"][0].as_str().unwrap().to_string();
                    let bytes = STANDARD.decode(&wire).unwrap();
                    let hash = bs58::encode(&bytes[1..65]).into_string();
                    sent.lock().unwrap().push(wire);
                    json!(hash)
                }
                "getTransaction" => {
                    let raw = sent.lock().unwrap().last().cloned();
                    raw.map(|raw|json!({"slot":101,"transaction":[raw,"base64"],"meta":{"fee":5000,"err":null}})).unwrap_or(Value::Null)
                }
                "getSignatureStatuses" => json!({"context":{"slot":101},"value":[null]}),
                _ => Value::Null,
            }
        } else {
            match method {
                "getMasterchainInfo" => {
                    json!({"init":{"root_hash":"F6OpKZKqvqeFp6CQmFomXNMfMj2EnaUSOXN+Mh+wVWk=","file_hash":"XplPz01CXAps5qeSWUtxcyBfdAo5zVb1N979KLSKD24=","seqno":0,"workchain":-1}})
                }
                "getAddressInformation" => json!({"balance":"10000000000","state":"uninitialized"}),
                "sendBoc" => {
                    sent.lock()
                        .unwrap()
                        .push(body["params"]["boc"].as_str().unwrap().to_string());
                    json!({"@type":"ok"})
                }
                _ => Value::Null,
            }
        };
        Reply::Json(json!({"jsonrpc":"2.0","id":1,"result":result}))
    })
}
fn command(home: &Home, chain: &str, url: &str) -> std::process::Command {
    let mut command = home.command(&["tui", "--chain", chain, "--allow-ptrace-scope-0"]);
    for c in chains::CHAINS {
        command.env(c.rpc_env_var(), "http://127.0.0.1:9");
        command.env(
            format!("ZWALLET_EXPLORER_{}", c.key.to_ascii_uppercase()),
            "http://127.0.0.1:9",
        );
    }
    command.env(chains::find(chain).unwrap().rpc_env_var(), url);
    command
}
#[test]
fn native_send_real_terminal_keeps_selected_key_and_requires_separate_confirmation() {
    let _exclusive = common::exclusive();
    let home = Home::new();
    home.create("a", &[]);
    home.create("b", &[]);
    let solana_to = bs58::encode([7u8; 32]).into_string();
    let ton_to = format!("0:{}", "11".repeat(32));
    home.allow(
        "solana-payee",
        &chains::solana::policy_address(&solana_to).unwrap(),
        &["solana"],
    );
    home.allow(
        "ton-payee",
        &chains::ton::policy_address(&ton_to).unwrap(),
        &["ton"],
    );
    home.unlock("10m");
    for (chain, to, nickname) in [
        ("solana", solana_to, "solana-payee"),
        ("ton", ton_to, "ton-payee"),
    ] {
        let from = home
            .run(&["address", "--chain", chain, "--key", "a", "--json"])
            .ok()["address"]
            .as_str()
            .unwrap()
            .to_string();
        let sent = Arc::new(Mutex::new(Vec::new()));
        let rpc = node(chain, sent.clone());
        let url = if chain == "ton" {
            format!("{}/api/v2/jsonRPC", rpc.url)
        } else {
            rpc.url.clone()
        };
        let mut tui = PtyChild::spawn_full_screen(command(&home, chain, &url), (80, 24));
        tui.wait_frame(WAIT, |f| {
            shown(f, "* b ") && shown(f, "signing (agent too)")
        });
        tui.send(b"\rV");
        tui.wait_frame(WAIT, |f| shown(f, "Wallet password:"));
        tui.send(format!("{PW}\r").as_bytes());
        tui.wait_frame(WAIT, |f| {
            shown(f, "verified by password") && shown(f, "balances") && !shown(f, "checking...")
        });
        tui.send(b"3");
        tui.wait_frame(WAIT, |f| shown(f, "Select an asset"));
        tui.send(b"\r");
        let picker = tui.wait_frame(WAIT, |f| {
            shown(f, "password-checked payee scoped") && shown(f, nickname)
        });
        assert!(shown(&picker, &to));
        tui.send(b"\r");
        tui.wait_frame(WAIT, |f| shown(f, "Amount:"));
        tui.send(b"0.001\r");
        let review = tui.wait_frame(WAIT, |f| {
            shown(f, "Whitelist allowed; balance sufficient")
                && shown(f, "Exact base units: 1000000")
        });
        assert!(shown(&review, &from) && shown(&review, &to), "{review}");
        assert!(sent.lock().unwrap().is_empty());
        if chain == "ton" {
            assert!(shown(&review, "TON fees are not capped"), "{review}");
            assert!(shown(&review, "V4R2 seqno 0; expires at Unix "), "{review}");
        }
        tui.send(b"\r");
        tui.wait_frame(WAIT, |f| shown(f, "Type payee nickname:"));
        tui.send(b"wrong\r");
        tui.wait_frame(WAIT, |f| shown(f, "type the exact payee nickname"));
        assert!(sent.lock().unwrap().is_empty());
        tui.send(b"\x7f\x7f\x7f\x7f\x7f");
        tui.send(format!("{nickname}\r").as_bytes());
        let status = tui.wait_frame(WAIT, |f| {
            shown(f, "Send recorded;") && shown(f, "No replacement:")
        });
        assert_eq!(sent.lock().unwrap().len(), 1);
        if chain == "solana" {
            tui.wait_frame(WAIT, |f| shown(f, "mined: success"));
        }
        tui.send(b"q");
        let (exit, stderr) = tui.wait_exit(WAIT);
        assert!(exit.success(), "{exit}: {stderr}");
        assert!(stderr.is_empty());
        assert_eq!(sent.lock().unwrap().len(), 1);
        assert_eq!(home.run(&["list", "--json"]).ok()["active"], "b");
        let wire = sent.lock().unwrap()[0].clone();
        let parsed = if chain == "solana" {
            chains::solana::Solana
                .inspect_signed(chains::find(chain).unwrap(), &json!({"rawTx":wire}))
                .unwrap()
        } else {
            chains::ton::Ton.inspect_signed(chains::find(chain).unwrap(),&json!({"rawTx":format!("0x{}",alloy::hex::encode(STANDARD.decode(wire).unwrap()))})).unwrap()
        };
        assert_eq!(parsed.signer, from);
        std::fs::write(
            format!("/tmp/zwallet-native-tui-{chain}-80x24.txt"),
            format!("PICKER\n{picker}\nREVIEW\n{review}\nSTATUS\n{status}\n"),
        )
        .unwrap();
    }
}
#[test]
fn tron_verified_qr_real_terminal_has_address_on_same_row_at_both_sizes() {
    let _exclusive = common::exclusive();
    let home = Home::new();
    home.create("a", &[]);
    home.unlock("10m");
    let address = home.run(&["address", "--chain", "tron", "--json"]).ok()["address"]
        .as_str()
        .unwrap()
        .to_string();
    for size in [(80, 24), (120, 40)] {
        let mut tui =
            PtyChild::spawn_full_screen(command(&home, "tron", "http://127.0.0.1:9"), size);
        tui.wait_frame(WAIT, |f| {
            shown(f, "* a ") && shown(f, "signing (agent too)")
        });
        tui.send(b"\rV");
        tui.wait_frame(WAIT, |f| shown(f, "Wallet password:"));
        tui.send(format!("{PW}\r").as_bytes());
        tui.wait_frame(WAIT, |f| {
            shown(f, "verified by password") && !shown(f, "checking...")
        });
        tui.send(b"4Q");
        let frame = tui.wait_frame(WAIT, |f| {
            shown(f, "Verified by password") && shown(f, &address)
        });
        let row = frame
            .rows
            .iter()
            .position(|r| r.contains(&address))
            .unwrap();
        assert!(
            frame.rows[row]
                .chars()
                .any(|c| matches!(c, '█' | '▀' | '▄')),
            "address not alongside QR: {frame}"
        );
        assert!(
            frame.stray.is_empty() && frame.pending.is_empty(),
            "{frame}"
        );
        std::fs::write(
            format!("/tmp/zwallet-tron-qr-pty-{}x{}.txt", size.0, size.1),
            format!("address row={row}\n{frame}\n"),
        )
        .unwrap();
        tui.send(b"q");
        let (exit, stderr) = tui.wait_exit(WAIT);
        assert!(exit.success(), "{exit}: {stderr}");
        assert!(stderr.is_empty());
        assert_ne!(tui.local_modes() & libc::ICANON, 0);
        assert_ne!(tui.local_modes() & libc::ECHO, 0);
    }
}
