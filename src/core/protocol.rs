//! Daemon wire protocol and client.
//!
//! One JSON request line and one JSON response line per connection over `$ZWALLET_HOME/d.sock`
//! (0600 inside the 0700 home; peer uid checked on both ends). Requests are family-agnostic:
//! `chain` picks the family, `index` the key, and `tx` is the family's own unsigned-tx JSON.
//!
//! No request carries a secret. The password never travels over the socket: [`start_daemon`]
//! hands it to the daemon child it spawns over one end of a socketpair, the child's fds 0 and 1
//! (a socket, unlike a pipe, cannot be reopened through `/proc/<pid>/fd`), and reads the child's
//! progress on it. Whatever else listens on `d.sock` (a stale socket, or a squatter run by the
//! same user) can learn nothing from a client and is replaced by the new daemon, which binds under
//! an exclusive lock on `d.lock`.
//!
//! Which daemons exist is never read from `d.sock` or `d.lock` (any same-user process can rename
//! or replace those): they are found by argv in the OS process table and stopped by signal.
//! `lock` stops every daemon of this user ([`stop_all_daemons`]) without reading any home, so no
//! path there can stall or fail it; `unlock` stops the ones for its home ([`stop_daemons`]).

use crate::core::error::{ErrorCode, Result, WalletError};
use crate::core::password::{MAX_SECRET, read_exact_secret};
use crate::core::store;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::net::Shutdown;
use std::os::fd::{OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::net::UnixStream as ChildChannel;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};
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

/// Asks whatever answers on `d.sock` to lock; the answer decides nothing. Only `unlock` does
/// this (it reads the home anyway); `zwallet lock` touches no home ([`stop_all_daemons`]).
async fn polite_lock(home: &Path) {
    let _ = send(home, &Request::Lock, QUICK).await;
}

/// Where Linux's Yama LSM publishes its ptrace policy.
pub const YAMA_PTRACE_SCOPE: &str = "/proc/sys/kernel/yama/ptrace_scope";

/// Before a daemon is started (and before the password is asked for): on Linux, refuses where
/// any same-user process may ptrace it, i.e. without Yama or with `kernel.yama.ptrace_scope` 0,
/// unless `allow_scope_0`. From exec until its first line makes it non-dumpable, the daemon (like
/// any new process) can be attached to, and a tracer attached then stays attached when the
/// password arrives. Elsewhere this checks nothing.
pub fn check_ptrace_scope(allow_scope_0: bool) -> Result<()> {
    #[cfg(target_os = "linux")]
    return ptrace_scope_check(Path::new(YAMA_PTRACE_SCOPE), allow_scope_0);
    #[cfg(not(target_os = "linux"))]
    {
        let _ = allow_scope_0;
        Ok(())
    }
}

/// [`check_ptrace_scope`] reading the policy from `path`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn ptrace_scope_check(path: &Path, allow_scope_0: bool) -> Result<()> {
    let scope = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok());
    if allow_scope_0 || scope.is_some_and(|s| s >= 1) {
        return Ok(());
    }
    let found = match scope {
        Some(0) => "kernel.yama.ptrace_scope is 0".to_string(),
        _ => format!(
            "{path} is missing or unreadable (no Yama)",
            path = path.display()
        ),
    };
    Err(WalletError::validation(format!(
        "{found}: any same-user process can attach to the daemon during start-up and read the \
         password; set kernel.yama.ptrace_scope ≥ 1 (sysctl), or pass --allow-ptrace-scope-0 to \
         accept that risk"
    )))
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

/// The binary `unlock` starts as the daemon, pinned before the password prompt. On Linux that is
/// the image this process runs (`/proc/self/exe`), so a binary swapped in at that path meanwhile
/// is never started. Elsewhere it is the canonical path of `current_exe()`, whose (dev, ino) is
/// recorded now and checked again right before the spawn: that refuses a binary swapped in while
/// the human types, but not one swapped in between that check and the exec (macOS has no
/// `fexecve`). A binary swapped in before `unlock` started gets the password either way; the
/// README's "Known limits" says where to install zwallet so an agent cannot write it.
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
/// home>`, no environment, no descriptor but its channel), started from `exe`. Its fds 0 and 1
/// are one end of a socketpair whose other end only this process holds. Over that channel:
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
    let (mut child, chan) = spawn_daemon(exe, &canonical)?;
    let ready = drive_daemon(home, &mut child, chan, password, ttl).await;
    if ready.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    ready
}

