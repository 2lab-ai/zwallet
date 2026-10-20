//! Daemon wire protocol and client.
//!
//! One JSON request line and one JSON response line per connection over `$ZWALLET_HOME/d.sock`
//! (0600 inside the 0700 home; peer uid checked on both ends). Requests are family-agnostic:
//! `chain` picks the family, `index` the key, and `tx` is the family's own unsigned-tx JSON.
//!
//! No request carries a secret. The password never travels over the socket: [`start_daemon`]
//! hands it to the daemon child it spawns, on that child's inherited stdin pipe, and reads the
//! child's progress on its stdout pipe. Whatever else listens on `d.sock` (a stale socket, or a
//! squatter run by the same user) can learn nothing from a client and is replaced by the new
//! daemon, which binds under an exclusive lock on `d.lock`.
//!
//! Which daemons exist is never read from `d.sock` or `d.lock` (any same-user process can rename
//! or replace those): `lock` and `unlock` find them by argv in the OS process table and stop them
//! by signal ([`stop_daemons`]).

use crate::core::error::{ErrorCode, Result, WalletError};
use crate::core::password::{MAX_SECRET, read_exact_secret};
use crate::core::store;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::ffi::{OsStr, OsString};
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::os::fd::RawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use zeroize::Zeroizing;

pub const PROTOCOL_VERSION: u32 = 2;
pub const SOCKET_NAME: &str = "d.sock";
/// Held (flock, exclusive) by the one daemon serving a home, for its whole life.
pub const LOCK_NAME: &str = "d.lock";
/// Held (flock, exclusive) by an unlocking CLI from after the password prompt until its daemon
/// reported readiness, so concurrent unlocks replace the serving daemon one at a time.
pub const UNLOCK_LOCK_NAME: &str = "u.lock";
pub const MAX_MESSAGE: u64 = 1 << 20;
/// `sizeof(sockaddr_un.sun_path)` including the terminating NUL.
#[cfg(target_os = "linux")]
pub const SUN_PATH_MAX: usize = 108;
#[cfg(not(target_os = "linux"))]
pub const SUN_PATH_MAX: usize = 104;
const QUICK: Duration = Duration::from_secs(3);
/// The daemon child decrypts (scrypt) and verifies every key before it reports `decrypted`.
pub const UNLOCK_TIMEOUT: Duration = Duration::from_secs(60);
pub const SIGN_TIMEOUT: Duration = Duration::from_secs(15);
/// How long an unlocking CLI waits for another one's `u.lock`.
const UNLOCK_LOCK_WAIT: Duration = Duration::from_secs(30);
/// How long the child gets to ask for its input (it only disables core dumps first).
const INPUT_TIMEOUT: Duration = Duration::from_secs(10);
/// After the go byte: the child's wait for `d.lock` (5 s), then the bind.
const BIND_TIMEOUT: Duration = Duration::from_secs(20);
/// Longest header line before the password: `<ttl secs> <password length>`.
const MAX_HEADER: usize = 48;
/// Child → CLI progress, the `stage` of an ok [`Response`] line: ready for the password (core
/// dumps are off), then the wallet decrypted with it. The third line is the readiness itself.
pub const STAGE_INPUT: &str = "input";
pub const STAGE_DECRYPTED: &str = "decrypted";
/// CLI → child once the old daemons are gone: take `d.lock`, bind and serve.
pub const GO: u8 = b'G';

/// A daemon request. None carries a secret; the password reaches the daemon only through
/// [`start_daemon`].
#[derive(Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum Request {
    Lock,
    Status,
    Address {
        chain: String,
        index: u32,
    },
    SignMessage {
        chain: String,
        index: u32,
        message_hex: String,
    },
    SignTx {
        chain: String,
        index: u32,
        tx: Value,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Response {
    pub v: u32,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<ErrorCode>,
}

impl Response {
    pub fn ok(result: Value) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            ok: true,
            result: Some(result),
            error: None,
            error_code: None,
        }
    }

    pub fn err(e: &WalletError) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            ok: false,
            result: None,
            error: Some(e.message.clone()),
            error_code: Some(e.code),
        }
    }

    pub fn into_result(self) -> Result<Value> {
        if self.ok {
            Ok(self.result.unwrap_or(Value::Null))
        } else {
            Err(WalletError::new(
                self.error_code.unwrap_or(ErrorCode::ValidationError),
                self.error.unwrap_or_else(|| "daemon error".into()),
            ))
        }
    }
}

