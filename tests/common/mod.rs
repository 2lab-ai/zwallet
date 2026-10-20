//! Helpers for driving the real binary. Every test gets its own short temp home under /tmp
//! (the daemon socket path must fit `sun_path`), an empty environment except `ZWALLET_HOME`,
//! and a guard that locks and, if needed, kills any daemon it started.

#![allow(dead_code)]

use serde_json::Value;
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::net::UnixListener;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
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

    /// Every zwallet daemon process for this home, by argv in the process table.
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

/// Pids of the zwallet daemons for `home` (other than this process), found by argv
/// (`zwallet daemon --home <home>`) in the OS process table: the same lookup `lock` uses, which
/// also sees a non-dumpable daemon on Linux whose /proc fd directory is unreadable.
pub fn daemon_pids(home: &Path) -> Vec<i32> {
    zwallet::core::protocol::daemon_pids(home, None).expect("process table")
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

    fn bind_with_reply(path: &Path, reply: Vec<u8>) -> Self {
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
                        let _ = s.write_all(&reply);
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

/// `TIOCSCTTY` (make this terminal the controlling one); libc does not define it for macOS.
#[cfg(target_os = "macos")]
const TIOCSCTTY: libc::c_ulong = 0x2000_7461;
#[cfg(not(target_os = "macos"))]
const TIOCSCTTY: libc::c_ulong = libc::TIOCSCTTY as libc::c_ulong;

/// A process whose stdin and controlling terminal (`/dev/tty`) is a pty, as when a human runs it
/// at a terminal; stdout and stderr are piped.
pub struct PtyChild {
    pub child: Child,
    pub master: File,
    seen: String,
    /// Kept open: on macOS the pty drops unread output once every slave is closed.
    _slave: OwnedFd,
}

impl PtyChild {
    pub fn spawn(mut cmd: Command) -> Self {
        let (master, slave) = openpty();
        cmd.stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
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
            _slave: slave,
        }
    }

    /// Waits (bounded) until the terminal shows `needle`, e.g. a password prompt.
    pub fn wait_for(&mut self, needle: &str, limit: Duration) {
        let start = Instant::now();
        loop {
            self.seen.push_str(&drain_pty(&self.master));
            if self.seen.contains(needle) {
                self.seen.clear();
                return;
            }
            assert!(
                start.elapsed() < limit,
                "no {needle:?} on the terminal within {limit:?}; saw {:?}",
                self.seen
            );
            if let Ok(Some(status)) = self.child.try_wait() {
                panic!(
                    "exited ({status}) before showing {needle:?}; saw {:?}",
                    self.seen
                );
            }
            std::thread::sleep(Duration::from_millis(20));
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
    pub fn finish(mut self) -> Output {
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
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            if start.elapsed() > Duration::from_secs(120) {
                let _ = self.child.kill();
                panic!("still running after 120s; terminal {:?}", self.seen);
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        Output {
            status,
            stdout: out.join().unwrap(),
            stderr: err.join().unwrap(),
        }
    }
}

/// A minimal JSON-RPC node on 127.0.0.1 answering each method with `result(method)`, for the
/// commands that need RPC (e.g. `balance`). Returns its URL; it lives until the test exits.
pub fn tiny_rpc(result: fn(&str) -> Value) -> String {
    use std::io::BufRead;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
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
                    let text = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": req["id"],
                        "result": result(req["method"].as_str().unwrap_or("")),
                    })
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