/// Spawns the daemon with one end of a socketpair as its fds 0 and 1 and returns the other end.
/// A pipe there could be reopened by any same-user process through `/proc/<pid>/fd/0` or `/1`
/// until the child turns non-dumpable, to read the password or to write a fake readiness line;
/// opening a socket that way fails (ENXIO).
fn spawn_daemon(exe: &DaemonExe, canonical_home: &Path) -> Result<(Child, ChildChannel)> {
    let fail = io_err("cannot start the key daemon");
    let (ours, theirs) = ChildChannel::pair().map_err(&fail)?;
    let theirs_out = theirs.try_clone().map_err(&fail)?;
    let mut fds = OpenFds::prepare()?;
    let mut cmd = Command::new(&exe.path);
    cmd.arg0(DAEMON_ARG0)
        .arg(DAEMON_SUBCOMMAND)
        .arg(HOME_FLAG)
        .arg(canonical_home)
        .env_clear()
        .current_dir("/")
        .stdin(Stdio::from(OwnedFd::from(theirs)))
        .stdout(Stdio::from(OwnedFd::from(theirs_out)))
        .stderr(Stdio::null());
    // SAFETY: runs between fork and exec and only calls async-signal-safe functions (setsid,
    // syscall, open, fcntl, close, proc_pidinfo) on memory allocated before the fork; the
    // channel is already on 0 and 1.
    unsafe {
        cmd.pre_exec(move || {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            cloexec_from(3, &mut fds)
        });
    }
    let child = cmd.spawn().map_err(&fail);
    // The command holds this process's copies of the child's end: dropped here, so that the
    // child's exit reads as EOF on `ours`.
    drop(cmd);
    Ok((child?, ours))
}

/// Marks every open descriptor from `first` up close-on-exec, so the daemon starts with only its
/// channel (and /dev/null on fd 2) whatever this process inherited: Linux `close_range`, else
/// the descriptors actually open (not a scan up to the soft `RLIMIT_NOFILE`, which misses one
/// opened above a limit lowered later). A failure to mark one fails the spawn.
/// Async-signal-safe.
fn cloexec_from(first: libc::c_int, fds: &mut OpenFds) -> std::io::Result<()> {
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
            return Ok(());
        }
        // Before Linux 5.11, or under a seccomp filter: list the open ones instead.
    }
    fds.for_each(|fd| if fd >= first { set_cloexec(fd) } else { Ok(()) })
}