/// Effective uid of this process.
pub fn current_uid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

/// `<home>/d.sock`, refusing paths the OS cannot bind (`sun_path` limit) instead of failing
/// obscurely later.
pub fn socket_path(home: &Path) -> Result<PathBuf> {
    let path = home.join(SOCKET_NAME);
    let len = path.as_os_str().len();
    if len >= SUN_PATH_MAX {
        return Err(WalletError::validation(format!(
            "daemon socket path {} is {len} bytes; this OS allows at most {} (sun_path). Set \
             ZWALLET_HOME to a shorter directory",
            path.display(),
            SUN_PATH_MAX - 1
        )));
    }
    Ok(path)
}

fn io_err(what: &str) -> impl Fn(std::io::Error) -> WalletError + '_ {
    move |e| WalletError::validation(format!("{what}: {e}"))
}

fn is_disconnect(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        ErrorKind::ConnectionReset | ErrorKind::BrokenPipe | ErrorKind::UnexpectedEof
    )
}

/// Sends one request. `Ok(None)` means no daemon is listening, i.e. the wallet is locked.
pub async fn send(home: &Path, req: &Request, timeout: Duration) -> Result<Option<Value>> {
    let path = socket_path(home)?;
    match std::fs::symlink_metadata(&path) {
        Ok(meta) if meta.uid() != current_uid() => {
            return Err(WalletError::validation(format!(
                "refusing to use {}: it is owned by uid {}, not this user",
                path.display(),
                meta.uid()
            )));
        }
        // Never follow a symlink (or anything else) planted in place of the socket.
        Ok(meta) if !meta.file_type().is_socket() => {
            return Err(WalletError::validation(format!(
                "refusing to use {}: it is not a socket",
                path.display()
            )));
        }
        Ok(_) => {}
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_err("cannot inspect daemon socket")(e)),
    }
    let exchange = async {
        let mut stream = match UnixStream::connect(&path).await {
            Ok(s) => s,
            Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::ConnectionRefused) => {
                return Ok(None);
            }
            Err(e) => return Err(io_err("cannot reach the key daemon")(e)),
        };
        let peer = stream
            .peer_cred()
            .map_err(io_err("cannot check daemon credentials"))?;
        if peer.uid() != current_uid() {
            return Err(WalletError::validation(
                "refusing to talk to a key daemon run by another user",
            ));
        }
        let mut line = Zeroizing::new(
            serde_json::to_vec(req)
                .map_err(|e| WalletError::validation(format!("cannot encode request: {e}")))?,
        );
        line.push(b'\n');
        let mut buf = Vec::new();
        let io = async {
            stream.write_all(&line).await?;
            stream.shutdown().await?;
            (&mut stream).take(MAX_MESSAGE).read_to_end(&mut buf).await
        };
        match io.await {
            Ok(_) => {}
            // The daemon went away mid-exchange (it was locking or expiring): locked.
            Err(e) if is_disconnect(&e) => return Ok(None),
            Err(e) => return Err(io_err("cannot talk to the key daemon")(e)),
        }
        if buf.is_empty() {
            // The daemon closed without answering (e.g. it was exiting); treat as locked.
            return Ok(None);
        }
        let resp: Response = serde_json::from_slice(&buf).map_err(|e| {
            WalletError::validation(format!("malformed reply from the key daemon: {e}"))
        })?;
        resp.into_result().map(Some)
    };
    match tokio::time::timeout(timeout, exchange).await {
        Ok(r) => r,
        Err(_) => Err(WalletError::validation(format!(
            "the key daemon did not answer within {}s",
            timeout.as_secs()
        ))),
    }
}

/// Like [`send`], but a missing daemon is `WALLET_LOCKED`.
pub async fn send_unlocked(home: &Path, req: &Request, timeout: Duration) -> Result<Value> {
    send(home, req, timeout)
        .await?
        .ok_or_else(WalletError::locked)
}

/// Daemon status, or `None` when no daemon is running.
pub async fn status(home: &Path) -> Result<Option<Value>> {
    send(home, &Request::Status, QUICK).await
}

/// Locks the wallet for `home`. First, politely, asks whatever answers on `d.sock` to lock (the
/// request carries nothing; errors and lies there are ignored); then stops every daemon for this
/// home found in the process table. Succeeds only when none is left (see [`stop_daemons`]).
/// Returns whether a daemon was running.
pub async fn lock(home: &Path) -> Result<bool> {
    let polite = polite_lock(home).await;
    let stopped = stop_daemons(home, None)?;
    Ok(polite || !stopped.is_empty())
}

