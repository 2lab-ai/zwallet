//! What the key daemon makes of the signed whitelist without any RPC: the `whitelist` field of
//! `unlock` and `status` (the list as the daemon verified it), and the `sign-message` gate
//! (`.prd/04-whitelist.md` W5b, W10). Sends are in `tests/evm_offline.rs`.

mod common;

use common::Home;
use serde_json::json;

const ALICE: &str = "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed";
const ROUTER: &str = "0x2222222222222222222222222222222222222222";

fn sign(home: &Home) -> common::Run {
    home.run(&["sign-message", "--message", "hello zwallet", "--json"])
}

#[test]
fn unlock_and_status_show_the_list_the_daemon_verified() {
    let home = Home::new();
    home.create("main", &[]);
    home.allow("alice", ALICE, &["base"]);
    home.allow_contract("router", ROUTER, &["base"], &["0x12345678"]);
    let unlocked = home.unlock("30s");
    let expected = json!({ "gen": 2, "verified": true, "entries": ["alice", "router"] });
    assert_eq!(unlocked["whitelist"], expected, "{unlocked:#}");
    let status = home.run(&["status", "--json"]).ok();
    assert_eq!(status["whitelist"], expected, "{status:#}");
    // The daemon reads the list on every request: a hand edit shows at once, unverified.
    let path = home.path().join("whitelist.json");
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, text.replace("\"alice\"", "\"alicf\"")).unwrap();
    let status = home.run(&["status", "--json"]).ok();
    let w = &status["whitelist"];
    assert_eq!(w["verified"], false, "{status:#}");
    assert_eq!(w["gen"], 2, "{status:#}");
    assert_eq!(w["entries"], json!([]), "{status:#}");
    assert!(
        w["reason"].as_str().unwrap().contains("does not verify"),
        "{status:#}"
    );
    // Locked: no daemon, no whitelist field.
    home.stop_daemons();
    let status = home.run(&["status", "--json"]).ok();
    assert!(status.get("whitelist").is_none(), "{status:#}");
}

#[test]
fn w10_sign_message_needs_allow_message_signing() {
    let home = Home::new();
    home.create("main", &[]);
    home.allow("alice", ALICE, &["*"]);
    home.unlock("30s");
    let r = sign(&home);
    let msg = r.err("WHITELIST_DENIED");
    let cmd = "zwallet whitelist allow-message-signing on";
    assert!(msg.contains(cmd), "{msg}");
    assert_eq!(r.json()["command"], cmd);
    // On (a new generation re-seals wallet.json, which ends the session): signed.
    home.edit_whitelist(|l| l.policy.allow_message_signing = true);
    assert!(home.wait_locked(std::time::Duration::from_secs(15)));
    home.unlock("30s");
    let signed = sign(&home).ok();
    assert!(signed["signature"].as_str().unwrap().starts_with("0x"));
}

#[test]
fn without_a_list_unlock_says_so_and_message_signing_is_refused() {
    let home = Home::new();
    let created = home.create("main", &["--unlock", "30s"]);
    assert_eq!(created["locked"], false);
    let status = home.run(&["status", "--json"]).ok();
    let w = &status["whitelist"];
    assert_eq!(w["verified"], false, "{status:#}");
    assert_eq!(w["gen"], serde_json::Value::Null, "{status:#}");
    assert_eq!(w["entries"], json!([]), "{status:#}");
    let reason = w["reason"].as_str().unwrap();
    assert!(reason.contains("no whitelist yet"), "{reason}");
    assert!(
        reason.contains("zwallet whitelist add <nick> <addr> --chains <chain>"),
        "{reason}"
    );
    home.stop_daemons();
    let unlocked = home.unlock("30s");
    assert_eq!(unlocked["whitelist"], status["whitelist"], "{unlocked:#}");
    let r = sign(&home);
    let msg = r.err("WHITELIST_INVALID");
    assert!(msg.contains("no whitelist yet"), "{msg}");
    assert_eq!(
        r.json()["command"],
        "zwallet whitelist allow-message-signing on"
    );
}
