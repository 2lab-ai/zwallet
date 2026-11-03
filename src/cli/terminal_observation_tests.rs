//! Real runtime/terminal/CLI error-output boundary, with only the stop result injected.
use super::*;
use crate::tui::terminal_observation_tests::{self as observe, Pty, frame};
use std::process::Command;
use std::time::Duration;
const CHILD: &str = "ZWALLET_TEST_FAILED_LOCK_OUTPUT_HOME";

#[test]
fn failed_lock_output_child() {
    let Some(home) = std::env::var_os(CHILD) else {
        return;
    };
    let home = std::path::PathBuf::from(home);
    observe::arm_lock_failure(home.join("release-lock-error"));
    let net = crate::chains::Net {
        chain: crate::chains::find("ethereum").unwrap(),
        rpc_url: "http://127.0.0.1:1".into(),
    };
    let result = run_to_end(
        cli_runtime().unwrap(),
        crate::tui::run(home, net, Duration::from_secs(60), false),
    );
    match result {
        Err(error) => {
            assert_eq!(error.message, observe::LOCK_ERROR);
            print_error(false, &error);
            std::process::exit(1);
        }
        Ok(()) => std::process::exit(0),
    }
}

#[test]
fn failed_lock_then_quit_restores_before_exact_cli_error_output() {
    let home = observe::fixture_home();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .env_clear()
        .env(CHILD, home.path())
        .env(crate::core::protocol::TEST_SCOPE_ENV, home.path())
        .args([
            "cli::terminal_observation_tests::failed_lock_output_child",
            "--exact",
            "--nocapture",
        ]);
    // One PTY carries both outputs, so this also observes restoration before the error line.
    let mut child = Pty::spawn(command, true, (80, 24));
    child.wait(|bytes| {
        let f = frame::Frame::parse(bytes, (80, 24));
        f.rows[22].contains("main") && f.rows[23].starts_with("locked | ")
    });
    child.send(b"L");
    child.wait(|bytes| {
        frame::Frame::parse(bytes, (80, 24))
            .rows
            .iter()
            .any(|r| r.contains("locking every zwallet daemon..."))
    });
    child.send(b"q");
    // Observe the real App's deferred quit before releasing the injected stop failure.
    child.wait(|bytes| {
        frame::Frame::parse(bytes, (80, 24))
            .rows
            .iter()
            .any(|r| r.contains("locking... (quits when done)"))
    });
    std::fs::write(home.path().join("release-lock-error"), b"release").unwrap();
    let (status, bytes, stderr) = child.finish();
    assert_eq!(status.code(), Some(1));
    let last = frame::Frame::last(&bytes, (80, 24)).unwrap();
    assert!(last.stray.is_empty() && last.pending.is_empty(), "{last}");
    let expected = format!(
        "\x1b[?25herror: {} [VALIDATION_ERROR]\r\n",
        observe::LOCK_ERROR
    );
    assert_eq!(frame::restored_tail(&bytes), Some(expected.as_bytes()));
    assert!(stderr.is_empty());
}