/// Sets `FD_CLOEXEC` on `fd`; a descriptor closed meanwhile is fine. Async-signal-safe.
fn set_cloexec(fd: libc::c_int) -> std::io::Result<()> {
    // SAFETY: fcntl(2) on a descriptor number only reads or changes its flags.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFD);
        if flags == -1 {
            let e = std::io::Error::last_os_error();
            return if e.raw_os_error() == Some(libc::EBADF) {
                Ok(())
            } else {
                Err(e)
            };
        }
        if flags & libc::FD_CLOEXEC == 0
            && libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) == -1
        {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Lists this process's open descriptors where nothing may allocate (in the child, between fork
/// and exec): the buffer is allocated by [`OpenFds::prepare`] before the fork.
struct OpenFds {
    #[cfg(target_os = "macos")]
    buf: Vec<libc::proc_fdinfo>,
    #[cfg(not(target_os = "macos"))]
    buf: Vec<u8>,
}

impl OpenFds {
    #[cfg(target_os = "macos")]
    fn prepare() -> Result<Self> {
        // SAFETY: with a null buffer proc_pidinfo returns the size the list needs now.
        let need = unsafe {
            libc::proc_pidinfo(
                libc::getpid(),
                libc::PROC_PIDLISTFDS,
                0,
                std::ptr::null_mut(),
                0,
            )
        };
        if need <= 0 {
            return Err(WalletError::validation(
                "cannot list this process's open descriptors",
            ));
        }
        // Room for descriptors opened before the fork.
        let n = need as usize / std::mem::size_of::<libc::proc_fdinfo>() + 64;
        let empty = libc::proc_fdinfo {
            proc_fd: 0,
            proc_fdtype: 0,
        };
        Ok(Self {
            buf: vec![empty; n],
        })
    }

    #[cfg(not(target_os = "macos"))]
    fn prepare() -> Result<Self> {
        Ok(Self { buf: vec![0; 8192] })
    }

    /// Calls `f` with each open descriptor. Async-signal-safe; an error from `f` stops the walk.
    #[cfg(target_os = "macos")]
    fn for_each(
        &mut self,
        mut f: impl FnMut(libc::c_int) -> std::io::Result<()>,
    ) -> std::io::Result<()> {
        let entry = std::mem::size_of::<libc::proc_fdinfo>();
        let size = libc::c_int::try_from(self.buf.len() * entry).unwrap_or(libc::c_int::MAX);
        // SAFETY: the kernel writes at most `size` bytes of proc_fdinfo into `buf`.
        let got = unsafe {
            libc::proc_pidinfo(
                libc::getpid(),
                libc::PROC_PIDLISTFDS,
                0,
                self.buf.as_mut_ptr().cast(),
                size,
            )
        };
        if got <= 0 {
            return Err(std::io::Error::last_os_error());
        }
        if got >= size {
            // A full buffer may have been cut short.
            return Err(std::io::Error::from_raw_os_error(libc::EMFILE));
        }
        for info in &self.buf[..got as usize / entry] {
            f(info.proc_fd)?;
        }
        Ok(())
    }

    /// Calls `f` with each open descriptor, from `/proc/self/fd` read with raw getdents64.
    /// Async-signal-safe; an error from `f` stops the walk.
    #[cfg(target_os = "linux")]
    fn for_each(
        &mut self,
        mut f: impl FnMut(libc::c_int) -> std::io::Result<()>,
    ) -> std::io::Result<()> {
        // SAFETY: open(2) of a NUL-terminated literal.
        let dir = unsafe {
            libc::open(
                c"/proc/self/fd".as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        };
        if dir < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let result = (|| loop {
            // SAFETY: the kernel writes at most `buf.len()` bytes of dirent64 records.
            let n = unsafe {
                libc::syscall(
                    libc::SYS_getdents64,
                    dir,
                    self.buf.as_mut_ptr(),
                    self.buf.len(),
                )
            };
            if n < 0 {
                return Err(std::io::Error::last_os_error());
            }
            if n == 0 {
                return Ok(());
            }
            // linux_dirent64: d_ino u64, d_off i64, d_reclen u16, d_type u8, d_name (NUL-ended).
            let mut at = 0usize;
            while at < n as usize {
                let bad = || std::io::Error::from_raw_os_error(libc::EIO);
                let rec = self.buf.get(at..).ok_or_else(bad)?;
                let reclen = match (rec.get(16), rec.get(17)) {
                    (Some(lo), Some(hi)) => u16::from_ne_bytes([*lo, *hi]) as usize,
                    _ => return Err(bad()),
                };
                let name = rec.get(19..reclen).ok_or_else(bad)?;
                if let Some(fd) = parse_fd(name)
                    && fd != dir
                {
                    f(fd)?;
                }
                at += reclen;
            }
        })();
        // SAFETY: closes the directory opened above.
        unsafe { libc::close(dir) };
        result
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn for_each(
        &mut self,
        _f: impl FnMut(libc::c_int) -> std::io::Result<()>,
    ) -> std::io::Result<()> {
        Err(std::io::Error::from_raw_os_error(libc::ENOSYS))
    }
}

/// A descriptor number from a NUL-terminated `/proc/self/fd` entry name (`.` and `..` are
/// `None`). Async-signal-safe.
#[cfg(target_os = "linux")]
fn parse_fd(name: &[u8]) -> Option<libc::c_int> {
    let mut fd: libc::c_int = 0;
    let mut digits = 0;
    for &b in name {
        if b == 0 {
            break;
        }
        if !b.is_ascii_digit() {
            return None;
        }
        fd = fd
            .checked_mul(10)?
            .checked_add(libc::c_int::from(b - b'0'))?;
        digits += 1;
    }
    (digits > 0).then_some(fd)
}

async fn drive_daemon(
    home: &Path,
    child: &mut Child,
    chan: ChildChannel,
    password: &str,
    ttl: Duration,
) -> Result<Value> {
    let lines = line_reader(
        chan.try_clone()
            .map_err(io_err("cannot read from the key daemon"))?,
    );
    expect_stage(&lines, STAGE_INPUT, INPUT_TIMEOUT)?;
    // Unbuffered socket writes: no intermediate copy of the password in this process.
    (&chan)
        .write_all(&encode_unlock_input(ttl, password)?)
        .map_err(io_err("cannot hand the password to the key daemon"))?;
    expect_stage(&lines, STAGE_DECRYPTED, UNLOCK_TIMEOUT)?;
    // The password is right: only now is the running session replaced.
    polite_lock(home).await;
    stop_daemons(home, Some(child.id() as i32))?;
    (&chan)
        .write_all(&[GO])
        .map_err(io_err("cannot start the key daemon"))?;
    let _ = chan.shutdown(Shutdown::Write);
    next_response(&lines, BIND_TIMEOUT)?.into_result()
}

/// Lines from the daemon's end of the channel, read on a thread so each can be awaited with a
/// timeout. `None` once it closed.
fn line_reader(chan: ChildChannel) -> mpsc::Receiver<Option<Vec<u8>>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(chan.take(4 * MAX_MESSAGE));
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
// A socket or lock file is a name any process of this user can rename or replace, so `lock`,
// `unlock` and `status` never trust one to say which daemons exist. Every daemon is started as
// `zwallet daemon --home <canonical home>`; its argv is fixed for its life and readable from the
// process table (macOS `KERN_PROCARGS2`, Linux `/proc/<pid>/cmdline`, both readable while the
// daemon is non-dumpable), so that is how they are found and stopped. Only an absolute `--home`
// counts: every daemon zwallet starts has one.

/// `argv[0]` of every daemon this binary starts, whatever the binary file is called.
pub const DAEMON_ARG0: &str = "zwallet";
pub const DAEMON_SUBCOMMAND: &str = "daemon";
pub const HOME_FLAG: &str = "--home";
/// After SIGTERM, how long a daemon gets to wipe its seed and exit before SIGKILL.
const STOP_GRACE: Duration = Duration::from_secs(2);
/// After SIGKILL, how long the kernel gets to remove it before `lock` reports failure.
const STOP_KILL_WAIT: Duration = Duration::from_secs(3);
/// How long a lookup waits for the stat of other processes' `--home` paths, all together. A path
/// on a FUSE mount can hang a stat; an answer that does not come in time is no match.
const STAT_WAIT: Duration = Duration::from_secs(1);

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
    /// Identity verdicts already reached per argv path, so that repeated looks at the process
    /// table (while stopping daemons) stat each path once.
    same_dir: RefCell<BTreeMap<OsString, bool>>,
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
        Self {
            names,
            id,
            same_dir: RefCell::new(BTreeMap::new()),
        }
    }

    /// The pids among `daemons` (pid, absolute argv home) that serve this home. Names are
    /// compared first, and only the rest are stat'ed for the home's identity, each on its own
    /// thread and for [`STAT_WAIT`] at most, so another process's argv cannot hang the lookup.
    fn select(&self, daemons: &[(i32, OsString)]) -> Vec<i32> {
        let (named, rest): (Vec<_>, Vec<_>) = daemons
            .iter()
            .partition(|(_, home)| self.names.contains(home));
        let mut pids: Vec<i32> = named.iter().map(|(pid, _)| *pid).collect();
        let Some(id) = self.id else {
            return pids;
        };
        let unseen: Vec<OsString> = {
            let seen = self.same_dir.borrow();
            let mut v: Vec<OsString> = rest
                .iter()
                .map(|(_, home)| home.clone())
                .filter(|home| !seen.contains_key(home))
                .collect();
            v.sort();
            v.dedup();
            v
        };
        self.stat_bounded(unseen, id);
        let seen = self.same_dir.borrow();
        pids.extend(
            rest.iter()
                .filter(|(_, home)| seen.get(home) == Some(&true))
                .map(|(pid, _)| *pid),
        );
        pids
    }

    /// Records, for each path, whether it is the directory `id`; a stat that does not answer
    /// within [`STAT_WAIT`] (its thread is left behind) records no match.
    fn stat_bounded(&self, paths: Vec<OsString>, id: (u64, u64)) {
        if paths.is_empty() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let mut seen = self.same_dir.borrow_mut();
        for path in paths {
            seen.insert(path.clone(), false);
            let tx = tx.clone();
            let _ = std::thread::Builder::new().spawn(move || {
                let same = std::fs::metadata(&path).is_ok_and(|m| (m.dev(), m.ino()) == id);
                let _ = tx.send((path, same));
            });
        }
        drop(tx);
        let deadline = Instant::now() + STAT_WAIT;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match rx.recv_timeout(left) {
                Ok((path, same)) => {
                    seen.insert(path, same);
                }
                // Every thread answered, or the time is up.
                Err(_) => break,
            }
        }
    }
}

