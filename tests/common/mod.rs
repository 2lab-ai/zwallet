//! Helpers for driving the real binary. Every test gets its own short temp home under /tmp
//! (the daemon socket path must fit `sun_path`), an empty environment except `ZWALLET_HOME` and
//! the test scope, and a guard that stops any daemon serving that home.
//!
//! `zwallet lock` stops every zwallet daemon of this user. Every zwallet the tests start carries
//! the debug-only test scope ([`zwallet_at`]), so there it stops the daemons of the test homes
//! only, never the user's own session; but that still ends the daemons of tests running
//! alongside. Every [`Home`] therefore holds a shared flock on a per-user file, and a test that
//! runs `zwallet lock` takes it exclusively first ([`exclusive`]). The lock is cross-process, so
//! concurrent `cargo test` runs on this machine take turns too.

#![allow(dead_code)]

pub mod executable;
pub mod tron;

use serde_json::Value;
use std::cell::Cell;
use std::ffi::OsStr;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use zwallet::chains::evm::registry::WHITELIST_RULES;
use zwallet::core::protocol::{TEST_END_NOTES, TEST_SCOPE_ENV};
use zwallet::core::whitelist::{self, Entry, Whitelist};

pub const PW: &str = "correct horse battery staple";

/// The start of every test home's path: each [`Home`] (and every other temp dir a test makes for
/// a home or a binary) is a `zwt-` temp dir in /tmp.
pub const TEST_HOMES: &str = "/tmp/zwt-";

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

fn serial_lock_file() -> File {
    // SAFETY: geteuid has no preconditions.
    let uid = unsafe { libc::geteuid() };
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(format!("/tmp/zwt-serial-{uid}.lock"))
        .expect("open the test serialization lock")
}

thread_local! {
    /// This thread holds the serialization lock exclusively (see [`exclusive`]).
    static EXCLUSIVE: Cell<bool> = const { Cell::new(false) };
    /// Shared holds this thread has open (one per live [`Home`]).
    static SHARED: Cell<usize> = const { Cell::new(0) };
}

/// Held by a test that runs `zwallet lock` (which stops every daemon of a test home): no other
/// test with a [`Home`] runs meanwhile, in this process or another. Take it before creating any
/// `Home`.
pub struct Exclusive {
    _file: File,
}

pub fn exclusive() -> Exclusive {
    assert_eq!(
        SHARED.with(Cell::get),
        0,
        "take common::exclusive() before creating a Home in this test"
    );
    let file = serial_lock_file();
    file.lock().expect("exclusive test lock");
    EXCLUSIVE.with(|e| e.set(true));
    Exclusive { _file: file }
}

impl Drop for Exclusive {
    fn drop(&mut self) {
        EXCLUSIVE.with(|e| e.set(false));
    }
}

/// A shared hold on the serialization lock, unless this thread holds it exclusively.
struct Shared(Option<File>);

impl Shared {
    fn take() -> Self {
        if EXCLUSIVE.with(Cell::get) {
            return Self(None);
        }
        let file = serial_lock_file();
        file.lock_shared().expect("shared test lock");
        SHARED.with(|n| n.set(n.get() + 1));
        Self(Some(file))
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        if self.0.is_some() {
            SHARED.with(|n| n.set(n.get() - 1));
        }
    }
}

pub struct Home {
    dir: tempfile::TempDir,
    /// Dropped last: after the daemons are stopped and the directory is gone.
    _serial: Shared,
}

