//! Real TUI + isolated key daemon proof for T36. No RPC read or send needs a public endpoint.
//! The orchestrator serializes this test with the other real-daemon / PTY suites.
mod common;
use common::{Frame, Home, PTY_SIZE, PW, PtyChild};
use serde_json::json;
use std::time::Duration;
use zwallet::chains::{ChainFamily, evm::Evm};

const WAIT: Duration = Duration::from_secs(60);
fn contains(frame: &Frame, needle: &str) -> bool {
    frame.rows.iter().any(|line| line.contains(needle))
}
fn hex_block(frame: &Frame, label: &str) -> String {
    let start = frame
        .rows
        .iter()
        .position(|line| line.trim_matches('|').trim() == label)
        .unwrap()
        + 2;
    frame
        .rows
        .iter()
        .skip(start)
        .map(|line| line.trim_matches('|').trim())
        .take_while(|line| {
            !line.is_empty() && line.chars().all(|c| c.is_ascii_hexdigit() || c == 'x')
        })
        .collect()
}

#[test]
fn t36_real_daemon_signs_the_512_bytes_shown_by_the_tui() {
    let home = Home::new();
    let created = home.create("main", &[]);
    home.edit_whitelist(|list| list.policy.allow_message_signing = true);
    let mut command = home.command(&["tui", "--chain", "ethereum"]);
    command.env("ZWALLET_RPC_ETHEREUM", "http://127.0.0.1:1");
    let mut tui = PtyChild::spawn_full_screen(command, PTY_SIZE);
    tui.wait_frame(WAIT, |f| contains(f, "main") && contains(f, "Enter"));
    tui.send(b"\r");
    tui.wait_frame(WAIT, |f| {
        contains(f, "Password") || contains(f, "password:")
    });
    // In raw mode a literal CR is Enter; the generic type_line helper writes LF.
    tui.send(format!("{PW}\r").as_bytes());
    tui.wait_frame(WAIT, |f| {
        contains(f, "verified by password") && contains(f, "signing (agent too)")
    });
    tui.send(b"6");
    tui.wait_frame(WAIT, |f| contains(f, "sign message"));
    tui.send(b"m");
    tui.wait_frame(WAIT, |f| contains(f, "Message: printable ASCII"));
    let message = format!("{}Z", "a".repeat(511));
    tui.send(format!("\x1b[200~{message}\x1b[201~\r").as_bytes());
    let review = tui.wait_frame(WAIT, |f| {
        contains(f, "512 bytes; leading spaces 0, trailing spaces 0")
    });
    let rows: Vec<_> = review
        .rows
        .iter()
        .filter(|line| line.starts_with("|["))
        .collect();
    assert_eq!(rows.len(), 7, "{review}");
    let shown = rows
        .iter()
        .map(|line| line.strip_prefix("|[").unwrap().split(']').next().unwrap())
        .collect::<String>();
    assert_eq!(shown, message);
    tui.send(b"\r");
    tui.wait_frame(WAIT, |f| contains(f, "Type sign exactly"));
    tui.send(b"sign\r");
    let signature_frame = tui.wait_frame(WAIT, |f| {
        f.rows
            .iter()
            .any(|line| line.trim_matches('|').trim() == "signature")
            && hex_block(f, "signature").len() == 132
    });
    let signature = hex_block(&signature_frame, "signature");
    assert_eq!(signature.len(), 132, "{signature_frame}");
    tui.send(b"r");
    let digest_frame = tui.wait_frame(WAIT, |f| {
        f.rows
            .iter()
            .any(|line| line.trim_matches('|').trim() == "digest")
            && hex_block(f, "digest").len() == 66
    });
    let digest = hex_block(&digest_frame, "digest");
    assert_eq!(digest.len(), 66, "{digest_frame}");
    Evm.verify_message(
        message.as_bytes(),
        &json!({"signature":signature,"digest":digest}),
        created["address"].as_str().unwrap(),
    )
    .unwrap();
    tui.send(b"\x1bq");
    let (status, stderr) = tui.wait_exit(WAIT);
    assert!(status.success(), "{stderr}");
    assert!(stderr.is_empty());
    let frame = tui.final_frame(WAIT);
    assert!(
        frame.stray.is_empty() && frame.pending.is_empty(),
        "{frame}"
    );
    assert_ne!(tui.local_modes() & libc::ICANON, 0);
    assert_ne!(tui.local_modes() & libc::ECHO, 0);
}