/// Whether whatever answers on `d.sock` says it was unlocked and now locks.
async fn polite_lock(home: &Path) -> bool {
    matches!(
        send(home, &Request::Lock, QUICK).await,
        Ok(Some(v)) if v["wasUnlocked"] == json!(true)
    )
}

/// What the CLI writes to the daemon child's stdin once the child asked for it: a header line
/// `<ttl secs> <password length>`, then exactly that many password bytes. Later, once the old
/// daemons are gone, the single byte [`GO`].
pub fn encode_unlock_input(ttl: Duration, password: &str) -> Result<Zeroizing<Vec<u8>>> {
    if password.is_empty() || password.len() > MAX_SECRET {
        return Err(WalletError::validation(format!(
            "the password must be 1 to {MAX_SECRET} bytes"
        )));
    }
    let head = format!("{} {}\n", ttl.as_secs(), password.len());
    let mut out = Zeroizing::new(Vec::with_capacity(head.len() + password.len()));
    out.extend_from_slice(head.as_bytes());
    out.extend_from_slice(password.as_bytes());
    Ok(out)
}

/// Parses the header line of [`encode_unlock_input`] (without its newline).
pub fn decode_unlock_header(line: &[u8]) -> Result<(Duration, usize)> {
    let bad = || WalletError::validation("malformed unlock input");
    let digits = |d: &[u8]| -> Option<u64> {
        (!d.is_empty() && d.len() <= 20 && d.iter().all(u8::is_ascii_digit))
            .then(|| std::str::from_utf8(d).ok()?.parse().ok())
            .flatten()
    };
    let mut parts = line.split(|b| *b == b' ');
    let (Some(ttl), Some(len), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err(bad());
    };
    let ttl = digits(ttl).filter(|s| *s > 0).ok_or_else(bad)?;
    let len = digits(len)
        .and_then(|l| usize::try_from(l).ok())
        .ok_or_else(bad)?;
    if len == 0 {
        return Err(WalletError::validation(
            "no password given to the key daemon",
        ));
    }
    if len > MAX_SECRET {
        return Err(bad());
    }
    Ok((Duration::from_secs(ttl), len))
}

/// The daemon's side of [`encode_unlock_input`], reading `fd`: the header one byte at a time
/// (so not one password byte lands in another buffer), then the password with raw read(2) into
/// one zeroized buffer of exactly its length.
pub fn read_unlock_input(fd: RawFd) -> Result<(Duration, Zeroizing<Vec<u8>>)> {
    let mut header = Vec::with_capacity(MAX_HEADER);
    loop {
        let mut byte = 0u8;
        // SAFETY: reads at most one byte into a valid local.
        let n = unsafe { libc::read(fd, (&mut byte as *mut u8).cast(), 1) };
        if n < 0 {
            if std::io::Error::last_os_error().kind() == ErrorKind::Interrupted {
                continue;
            }
            return Err(WalletError::validation("cannot read the unlock input"));
        }
        if n == 0 || header.len() == MAX_HEADER {
            return Err(WalletError::validation("malformed unlock input"));
        }
        if byte == b'\n' {
            break;
        }
        header.push(byte);
    }
    let (ttl, len) = decode_unlock_header(&header)?;
    let password = read_exact_secret(fd, len)
        .map_err(|_| WalletError::validation("cannot read the password from the unlocking CLI"))?;
    Ok((ttl, password))
}

/// The daemon waits (bounded) on `fd` for [`GO`] from the CLI that started it.
pub fn read_go(fd: RawFd, wait: Duration) -> Result<()> {
    let gone = || WalletError::validation("the unlocking process went away");
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let ms = libc::c_int::try_from(wait.as_millis()).unwrap_or(libc::c_int::MAX);
    loop {
        // SAFETY: polls one valid pollfd.
        let rc = unsafe { libc::poll(&mut pfd, 1, ms) };
        if rc < 0 && std::io::Error::last_os_error().kind() == ErrorKind::Interrupted {
            continue;
        }
        if rc <= 0 {
            return Err(gone());
        }
        break;
    }
    let mut byte = 0u8;
    // SAFETY: reads at most one byte into a valid local.
    let n = unsafe { libc::read(fd, (&mut byte as *mut u8).cast(), 1) };
    if n == 1 && byte == GO {
        Ok(())
    } else {
        Err(gone())
    }
}

