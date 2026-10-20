//! Helpers for driving the real binary. Every test gets its own short temp home under /tmp
//! (the daemon socket path must fit `sun_path`), an empty environment except `ZWALLET_HOME`,
//! and a guard that locks and, if needed, kills any daemon it started.

#![allow(dead_code)]

use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const PW: &str = "correct horse battery staple";

pub struct Run {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Run {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.stdout).unwrap_or_else(|e| {
            panic!(
                "stdout is not JSON ({e}): {:?} / stderr {:?}",
                self.stdout, self.stderr
            )
        })
    }

    /// Asserts success and returns the JSON body.
    pub fn ok(&self) -> Value {
        assert_eq!(
            self.code, 0,
            "expected success; stdout {:?} stderr {:?}",
            self.stdout, self.stderr
        );
        self.json()
    }

    /// Asserts a `--json` failure with `code` and returns the message.
    pub fn err(&self, code: &str) -> String {
        assert_eq!(self.code, 1, "expected failure; stdout {:?}", self.stdout);
        let v = self.json();
        assert_eq!(v["errorCode"], code, "{v}");
        v["error"].as_str().unwrap().to_string()
    }
}

pub struct Home {
    dir: tempfile::TempDir,
}

impl Home {
    pub fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("zwt-")
            .tempdir_in("/tmp")
            .expect("temp home");
        Self { dir }
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    pub fn wallet_json(&self) -> PathBuf {
        self.path().join("wallet.json")
    }

    pub fn run(&self, args: &[&str]) -> Run {
        run_in(self.path(), args, None, &[])
    }

    pub fn run_stdin(&self, args: &[&str], stdin: &str) -> Run {
        run_in(self.path(), args, Some(stdin), &[])
    }

    pub fn run_env(&self, args: &[&str], env: &[(&str, &str)]) -> Run {
        run_in(self.path(), args, None, env)
    }

    /// `create <name> --password-stdin --json` with the shared test password.
    pub fn create(&self, name: &str, extra: &[&str]) -> Value {
        let mut args = vec!["create", name, "--password-stdin", "--json"];
        args.extend_from_slice(extra);
        self.run_stdin(&args, &format!("{PW}\n")).ok()
    }

    pub fn unlock(&self, duration: &str) -> Value {
        self.run_stdin(
            &["unlock", duration, "--password-stdin", "--json"],
            &format!("{PW}\n"),
        )
        .ok()
    }

    pub fn daemon_pid(&self) -> Option<i32> {
        let v = self.run(&["status", "--json"]).json();
        v["daemonPid"].as_i64().map(|p| p as i32)
    }

    /// Waits (bounded) until no daemon answers for this home.
    pub fn wait_locked(&self, limit: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < limit {
            if self.daemon_pid().is_none() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        false
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let pid = self.daemon_pid();
        let _ = self.run(&["lock", "--json"]);
        if let Some(pid) = pid
            && pid_alive(pid)
        {
            // SAFETY: plain kill(2) on a pid this test started.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
    }
}

pub fn pid_alive(pid: i32) -> bool {
    // SAFETY: signal 0 only checks for existence/permission.
    unsafe { libc::kill(pid, 0) == 0 }
}

pub fn run_in(home: &Path, args: &[&str], stdin: Option<&str>, env: &[(&str, &str)]) -> Run {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_zwallet"));
    cmd.env_clear()
        .env("ZWALLET_HOME", home)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("spawn zwallet");
    if let Some(input) = stdin {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
    }
    let out = child.wait_with_output().expect("wait zwallet");
    Run {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}