/// The `--home` of a `<any> daemon --home <home>` argv.
fn daemon_argv_home(argv: &[OsString]) -> Option<&OsStr> {
    match argv {
        [_, sub, flag, home] if sub == DAEMON_SUBCOMMAND && flag == HOME_FLAG => Some(home),
        _ => None,
    }
}

/// `(pid, --home)` of every zwallet daemon of this user, other than this process and `keep`:
/// any process whose argv is `<any> daemon --home <absolute path>`.
fn daemon_processes(keep: Option<i32>) -> Result<Vec<(i32, OsString)>> {
    let me = std::process::id() as i32;
    Ok(process_argvs()?
        .into_iter()
        .filter(|(pid, _)| *pid > 1 && *pid != me && Some(*pid) != keep)
        .filter_map(|(pid, argv)| {
            let home = daemon_argv_home(&argv)?;
            Path::new(home)
                .is_absolute()
                .then(|| (pid, home.to_os_string()))
        })
        .collect())
}

fn sorted(mut pids: Vec<i32>) -> Vec<i32> {
    pids.sort_unstable();
    pids.dedup();
    pids
}

/// Pids of every zwallet daemon of this user, whatever home it serves.
pub fn all_daemon_pids() -> Result<Vec<i32>> {
    Ok(sorted(
        daemon_processes(None)?
            .into_iter()
            .map(|(pid, _)| pid)
            .collect(),
    ))
}