/// The binary `unlock` starts as the daemon, pinned before the password prompt so a binary
/// swapped in meanwhile is never handed the password. On Linux that is the image this process
/// runs (`/proc/self/exe`); elsewhere the canonical path of `current_exe()`, whose (dev, ino) is
/// recorded now and checked again right before the spawn.
pub struct DaemonExe {
    path: PathBuf,
    #[cfg(not(target_os = "linux"))]
    id: (u64, u64),
}

impl DaemonExe {
    pub fn resolve() -> Result<Self> {
        #[cfg(target_os = "linux")]
        {
            Ok(Self {
                path: PathBuf::from("/proc/self/exe"),
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            let exe = std::env::current_exe()
                .and_then(std::fs::canonicalize)
                .map_err(io_err("cannot locate the zwallet binary"))?;
            let meta =
                std::fs::metadata(&exe).map_err(io_err("cannot inspect the zwallet binary"))?;
            Ok(Self {
                path: exe,
                id: (meta.dev(), meta.ino()),
            })
        }
    }

    /// Fails if the file at the pinned path is no longer the one recorded by [`Self::resolve`].
    fn check(&self) -> Result<()> {
        #[cfg(not(target_os = "linux"))]
        {
            let now = std::fs::metadata(&self.path).map(|m| (m.dev(), m.ino()));
            if now.ok() != Some(self.id) {
                return Err(WalletError::validation(format!(
                    "the zwallet binary at {} was replaced while the password was typed; not \
                     starting it",
                    self.path.display()
                )));
            }
        }
        Ok(())
    }
}

/// Starts a new key daemon for `home` holding the seed for `ttl`, and returns its readiness
/// report `{ locked, expiresAt, active, index, address, pid }` (the address derived by the
/// daemon from the seed it decrypted with `password`, never read from the cache in wallet.json).
///
/// The new daemon is this process's own child (`setsid`, argv `zwallet daemon --home <canonical
/// home>`, no environment, no descriptor but its pipes), started from `exe`. Over its pipes:
/// 1. it asks for input only once core dumps are off; it then gets the TTL and the password,
/// 2. it decrypts and reports `decrypted`. A wrong password ends here and leaves any running
///    session untouched;
/// 3. only then are the old daemons stopped (polite `lock`, then the process table);
/// 4. after [`GO`] it takes `d.lock`, binds `d.sock` (replacing whatever is there) and reports
///    readiness.
///
/// Concurrent unlocks serialize on `u.lock`. On any failure or timeout this kills and reaps the
/// child.
pub async fn start_daemon(
    home: &Path,
    exe: &DaemonExe,
    password: &str,
    ttl: Duration,
) -> Result<Value> {
    socket_path(home)?;
    let canonical = daemon_home(home)?;
    socket_path(&canonical)?;
    let _serial = store::lock_exclusive(
        &home.join(UNLOCK_LOCK_NAME),
        UNLOCK_LOCK_WAIT,
        "another `zwallet unlock` is running",
    )?;
    exe.check()?;
    let mut child = spawn_daemon(exe, &canonical)?;
    let ready = drive_daemon(home, &mut child, password, ttl).await;
    if ready.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    ready
}

fn spawn_daemon(exe: &DaemonExe, canonical_home: &Path) -> Result<Child> {
    let mut cmd = Command::new(&exe.path);
    cmd.arg0(DAEMON_ARG0)
        .arg(DAEMON_SUBCOMMAND)
        .arg(HOME_FLAG)
        .arg(canonical_home)
        .env_clear()
        .current_dir("/")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // SAFETY: runs between fork and exec and only calls async-signal-safe functions (setsid,
    // getrlimit, fcntl, syscall); stdio is already in place on 0-2.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            cloexec_from(3);
            Ok(())
        });
    }
    cmd.spawn().map_err(io_err("cannot start the key daemon"))
}