impl Home {
    pub fn new() -> Self {
        sign_the_binary();
        let serial = Shared::take();
        let dir = tempfile::Builder::new()
            .prefix("zwt-")
            .tempdir_in("/tmp")
            .expect("temp home");
        Self {
            dir,
            _serial: serial,
        }
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    pub fn wallet_json(&self) -> PathBuf {
        self.path().join("wallet.json")
    }

    pub fn sock(&self) -> PathBuf {
        self.path().join("d.sock")
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

    /// The bare command (empty env except `ZWALLET_HOME` and the test scope, see [`zwallet_at`])
    /// for tests that wire stdio themselves.
    pub fn command(&self, args: &[&str]) -> Command {
        command_in(self.path(), args)
    }

    /// `zwallet <args>` (plus `env`) as a human runs it: stdin and the controlling terminal on a
    /// pty, stdout on a terminal ([`PtyChild::spawn_on_terminal`]). Types [`PW`] at each of
    /// `prompts` in turn, and returns what stdout's terminal showed, `\r\n` read as `\n`. Panics
    /// unless the command succeeds.
    pub fn run_on_terminal(&self, args: &[&str], prompts: &[&str], env: &[(&str, &str)]) -> String {
        let mut cmd = self.command(args);
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut pty = PtyChild::spawn_on_terminal(cmd);
        for prompt in prompts {
            pty.wait_for(prompt, Duration::from_secs(30));
            pty.type_line(PW);
        }
        let (out, shown) = pty.finish_terminal();
        assert!(
            out.status.success(),
            "{args:?}: {shown:?}; stderr {:?}",
            String::from_utf8_lossy(&out.stderr)
        );
        shown
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

    /// `status --json` where the test expects this home's session to be live: panics with the
    /// status and why the daemon ended it ([`daemon_end_notes`]) when it says `locked`.
    #[track_caller]
    pub fn assert_unlocked(&self) -> Value {
        let status = self.run(&["status", "--json"]).ok();
        assert_eq!(
            status["locked"],
            false,
            "{status}; {}",
            daemon_end_notes(self.path())
        );
        status
    }

    /// Waits (bounded) until `status` reports this home locked: no daemon process for it.
    pub fn wait_locked(&self, limit: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < limit {
            if self.run(&["status", "--json"]).json()["locked"] == true {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        false
    }

    /// Every zwallet daemon process for this home, by argv in the process table.
    pub fn daemon_pids(&self) -> Vec<i32> {
        daemon_pids(self.path())
    }

    /// Ends this home's session the way `unlock` stops old daemons (only this home's), for tests
    /// that need the wallet locked but do not test `zwallet lock` itself (which stops every
    /// daemon of a test home, see [`exclusive`]).
    pub fn stop_daemons(&self) {
        stop_daemons(self.path()).expect("stop this home's daemons");
    }

    /// Rewrites `wallet.json` through a JSON edit (metadata only unless the edit says otherwise).
    pub fn edit_wallet(&self, edit: impl FnOnce(&mut Value)) {
        let mut v: Value =
            serde_json::from_slice(&std::fs::read(self.wallet_json()).unwrap()).unwrap();
        edit(&mut v);
        replace_file(&self.wallet_json(), &serde_json::to_vec_pretty(&v).unwrap());
    }
}

/// Replaces `path` with `bytes` (mode 0600) at once, as zwallet itself writes wallet.json: a
/// temp file beside it renamed over it. A write in place (`std::fs::write`) truncates the file
/// first, and a running daemon, which reads wallet.json every second, ends its session when it
/// reads the empty or half-written file ("wallet.json changed since unlock"): issue #52.
pub fn replace_file(path: &Path, bytes: &[u8]) {
    use std::os::unix::fs::OpenOptionsExt;
    let mut name = path.file_name().unwrap().to_os_string();
    name.push(".test-replace");
    let tmp = path.with_file_name(name);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)
        .unwrap_or_else(|e| panic!("create {}: {e}", tmp.display()));
    file.write_all(bytes).unwrap();
    drop(file);
    std::fs::rename(&tmp, path).unwrap_or_else(|e| panic!("rename onto {}: {e}", path.display()));
}

/// Seeds the signed whitelist of `home` (a wallet created with [`PW`]) through the core edit
/// path, `zwallet::core::whitelist::edit` with the real chain rules: `change` edits the current
/// list (the first call seals the whitelist salt, so the key, and generation 1; each call adds one
/// generation).
/// Returns the list now in `whitelist.json`, verified against the new seal.
///
/// Every call re-seals wallet.json, and a running daemon ends its session when wallet.json's
/// ciphertext changes: seed before `unlock`, or unlock again afterwards (see
/// `tests/whitelist_seed.rs`).
pub fn seed_whitelist(home: &Path, change: impl FnOnce(&mut Whitelist)) -> Whitelist {
    try_seed_whitelist(home, |list| {
        change(list);
        Ok(())
    })
    .unwrap_or_else(|e| panic!("seeding the whitelist failed: {}", e.message))
}

/// [`seed_whitelist`] for an edit a test expects to be refused: `change` may fail, and the
/// edit's error (or the change's) comes back as is. A refused edit writes nothing.
pub fn try_seed_whitelist(
    home: &Path,
    change: impl FnOnce(&mut Whitelist) -> zwallet::core::error::Result<()>,
) -> zwallet::core::error::Result<Whitelist> {
    whitelist::edit(home, PW, &WHITELIST_RULES, change).map(|edited| edited.list)
}

impl Home {
    /// [`seed_whitelist`] on this home.
    pub fn edit_whitelist(&self, change: impl FnOnce(&mut Whitelist)) -> Whitelist {
        seed_whitelist(self.path(), change)
    }

    /// Lists `address` (any case) as payee `nickname` on `chains` (`&["*"]`: every chain).
    pub fn allow(&self, nickname: &str, address: &str, chains: &[&str]) -> Whitelist {
        self.edit_whitelist(|list| {
            list.add(Entry::new_address(nickname, address, chains))
                .unwrap_or_else(|e| panic!("{}", e.message))
        })
    }

    /// Lists contract `address` as `nickname` on `chains`, callable with `selectors`
    /// (`0x` + 8 hex digits each).
    pub fn allow_contract(
        &self,
        nickname: &str,
        address: &str,
        chains: &[&str],
        selectors: &[&str],
    ) -> Whitelist {
        self.edit_whitelist(|list| {
            list.add(Entry::new_contract(nickname, address, chains, selectors))
                .unwrap_or_else(|e| panic!("{}", e.message))
        })
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        // Only this home's daemons, by the process table: `zwallet lock` would also stop the
        // daemons of tests running alongside.
        let _ = stop_daemons(self.path());
        for pid in self.daemon_pids() {
            // Never 0 or 1: kill(0) would hit this whole process group.
            if pid > 1 && pid_alive(pid) {
                // SAFETY: plain kill(2) on a daemon serving this test's home.
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                }
            }
        }
    }
}

/// Why the daemons of `home` ended their sessions or exited, as a debug daemon of a test home
/// notes it ([`TEST_END_NOTES`]): for a failure message, one escaped line per note, at most the
/// last 4 KB.
pub fn daemon_end_notes(home: &Path) -> String {
    match std::fs::read(home.join(TEST_END_NOTES)) {
        Ok(bytes) => {
            let tail = String::from_utf8_lossy(&bytes[bytes.len().saturating_sub(4096)..])
                .lines()
                .map(|line| {
                    let inert = line.chars().map(|c| {
                        if c.is_control() {
                            c.escape_default().to_string()
                        } else {
                            c.to_string()
                        }
                    });
                    format!("\n  {}", inert.collect::<String>())
                })
                .collect::<String>();
            format!("daemon end notes ({TEST_END_NOTES}, `<pid> <unix ms> <why>`):{tail}")
        }
        Err(e) => format!("no daemon end notes ({TEST_END_NOTES}: {e})"),
    }
}

pub fn pid_alive(pid: i32) -> bool {
    // SAFETY: signal 0 only checks for existence/permission.
    unsafe { libc::kill(pid, 0) == 0 }
}

/// Waits (bounded) for `pid` to exit.
pub fn wait_dead(pid: i32, limit: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < limit {
        if !pid_alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    !pid_alive(pid)
}

/// Linux without Yama, or with `kernel.yama.ptrace_scope` 0 (e.g. a Docker kernel): `unlock`
/// refuses to start a daemon unless given `--allow-ptrace-scope-0`.
pub fn ptrace_scope_is_open() -> bool {
    cfg!(target_os = "linux")
        && std::fs::read_to_string(zwallet::core::protocol::YAMA_PTRACE_SCOPE)
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .is_none_or(|scope| scope == 0)
}

/// The zwallet binary at `exe` with an empty environment but the test scope: [`TEST_SCOPE_ENV`]
/// set to [`TEST_HOMES`], so a `lock` or a whitelist edit it runs stops only the daemons of test
/// homes, never the user's own session. Every zwallet the tests start is built here, so a
/// release-profile run (whose zwallet ignores the scope) fails here before any starts
/// ([`zwallet_at_with`]).
pub fn zwallet_at(exe: impl AsRef<OsStr>) -> Command {
    zwallet_at_with(exe, cfg!(debug_assertions))
}

/// macOS: `CARGO_BIN_EXE_zwallet` ad-hoc signed, once per test process, before any test starts
/// or copies it (from [`Home::new`] and [`zwallet_at_with`]). `unlock` refuses a zwallet build
/// without a code signature (it compares the started daemon's cdhash with its own). The linker
/// signs every arm64 build, so there this only verifies; an x86_64 build it does not sign.
pub fn sign_the_binary() {
    #[cfg(target_os = "macos")]
    {
        static SIGNED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
        SIGNED.get_or_init(|| sign_if_unsigned(Path::new(env!("CARGO_BIN_EXE_zwallet"))));
    }
}

/// macOS: `path` ad-hoc signed when `codesign --verify --strict` fails on it (the linker signs
/// arm64 output, not x86_64). Elsewhere nothing.
pub fn sign_if_unsigned(path: &Path) {
    #[cfg(target_os = "macos")]
    {
        let codesign = |args: &[&str]| {
            Command::new("/usr/bin/codesign")
                .args(args)
                .arg(path)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        };
        if !codesign(&["--verify", "--strict"]) {
            assert!(
                codesign(&["--force", "--sign", "-"]),
                "codesign could not sign {}",
                path.display()
            );
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = path;
}

/// [`zwallet_at`] for a build with or without `debug_assertions`: without, it panics before it
/// signs or builds anything. `CARGO_BIN_EXE_zwallet` is built with the same profile as these
/// tests, and only a build with `debug_assertions` reads [`TEST_SCOPE_ENV`]: under
/// `cargo test --release` every `lock` would stop the user's own session too.
pub fn zwallet_at_with(exe: impl AsRef<OsStr>, debug_assertions: bool) -> Command {
    assert!(
        debug_assertions,
        "a release zwallet ignores {TEST_SCOPE_ENV}, so a `lock` the tests run would stop the \
         user's own session: run the tests without --release"
    );
    sign_the_binary();
    let mut cmd = Command::new(exe);
    cmd.env_clear().env(TEST_SCOPE_ENV, TEST_HOMES);
    // No test reaches a public explorer: every chain's history explorer is a closed loopback
    // port unless the test names its own mock (`ZWALLET_EXPLORER_<CHAIN>`).
    for chain in zwallet::chains::CHAINS {
        cmd.env(chain.explorer_env_var(), CLOSED_EXPLORER);
    }
    cmd
}

/// A loopback port nothing listens on: the history explorer of every test process.
pub const CLOSED_EXPLORER: &str = "http://127.0.0.1:9";

/// The binary with `args`, an empty environment but `ZWALLET_HOME` and the test scope
/// ([`zwallet_at`]), and, where the host's ptrace policy makes `unlock` refuse
/// ([`ptrace_scope_is_open`]), `--allow-ptrace-scope-0` on every command that starts a daemon.
/// `r1_unlock_refuses_ptrace_scope_0_without_the_flag` builds its command without this helper.
pub fn command_in(home: &Path, args: &[&str]) -> Command {
    let mut cmd = zwallet_at(env!("CARGO_BIN_EXE_zwallet"));
    cmd.env("ZWALLET_HOME", home).args(args);
    // A caller that passes the flag itself (clap refuses it twice) keeps exactly one.
    if ptrace_scope_is_open()
        && args.iter().any(|a| *a == "unlock" || *a == "--unlock")
        && !args.contains(&"--allow-ptrace-scope-0")
    {
        cmd.arg("--allow-ptrace-scope-0");
    }
    cmd
}

pub fn run_in(home: &Path, args: &[&str], stdin: Option<&str>, env: &[(&str, &str)]) -> Run {
    let mut cmd = command_in(home, args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    run_command(cmd, stdin)
}

/// The binary with `args` and an empty environment but the test scope ([`zwallet_at`]): no
/// `ZWALLET_HOME`, no `HOME`.
pub fn command_without_home(args: &[&str]) -> Command {
    let mut cmd = zwallet_at(env!("CARGO_BIN_EXE_zwallet"));
    cmd.args(args);
    cmd
}

/// Runs `cmd` to its exit with stdout and stderr piped, and `stdin` (or /dev/null) as input.
pub fn run_command(mut cmd: Command, stdin: Option<&str>) -> Run {
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    let mut child = cmd.spawn().expect("spawn zwallet");
    if let Some(input) = stdin {
        // A command may refuse and exit before reading stdin (e.g. `backup` without a terminal).
        if let Err(e) = child.stdin.take().unwrap().write_all(input.as_bytes()) {
            assert_eq!(e.kind(), std::io::ErrorKind::BrokenPipe, "{e}");
        }
    }
    let out = child.wait_with_output().expect("wait zwallet");
    Run {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// Pids of the zwallet daemons for `home` (other than this process), found by argv
/// (`zwallet daemon --home <home>`) in the OS process table: the same lookup `unlock` and
/// `status` use, which also sees a non-dumpable daemon on Linux whose /proc fd directory is
/// unreadable. Only among the daemons of test homes: [`TEST_HOMES`] is given here, so a
/// [`TEST_SCOPE_ENV`] exported in the shell changes nothing.
pub fn daemon_pids(home: &Path) -> Vec<i32> {
    zwallet::core::protocol::home_daemon_pids_under(Path::new(TEST_HOMES), home)
        .expect("process table")
}

/// Stops the daemons of `home` (only this home's) among those of test homes, as [`daemon_pids`]
/// finds them.
pub fn stop_daemons(home: &Path) -> zwallet::core::error::Result<Vec<i32>> {
    zwallet::core::protocol::stop_daemons_under(Path::new(TEST_HOMES), home)
}

/// Pids of every zwallet daemon of a test home, whatever the home: what `zwallet lock` stops when
/// a test runs it. This process carries no test scope, so [`TEST_HOMES`] is given here.
pub fn all_daemon_pids() -> Vec<i32> {
    zwallet::core::protocol::daemon_pids_under(Path::new(TEST_HOMES)).expect("process table")
}

/// A `zwallet daemon --home <canonical home>` child started the way `unlock` starts it (fds 0
/// and 1 one end of a socketpair), with this test playing the unlocking CLI on the other end.
pub struct DaemonChild {
    pub child: Child,
    chan: Option<UnixStream>,
    lines: Option<BufReader<UnixStream>>,
}

impl DaemonChild {
    pub fn spawn(home: &Path) -> Self {
        let (ours, theirs) = UnixStream::pair().expect("socketpair");
        let theirs_out = theirs.try_clone().unwrap();
        let mut cmd = command_in(home, &[]);
        cmd.args(["daemon", "--home"])
            .arg(std::fs::canonicalize(home).unwrap())
            .stdin(Stdio::from(OwnedFd::from(theirs)))
            .stdout(Stdio::from(OwnedFd::from(theirs_out)))
            .stderr(Stdio::null());
        let child = cmd.spawn().expect("spawn the daemon");
        // Drops this process's copies of the child's end: its exit must read as EOF here.
        drop(cmd);
        ours.set_read_timeout(Some(Duration::from_secs(30)))
            .unwrap();
        let lines = BufReader::new(ours.try_clone().unwrap());
        Self {
            child,
            chan: Some(ours),
            lines: Some(lines),
        }
    }

    /// The next JSON line the daemon writes (panics on EOF or after 30 s).
    pub fn next(&mut self) -> Value {
        let mut line = String::new();
        let n = self
            .lines
            .as_mut()
            .expect("channel open")
            .read_line(&mut line)
            .expect("read from the daemon");
        assert!(n > 0, "the daemon closed its end");
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line:?}"))
    }

    pub fn send(&mut self, bytes: &[u8]) {
        self.chan
            .as_mut()
            .expect("channel open")
            .write_all(bytes)
            .expect("write to the daemon");
    }

    /// Closes every handle this process has on the channel, as a killed CLI would.
    pub fn close(&mut self) {
        self.chan = None;
        self.lines = None;
    }

    /// Waits (bounded) for the daemon to exit; `None` if it is still running.
    pub fn wait_exit(&mut self, limit: Duration) -> Option<std::process::ExitStatus> {
        let start = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return Some(status);
            }
            if start.elapsed() >= limit {
                return None;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for DaemonChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A listener squatting on a socket path, recording every byte any client sends and answering
/// every request with a plausible "unlocked" reply.
pub struct FakeDaemon {
    pub received: Arc<Mutex<Vec<Vec<u8>>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl FakeDaemon {
    pub fn bind(path: &Path) -> Self {
        Self::bind_with_reply(path, FAKE_REPLY.to_vec())
    }

    /// Answers every request with `result` (as an ok reply), e.g. the address it read from the
    /// cache in wallet.json.
    pub fn answering(path: &Path, result: Value) -> Self {
        let mut reply =
            serde_json::to_vec(&serde_json::json!({ "v": 2, "ok": true, "result": result }))
                .unwrap();
        reply.push(b'\n');
        Self::bind_with_reply(path, reply)
    }

    /// Answers every request with `reply`, a whole reply object (say an `ok: false` error).
    pub fn replying(path: &Path, reply: Value) -> Self {
        let mut bytes = serde_json::to_vec(&reply).unwrap();
        bytes.push(b'\n');
        Self::bind_with_reply(path, bytes)
    }

    /// [`FakeDaemon::answering`] with `result` for every request but an `address` request on
    /// chain `tron`, which gets the same result with `address` = `tron` and `chain` = `tron`: a
    /// squatter that answers each chain family with an address of its own shape.
    pub fn answering_per_family(path: &Path, result: Value, tron: &str) -> Self {
        let reply = |result: &Value| {
            let mut r =
                serde_json::to_vec(&serde_json::json!({ "v": 2, "ok": true, "result": result }))
                    .unwrap();
            r.push(b'\n');
            r
        };
        let mut on_tron = result.clone();
        on_tron["address"] = Value::from(tron);
        on_tron["chain"] = Value::from("tron");
        let (evm, tron) = (reply(&result), reply(&on_tron));
        Self::bind_with(path, move |request| {
            let asked: Value = serde_json::from_slice(request).unwrap_or(Value::Null);
            if asked["op"] == "address" && asked["chain"] == "tron" {
                tron.clone()
            } else {
                evm.clone()
            }
        })
    }

    fn bind_with_reply(path: &Path, reply: Vec<u8>) -> Self {
        Self::bind_with(path, move |_| reply.clone())
    }

    /// Count wire requests while forwarding to a controlled mock signer fixture.
    /// Never relocate a production daemon socket: its integrity guard must stop the session.
    pub fn proxy(path: &Path, upstream: PathBuf) -> Self {
        Self::bind_with(path, move |request| {
            let mut socket = UnixStream::connect(&upstream).expect("test daemon upstream");
            socket
                .set_read_timeout(Some(Duration::from_secs(30)))
                .unwrap();
            socket.write_all(request).unwrap();
            socket.shutdown(std::net::Shutdown::Write).unwrap();
            let mut reply = Vec::new();
            socket.read_to_end(&mut reply).unwrap();
            reply
        })
    }

    /// Answers each request with what `reply` makes of its bytes.
    pub fn bind_with(path: &Path, reply: impl Fn(&[u8]) -> Vec<u8> + Send + 'static) -> Self {
        let listener = UnixListener::bind(path).expect("bind fake daemon");
        listener.set_nonblocking(true).unwrap();
        let received = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (rx, st) = (received.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            while !st.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut s, _)) => {
                        s.set_nonblocking(false).unwrap();
                        s.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                        let mut buf = Vec::new();
                        let _ = s.read_to_end(&mut buf);
                        let answer = reply(&buf);
                        rx.lock().unwrap().push(buf);
                        let _ = s.write_all(&answer);
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(5)),
                }
            }
        });
        Self {
            received,
            stop,
            thread: Some(thread),
        }
    }

    /// Everything received so far, concatenated.
    pub fn bytes(&self) -> Vec<u8> {
        self.received.lock().unwrap().concat()
    }
}

const FAKE_REPLY: &[u8] = br#"{"v":1,"ok":true,"result":{"locked":false,"wasUnlocked":true,"pid":1,"expiresAt":"2099-01-01T00:00:00Z","active":"main","address":"0x0000000000000000000000000000000000000001"}}
"#;

impl Drop for FakeDaemon {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Whether `tool` is on the PATH, for tests that need `cc` or `codesign`.
pub fn have(tool: &str) -> bool {
    std::process::Command::new(tool)
        .arg(if tool == "codesign" {
            "--help"
        } else {
            "--version"
        })
        .output()
        .map(|o| o.status.success() || o.status.code().is_some())
        .unwrap_or(false)
}

/// A child process killed and reaped when this drops, so a failing assertion never leaves a test
/// double's `accept()` loop running (as [`DaemonChild`] does for a daemon).
pub struct KillOnDrop(pub std::process::Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Waits (bounded by `limit`) until the listener on `home`'s `d.sock` answers a request with a
/// non-empty reply. The socket file exists from `bind`, but a connect before `listen` is refused
/// (ECONNREFUSED), so existence alone is not readiness. Each attempt is one full exchange, as the
/// CLI does it (a status request, EOF, then the reply read with a timeout): a probe that hung up
/// early would leave the listener writing to a gone peer (SIGPIPE), and one that read without a
/// timeout would hang on a listener that accepts but never replies. A refused connect, an error,
/// a timeout or an empty reply is retried; after `limit` this panics saying which it last saw.
pub fn wait_until_answering(home: &Home, limit: Duration) {
    let start = Instant::now();
    let sock = home.sock();
    loop {
        let last = match std::os::unix::net::UnixStream::connect(&sock) {
            Err(e) => format!("connect: {e}"),
            Ok(mut s) => {
                let exchange = s
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .and_then(|()| s.write_all(b"{\"op\":\"status\"}\n"))
                    .and_then(|()| s.shutdown(std::net::Shutdown::Write))
                    .and_then(|()| {
                        let mut buf = Vec::new();
                        s.read_to_end(&mut buf).map(|_| buf)
                    });
                match exchange {
                    Ok(reply) if !reply.is_empty() => return,
                    Ok(_) => "an empty reply".to_string(),
                    Err(e) => format!("exchange: {e}"),
                }
            }
        };
        assert!(
            start.elapsed() < limit,
            "nothing answered on {} within {limit:?}; last attempt: {last}",
            sock.display()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A tiny attacker: binds `$1/d.sock` and answers every request with an ok reply carrying the
/// POISON address, from its own code. Built with `cc` in the test, so its cdhash and executable
/// are not zwallet's. `ATTACKER_REPLY` replaces the whole reply (one JSON line), and
/// `ATTACKER_NONDUMPABLE` makes it non-dumpable on Linux, as a genuine key daemon is, so its
/// executable cannot be read and who answered is not known (`Unknown`).
pub const ATTACKER_C: &str = r#"
#include <sys/socket.h>
#include <sys/un.h>
#include <sys/time.h>
#include <fcntl.h>
#include <unistd.h>
#include <string.h>
#include <stdio.h>
#include <stdlib.h>
#include <signal.h>
#ifdef __linux__
#include <sys/prctl.h>
#endif
/* When $ATTACKER_LOG is set, append a timestamped event (sub-second) so a flake can be read from
   the log rather than guessed. Inert (one getenv) when the variable is unset. */
static void logev(const char *ev, long n) {
  const char *p = getenv("ATTACKER_LOG");
  if (!p) return;
  FILE *f = fopen(p, "a");
  if (!f) return;
  struct timeval tv; gettimeofday(&tv, 0);
  fprintf(f, "%ld.%06d pid=%d %s %ld\n", (long)tv.tv_sec, (int)tv.tv_usec, (int)getpid(), ev, n);
  fclose(f);
}
/* One connection, in its own process: read what the client sends (bounded by SO_RCVTIMEO so a
   client that is slow to send its request or its EOF never blocks us), then answer. */
static void handle(int s, const char *reply, const char *reply_tron) {
  struct timeval to; to.tv_sec = 2; to.tv_usec = 0;
  setsockopt(s, SOL_SOCKET, SO_RCVTIMEO, &to, sizeof to);
  /* The request's first 4 KiB, to tell an address request on tron from any other. */
  char req[4096]; size_t kept = 0;
  char b[1024]; ssize_t n; long got = 0;
  while ((n = read(s, b, sizeof b)) > 0) {
    got += n;
    size_t take = (size_t)n < sizeof req - 1 - kept ? (size_t)n : sizeof req - 1 - kept;
    memcpy(req + kept, b, take); kept += take;
  }
  req[kept] = 0;
  logev(n == 0 ? "eof" : "read-stop", got);
  if (strstr(req, "\"op\":\"address\"") && strstr(req, "\"chain\":\"tron\"")) reply = reply_tron;
  size_t len = strlen(reply), off = 0;
  while (off < len) { ssize_t w = write(s, reply + off, len - off); if (w <= 0) break; off += w; }
  logev("replied", got);
  shutdown(s, SHUT_WR); close(s);
}
int main(int argc, char **argv) {
  if (argc < 3) return 2;              /* argv: daemon --home <HOME> */
  signal(SIGPIPE, SIG_IGN);            /* a client that hangs up must not kill us mid-reply */
  signal(SIGCHLD, SIG_IGN);            /* reap per-connection handlers, no zombies */
#ifdef __linux__
  if (getenv("ATTACKER_NONDUMPABLE")) prctl(PR_SET_DUMPABLE, 0);
#endif
  /* Release any descriptor inherited from the test (its captured stdout/stderr pipe, say) so
     nothing waits on this process for EOF and no handler shares one. */
  int dn = open("/dev/null", O_RDWR);
  if (dn >= 0) { dup2(dn, 0); dup2(dn, 1); dup2(dn, 2); if (dn > 2) close(dn); }
  char path[512];
  snprintf(path, sizeof path, "%s/d.sock", argv[argc - 1]);
  unlink(path);
  int l = socket(AF_UNIX, SOCK_STREAM, 0);
  struct sockaddr_un a; memset(&a, 0, sizeof a); a.sun_family = AF_UNIX;
  strncpy(a.sun_path, path, sizeof a.sun_path - 1);
  if (bind(l, (struct sockaddr *)&a, sizeof a) || listen(l, 8)) return 3;
  /* Its own pid, so `status` takes the reply as coming from a listed daemon process. The
     expiry (JSON string text) and the whitelist (a JSON value) can be set from the environment. */
  const char *exp = getenv("ATTACKER_EXPIRES");
  if (!exp) exp = "2099-01-01T00:00:00Z";
  const char *wl = getenv("ATTACKER_WHITELIST");
  if (!wl) wl = "null";
  const char *raw = getenv("ATTACKER_REPLY");
  size_t cap = (raw ? strlen(raw) : strlen(exp) + strlen(wl)) + 512;
  char *reply = malloc(cap), *reply_tron = malloc(cap);
  if (!reply || !reply_tron) return 4;
  if (raw) { snprintf(reply, cap, "%s\n", raw); snprintf(reply_tron, cap, "%s\n", raw); }
  else {
    snprintf(reply, cap,
      "{\"v\":2,\"ok\":true,\"result\":{\"locked\":false,\"pid\":%d,"
      "\"expiresAt\":\"%s\",\"whitelist\":%s,\"chain\":\"ethereum\",\"index\":0,"
      "\"active\":\"main\",\"address\":\"0x70997970C51812dc3A010C7d01b50e0d17dc79C8\"}}\n",
      (int)getpid(), exp, wl);
    snprintf(reply_tron, cap,
      "{\"v\":2,\"ok\":true,\"result\":{\"locked\":false,\"pid\":%d,"
      "\"expiresAt\":\"%s\",\"whitelist\":%s,\"chain\":\"tron\",\"index\":0,"
      "\"active\":\"main\",\"address\":\"TPjjvMwjPoDC32V2dGDYTkLH4E5LAtBZ6C\"}}\n",
      (int)getpid(), exp, wl);
  }
  logev("listening", 0);
  for (;;) {
    int s = accept(l, 0, 0);
    if (s < 0) { logev("accept-err", 0); continue; }
    logev("accepted", 0);
    /* Each connection is served in its own child, so a client that is slow on one connection
       never keeps us from accepting and answering another. */
    pid_t pid = fork();
    if (pid == 0) { close(l); handle(s, reply, reply_tron); _exit(0); }
    close(s);
  }
}
"#;

/// Builds [`ATTACKER_C`] at `bin` with `cc`, ad-hoc signed on macOS where the linker did not sign
/// it (x86_64), so its code is named by its cdhash on every Mac, as an attacker that signs its
/// binary is.
pub fn build_attacker(src_dir: &Path, bin: &Path) {
    let src = src_dir.join("attacker.c");
    std::fs::write(&src, ATTACKER_C).unwrap();
    let cc = std::process::Command::new("cc")
        .arg("-o")
        .arg(bin)
        .arg(&src)
        .output()
        .expect("run cc");
    assert!(
        cc.status.success(),
        "cc: {}",
        String::from_utf8_lossy(&cc.stderr)
    );
    sign_if_unsigned(bin);
}

/// Starts [`ATTACKER_C`] (built into `dir`) as a daemon-argv impostor for `home`, with `env`
/// set, and waits until it answers.
pub fn daemon_argv_impostor(home: &Home, dir: &Path, env: &[(&str, &str)]) -> KillOnDrop {
    assert!(
        have("cc"),
        "needs cc (rustc's linker) to build the attacker"
    );
    let bin = dir.join("attacker");
    build_attacker(dir, &bin);
    let canonical = std::fs::canonicalize(home.path()).unwrap();
    let mut cmd = std::process::Command::new(&bin);
    cmd.arg("daemon").arg("--home").arg(&canonical);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let attacker = KillOnDrop(cmd.spawn().expect("spawn attacker"));
    wait_until_answering(home, Duration::from_secs(15));
    attacker
}

/// Whether every byte of `s` is printable ASCII (0x20..=0x7E): no control, no `\n`, no non-ASCII.
/// The one rule the tests assert of any text that came from outside zwallet — a daemon reply, an
/// RPC answer, a file. `\n` is NOT exempt: a value from outside must never carry a line break,
/// and a whole multi-line output is checked line by line, with a separate assertion on the line
/// count, so an injected `\n` cannot hide.
pub fn ascii_printable(s: &str) -> bool {
    s.bytes().all(|b| (0x20..=0x7e).contains(&b))
}

/// The window [`openpty`] gives a pty, `(columns, rows)`: the acceptance frames' 120x40
/// (03-tui.md §Acceptance).
pub const PTY_SIZE: (u16, u16) = (120, 40);

/// A pseudo-terminal pair: `(master, slave)`, its window [`PTY_SIZE`].
pub fn openpty() -> (File, OwnedFd) {
    openpty_sized(PTY_SIZE)
}

/// [`openpty`] with a window of `(columns, rows)`, which `zwallet tui` reads from its controlling
/// terminal. Both ends are made close-on-exec as soon as `openpty` returns: the child a test wires
/// them to gets them as its fds 0-2, and a process another thread starts later gets neither, so it
/// cannot hold the pty open. One started in the moment between `openpty` and the `fcntl` can
/// still inherit both (macOS has no close-on-exec flag for `openpty`).
pub fn openpty_sized((columns, rows): (u16, u16)) -> (File, OwnedFd) {
    let (mut master, mut slave) = (-1, -1);
    let mut window = libc::winsize {
        ws_row: rows,
        ws_col: columns,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: openpty writes two fds into the out-params and reads the window; a null name and
    // termios are allowed. The window goes as a raw pointer: libc declares it `*mut` on macOS and
    // `*const` on Linux, where clippy rejects a `&mut` (unnecessary_mut_passed).
    let rc = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &raw mut window,
        )
    };
    assert_eq!(rc, 0, "openpty failed");
    for fd in [master, slave] {
        // SAFETY: fcntl on an fd openpty just returned.
        assert_ne!(
            unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) },
            -1
        );
    }
    // SAFETY: both fds were just returned by openpty and are owned by nobody else.
    unsafe { (File::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) }
}

/// Reads whatever the pty master has buffered right now, without blocking.
pub fn drain_pty(master: &File) -> String {
    String::from_utf8_lossy(&drain_pty_bytes(master)).into_owned()
}

/// [`drain_pty`] as bytes: a read may end inside a multi-byte character.
pub fn drain_pty_bytes(master: &File) -> Vec<u8> {
    use std::os::fd::AsRawFd;
    let fd = master.as_raw_fd();
    // SAFETY: fcntl on an fd we own.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
    }
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match (&*master).read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => out.extend_from_slice(&buf[..n]),
        }
    }
    out
}

/// `TIOCSCTTY` (make this terminal the controlling one); libc does not define it for macOS.
#[cfg(target_os = "macos")]
const TIOCSCTTY: libc::c_ulong = 0x2000_7461;
#[cfg(not(target_os = "macos"))]
const TIOCSCTTY: libc::c_ulong = libc::TIOCSCTTY as libc::c_ulong;

/// A process whose stdin and controlling terminal (`/dev/tty`) is a pty, as when a human runs it
/// at a terminal; stdout and stderr are piped (stdout on the pty too with
/// [`PtyChild::spawn_full_screen`]).
pub struct PtyChild {
    pub child: Child,
    pub master: File,
    seen: String,
    /// This process's copy of the slave. Kept for [`PtyChild::spawn`] and
    /// [`PtyChild::spawn_on_terminal`], whose output is read only from time to time (it was kept
    /// because macOS was seen to drop unread output once every slave is closed). `None` for
    /// [`PtyChild::spawn_full_screen`]: with no slave left here, the pty ends with the child and
    /// the reader ends after the last byte, also on Linux, whose master gives no end while any
    /// slave is open. Measured on 2026-10-28 (a session leader writing 25,603 bytes, then exiting,
    /// read all along): with this copy closed every byte arrived, then the end, 200 of 200 runs
    /// on macOS and on Linux; with it open, Linux gave no end within 10 s.
    _slave: Option<OwnedFd>,
    /// With [`PtyChild::spawn_on_terminal`]: the master of the terminal stdout is on (and its
    /// slave, kept open), and the bytes it showed.
    out: Option<(File, OwnedFd)>,
    shown: Vec<u8>,
    /// With [`PtyChild::spawn_full_screen`]: the thread reading the master all along.
    screen: Option<FullScreen>,
    /// The child's `ZWALLET_HOME`, if it has one: a failed wait says why its daemons ended.
    home: Option<PathBuf>,
}

/// What [`PtyChild::spawn_full_screen`] keeps: the grid frames are parsed on, every byte read from
/// the master so far (by a thread, all along, so a full pty never holds the child's writes), the
/// thread's stop flag, and the thread reading stderr.
struct FullScreen {
    /// The widest and tallest the window has been ([`PtyChild::resize`]): after a shrink nothing
    /// drawn can lie outside the grid a frame is parsed on.
    size: (u16, u16),
    read: Arc<Mutex<Vec<u8>>>,
    stop: Arc<AtomicBool>,
    reader: Option<JoinHandle<()>>,
    stderr: Option<JoinHandle<Vec<u8>>>,
}

impl PtyChild {
    pub fn spawn(cmd: Command) -> Self {
        Self::spawn_with(cmd, None)
    }

    /// [`PtyChild::spawn`] with stdout on a terminal too, as when a human reads the output;
    /// [`PtyChild::finish_terminal`] returns what it printed. That terminal is a second pty, so
    /// the output is not mixed with the password prompts on the controlling one.
    pub fn spawn_on_terminal(cmd: Command) -> Self {
        Self::spawn_with(cmd, Some(openpty()))
    }

    fn spawn_with(mut cmd: Command, out: Option<(File, OwnedFd)>) -> Self {
        let (master, slave) = openpty();
        cmd.stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(match &out {
                Some((_, out_slave)) => Stdio::from(out_slave.try_clone().unwrap()),
                None => Stdio::piped(),
            })
            .stderr(Stdio::piped());
        Self::start(cmd, master, slave, out)
    }

    /// `cmd` as a full-screen program runs at a terminal of `(columns, rows)`: stdin and stdout
    /// on one pty, its controlling terminal; stderr piped. A thread reads the master all along
    /// ([`PtyChild::frame`] parses what it read), another reads stderr.
    pub fn spawn_full_screen(mut cmd: Command, size: (u16, u16)) -> Self {
        let (master, slave) = openpty_sized(size);
        cmd.stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::piped());
        let mut pty = Self::start(cmd, master, slave, None);
        // The child has its own; the reader must see the pty end when the child does.
        pty._slave = None;
        let (read, stop) = (
            Arc::new(Mutex::new(Vec::new())),
            Arc::new(AtomicBool::new(false)),
        );
        let reader = read_all_along(pty.master.try_clone().unwrap(), read.clone(), stop.clone());
        let mut stderr = pty.child.stderr.take().unwrap();
        let stderr = std::thread::spawn(move || {
            let mut v = Vec::new();
            let _ = stderr.read_to_end(&mut v);
            v
        });
        pty.screen = Some(FullScreen {
            size,
            read,
            stop,
            reader: Some(reader),
            stderr: Some(stderr),
        });
        pty
    }

    /// Starts `cmd`, its stdio set, in a session of its own with `slave` as its controlling
    /// terminal.
    fn start(mut cmd: Command, master: File, slave: OwnedFd, out: Option<(File, OwnedFd)>) -> Self {
        let home = cmd
            .get_envs()
            .find(|(k, _)| *k == "ZWALLET_HOME")
            .and_then(|(_, v)| v)
            .map(PathBuf::from);
        // SAFETY: setsid and ioctl are async-signal-safe; fd 0 is the pty slave by now.
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 || libc::ioctl(0, TIOCSCTTY as _, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = cmd.spawn().expect("spawn on pty");
        Self {
            child,
            master,
            seen: String::new(),
            _slave: Some(slave),
            out,
            shown: Vec::new(),
            screen: None,
            home,
        }
    }

    /// For a failure message: why the daemons of the child's home ended ([`daemon_end_notes`]).
    fn end_notes(&self) -> String {
        self.home
            .as_deref()
            .map_or_else(|| "no ZWALLET_HOME".into(), daemon_end_notes)
    }

    fn full_screen(&self) -> &FullScreen {
        self.screen
            .as_ref()
            .expect("spawned with PtyChild::spawn_full_screen")
    }

    /// Every byte the child wrote to the terminal so far.
    pub fn output(&self) -> Vec<u8> {
        self.full_screen().read.lock().unwrap().clone()
    }

    /// What the terminal shows now ([`Frame`]).
    pub fn frame(&self) -> Frame {
        Frame::parse(&self.output(), self.full_screen().size)
    }

    /// Waits (bounded) until what the child wrote to the terminal so far satisfies `until`, and
    /// returns it. Panics with the frame (and the exit and stderr, once the child has exited) when
    /// the child has exited and the reader has every byte (it ends when the pty does, after the
    /// last one: this process holds no slave of a full-screen child's pty), or past `limit`.
    #[track_caller]
    pub fn wait_output(&mut self, limit: Duration, until: impl Fn(&[u8]) -> bool) -> Vec<u8> {
        let start = Instant::now();
        loop {
            // Looked at before the bytes: if the reader had ended, they are all of them.
            let all_read = self
                .full_screen()
                .reader
                .as_ref()
                .is_none_or(JoinHandle::is_finished);
            let output = self.output();
            if until(&output) {
                return output;
            }
            let exited = self.child.try_wait().ok().flatten();
            if (exited.is_some() && all_read) || start.elapsed() >= limit {
                let size = self.full_screen().size;
                let exit = match exited {
                    Some(status) => format!("exited ({status}), stderr {:?}", self.stderr()),
                    None => format!("still running after {limit:?}"),
                };
                panic!(
                    "not shown; {exit}; the terminal showed:\n{}\n{}",
                    Frame::parse(&output, size),
                    self.end_notes()
                );
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// [`PtyChild::wait_output`] for a frame: until the terminal shows what `until` looks for.
    /// A frame reaches the terminal in pieces (a pty read ends anywhere), so `until` should look
    /// for the last thing the test reads in it; whether the frame holds a stray byte is decided on
    /// the whole output ([`PtyChild::final_frame`]).
    #[track_caller]
    pub fn wait_frame(&mut self, limit: Duration, until: impl Fn(&Frame) -> bool) -> Frame {
        let size = self.full_screen().size;
        Frame::parse(
            &self.wait_output(limit, |output| until(&Frame::parse(output, size))),
            size,
        )
    }

    /// Once the child has exited: its last frame ([`Frame::last`]), from everything it wrote
    /// before it gave the terminal back. All of it was written before the exit; the wait is for
    /// the reader to catch up, and fails once the reader has ended (the pty ended with the child)
    /// if the terminal was never given back.
    #[track_caller]
    pub fn final_frame(&mut self, limit: Duration) -> Frame {
        assert!(
            matches!(self.child.try_wait(), Ok(Some(_))),
            "the child still runs"
        );
        let size = self.full_screen().size;
        let output = self.wait_output(limit, |output| Frame::last(output, size).is_some());
        Frame::last(&output, size).unwrap()
    }

    /// Sets the window to `(columns, rows)`, as a human resizing the terminal does: the kernel
    /// sends SIGWINCH to the child, its terminal's foreground process group.
    pub fn resize(&mut self, (columns, rows): (u16, u16)) {
        use std::os::fd::AsRawFd;
        let window = libc::winsize {
            ws_row: rows,
            ws_col: columns,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: TIOCSWINSZ reads one winsize this frame owns, on an fd this test owns.
        let set = unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ, &window) };
        assert_eq!(set, 0, "TIOCSWINSZ: {}", std::io::Error::last_os_error());
        let screen = self.screen.as_mut().expect("spawned full screen");
        screen.size = (screen.size.0.max(columns), screen.size.1.max(rows));
    }

    /// Keys as typed at the terminal, no newline added (raw mode takes each byte as it comes).
    pub fn send(&self, keys: &[u8]) {
        (&self.master).write_all(keys).unwrap();
    }

    /// Waits (bounded) for the exit and returns its status and stderr; past `limit` the child is
    /// killed and this panics with the last frame.
    #[track_caller]
    pub fn wait_exit(&mut self, limit: Duration) -> (std::process::ExitStatus, String) {
        let start = Instant::now();
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            if start.elapsed() > limit {
                let _ = self.child.kill();
                panic!(
                    "still running after {limit:?}; the terminal showed:\n{}\n{}",
                    self.frame(),
                    self.end_notes()
                );
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        (status, self.stderr())
    }

    /// What the child wrote on stderr; called once it has exited, which ends the pipe.
    fn stderr(&mut self) -> String {
        let reading = self.screen.as_mut().and_then(|s| s.stderr.take());
        reading.map_or_else(String::new, |t| {
            String::from_utf8_lossy(&t.join().unwrap()).into_owned()
        })
    }

    /// The pty's local modes now (`c_lflag`: `ICANON`, `ECHO`, ...), as the child left them once
    /// it has exited. Read through the master, which shares them on macOS and Linux: this process
    /// keeps no slave of a full-screen child's pty, and on macOS the exit of the child, its
    /// session's leader, revokes every open slave of its terminal (`ENOTTY` then).
    pub fn local_modes(&self) -> libc::tcflag_t {
        use std::os::fd::AsRawFd;
        // SAFETY: tcgetattr fills a zeroed termios for an fd this test owns.
        unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            assert_eq!(
                libc::tcgetattr(self.master.as_raw_fd(), &mut t),
                0,
                "tcgetattr"
            );
            t.c_lflag
        }
    }

    /// Waits (bounded) until the terminal shows `needle`, e.g. a password prompt.
    #[track_caller]
    pub fn wait_for(&mut self, needle: &str, limit: Duration) {
        let start = Instant::now();
        loop {
            self.seen.push_str(&drain_pty(&self.master));
            self.drain_out();
            if self.seen.contains(needle) {
                self.seen.clear();
                return;
            }
            assert!(
                start.elapsed() < limit,
                "no {needle:?} on the terminal within {limit:?}; saw {}",
                self.seen_tail()
            );
            if let Ok(Some(status)) = self.child.try_wait() {
                panic!(
                    "exited ({status}) before showing {needle:?}; saw {}",
                    self.seen_tail()
                );
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// For a failure message: the last 4 KB of what the terminal showed, escaped, and why the
    /// daemons of the child's home ended.
    fn seen_tail(&self) -> String {
        let from = self.seen.len().saturating_sub(4096);
        let from = (from..=self.seen.len())
            .find(|i| self.seen.is_char_boundary(*i))
            .unwrap_or(0);
        format!("{:?}; {}", &self.seen[from..], self.end_notes())
    }

    /// Reads what stdout's terminal holds, so the child never blocks on a full one.
    fn drain_out(&mut self) {
        if let Some((master, _)) = &self.out {
            self.shown.extend(drain_pty_bytes(master));
        }
    }

    /// Types one line, as a human would at the prompt.
    pub fn type_line(&self, line: &str) {
        (&self.master)
            .write_all(format!("{line}\n").as_bytes())
            .unwrap();
    }

    /// Waits (bounded) for the exit and collects stdout and stderr. The terminal is drained
    /// meanwhile: a session leader's exit waits until its terminal output has been read.
    pub fn finish(self) -> Output {
        self.finish_terminal().0
    }

    /// [`PtyChild::finish`], plus what stdout's terminal showed (with
    /// [`PtyChild::spawn_on_terminal`]), its `\r\n` read as `\n`.
    pub fn finish_terminal(mut self) -> (Output, String) {
        let pipe = |r: Option<Box<dyn Read + Send>>| {
            std::thread::spawn(move || {
                let mut v = Vec::new();
                if let Some(mut r) = r {
                    let _ = r.read_to_end(&mut v);
                }
                v
            })
        };
        let out = pipe(self.child.stdout.take().map(|s| Box::new(s) as _));
        let err = pipe(self.child.stderr.take().map(|s| Box::new(s) as _));
        let start = Instant::now();
        let status = loop {
            self.seen.push_str(&drain_pty(&self.master));
            self.drain_out();
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            if start.elapsed() > Duration::from_secs(120) {
                let _ = self.child.kill();
                panic!("still running after 120s; terminal {:?}", self.seen);
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        // What it wrote just before exiting.
        self.drain_out();
        let output = Output {
            status,
            stdout: out.join().unwrap(),
            stderr: err.join().unwrap(),
        };
        (
            output,
            String::from_utf8(std::mem::take(&mut self.shown))
                .expect("UTF-8 output")
                .replace("\r\n", "\n"),
        )
    }
}

/// Runs `cmd` (stdin as it has it) with stdout on a terminal of its own and stderr piped, reading
/// the terminal all along: a pty nobody reads holds a writer after about 1 KB on macOS, so a
/// command that prints more never exits. Fails after 60 s rather than hang. Returns the output
/// and what the terminal showed, `\r\n` read as `\n`.
pub fn run_stdout_on_terminal(mut cmd: Command) -> (Output, String) {
    let (master, slave) = openpty();
    cmd.stdout(Stdio::from(slave.try_clone().unwrap()))
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn with stdout on a pty");
    drop(cmd);
    let mut stderr = child.stderr.take().unwrap();
    let err = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = stderr.read_to_end(&mut v);
        v
    });
    let mut shown = Vec::new();
    let start = Instant::now();
    let status = loop {
        shown.extend(drain_pty_bytes(&master));
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if start.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "still running after 60s; terminal {:?}",
                String::from_utf8_lossy(&shown)
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    // What it wrote just before exiting (the slave is still open here).
    shown.extend(drain_pty_bytes(&master));
    drop(slave);
    let output = Output {
        status,
        stdout: Vec::new(),
        stderr: err.join().unwrap(),
    };
    let shown = String::from_utf8(shown).expect("UTF-8 output");
    (output, shown.replace("\r\n", "\n"))
}

/// The QR codes in `terminal` (as [`PtyChild::finish_terminal`] returns it), decoded: each run
/// of lines drawn only with half blocks and spaces once ANSI escapes are removed, read as two rows
/// of modules per line. In a coloured code (zwallet's default) a block is a dark module; in a
/// plain one (`NO_COLOR`) a block is a light one, as on a dark terminal. Read from a picture of
/// the code on a dark terminal, so the code must carry its own quiet zone.
pub fn scan_qr(terminal: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut block: Vec<String> = Vec::new();
    let mut colored = false;
    for line in terminal.lines().chain([""]) {
        let plain = strip_ansi(line);
        if !plain.is_empty() && plain.chars().all(|c| " ▀▄█".contains(c)) {
            colored |= plain.len() < line.len();
            block.push(plain);
            continue;
        }
        if !block.is_empty() {
            found.push(decode_qr_lines(&std::mem::take(&mut block), colored));
            colored = false;
        }
    }
    found
}

/// `text` without its ANSI escape sequences (`ESC [`, up to a final byte from `@` to `~`).
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.next() == Some('[') {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

fn decode_qr_lines(lines: &[String], colored: bool) -> String {
    // Modules, `true` for dark: the block characters draw the dark modules in colour, the light
    // ones without.
    let mut rows: Vec<Vec<bool>> = Vec::new();
    for line in lines {
        let (mut top, mut bottom) = (Vec::new(), Vec::new());
        for c in line.chars() {
            let (t, b) = match c {
                '█' => (true, true),
                '▀' => (true, false),
                '▄' => (false, true),
                _ => (false, false),
            };
            top.push(if colored { t } else { !t });
            bottom.push(if colored { b } else { !b });
        }
        rows.push(top);
        rows.push(bottom);
    }
    // A picture of a dark terminal showing them: 4 px per module, 8 modules of background around.
    let (scale, pad) = (4, 8);
    let width = rows.iter().map(Vec::len).max().unwrap_or(0) + 2 * pad;
    let height = rows.len() + 2 * pad;
    let mut img =
        rqrr::PreparedImage::prepare_from_greyscale(width * scale, height * scale, |x, y| {
            let (x, y) = (x / scale, y / scale);
            let dark = x < pad
                || y < pad
                || rows
                    .get(y - pad)
                    .is_none_or(|r| r.get(x - pad).is_none_or(|&d| d));
            if dark { 0 } else { 255 }
        });
    let grids = img.detect_grids();
    assert_eq!(grids.len(), 1, "one QR code in {lines:#?}");
    grids[0]
        .decode()
        .unwrap_or_else(|e| panic!("{e:?}: {lines:#?}"))
        .1
}

impl Drop for PtyChild {
    /// A test that fails while the child still runs (say, waiting at a password prompt) must not
    /// leave it behind.
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        if let Some(screen) = &mut self.screen {
            screen.stop.store(true, Ordering::SeqCst);
            if let Some(t) = screen.reader.take() {
                let _ = t.join();
            }
        }
    }
}

/// Reads `master` into `into` until `stop`, or until the pty is gone: `poll` first, so the read
/// never blocks past a look at `stop`.
fn read_all_along(
    mut master: File,
    into: Arc<Mutex<Vec<u8>>>,
    stop: Arc<AtomicBool>,
) -> JoinHandle<()> {
    use std::os::fd::AsRawFd;
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while !stop.load(Ordering::SeqCst) {
            let mut ready = libc::pollfd {
                fd: master.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: poll(2) on one pollfd this frame owns.
            if unsafe { libc::poll(&mut ready, 1, 50) } < 1 {
                continue;
            }
            match master.read(&mut buf) {
                Ok(0) => return,
                Ok(n) => into.lock().unwrap().extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => return,
            }
        }
    })
}

/// The sequence a full-screen program writes when it gives the terminal back (crossterm's
/// `LeaveAlternateScreen`); `zwallet tui`'s guard writes it once, last.
pub const LEAVE_ALTERNATE_SCREEN: &[u8] = b"\x1b[?1049l";

/// The whitelist summary's caption, `Summary rows <start>-<end> of <total>`: (start, total).
pub fn whitelist_summary_position(frame: &Frame) -> Option<(usize, usize)> {
    frame.rows.iter().find_map(|row| {
        let (_, caption) = row.split_once("Summary rows ")?;
        let (range, total) = caption.split_once(" of ")?;
        let start = range.split_once('-')?.0.parse().ok()?;
        let total = total.split_whitespace().next()?.parse().ok()?;
        Some((start, total))
    })
}

/// Pages a whitelist summary with Enter until every row has been on screen (10 rows a page),
/// then opens the password field. End no longer jumps to the last page.
pub fn open_whitelist_password(tui: &mut PtyChild, limit: Duration) {
    let mut frame = tui.wait_frame(limit, |frame| whitelist_summary_position(frame).is_some());
    loop {
        let (start, total) = whitelist_summary_position(&frame).expect("review caption");
        if start - 1 + 10 >= total {
            break;
        }
        let next = (start - 1 + 10).min(total.saturating_sub(10)) + 1;
        tui.send(b"\r");
        frame = tui.wait_frame(limit, |frame| {
            whitelist_summary_position(frame).is_some_and(|(start, _)| start == next)
        });
    }
    tui.send(b"\r");
    tui.wait_frame(limit, |frame| {
        frame.rows.join("\n").contains("Wallet password")
    });
}

/// What a terminal shows after a full-screen program's output, as ratatui's crossterm backend
/// writes it: `CSI row;col H` moves the cursor, `CSI 2J` and `CSI ?1049h` (the alternate screen)
/// clear it, any other CSI (styles, the cursor's visibility, leaving the alternate screen) changes
/// no cell; CR, LF (raw mode: no CR added) and BS move the cursor. Every other ESC and every
/// other control character is kept in `stray`: none may come from a cell's content (03-tui.md
/// §Display integrity rule 1). A sequence the bytes end inside (a lone ESC, or a CSI before its
/// final byte) is `pending`, not stray: a read of a live terminal ends anywhere.
pub struct Frame {
    /// One string per row, every column (trailing spaces included).
    pub rows: Vec<String>,
    pub stray: Vec<String>,
    /// The unfinished sequence the bytes end with, if any.
    pub pending: String,
}

impl Frame {
    pub fn parse(bytes: &[u8], (columns, rows): (u16, u16)) -> Self {
        let blank = || vec![vec![' '; usize::from(columns)]; usize::from(rows)];
        let (mut cells, mut stray, mut pending) = (blank(), Vec::new(), String::new());
        let (mut x, mut y) = (0usize, 0usize);
        let text = String::from_utf8_lossy(bytes);
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\u{1b}' if chars.peek() == Some(&'[') => {
                    chars.next();
                    let mut params = String::new();
                    let end = loop {
                        match chars.next() {
                            Some(c) if ('\u{20}'..='\u{3f}').contains(&c) => params.push(c),
                            other => break other,
                        }
                    };
                    match (end, params.as_str()) {
                        (None, _) => pending = format!("\u{1b}[{params}"),
                        (Some('H'), at) => {
                            let mut at = at.split(';').map(|n| n.parse().unwrap_or(1).max(1));
                            y = at.next().unwrap_or(1) - 1;
                            x = at.next().unwrap_or(1) - 1;
                        }
                        (Some('J'), "2") | (Some('h'), "?1049") => cells = blank(),
                        (Some('@'..='~'), _) => {}
                        _ => stray.push(format!("CSI {params:?} {end:?}")),
                    }
                }
                '\u{1b}' if chars.peek().is_none() => pending.push(c),
                '\r' => x = 0,
                '\n' => y += 1,
                '\u{8}' => x = x.saturating_sub(1),
                c if c.is_control() => stray.push(format!("{c:?}")),
                c => {
                    if let Some(cell) = cells.get_mut(y).and_then(|r| r.get_mut(x)) {
                        *cell = c;
                    }
                    x += 1;
                }
            }
        }
        Self {
            rows: cells.into_iter().map(String::from_iter).collect(),
            stray,
            pending,
        }
    }

    /// The last frame of a full-screen program, from its whole output: what the terminal showed
    /// when the program gave it back (the bytes before [`LEAVE_ALTERNATE_SCREEN`]); `None` before
    /// it has. Every draw is done by then, so the frame is finished, and its `stray` covers the
    /// whole run.
    pub fn last(bytes: &[u8], size: (u16, u16)) -> Option<Self> {
        let end = bytes
            .windows(LEAVE_ALTERNATE_SCREEN.len())
            .position(|w| w == LEAVE_ALTERNATE_SCREEN)?;
        Some(Self::parse(&bytes[..end], size))
    }
}

impl std::fmt::Display for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for row in &self.rows {
            writeln!(f, "{}", row.trim_end())?;
        }
        write!(f, "stray: {:?}, pending: {:?}", self.stray, self.pending)
    }
}

/// A minimal JSON-RPC node on 127.0.0.1 answering each method with `result(method)`, for the
/// commands that need RPC (e.g. `balance`). Returns its URL; it lives until the test exits.
pub fn tiny_rpc(result: fn(&str) -> Value) -> String {
    tiny_rpc_reply(move |method, _| Ok(result(method)))
}

/// [`tiny_rpc`] whose `reply(method, params)` is either a `result` (`Ok`) or a JSON-RPC `error`
/// object (`Err`), for tests of what zwallet makes of a node's text.
pub fn tiny_rpc_reply(
    reply: impl Fn(&str, &Value) -> std::result::Result<Value, Value> + Send + Sync + 'static,
) -> String {
    let reply = Arc::new(reply);
    use std::io::BufRead;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let reply = reply.clone();
            std::thread::spawn(move || {
                let mut writer = stream.try_clone().unwrap();
                let mut reader = std::io::BufReader::new(stream);
                loop {
                    let mut len = 0usize;
                    let mut line = String::new();
                    loop {
                        line.clear();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 {
                            return;
                        }
                        let l = line.trim_end();
                        if l.is_empty() {
                            break;
                        }
                        if let Some((k, v)) = l.split_once(':')
                            && k.eq_ignore_ascii_case("content-length")
                        {
                            len = v.trim().parse().unwrap();
                        }
                    }
                    let mut body = vec![0u8; len];
                    if reader.read_exact(&mut body).is_err() {
                        return;
                    }
                    let req: Value = serde_json::from_slice(&body).unwrap();
                    let text = match reply(req["method"].as_str().unwrap_or(""), &req["params"]) {
                        Ok(result) => {
                            serde_json::json!({ "jsonrpc": "2.0", "id": req["id"], "result": result })
                        }
                        Err(error) => {
                            serde_json::json!({ "jsonrpc": "2.0", "id": req["id"], "error": error })
                        }
                    }
                    .to_string();
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                        text.len()
                    );
                    if writer.write_all(head.as_bytes()).is_err()
                        || writer.write_all(text.as_bytes()).is_err()
                    {
                        return;
                    }
                }
            });
        }
    });
    url
}