/// Pids of this user's zwallet daemons for `home`, other than this process and `keep`.
pub fn daemon_pids(home: &Path, keep: Option<i32>) -> Result<Vec<i32>> {
    home_daemons(&HomeMatch::new(home), keep)
}

fn home_daemons(wanted: &HomeMatch, keep: Option<i32>) -> Result<Vec<i32>> {
    Ok(sorted(wanted.select(&daemon_processes(keep)?)))
}

/// Pids of this user's zwallet daemons for any home but `home` (`zwallet lock` stops them too):
/// one look at the process table, split by the same match as [`daemon_pids`], so a daemon of
/// this home is never counted as another's.
pub fn other_daemon_pids(home: &Path) -> Result<Vec<i32>> {
    let all = daemon_processes(None)?;
    let this_home = HomeMatch::new(home).select(&all);
    Ok(sorted(
        all.into_iter()
            .map(|(pid, _)| pid)
            .filter(|pid| !this_home.contains(pid))
            .collect(),
    ))
}

/// Stops every daemon for `home` except `keep` (`unlock` replacing a session); see
/// [`stop_until_gone`].
pub fn stop_daemons(home: &Path, keep: Option<i32>) -> Result<Vec<i32>> {
    let wanted = HomeMatch::new(home);
    stop_until_gone(
        || home_daemons(&wanted, keep),
        &format!(" for {}", home.display()),
    )
}

/// Stops every zwallet daemon of this user, whatever home it serves (`zwallet lock`); see
/// [`stop_until_gone`].
pub fn stop_all_daemons() -> Result<Vec<i32>> {
    stop_until_gone(all_daemon_pids, "")
}

/// Stops the daemons `alive` lists: SIGTERM (plus SIGCONT, so a stopped one runs its handler and
/// wipes its seed), then SIGKILL for any still there after [`STOP_GRACE`]. Succeeds, returning
/// the pids it signalled, only once a fresh look at the process table finds none left.
fn stop_until_gone(mut alive: impl FnMut() -> Result<Vec<i32>>, scope: &str) -> Result<Vec<i32>> {
    let start = Instant::now();
    let mut signalled: Vec<i32> = Vec::new();
    loop {
        let alive = alive()?;
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
                "could not stop the key daemon process(es) {list}{scope}; the wallet may still \
                 be unlocked"
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
            // SAFETY: kill(2) on a pid > 1 that is not this process (see `daemon_processes`).
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
    let proc = Path::new("/proc");
    let dir = std::fs::read_dir(proc).map_err(io_err("cannot list /proc"))?;
    proc_argvs(
        proc,
        dir.map(|entry| entry.map(|e| e.file_name())),
        current_uid(),
        |path: &Path| std::fs::read(path),
    )
}

/// [`process_argvs`] over the entry `names` of a `/proc` at `root`, reading its files with
/// `read`. An error from the listing itself fails the whole lookup, as a full pid buffer does on
/// macOS (`list_pids`): a listing cut short may miss a daemon, which `lock` would then leave
/// running and `status` would not report. A pid whose files cannot be read because it exited
/// (ENOENT, ESRCH) or belongs to another user (EACCES; EPERM where `/proc` is mounted with
/// `hidepid`) is skipped; any other error reading them fails the lookup too.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn proc_argvs(
    root: &Path,
    names: impl Iterator<Item = std::io::Result<OsString>>,
    uid: u32,
    read: impl Fn(&Path) -> std::io::Result<Vec<u8>>,
) -> Result<Vec<(i32, Vec<OsString>)>> {
    let read_pid_file = |name: &OsStr, file: &str| -> Result<Option<Vec<u8>>> {
        let path = root.join(name).join(file);
        match read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if pid_gone_or_foreign(&e) => Ok(None),
            Err(e) => Err(WalletError::validation(format!(
                "cannot read {}: {e}; not trusting a partial list of key daemons",
                path.display()
            ))),
        }
    };
    let mut out = Vec::new();
    for name in names {
        let name = name.map_err(|e| {
            WalletError::validation(format!(
                "cannot list {}: {e}; not trusting a partial list of key daemons",
                root.display()
            ))
        })?;
        let Some(pid) = name.to_str().and_then(|n| n.parse::<i32>().ok()) else {
            continue;
        };
        // /proc/<pid>/status and cmdline stay readable for a non-dumpable process (its fd
        // directory and memory do not).
        let Some(status) = read_pid_file(&name, "status")? else {
            continue;
        };
        if status_euid(&status) != Some(uid) {
            continue;
        }
        let Some(cmdline) = read_pid_file(&name, "cmdline")? else {
            continue;
        };
        out.push((pid, split_nul(&cmdline, usize::MAX)));
    }
    Ok(out)
}