/// Marks every descriptor from `first` up close-on-exec, so the daemon starts with only its
/// three pipes whatever this process inherited. Async-signal-safe.
fn cloexec_from(first: libc::c_int) {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: close_range(2) with CLOSE_RANGE_CLOEXEC only changes descriptor flags.
        let rc = unsafe {
            libc::syscall(
                libc::SYS_close_range,
                first as libc::c_uint,
                libc::c_uint::MAX,
                libc::CLOSE_RANGE_CLOEXEC,
            )
        };
        if rc == 0 {
            return;
        }
    }
    let mut lim = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: getrlimit fills a valid rlimit.
    let ok = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim) } == 0;
    let max = if ok && lim.rlim_cur != libc::RLIM_INFINITY {
        lim.rlim_cur.min(1 << 20) as libc::c_int
    } else {
        1 << 16
    };
    for fd in first..max {
        // SAFETY: fcntl on a descriptor number; EBADF for unused ones is ignored.
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFD);
            if flags >= 0 && flags & libc::FD_CLOEXEC == 0 {
                libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
            }
        }
    }
}

async fn drive_daemon(
    home: &Path,
    child: &mut Child,
    password: &str,
    ttl: Duration,
) -> Result<Value> {
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        return Err(WalletError::validation("the key daemon has no pipes"));
    };
    let lines = line_reader(stdout);
    expect_stage(&lines, STAGE_INPUT, INPUT_TIMEOUT)?;
    // Unbuffered pipe writes: no intermediate copy of the password in this process.
    stdin
        .write_all(&encode_unlock_input(ttl, password)?)
        .map_err(io_err("cannot hand the password to the key daemon"))?;
    expect_stage(&lines, STAGE_DECRYPTED, UNLOCK_TIMEOUT)?;
    // The password is right: only now is the running session replaced.
    let _ = polite_lock(home).await;
    stop_daemons(home, Some(child.id() as i32))?;
    stdin
        .write_all(&[GO])
        .map_err(io_err("cannot start the key daemon"))?;
    drop(stdin);
    next_response(&lines, BIND_TIMEOUT)?.into_result()
}

/// Lines of the child's stdout, read on a thread so each can be awaited with a timeout. `None`
/// once it closed.
fn line_reader(stdout: ChildStdout) -> mpsc::Receiver<Option<Vec<u8>>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout.take(4 * MAX_MESSAGE));
        loop {
            let mut line = Vec::new();
            match reader.read_until(b'\n', &mut line) {
                Ok(n) if n > 0 => {
                    if tx.send(Some(line)).is_err() {
                        return;
                    }
                }
                _ => {
                    let _ = tx.send(None);
                    return;
                }
            }
        }
    });
    rx
}

fn next_response(lines: &mpsc::Receiver<Option<Vec<u8>>>, wait: Duration) -> Result<Response> {
    match lines.recv_timeout(wait) {
        Ok(Some(line)) => serde_json::from_slice(&line).map_err(|e| {
            WalletError::validation(format!("malformed reply from the key daemon: {e}"))
        }),
        Ok(None) | Err(mpsc::RecvTimeoutError::Disconnected) => Err(WalletError::validation(
            "the key daemon exited without reporting readiness",
        )),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(WalletError::validation(format!(
            "the key daemon did not report readiness within {}s",
            wait.as_secs()
        ))),
    }
}

fn expect_stage(
    lines: &mpsc::Receiver<Option<Vec<u8>>>,
    stage: &str,
    wait: Duration,
) -> Result<()> {
    let v = next_response(lines, wait)?.into_result()?;
    if v["stage"] == json!(stage) {
        Ok(())
    } else {
        Err(WalletError::validation(format!(
            "the key daemon sent {v} where it should report '{stage}'"
        )))
    }
}

// ---- Finding daemons by the OS process table ----
//
// A socket or lock file is a name any process of this user can rename or replace, so `lock` and
// `unlock` never trust one to say which daemons exist. Every daemon is started as
// `zwallet daemon --home <canonical home>`; its argv is fixed for its life and readable from the
// process table (macOS `KERN_PROCARGS2`, Linux `/proc/<pid>/cmdline`, both readable while the
// daemon is non-dumpable), so that is how they are found and stopped.

/// `argv[0]` of every daemon this binary starts, whatever the binary file is called.
pub const DAEMON_ARG0: &str = "zwallet";
pub const DAEMON_SUBCOMMAND: &str = "daemon";
pub const HOME_FLAG: &str = "--home";
/// After SIGTERM, how long a daemon gets to wipe its seed and exit before SIGKILL.
const STOP_GRACE: Duration = Duration::from_secs(2);
/// After SIGKILL, how long the kernel gets to remove it before `lock` reports failure.
const STOP_KILL_WAIT: Duration = Duration::from_secs(3);

