//! Helpers for driving the real binary. Every test gets its own short temp home under /tmp
//! (the daemon socket path must fit `sun_path`), an empty environment except `ZWALLET_HOME`,
//! and a guard that locks and, if needed, kills any daemon it started.

#![allow(dead_code)]

use serde_json::Value;
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
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

    /// The bare command (empty env except `ZWALLET_HOME`) for tests that wire stdio themselves.
    pub fn command(&self, args: &[&str]) -> Command {
        command_in(self.path(), args)
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

    /// Every process other than this test that is bound to this home's `d.sock` (by name, so
    /// an orphan whose path was unlinked still counts) or holds its `d.lock`.
    pub fn daemon_pids(&self) -> Vec<i32> {
        daemon_pids(self.path())
    }

    /// Rewrites `wallet.json` through a JSON edit (metadata only unless the edit says otherwise).
    pub fn edit_wallet(&self, edit: impl FnOnce(&mut Value)) {
        let mut v: Value =
            serde_json::from_slice(&std::fs::read(self.wallet_json()).unwrap()).unwrap();
        edit(&mut v);
        std::fs::write(self.wallet_json(), serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let pid = self.daemon_pid();
        let _ = self.run(&["lock", "--json"]);
        let mut pids = self.daemon_pids();
        pids.extend(pid);
        for pid in pids {
            // Never 0 or 1 (a fake daemon's reply could carry anything): kill(0) would hit
            // this whole process group.
            if pid > 1 && pid_alive(pid) {
                // SAFETY: plain kill(2) on a daemon this test's home started.
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                }
            }
        }
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

pub fn command_in(home: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_zwallet"));
    cmd.env_clear().env("ZWALLET_HOME", home).args(args);
    cmd
}

pub fn run_in(home: &Path, args: &[&str], stdin: Option<&str>, env: &[(&str, &str)]) -> Run {
    let mut cmd = command_in(home, args);
    cmd.stdout(Stdio::piped())
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

/// Pids (other than this process) bound to `<home>/d.sock` or holding `<home>/d.lock` open.
pub fn daemon_pids(home: &Path) -> Vec<i32> {
    let sock = home.join("d.sock");
    let lock = home.join("d.lock");
    let me = std::process::id() as i32;
    let mut pids = platform_pids(&sock, &lock);
    pids.retain(|p| *p != me);
    pids.sort_unstable();
    pids.dedup();
    pids
}

#[cfg(target_os = "linux")]
fn platform_pids(sock: &Path, lock: &Path) -> Vec<i32> {
    // Socket inodes bound to `sock` by name, from /proc/net/unix (last column is the path).
    let sock_name = sock.to_string_lossy().into_owned();
    let inodes: Vec<String> = std::fs::read_to_string("/proc/net/unix")
        .unwrap_or_default()
        .lines()
        .skip(1)
        .filter_map(|l| {
            let cols: Vec<&str> = l.split_whitespace().collect();
            (cols.len() >= 8 && cols[7] == sock_name).then(|| cols[6].to_string())
        })
        .collect();
    let mut pids = Vec::new();
    for entry in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<i32>() else {
            continue;
        };
        let fds = entry.path().join("fd");
        for fd in std::fs::read_dir(fds).into_iter().flatten().flatten() {
            let Ok(target) = std::fs::read_link(fd.path()) else {
                continue;
            };
            let t = target.to_string_lossy();
            if target == lock || inodes.iter().any(|i| t == format!("socket:[{i}]")) {
                pids.push(pid);
            }
        }
    }
    pids
}

#[cfg(not(target_os = "linux"))]
fn platform_pids(sock: &Path, lock: &Path) -> Vec<i32> {
    let mut pids = Vec::new();
    // `-F pn`: one `p<pid>` line per process, then `n<name>` per file; unix sockets carry the
    // bound path as their name even after the path itself was unlinked.
    let out = Command::new("lsof")
        .args(["-nP", "-U", "-F", "pn"])
        .stderr(Stdio::null())
        .output()
        .expect("lsof");
    let want = format!("n{}", sock.display());
    let mut current = None;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if let Some(p) = line.strip_prefix('p') {
            current = p.parse::<i32>().ok();
        } else if line == want
            && let Some(p) = current
        {
            pids.push(p);
        }
    }
    if lock.exists() {
        let out = Command::new("lsof")
            .arg("-t")
            .arg(lock)
            .stderr(Stdio::null())
            .output()
            .expect("lsof");
        pids.extend(
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter_map(|l| l.trim().parse::<i32>().ok()),
        );
    }
    pids
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
                        rx.lock().unwrap().push(buf);
                        let _ = s.write_all(FAKE_REPLY);
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

/// A pseudo-terminal pair: `(master, slave)`.
pub fn openpty() -> (File, OwnedFd) {
    let (mut master, mut slave) = (-1, -1);
    // SAFETY: openpty writes two fds into the out-params; null name/termios/winsize are allowed.
    let rc = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(rc, 0, "openpty failed");
    // SAFETY: both fds were just returned by openpty and are owned by nobody else.
    unsafe { (File::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) }
}

/// Reads whatever the pty master has buffered right now, without blocking.
pub fn drain_pty(master: &File) -> String {
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
    String::from_utf8_lossy(&out).into_owned()
}