/// Whether reading a `/proc/<pid>` file failed because the process is gone or is another user's,
/// rather than because the listing cannot be trusted.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn pid_gone_or_foreign(e: &std::io::Error) -> bool {
    matches!(
        e.raw_os_error(),
        Some(libc::ENOENT | libc::ESRCH | libc::EACCES | libc::EPERM)
    )
}

/// The effective uid in a `/proc/<pid>/status` (`Uid:` real, effective, saved, fs). Read as
/// bytes: the `Name:` line above it is the process's own comm, which need not be UTF-8.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn status_euid(status: &[u8]) -> Option<u32> {
    let line = status
        .split(|b| *b == b'\n')
        .find_map(|l| l.strip_prefix(b"Uid:"))?;
    std::str::from_utf8(line)
        .ok()?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
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
    let first = n as usize + 256;
    let pids = list_pids(&[first, first * 2])?;
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

/// Every pid, from `proc_listallpids` into a buffer of each of `capacities` pids in turn. The
/// kernel fills at most the buffer and says how many it wrote, so a buffer filled to its last
/// slot may have been cut short (as `OpenFds` treats a full fd list): it is tried again with the
/// next capacity, and a list that fills even the last one is an error, never a shorter list (a
/// daemon missing from it would survive `lock`).
#[cfg(target_os = "macos")]
fn list_pids(capacities: &[usize]) -> Result<Vec<libc::pid_t>> {
    let fail = || WalletError::validation("cannot list processes");
    for &cap in capacities {
        let mut pids = vec![0 as libc::pid_t; cap];
        let bytes = cap
            .checked_mul(std::mem::size_of::<libc::pid_t>())
            .and_then(|b| libc::c_int::try_from(b).ok())
            .ok_or_else(fail)?;
        // SAFETY: the buffer holds `bytes` bytes of pid_t.
        let n = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes) };
        let Ok(n) = usize::try_from(n) else {
            return Err(fail());
        };
        if n == 0 {
            return Err(fail());
        }
        if n < cap {
            pids.truncate(n);
            return Ok(pids);
        }
    }
    Err(WalletError::validation(
        "the process list filled every buffer it was read into and may be incomplete; not \
         trusting a partial list of key daemons",
    ))
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

    #[cfg(target_os = "macos")]
    #[test]
    fn a_full_pid_buffer_is_retried_then_refused_never_cut_short() {
        // There is always more than one process, so a one-slot buffer always fills: a list that
        // fills its buffer may be cut short and must never be used as it is.
        let err = list_pids(&[1]).unwrap_err();
        assert_eq!(err.code, ErrorCode::ValidationError);
        assert!(err.message.contains("process list"), "{}", err.message);
        let err = list_pids(&[1, 2]).unwrap_err();
        assert!(err.message.contains("process list"), "{}", err.message);
        // Retried with room, it lists every process, this one included.
        let pids = list_pids(&[1, 1 << 16]).unwrap();
        assert!(pids.contains(&(std::process::id() as libc::pid_t)));
        assert!(pids.len() > 1);
    }

    const FAKE_PROC: &str = "/fake-proc";
    const ME: u32 = 1001;
    const DAEMON_A: &[u8] = b"/usr/bin/zwallet\0daemon\0--home\0/h/a\0";

    /// A `/proc` for [`proc_argvs`]: `<pid>/<file>` to its bytes or an errno. Any other path is
    /// ENOENT, as for a pid that exited after it was listed.
    fn fake_proc(
        files: Vec<(&str, std::result::Result<Vec<u8>, i32>)>,
    ) -> impl Fn(&Path) -> std::io::Result<Vec<u8>> {
        let files: BTreeMap<PathBuf, std::result::Result<Vec<u8>, i32>> = files
            .into_iter()
            .map(|(rel, content)| (Path::new(FAKE_PROC).join(rel), content))
            .collect();
        move |path: &Path| match files.get(path) {
            Some(Ok(bytes)) => Ok(bytes.clone()),
            Some(Err(errno)) => Err(std::io::Error::from_raw_os_error(*errno)),
            None => Err(std::io::Error::from_raw_os_error(libc::ENOENT)),
        }
    }

    fn proc_status(uid: u32) -> Vec<u8> {
        format!("Name:\tzwallet\nState:\tS (sleeping)\nUid:\t{uid}\t{uid}\t{uid}\t{uid}\nGid:\t9\n")
            .into_bytes()
    }

    fn listing(names: &[&str]) -> std::vec::IntoIter<std::io::Result<OsString>> {
        names
            .iter()
            .map(|n| Ok(OsString::from(n)))
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn a_proc_listing_yields_every_process_of_this_uid() {
        let mut odd_name = b"Name:\t\xff\xfe\n".to_vec();
        odd_name.extend_from_slice(&proc_status(ME)[b"Name:\tzwallet\n".len()..]);
        let read = fake_proc(vec![
            ("1/status", Ok(proc_status(0))),
            ("1/cmdline", Ok(b"/sbin/init\0".to_vec())),
            ("40/status", Ok(proc_status(ME))),
            ("40/cmdline", Ok(DAEMON_A.to_vec())),
            // A comm that is not UTF-8 does not hide the process.
            ("41/status", Ok(odd_name)),
            ("41/cmdline", Ok(b"bash\0-l\0".to_vec())),
            // A zombie: no argv.
            ("42/status", Ok(proc_status(ME))),
            ("42/cmdline", Ok(Vec::new())),
            // Another user's daemon (the effective uid counts, not the real one).
            ("50/status", Ok(b"Uid:\t1001\t2000\t2000\t2000\n".to_vec())),
            (
                "50/cmdline",
                Ok(b"/usr/bin/zwallet\0daemon\0--home\0/h/b\0".to_vec()),
            ),
        ]);
        let names = listing(&["self", "1", "40", "acpi", "41", "42", "50", "thread-self"]);
        let got = proc_argvs(Path::new(FAKE_PROC), names, ME, read).unwrap();
        let argv = |v: &[&str]| v.iter().map(OsString::from).collect::<Vec<_>>();
        assert_eq!(
            got,
            vec![
                (40, argv(&["/usr/bin/zwallet", "daemon", "--home", "/h/a"])),
                (41, argv(&["bash", "-l"])),
                (42, argv(&[])),
            ]
        );
    }

    #[test]
    fn a_pid_that_exited_or_is_another_users_is_skipped() {
        let read = fake_proc(vec![
            // 60 exited before its status was read: it has no files (ENOENT).
            // 61 exited between its status and its cmdline.
            ("61/status", Ok(proc_status(ME))),
            ("62/status", Err(libc::ESRCH)),
            ("63/status", Ok(proc_status(ME))),
            ("63/cmdline", Err(libc::ESRCH)),
            // Another user's, closed to this one (EPERM is what `hidepid` answers).
            ("64/status", Err(libc::EACCES)),
            ("65/status", Err(libc::EPERM)),
            ("70/status", Ok(proc_status(ME))),
            ("70/cmdline", Ok(DAEMON_A.to_vec())),
        ]);
        let names = listing(&["60", "61", "62", "63", "64", "65", "70"]);
        let got = proc_argvs(Path::new(FAKE_PROC), names, ME, read).unwrap();
        assert_eq!(got, vec![(70, split_nul(DAEMON_A, usize::MAX))]);
    }

    #[test]
    fn a_proc_listing_error_fails_the_lookup_never_cuts_it_short() {
        let read = fake_proc(vec![
            ("70/status", Ok(proc_status(ME))),
            ("70/cmdline", Ok(DAEMON_A.to_vec())),
            ("71/status", Ok(proc_status(ME))),
            ("71/cmdline", Ok(DAEMON_A.to_vec())),
        ]);
        // First, between two daemons, and after both.
        for at in 0..=2 {
            let mut names: Vec<std::io::Result<OsString>> = listing(&["70", "71"]).collect();
            names.insert(at, Err(std::io::Error::from_raw_os_error(libc::EIO)));
            let err = proc_argvs(Path::new(FAKE_PROC), names.into_iter(), ME, &read).unwrap_err();
            assert_eq!(err.code, ErrorCode::ValidationError);
            assert!(
                err.message.contains("cannot list /fake-proc")
                    && err.message.contains("not trusting a partial list"),
                "{at}: {}",
                err.message
            );
        }
        assert_eq!(
            proc_argvs(Path::new(FAKE_PROC), listing(&["70", "71"]), ME, &read)
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn an_unexpected_error_reading_a_pid_fails_the_lookup() {
        for (rel, errno) in [
            ("70/status", libc::EIO),
            ("70/cmdline", libc::EIO),
            ("70/cmdline", libc::ENOMEM),
        ] {
            let mut files = vec![
                ("70/status", Ok(proc_status(ME))),
                ("70/cmdline", Ok(DAEMON_A.to_vec())),
            ];
            files.retain(|(r, _)| *r != rel);
            files.push((rel, Err(errno)));
            let names = listing(&["70"]);
            let err = proc_argvs(Path::new(FAKE_PROC), names, ME, fake_proc(files)).unwrap_err();
            assert_eq!(err.code, ErrorCode::ValidationError);
            assert!(
                err.message
                    .contains(&format!("cannot read /fake-proc/{rel}"))
                    && err.message.contains("not trusting a partial list"),
                "{}",
                err.message
            );
        }
    }

    #[test]
    fn ptrace_scope_check_reads_the_given_path() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("ptrace_scope");
        let missing = ptrace_scope_check(&path, false).unwrap_err();
        assert_eq!(missing.code, ErrorCode::ValidationError);
        assert!(
            missing.message.contains("missing") && missing.message.contains("ptrace_scope ≥ 1"),
            "{}",
            missing.message
        );
        ptrace_scope_check(&path, true).unwrap();
        for (content, ok) in [
            ("0\n", false),
            ("garbage\n", false),
            ("1\n", true),
            ("2\n", true),
            ("3", true),
        ] {
            std::fs::write(&path, content).unwrap();
            assert_eq!(ptrace_scope_check(&path, false).is_ok(), ok, "{content:?}");
            ptrace_scope_check(&path, true).unwrap();
        }
        std::fs::write(&path, "0\n").unwrap();
        let zero = ptrace_scope_check(&path, false).unwrap_err().message;
        assert!(
            zero.contains("ptrace_scope is 0")
                && zero.contains("attach to the daemon during start-up")
                && zero.contains("--allow-ptrace-scope-0"),
            "{zero}"
        );
    }

    #[test]
    fn open_descriptors_are_listed_and_marked_close_on_exec() {
        // A pipe without FD_CLOEXEC, as a parent could leak into zwallet.
        let mut fds = [0; 2];
        // SAFETY: pipe(2) fills two fds that are closed below.
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let (r, w) = unsafe {
            (
                std::fs::File::from_raw_fd(fds[0]),
                std::fs::File::from_raw_fd(fds[1]),
            )
        };
        let mut seen = Vec::with_capacity(4096);
        let mut open = OpenFds::prepare().unwrap();
        open.for_each(|fd| {
            if seen.len() < seen.capacity() {
                seen.push(fd);
            }
            Ok(())
        })
        .unwrap();
        assert!(
            seen.contains(&0) && seen.contains(&r.as_raw_fd()),
            "{seen:?}"
        );
        assert!(seen.contains(&w.as_raw_fd()), "{seen:?}");
        // An error from the callback stops the walk and comes back.
        let err = open
            .for_each(|_| Err(std::io::Error::from_raw_os_error(libc::EPERM)))
            .unwrap_err();
        assert_eq!(err.raw_os_error(), Some(libc::EPERM));

        let flags = |fd| unsafe { libc::fcntl(fd, libc::F_GETFD) };
        assert_eq!(flags(w.as_raw_fd()) & libc::FD_CLOEXEC, 0);
        set_cloexec(w.as_raw_fd()).unwrap();
        assert_ne!(flags(w.as_raw_fd()) & libc::FD_CLOEXEC, 0);
        // A closed descriptor is fine; a failing F_SETFD would not be.
        let closed = w.as_raw_fd();
        drop(w);
        set_cloexec(closed).unwrap();
        drop(r);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_pipe_reopens_through_proc_but_the_socket_channel_does_not() {
        // Why the daemon's channel is a socketpair: any same-user process may open
        // /proc/<pid>/fd/N of a dumpable process, which for a pipe yields a fresh read or write
        // end of it (the password, or a fake readiness line); for a socket it fails with ENXIO.
        let (reader, writer) = std::io::pipe().unwrap();
        let reopened =
            std::fs::File::open(format!("/proc/self/fd/{}", reader.as_raw_fd())).map(|_| ());
        assert!(reopened.is_ok(), "{reopened:?}");
        drop((reader, writer));
        let (ours, theirs) = ChildChannel::pair().unwrap();
        for fd in [ours.as_raw_fd(), theirs.as_raw_fd()] {
            let err = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(format!("/proc/self/fd/{fd}"))
                .unwrap_err();
            assert_eq!(err.raw_os_error(), Some(libc::ENXIO), "{err}");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn proc_fd_names_parse() {
        assert_eq!(parse_fd(b"12\0"), Some(12));
        assert_eq!(parse_fd(b"0\0\0\0"), Some(0));
        assert_eq!(parse_fd(b".\0"), None);
        assert_eq!(parse_fd(b"..\0"), None);
        assert_eq!(parse_fd(b"\0"), None);
        assert_eq!(parse_fd(b"99999999999\0"), None);
    }

    #[test]
    fn response_round_trip() {
        let e = WalletError::locked();
        let r: Response =
            serde_json::from_value(serde_json::to_value(Response::err(&e)).unwrap()).unwrap();
        assert_eq!(r.into_result().unwrap_err().code, ErrorCode::WalletLocked);
    }
}