/// The home as written into a daemon's argv: absolute and canonical (symlinks resolved).
pub fn daemon_home(home: &Path) -> Result<PathBuf> {
    std::fs::canonicalize(home).map_err(|e| {
        WalletError::validation(format!(
            "cannot resolve the wallet home {}: {e}",
            home.display()
        ))
    })
}

/// The names a daemon's `--home` may carry for this home, and the home's identity. A daemon's
/// argv was fixed at its start, and a same-user process can have renamed or re-linked the path
/// since; any of these matches counts.
struct HomeMatch {
    names: Vec<OsString>,
    id: Option<(u64, u64)>,
}

impl HomeMatch {
    fn new(home: &Path) -> Self {
        let abs = std::path::absolute(home).unwrap_or_else(|_| home.to_path_buf());
        let mut names = vec![abs.clone().into_os_string()];
        if let Ok(c) = std::fs::canonicalize(&abs) {
            names.push(c.into_os_string());
        }
        if let (Some(parent), Some(name)) = (abs.parent(), abs.file_name())
            && let Ok(p) = std::fs::canonicalize(parent)
        {
            names.push(p.join(name).into_os_string());
        }
        let id = std::fs::metadata(&abs).ok().map(|m| (m.dev(), m.ino()));
        Self { names, id }
    }

    fn matches(&self, home: &OsStr) -> bool {
        self.names.iter().any(|n| n == home)
            || self
                .id
                .is_some_and(|id| std::fs::metadata(home).is_ok_and(|m| (m.dev(), m.ino()) == id))
    }
}

/// The `--home` of a `<any> daemon --home <home>` argv.
fn daemon_argv_home(argv: &[OsString]) -> Option<&OsStr> {
    match argv {
        [_, sub, flag, home] if sub == DAEMON_SUBCOMMAND && flag == HOME_FLAG => Some(home),
        _ => None,
    }
}

/// Pids of this user's zwallet daemons for `home`, other than this process and `keep`.
pub fn daemon_pids(home: &Path, keep: Option<i32>) -> Result<Vec<i32>> {
    let wanted = HomeMatch::new(home);
    let me = std::process::id() as i32;
    let mut pids: Vec<i32> = process_argvs()?
        .into_iter()
        .filter(|(pid, argv)| {
            *pid > 1
                && *pid != me
                && Some(*pid) != keep
                && daemon_argv_home(argv).is_some_and(|h| wanted.matches(h))
        })
        .map(|(pid, _)| pid)
        .collect();
    pids.sort_unstable();
    pids.dedup();
    Ok(pids)
}

