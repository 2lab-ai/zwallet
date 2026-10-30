//! Real terminal acceptance for local task creation/list/report; no signed transaction or funds.
mod common;
use common::{Home, PtyChild};
use serde_json::{Value, json};
use std::time::Duration;
use zwallet::{chains, wallet::tasks};
const WAIT: Duration = Duration::from_secs(60);
fn shown(frame: &common::Frame, text: &str) -> bool {
    frame.rows.iter().any(|row| row.contains(text))
}
fn evm(method: &str) -> Value {
    match method {
        "eth_chainId" => json!("0x1"),
        "eth_getBalance" => json!("0x1000000000000000"),
        "eth_call" => json!(format!("0x{:064x}", 0)),
        "eth_blockNumber" | "eth_getTransactionCount" => json!("0x1"),
        _ => Value::Null,
    }
}
#[test]
fn task_create_list_report_real_terminal_both_chains_and_sizes() {
    let home = Home::new();
    home.create("a", &[]);
    home.create("b", &[]); // Global active b must never silently replace selected a.
    home.unlock("10m");
    let evm_url = common::tiny_rpc(evm);
    let tron = common::tron::MockTron::start(|path, body| {
        common::tron::Reply::Json(match path {
            "/wallet/getblockbynum" => common::tron::block0(common::tron::MAINNET_GENESIS),
            "/wallet/getaccount" => {
                common::tron::account(body["address"].as_str().unwrap(), 1_000_000)
            }
            "/wallet/getaccountresource" => json!({}),
            "/wallet/triggerconstantcontract" => {
                json!({"result":{"result":true},"constant_result":[format!("{:064x}",0)]})
            }
            _ => json!({}),
        })
    });
    for chain in ["ethereum", "tron"] {
        let expected_owner = home
            .run(&["address", "--chain", chain, "--key", "a", "--json"])
            .ok()["address"]
            .as_str()
            .unwrap()
            .to_string();
        for size in [(80, 24), (120, 40)] {
            let id = format!("tui-{chain}-{}", size.0);
            let mut command = home.command(&["tui", "--chain", chain]);
            for entry in chains::CHAINS {
                command.env(entry.rpc_env_var(), "http://127.0.0.1:9");
                command.env(
                    format!("ZWALLET_EXPLORER_{}", entry.key.to_ascii_uppercase()),
                    "http://127.0.0.1:9",
                );
            }
            command.env(
                chains::find(chain).unwrap().rpc_env_var(),
                if chain == "tron" { &tron.url } else { &evm_url },
            );
            let mut tui = PtyChild::spawn_full_screen(command, size);
            tui.wait_frame(WAIT, |f| {
                shown(f, "* b ") && shown(f, "signing (agent too)")
            });
            tui.send(b"\r6a");
            tui.wait_frame(WAIT, |f| {
                shown(f, "tasks / advisory accounting") && shown(f, "n create")
            });
            tui.send(b"n");
            tui.wait_frame(WAIT, |f| shown(f, "Task ID:"));
            tui.send(format!("{id}\r").as_bytes());
            tui.wait_frame(WAIT, |f| shown(f, "Optional allocations"));
            tui.send(b"native=123\r");
            let confirmation = tui.wait_frame(WAIT, |f| {
                shown(f, "Selected key: a / chain") && shown(f, "native = 123 base units")
            });
            assert!(shown(&confirmation, chain));
            tui.send(b"\r");
            let offline = tui.wait_frame(WAIT, |f| {
                shown(f, "local records only") && shown(f, &format!("Task {id}"))
            });
            assert!(shown(&offline, "addresses redacted"), "{offline}");
            let task = tasks::info(home.path(), &id).unwrap();
            assert_eq!(task.owner, expected_owner);
            tui.send(b"r");
            let online = tui.wait_frame(WAIT, |f| {
                shown(f, "session RPC observations") && shown(f, "observed")
            });
            assert!(
                !online.rows.iter().any(|r| r.contains(&expected_owner)),
                "owner was disclosed by default: {online}"
            );
            tui.send(b"\x1b");
            tui.wait_frame(WAIT, |f| shown(f, "tasks / reports") && shown(f, "a tasks"));
            tui.send(b"a");
            tui.wait_frame(WAIT, |f| shown(f, &id) && shown(f, "n create"));
            tui.send(b"q");
            let (status, stderr) = tui.wait_exit(WAIT);
            assert!(status.success(), "{status}: {stderr}");
            assert!(stderr.is_empty(), "{stderr}");
            let final_frame = tui.final_frame(WAIT);
            assert!(
                final_frame.stray.is_empty() && final_frame.pending.is_empty(),
                "{final_frame}"
            );
            assert_ne!(tui.local_modes() & libc::ICANON, 0);
            assert_ne!(tui.local_modes() & libc::ECHO, 0);
            // Explicit, reproducible test-fixture-only terminal evidence for the parent review.
            let stem = format!("/tmp/zwallet-task-tui-{chain}-{}x{}", size.0, size.1);
            std::fs::write(format!("{stem}.ansi"), tui.output()).unwrap();
            std::fs::write(format!("{stem}.txt"),format!("CREATE\n{confirmation}\nOFFLINE\n{offline}\nONLINE\n{online}\nFINAL\n{final_frame}\n")).unwrap();
            assert_eq!(home.run(&["list", "--json"]).ok()["active"], "b");
        }
    }
}