/// Stops every daemon for `home` except `keep`: SIGTERM (plus SIGCONT, so a stopped one runs its
/// handler and wipes its seed), then SIGKILL for any still there after [`STOP_GRACE`]. Succeeds,
/// returning the pids it signalled, only once a fresh look at the process table finds none left.
pub fn stop_daemons(home: &Path, keep: Option<i32>) -> Result<Vec<i32>> {
    let start = std::time::Instant::now();
    let mut signalled: Vec<i32> = Vec::new();
    loop {
        let alive = daemon_pids(home, keep)?;
        if alive.is_empty() {
            return Ok(signalled);
        }
        let elapsed = start.elapsed();
        if elapsed >= STOP_GRACE + STOP_KILL_WAIT {
            let list = alive
                .iter()
                .map(i32::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            return Err(WalletError::validation(format!(
                "could not stop the key daemon process(es) {list} for {}; the wallet may still \
                 be unlocked",
                home.display()
            )));
        }
        for pid in alive {
            let signal = if !signalled.contains(&pid) {
                signalled.push(pid);
                libc::SIGTERM
            } else if elapsed >= STOP_GRACE {
                libc::SIGKILL
            } else {
                continue;
            };
            // SAFETY: kill(2) on a pid > 1 that is not this process (see `daemon_pids`).
            unsafe {
                libc::kill(pid, signal);
                if signal == libc::SIGTERM {
                    libc::kill(pid, libc::SIGCONT);
                }
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// `(pid, argv)` of every process of this effective uid whose argv can be read.
#[cfg(target_os = "linux")]
fn process_argvs() -> Result<Vec<(i32, Vec<OsString>)>> {
    let uid = current_uid();
    let dir = std::fs::read_dir("/proc").map_err(io_err("cannot list /proc"))?;
    let mut out = Vec::new();
    for entry in dir.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<i32>().ok())
        else {
            continue;
        };
        // `Uid:` real, effective, saved, fs. /proc/<pid>/status and cmdline stay readable for
        // a non-dumpable process (its fd directory and memory do not).
        let Ok(status) = std::fs::read_to_string(entry.path().join("status")) else {
            continue;
        };
        let euid = status
            .lines()
            .find_map(|l| l.strip_prefix("Uid:"))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|u| u.parse::<u32>().ok());
        if euid != Some(uid) {
            continue;
        }
        let Ok(cmdline) = std::fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        out.push((pid, split_nul(&cmdline, usize::MAX)));
    }
    Ok(out)
}

/// `(pid, argv)` of every process of this effective uid whose argv can be read.
#[cfg(target_os = "macos")]
fn process_argvs() -> Result<Vec<(i32, Vec<OsString>)>> {
    use std::mem::size_of;
    let uid = current_uid();
    // SAFETY: with a null buffer proc_listallpids only returns the number of pids.
    let n = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if n <= 0 {
        return Err(WalletError::validation("cannot list processes"));
    }
    // Room for processes started in between.
    let mut pids = vec![0 as libc::pid_t; n as usize + 256];
    let bytes = (pids.len() * size_of::<libc::pid_t>()) as libc::c_int;
    // SAFETY: the buffer holds `bytes` bytes of pid_t.
    let n = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes) };
    if n <= 0 {
        return Err(WalletError::validation("cannot list processes"));
    }
    pids.truncate(n as usize);
    let mut argmax: libc::c_int = 0;
    let mut len = size_of::<libc::c_int>();
    let mut mib = [libc::CTL_KERN, libc::KERN_ARGMAX];
    // SAFETY: reads one c_int into `argmax`.
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            2,
            (&mut argmax as *mut libc::c_int).cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || argmax <= 0 {
        return Err(WalletError::validation("cannot read kern.argmax"));
    }
    let mut buf = vec![0u8; argmax as usize];
    let mut out = Vec::new();
    for pid in pids.into_iter().filter(|p| *p > 0) {
        // SAFETY: proc_bsdinfo is plain old data; proc_pidinfo fills at most its size.
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = size_of::<libc::proc_bsdinfo>() as libc::c_int;
        let got = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                (&mut info as *mut libc::proc_bsdinfo).cast(),
                size,
            )
        };
        if got != size || info.pbi_uid != uid {
            continue;
        }
        if let Some(argv) = procargs(pid, &mut buf) {
            out.push((pid, argv));
        }
    }
    Ok(out)
}

/// argv of `pid` from `KERN_PROCARGS2`: `argc` (c_int), the exec path, NUL padding, then argc
/// NUL-terminated strings. `None` for a process that is gone, a zombie, or another user's.
#[cfg(target_os = "macos")]
fn procargs(pid: libc::pid_t, buf: &mut [u8]) -> Option<Vec<OsString>> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    let mut len = buf.len();
    // SAFETY: the kernel writes at most `len` bytes into `buf`.
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buf.as_mut_ptr().cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || len < 4 {
        return None;
    }
    let data = &buf[..len];
    let argc = libc::c_int::from_ne_bytes(data[..4].try_into().ok()?);
    let rest = &data[4..];
    let path_end = rest.iter().position(|b| *b == 0)?;
    let rest = &rest[path_end..];
    let args_start = rest.iter().position(|b| *b != 0)?;
    let argv = split_nul(&rest[args_start..], usize::try_from(argc).ok()?);
    (argv.len() == argc as usize).then_some(argv)
}

/// Up to `max` NUL-terminated strings from `bytes` (a missing final NUL ends the last one).
fn split_nul(bytes: &[u8], max: usize) -> Vec<OsString> {
    let mut out = Vec::new();
    let mut rest = bytes;
    while !rest.is_empty() && out.len() < max {
        let end = rest.iter().position(|b| *b == 0).unwrap_or(rest.len());
        out.push(OsStr::from_bytes(&rest[..end]).to_os_string());
        rest = rest.get(end + 1..).unwrap_or(&[]);
    }
    out
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn process_argvs() -> Result<Vec<(i32, Vec<OsString>)>> {
    Err(WalletError::validation(
        "finding key daemons by process table is only implemented for macOS and Linux",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::{AsRawFd, FromRawFd};

    #[test]
    fn request_wire_shape() {
        let r = Request::SignTx {
            chain: "ethereum".into(),
            index: 2,
            tx: serde_json::json!({"nonce": 1}),
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["op"], "sign_tx");
        assert_eq!(v["index"], 2);
        assert_eq!(
            serde_json::to_value(Request::Lock).unwrap(),
            serde_json::json!({ "op": "lock" })
        );
        // There is no unlock request any more: a password can never be sent over the socket.
        let unlock = serde_json::json!({ "op": "unlock", "password": "x", "ttlSecs": 5 });
        assert!(serde_json::from_value::<Request>(unlock).is_err());
    }

    #[test]
    fn unlock_input_round_trip() {
        let raw = encode_unlock_input(Duration::from_secs(86_400), "pa ss\nword").unwrap();
        assert_eq!(raw.as_slice(), b"86400 10\npa ss\nword");
        // Through a real pipe, as the daemon reads it, with the go byte after it.
        let mut fds = [0; 2];
        // SAFETY: pipe(2) fills two fds that are closed below.
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let mut w = unsafe { std::fs::File::from_raw_fd(fds[1]) };
        let r = unsafe { std::fs::File::from_raw_fd(fds[0]) };
        w.write_all(&raw).unwrap();
        w.write_all(&[GO]).unwrap();
        let (ttl, pw) = read_unlock_input(r.as_raw_fd()).unwrap();
        assert_eq!(ttl, Duration::from_secs(86_400));
        assert_eq!(pw.as_slice(), b"pa ss\nword");
        assert_eq!(
            pw.capacity(),
            10,
            "one allocation of exactly the password's length"
        );
        read_go(r.as_raw_fd(), Duration::from_secs(1)).unwrap();
        drop(w);
        assert!(read_go(r.as_raw_fd(), Duration::from_secs(1)).is_err());
        for bad in [
            &b"x 2"[..],
            b"0 2",
            b" 2",
            b"10",
            b"10 0",
            b"-1 2",
            b"10 2 3",
            b"10 99999",
        ] {
            let err = decode_unlock_header(bad).unwrap_err();
            assert_eq!(err.code, ErrorCode::ValidationError, "{bad:?}");
        }
        assert!(encode_unlock_input(Duration::from_secs(1), "").is_err());
    }

    #[test]
    fn long_socket_path_is_a_validation_error() {
        let long = PathBuf::from(format!("/tmp/{}", "x".repeat(SUN_PATH_MAX)));
        let err = socket_path(&long).unwrap_err();
        assert_eq!(err.code, ErrorCode::ValidationError);
        assert!(err.message.contains("sun_path"));
        assert!(socket_path(Path::new("/tmp/zw")).is_ok());
    }

    #[test]
    fn daemon_argv_shape() {
        let argv = |v: &[&str]| v.iter().map(OsString::from).collect::<Vec<_>>();
        assert_eq!(
            daemon_argv_home(&argv(&["zwallet", "daemon", "--home", "/h"])),
            Some(OsStr::new("/h"))
        );
        for bad in [
            &["zwallet", "daemon"][..],
            &["zwallet", "daemon", "--home"],
            &["zwallet", "lock", "--home", "/h"],
            &["zwallet", "daemon", "--home", "/h", "x"],
        ] {
            assert_eq!(daemon_argv_home(&argv(bad)), None, "{bad:?}");
        }
        assert_eq!(
            split_nul(b"a\0bc\0\0d", usize::MAX),
            argv(&["a", "bc", "", "d"])
        );
        assert_eq!(split_nul(b"a\0b\0c\0", 2), argv(&["a", "b"]));
    }

    #[test]
    fn this_process_shows_up_in_the_process_table() {
        let me = std::process::id() as i32;
        let all = process_argvs().unwrap();
        let mine = all.iter().find(|(p, _)| *p == me).expect("own pid listed");
        assert!(!mine.1.is_empty(), "own argv read");
        // ...but never as a daemon of any home.
        assert!(daemon_pids(Path::new("/"), None).unwrap().is_empty());
    }

    #[test]
    fn response_round_trip() {
        let e = WalletError::locked();
        let r: Response =
            serde_json::from_value(serde_json::to_value(Response::err(&e)).unwrap()).unwrap();
        assert_eq!(r.into_result().unwrap_err().code, ErrorCode::WalletLocked);
    }
}
