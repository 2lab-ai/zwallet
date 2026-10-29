//! The key holder (`zwallet daemon`, hidden).
//!
//! Started by `zwallet unlock` (or `create --unlock`) as that process's own child, through
//! [`protocol::start_daemon`], as `zwallet daemon --home <canonical home>` (the argv `lock` finds
//! it by), with one end of a socketpair as its fds 0 and 1. Startup, in order:
//! 1. core dumps are already off: the first line of the process entry (`cli::main_entry`)
//!    disables them, and on Linux makes the process non-dumpable from then on, which keeps other
//!    same-user processes out of its memory and its `/proc/<pid>/fd`. Not before that line: exec
//!    makes every process dumpable again. What keeps the channel safe in that window is that it
//!    is a socket, which cannot be reopened through `/proc/<pid>/fd` (on Linux the daemon refuses
//!    to start on anything else); against ptrace in that window, `unlock` requires Yama
//!    `ptrace_scope` ≥ 1 on Linux (see [`protocol::check_ptrace_scope`]). Then it asks for input,
//! 2. read the TTL and the password from the channel (raw read(2) into one zeroized buffer),
//! 3. decrypt wallet.json, derive the active key's address from the seed, report `decrypted`,
//! 4. wait for the go byte: the unlocking CLI has stopped the old daemons meanwhile,
//! 5. take an exclusive flock on `<home>/d.lock`, held for the daemon's whole life,
//! 6. replace whatever is at `d.sock` and bind it (a squatter there is a visible failure, never a
//!    leak: no secret ever goes over the socket),
//! 7. report `{ expiresAt, active, index, address, pid, whitelist }` on the channel, then serve.
//!    If that write fails (the unlocking CLI is gone), it wipes the seed and exits instead.
//!
//! It keeps only the master seed (and the whitelist anchor sealed with it) in memory, derives keys
//! and signs on request, and nothing else: it makes no network request and writes no file. Every
//! signature first passes the signing policy (`chains::evm::policy`): on each signing request the
//! daemon reads `whitelist.json` (or a staged `whitelist.next.json`) through the safe reader and
//! trusts it only if it verifies against that anchor (`.prd/04-whitelist.md`). That read runs on a
//! thread of its own with the session unlocked and a bounded wait (`LIST_WAIT`); the session is
//! checked again after it, and a read that ends after lock, expiry or a re-seal signs nothing. At
//! most `MAX_READERS` reads are in flight; past the wait the list counts as untrusted, which signs
//! nothing. Each connection is served in its own task with I/O timeouts, while the main loop keeps
//! the clock: on `lock`, on expiry (wall clock, so system sleep counts), on SIGTERM/SIGINT, or as
//! soon as wallet.json's ciphertext changes or `d.sock` is no longer its socket (checked every
//! tick and on every request), it drops the seed (zeroized) and exits, even while connections are
//! open. Socket and wallet checks use at most `MAX_INSPECTORS` secret-free worker threads,
//! with `INSPECT_WAIT` including permit wait; a timeout ends the session. Clock checks and lock
//! never wait for filesystem I/O. Socket cleanup after wiping the seed is also bounded. A daemon
//! process exists only while the wallet is unlocked (apart from bounded shutdown cleanup).

use crate::chains;
use crate::chains::evm::policy::{self, ListView};
use crate::core::error::{ErrorCode, Result, WalletError};
use crate::core::password::into_password;
use crate::core::protocol::{self, MAX_MESSAGE, Request, Response};
use crate::core::seed::Seed;
use crate::core::store::{self, FILE_MODE, WhitelistAnchor};
use crate::core::time::{rfc3339_utc, unix_secs};
use crate::core::whitelist::{self, Whitelist};
use alloy::hex;
use serde_json::{Value, json};
use std::fs;
use std::io::{ErrorKind, IsTerminal};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::unix::OwnedReadHalf;
use tokio::net::{UnixListener, UnixStream};
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::{Notify, Semaphore};

/// How long a new daemon waits for the previous one to release `d.lock`.
const LOCK_WAIT: Duration = Duration::from_secs(5);
/// How long a decrypted daemon waits for the go byte from the CLI that started it.
const GO_WAIT: Duration = Duration::from_secs(60);
/// Longest sleep between expiry and tamper checks.
const TICK: Duration = Duration::from_secs(1);
/// Per-connection read and write budget.
const IO_TIMEOUT: Duration = Duration::from_secs(10);

struct Session {
    seed: Seed,
    expires_at: SystemTime,
    /// Ciphertext of the wallet.json this seed came from.
    sealed: String,
    /// The whitelist anchor sealed with the seed.
    anchor: Anchor,
}

/// The whitelist anchor sealed with the seed, as the session keeps it.
#[derive(Debug, Clone)]
enum Anchor {
    Sealed(WhitelistAnchor),
    /// wallet.json seals no whitelist yet (`ListView::NoList`): the session signs only native
    /// transfers to the key's own address, under the built-in fee cap.
    None,
    /// The sealed anchor is malformed (the reason): the session still serves (status, address,
    /// lock) but the list is untrusted (`ListView::Unusable`), so it signs nothing at all.
    Malformed(String),
}

impl Anchor {
    fn of(unsealed: &store::Unsealed) -> Self {
        match unsealed.whitelist_anchor() {
            Ok(Some(anchor)) => Self::Sealed(anchor),
            Ok(None) => Self::None,
            Err(e) => Self::Malformed(e.message),
        }
    }
}

/// The list one request is decided against.
enum Loaded {
    /// `whitelist.json` (or a staged `whitelist.next.json`) verified against the sealed anchor.
    Verified(Whitelist),
    /// No anchor is sealed yet.
    NoList,
    /// There is a list, or an anchor, but nothing that can be trusted (the reason): nothing is
    /// signed.
    Untrusted(String),
}

impl Loaded {
    fn view(&self) -> ListView<'_> {
        match self {
            Self::Verified(list) => ListView::Usable(list),
            Self::NoList => ListView::NoList,
            Self::Untrusted(reason) => ListView::Unusable(reason.clone()),
        }
    }

    /// What `unlock` and `status` show of the list: `{ gen, verified, entries: [nicknames] }`,
    /// or `{ gen (the sealed one, if any), verified: false, entries: [], reason }`.
    fn summary(&self, anchor: &Anchor) -> Value {
        let sealed_gen = match anchor {
            Anchor::Sealed(a) => Some(a.generation),
            _ => None,
        };
        let unverified = |reason: &str| json!({ "gen": sealed_gen, "verified": false, "entries": [], "reason": reason });
        match self {
            Self::Verified(list) => json!({
                "gen": list.generation,
                "verified": true,
                "entries": list.entries.iter().map(|e| e.nickname.as_str()).collect::<Vec<_>>(),
            }),
            Self::NoList => unverified(policy::NO_WHITELIST),
            Self::Untrusted(reason) => unverified(reason),
        }
    }
}

/// Reads the list a sealed anchor names (`whitelist::load_verified`; a seam for tests).
type ListReader = Arc<dyn Fn(&Path, &WhitelistAnchor) -> Result<Whitelist> + Send + Sync>;

/// Inspect only public filesystem state. The worker never receives a session or seed.
type Inspector = Arc<dyn Fn(&Path, &Path, (u64, u64), &str) -> Option<&'static str> + Send + Sync>;
const INSPECT_WAIT: Duration = Duration::from_secs(2);
const MAX_INSPECTORS: usize = 4;

fn socket_is_ours(sock: &Path, sock_id: (u64, u64)) -> bool {
    fs::symlink_metadata(sock)
        .is_ok_and(|m| m.file_type().is_socket() && (m.dev(), m.ino()) == sock_id)
}

fn inspect_files(
    home: &Path,
    sock: &Path,
    sock_id: (u64, u64),
    sealed: &str,
) -> Option<&'static str> {
    if !socket_is_ours(sock, sock_id) {
        return Some("the daemon socket was replaced or removed");
    }
    if !store::read_wallet_file(home).is_ok_and(|f| f.crypto.ciphertext == sealed) {
        return Some("wallet.json changed since unlock");
    }
    None
}

/// How long a request waits for the list to be read before deciding without it.
const LIST_WAIT: Duration = Duration::from_secs(2);
/// The most list reads in flight at once. A read holds its permit until it really returns, so
/// stalled reads (a home on a hung mount) cannot pile up threads past this.
const MAX_READERS: usize = 4;

/// What the main loop and the connection tasks share.
struct Shared {
    home: PathBuf,
    sock: PathBuf,
    /// `(dev, ino)` of the socket this daemon bound.
    sock_id: (u64, u64),
    session: Mutex<Option<Session>>,
    /// A connection task asks the main loop to stop (after `lock`, or after it saw tampering).
    stop: Notify,
    reader: ListReader,
    /// How long a request waits for the list, in milliseconds: [`LIST_WAIT`]. Read per request,
    /// so tests can shorten it to see a refusal and lengthen it again to see a signature.
    list_wait_ms: AtomicU64,
    /// [`MAX_READERS`] permits, one per list read in flight.
    readers: Arc<Semaphore>,
    inspector: Inspector,
    inspect_wait_ms: AtomicU64,
    inspectors: Arc<Semaphore>,
}

impl Shared {
    /// Pure memory/clock check: never waits for the filesystem under the session lock.
    fn check(&self, session: &mut Option<Session>) -> Option<&'static str> {
        let reason = match session.as_ref() {
            None => Some("the wallet is locked"),
            Some(s) if SystemTime::now() >= s.expires_at => Some("the unlock expired"),
            Some(_) => None,
        };
        if reason.is_some() {
            *session = None;
        }
        reason
    }

    /// A fresh check before answering any request other than lock, and on the monitor's tick.
    /// The deadline includes waiting for a permit; abandoned workers retain their permits
    /// until the syscall returns. Detached threads cannot delay runtime shutdown. Any failed
    /// inspection ends the session, so another concurrent successful result cannot revive it.
    async fn inspect(&self) -> Option<&'static str> {
        let sealed = {
            let mut session = self.session();
            if let Some(reason) = self.check(&mut session) {
                return Some(reason);
            }
            session.as_ref().unwrap().sealed.clone()
        };
        let deadline = tokio::time::Instant::now()
            + Duration::from_millis(self.inspect_wait_ms.load(Ordering::Relaxed));
        let inspect = async {
            let permit = self.inspectors.clone().acquire_owned().await.ok()?;
            let (home, sock, sock_id, inspector, marker) = (
                self.home.clone(),
                self.sock.clone(),
                self.sock_id,
                self.inspector.clone(),
                sealed.clone(),
            );
            let (tx, rx) = tokio::sync::oneshot::channel();
            std::thread::Builder::new()
                .name("wallet-inspect".into())
                .spawn(move || {
                    let result = inspector(&home, &sock, sock_id, &marker);
                    drop(permit);
                    let _ = tx.send(result);
                })
                .ok()?;
            rx.await.ok()
        };
        let result = match tokio::time::timeout_at(deadline, inspect).await {
            Ok(Some(result)) if tokio::time::Instant::now() < deadline => result,
            Ok(None) => Some("wallet state could not be inspected"),
            Ok(Some(_)) | Err(_) => Some("wallet state could not be inspected in time"),
        };
        let mut session = self.session();
        if let Some(reason) = self.check(&mut session) {
            return Some(reason);
        }
        let reason = if session.as_ref().is_none_or(|s| s.sealed != sealed) {
            Some("the session changed while wallet state was inspected")
        } else {
            result
        };
        if reason.is_some() {
            *session = None;
            self.stop.notify_one();
        }
        reason
    }

    fn session(&self) -> std::sync::MutexGuard<'_, Option<Session>> {
        self.session.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The list `anchor` names, read on a thread of its own and waited for at most
    /// `list_wait_ms`. Never called with the session locked: a stalled read must not hold up
    /// `lock`, expiry or the tamper checks. The read first takes one of [`MAX_READERS`] permits
    /// (waiting for it counts against the same deadline) and keeps it until the read returns. A
    /// read still running after the wait is abandoned; it holds no lock and no secret, and being
    /// detached (not tokio's blocking pool, which the runtime waits for on drop) it cannot keep
    /// the daemon from exiting.
    async fn load(&self, anchor: &Anchor) -> Loaded {
        let anchor = match anchor {
            Anchor::Sealed(a) => *a,
            Anchor::None => return Loaded::NoList,
            Anchor::Malformed(reason) => return Loaded::Untrusted(reason.clone()),
        };
        let wait = Duration::from_millis(self.list_wait_ms.load(Ordering::Relaxed));
        let deadline = tokio::time::Instant::now() + wait;
        let late = || {
            Loaded::Untrusted(format!(
                "the whitelist could not be read in time ({} ms)",
                wait.as_millis()
            ))
        };
        let permit =
            match tokio::time::timeout_at(deadline, self.readers.clone().acquire_owned()).await {
                Ok(Ok(permit)) => permit,
                Ok(Err(_)) | Err(_) => return late(),
            };
        let (home, reader) = (self.home.clone(), self.reader.clone());
        let (tx, rx) = tokio::sync::oneshot::channel();
        let spawned = std::thread::Builder::new()
            .name("whitelist-read".into())
            .spawn(move || {
                let read = reader(&home, &anchor);
                // Released only now that the read has returned, timed out or not.
                drop(permit);
                let _ = tx.send(read);
            });
        if spawned.is_err() {
            return Loaded::Untrusted("the whitelist could not be read (no thread)".into());
        }
        match tokio::time::timeout_at(deadline, rx).await {
            Ok(Ok(Ok(list))) => Loaded::Verified(list),
            Ok(Ok(Err(e))) => Loaded::Untrusted(e.message),
            Ok(Err(_)) => Loaded::Untrusted("the whitelist could not be read".into()),
            Err(_) => late(),
        }
    }
}

pub async fn run(home: PathBuf) -> Result<()> {
    // Core dumps are off already: the first line of `cli::main_entry`.
    // SAFETY: umask(2) only changes this process's file-creation mask.
    unsafe {
        libc::umask(0o077);
    }
    let started = match start(&home).await {
        Ok(s) => s,
        Err(e) => {
            report_last(&Response::err(&e));
            return Err(e);
        }
    };
    let Started {
        listener,
        shared,
        ready,
        _lock,
    } = started;
    // Readiness goes out only now that the socket serves. If the unlocking CLI is gone (it
    // timed out or was killed), nobody is waiting for this key: wipe it and leave.
    if !report_last(&Response::ok(ready)) {
        *shared.session() = None;
        remove_own_socket(&shared).await;
        return Err(WalletError::validation("the unlocking process went away"));
    }
    serve_until_done(listener, &shared).await;
    *shared.session() = None; // zeroizes the seed
    remove_own_socket(&shared).await;
    Ok(())
}

struct Started {
    listener: UnixListener,
    shared: Arc<Shared>,
    ready: Value,
    /// Held for the daemon's whole life.
    _lock: fs::File,
}

async fn start(home: &Path) -> Result<Started> {
    if !home.is_absolute() {
        return Err(WalletError::validation(
            "`zwallet daemon --home` needs an absolute path",
        ));
    }
    let sock = protocol::socket_path(home)?;
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        return Err(WalletError::validation(
            "`zwallet daemon` is started by `zwallet unlock`, not by hand",
        ));
    }
    #[cfg(target_os = "linux")]
    require_socket_channel()?;
    let gone = || WalletError::validation("the unlocking process went away");
    // Core dumps are off (first line of `cli::main_entry`): only now may the password come.
    if !report(&Response::ok(json!({ "stage": protocol::STAGE_INPUT }))) {
        return Err(gone());
    }
    let (ttl, password) = protocol::read_unlock_input(stdin.as_raw_fd())?;
    let password = into_password(password)?;
    let (unsealed, file) = store::unlock(home, &password, &chains::derive_addresses)?;
    drop(password);
    let key = file.key(None)?;
    let primary = chains::find(chains::PRIMARY_CHAIN)?;
    let address = primary
        .family
        .keys()
        .derive_address(&unsealed.seed, key.index)?;
    // The unlocking CLI stops the old daemons only after this, so a wrong password never ends
    // a running session; it answers with the go byte.
    if !report(&Response::ok(json!({ "stage": protocol::STAGE_DECRYPTED }))) {
        return Err(gone());
    }
    protocol::read_go(stdin.as_raw_fd(), GO_WAIT)?;
    redirect_to_null(libc::STDIN_FILENO);
    let lock = store::lock_exclusive(
        &home.join(protocol::LOCK_NAME),
        LOCK_WAIT,
        "another zwallet daemon serves this wallet",
    )?;
    // The TTL counts from now, not from before the KDF ran.
    let expires_at = SystemTime::now()
        .checked_add(ttl)
        .ok_or_else(|| WalletError::validation("invalid unlock duration"))?;

    let fail = |what: &'static str| {
        move |e: std::io::Error| WalletError::validation(format!("{what}: {e}"))
    };
    match fs::remove_file(&sock) {
        Ok(()) => {}
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(fail("cannot remove the old daemon socket")(e)),
    }
    let listener = UnixListener::bind(&sock).map_err(fail("cannot bind daemon socket"))?;
    fs::set_permissions(&sock, fs::Permissions::from_mode(FILE_MODE))
        .map_err(fail("cannot restrict daemon socket"))?;
    let meta = fs::symlink_metadata(&sock).map_err(fail("cannot inspect daemon socket"))?;
    let anchor = Anchor::of(&unsealed);
    let shared = Arc::new(Shared {
        home: home.to_path_buf(),
        sock,
        sock_id: (meta.dev(), meta.ino()),
        session: Mutex::new(Some(Session {
            seed: unsealed.seed,
            expires_at,
            sealed: file.crypto.ciphertext.clone(),
            anchor: anchor.clone(),
        })),
        stop: Notify::new(),
        reader: Arc::new(whitelist::load_verified),
        list_wait_ms: AtomicU64::new(LIST_WAIT.as_millis() as u64),
        readers: Arc::new(Semaphore::new(MAX_READERS)),
        inspector: Arc::new(inspect_files),
        inspect_wait_ms: AtomicU64::new(INSPECT_WAIT.as_millis() as u64),
        inspectors: Arc::new(Semaphore::new(MAX_INSPECTORS)),
    });
    let ready = json!({
        "locked": false,
        "expiresAt": rfc3339_utc(unix_secs(expires_at)),
        "active": file.active,
        "index": key.index,
        "address": address,
        "pid": std::process::id(),
        "whitelist": shared.load(&anchor).await.summary(&anchor),
    });
    // wallet.json may have changed while this daemon waited for the go byte.
    if let Some(reason) = shared.inspect().await {
        remove_own_socket(&shared).await;
        return Err(WalletError::validation(format!(
            "{reason}; not unlocked, run `zwallet unlock` again"
        )));
    }
    Ok(Started {
        listener,
        shared,
        ready,
        _lock: lock,
    })
}

/// Refuses to start unless fds 0 and 1 are sockets (the socketpair `unlock` gives its child). A
/// pipe there can be reopened by any same-user process through `/proc/<pid>/fd/0` or `/1` while
/// this process is still dumpable, to read the password or to fake the readiness line; a socket
/// cannot (ENXIO).
#[cfg(target_os = "linux")]
fn require_socket_channel() -> Result<()> {
    for fd in [libc::STDIN_FILENO, libc::STDOUT_FILENO] {
        // SAFETY: stat is plain old data; fstat fills it for a descriptor number.
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        let is_socket =
            unsafe { libc::fstat(fd, &mut st) } == 0 && st.st_mode & libc::S_IFMT == libc::S_IFSOCK;
        if !is_socket {
            return Err(WalletError::validation(
                "`zwallet daemon` needs the socket `zwallet unlock` gives it as its input and \
                 output (a pipe can be reopened through /proc); not starting",
            ));
        }
    }
    Ok(())
}

/// Writes one progress line to stdout (the unlocking CLI's channel). Returns whether it was
/// delivered.
fn report(resp: &Response) -> bool {
    let mut line = serde_json::to_vec(resp).unwrap_or_default();
    line.push(b'\n');
    let mut rest = line.as_slice();
    while !rest.is_empty() {
        // SAFETY: writes from a valid slice to fd 1.
        let n = unsafe { libc::write(libc::STDOUT_FILENO, rest.as_ptr().cast(), rest.len()) };
        if n < 0 {
            if std::io::Error::last_os_error().kind() == ErrorKind::Interrupted {
                continue;
            }
            return false;
        }
        rest = &rest[n as usize..];
    }
    true
}

/// The last line (readiness or the startup error), after which stdout points at /dev/null so
/// the channel closes.
fn report_last(resp: &Response) -> bool {
    let ok = report(resp);
    redirect_to_null(libc::STDOUT_FILENO);
    ok
}

fn redirect_to_null(fd: libc::c_int) {
    if let Ok(null) = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/null")
    {
        // SAFETY: dup2 onto a standard fd this process owns; `null` stays open for the call.
        unsafe {
            libc::dup2(null.as_raw_fd(), fd);
        }
    }
}

async fn remove_own_socket(shared: &Shared) {
    // The seed is already gone. Cleanup also must not strand the exiting process on a hung
    // mount. This one detached worker has no secret, and is never joined during shutdown.
    let (sock, sock_id) = (shared.sock.clone(), shared.sock_id);
    let (tx, rx) = tokio::sync::oneshot::channel();
    if std::thread::Builder::new()
        .name("socket-cleanup".into())
        .spawn(move || {
            if socket_is_ours(&sock, sock_id) {
                let _ = fs::remove_file(sock);
            }
            let _ = tx.send(());
        })
        .is_ok()
    {
        let _ = tokio::time::timeout(INSPECT_WAIT, rx).await;
    }
}

async fn serve_until_done(listener: UnixListener, shared: &Arc<Shared>) {
    let fail = |e: std::io::Error| WalletError::validation(format!("signal setup: {e}"));
    let (Ok(mut term), Ok(mut int), Ok(_hup)) = (
        signal(SignalKind::terminate()).map_err(fail),
        signal(SignalKind::interrupt()).map_err(fail),
        // No controlling terminal after setsid, but ignore a stray SIGHUP rather than die.
        signal(SignalKind::hangup()).map_err(fail),
    ) else {
        return;
    };
    let uid = protocol::current_uid();
    let monitor = async {
        loop {
            if shared.inspect().await.is_some() {
                return;
            }
            tokio::time::sleep(TICK).await;
        }
    };
    tokio::pin!(monitor);
    loop {
        let wait = {
            let mut session = shared.session();
            if shared.check(&mut session).is_some() {
                return;
            }
            let expires_at = session.as_ref().map_or(SystemTime::now(), |s| s.expires_at);
            expires_at
                .duration_since(SystemTime::now())
                .unwrap_or(Duration::ZERO)
                .clamp(Duration::from_millis(5), TICK)
        };
        tokio::select! {
            accepted = listener.accept() => {
                if let Ok((stream, _)) = accepted {
                    tokio::spawn(serve(shared.clone(), stream, uid));
                }
            }
            _ = tokio::time::sleep(wait) => {}
            _ = &mut monitor => return,
            _ = shared.stop.notified() => return,
            _ = term.recv() => return,
            _ = int.recv() => return,
        }
    }
}

async fn serve(shared: Arc<Shared>, stream: UnixStream, uid: u32) {
    match stream.peer_cred() {
        Ok(cred) if cred.uid() == uid => {}
        _ => return,
    }
    let (mut reader, mut writer) = stream.into_split();
    let request = match tokio::time::timeout(IO_TIMEOUT, read_request(&mut reader)).await {
        Ok(Ok(bytes)) if !bytes.is_empty() => bytes,
        _ => return,
    };
    let (response, stop) = handle(&shared, &request).await;
    let mut out = serde_json::to_vec(&response).unwrap_or_default();
    out.push(b'\n');
    let _ = tokio::time::timeout(IO_TIMEOUT, async {
        writer.write_all(&out).await?;
        writer.shutdown().await
    })
    .await;
    if stop {
        shared.stop.notify_one();
    }
}

/// One request line (or everything up to EOF), at most [`MAX_MESSAGE`] bytes. Requests carry
/// no secret.
async fn read_request(reader: &mut OwnedReadHalf) -> std::io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = reader.read(&mut chunk).await?;
        if n == 0 {
            return Ok(buf);
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() as u64 > MAX_MESSAGE {
            return Err(std::io::Error::other("request too large"));
        }
        if chunk[..n].contains(&b'\n') {
            return Ok(buf);
        }
    }
}

/// Answers one request; the flag says the daemon must stop afterwards.
///
/// `status` and the signing requests need the whitelist. It is read with the session unlocked
/// (only the anchor and the ciphertext marker are copied out, never the seed); then the session
/// is checked again (lock, expiry, socket, wallet.json) and must be the one the read started
/// from, so a read that ends after any of those signs nothing.
async fn handle(shared: &Shared, bytes: &[u8]) -> (Response, bool) {
    let Ok(req) = serde_json::from_slice::<Request>(bytes) else {
        let e = WalletError::validation("malformed daemon request");
        return (Response::err(&e), false);
    };
    // Lock needs no filesystem evidence: erase the seed immediately, even if all workers hang.
    if matches!(req, Request::Lock) {
        let mut session = shared.session();
        if let Some(reason) = shared.check(&mut session) {
            return ended(&req, reason);
        }
        return respond(&mut session, &req, None);
    }
    if let Some(reason) = shared.inspect().await {
        return ended(&req, reason);
    }
    let snapshot = {
        let mut session = shared.session();
        if let Some(reason) = shared.check(&mut session) {
            return ended(&req, reason);
        }
        match (&req, session.as_ref()) {
            (Request::Status | Request::SignMessage { .. } | Request::SignTx { .. }, Some(s)) => {
                (s.anchor.clone(), s.sealed.clone())
            }
            _ => return respond(&mut session, &req, None),
        }
    };
    let (anchor, sealed) = snapshot;
    let loaded = shared.load(&anchor).await;
    if let Some(reason) = shared.inspect().await {
        return ended(&req, reason);
    }
    let mut session = shared.session();
    if let Some(reason) = shared.check(&mut session) {
        return ended(&req, reason);
    }
    if session.as_ref().is_none_or(|s| s.sealed != sealed) {
        *session = None;
        return ended(&req, "the session changed while the whitelist was read");
    }
    respond(&mut session, &req, Some(&loaded))
}

/// The answer once the session has ended (`reason`): the daemon stops.
fn ended(req: &Request, reason: &str) -> (Response, bool) {
    let resp = match req {
        Request::Status => {
            Response::ok(json!({ "locked": true, "pid": std::process::id(), "reason": reason }))
        }
        Request::Lock => Response::ok(json!({ "locked": true, "wasUnlocked": false })),
        _ => Response::err(&WalletError::new(
            ErrorCode::WalletLocked,
            format!("{reason}; locked. A human must run `zwallet unlock 24h` (any duration)"),
        )),
    };
    (resp, true)
}

/// Answers `req` from a session that just passed [`Shared::check`], with the list read for it
/// (`None` for the requests that need none).
fn respond(
    session: &mut Option<Session>,
    req: &Request,
    loaded: Option<&Loaded>,
) -> (Response, bool) {
    let Some(s) = session.as_ref() else {
        return (Response::err(&WalletError::locked()), true);
    };
    let list = || {
        loaded.ok_or_else(|| WalletError::validation("the whitelist was not read for this request"))
    };
    let result = match req {
        Request::Lock => {
            *session = None;
            return (
                Response::ok(json!({ "locked": true, "wasUnlocked": true })),
                true,
            );
        }
        Request::Status => list().map(|list| {
            json!({
                "locked": false,
                "expiresAt": rfc3339_utc(unix_secs(s.expires_at)),
                "pid": std::process::id(),
                "whitelist": list.summary(&s.anchor),
            })
        }),
        Request::Address { chain, index } => chains::find(chain).and_then(|chain| {
            let address = chain.family.keys().derive_address(&s.seed, *index)?;
            Ok(json!({ "chain": chain.key, "index": index, "address": address }))
        }),
        Request::SignMessage {
            chain,
            index,
            message_hex,
        } => chains::find(chain).and_then(|chain| {
            let message = hex::decode(message_hex)
                .map_err(|_| WalletError::validation("message_hex is not hex"))?;
            chain
                .family
                .keys()
                .sign_message(&s.seed, *index, chain, &message, &list()?.view())
        }),
        Request::SignTx { chain, index, tx } => chains::find(chain).and_then(|chain| {
            chain
                .family
                .keys()
                .sign_tx(&s.seed, *index, chain, tx, &list()?.view())
        }),
    };
    let resp = match result {
        Ok(v) => Response::ok(v),
        Err(e) => Response::err(&e),
    };
    (resp, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chains::evm::registry::WHITELIST_RULES;
    use crate::chains::evm::tx::{self, EvmFees, UnsignedEvmTx};
    use crate::core::store::{KDF_MIN_LOG_N, KdfParams, MasterOptions, MetaLock};
    use crate::core::whitelist::Entry;
    use alloy::primitives::{U128, U256};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    const PW: &str = "correct horse battery staple";
    const BOB: &str = "0xfB6916095ca1df60bB79Ce92cE3Ea74c37c5d359";

    /// A daemon's shared state as `start` builds it, in-process: a real wallet.json in a temp
    /// home, its socket bound, a session for key 0, and `reader` as the list reader.
    struct Fixture {
        shared: Arc<Shared>,
        me: String,
        _listener: std::os::unix::net::UnixListener,
        _dir: tempfile::TempDir,
    }

    /// Long enough for a fresh reader thread to be scheduled and return on a loaded host: the
    /// list wait for every phase that expects a signature.
    const GENEROUS: Duration = Duration::from_secs(30);

    impl Fixture {
        /// Sets the list wait for the requests that follow.
        fn set_wait(&self, wait: Duration) {
            self.shared
                .list_wait_ms
                .store(wait.as_millis() as u64, Ordering::Relaxed);
        }
    }

    fn fixture(reader: ListReader, list_wait: Duration, ttl: Duration) -> Fixture {
        fixture_with_inspector(
            reader,
            list_wait,
            ttl,
            Arc::new(inspect_files),
            INSPECT_WAIT,
        )
    }

    fn fixture_with_inspector(
        reader: ListReader,
        list_wait: Duration,
        ttl: Duration,
        inspector: Inspector,
        inspect_wait: Duration,
    ) -> Fixture {
        let dir = tempfile::Builder::new()
            .prefix("zwt-")
            .tempdir_in("/tmp")
            .unwrap();
        let home = dir.path();
        let opts = MasterOptions {
            kdf: KdfParams {
                log_n: KDF_MIN_LOG_N,
                ..KdfParams::default()
            },
            ..MasterOptions::default()
        };
        store::create_master(home, PW, "main", opts, &chains::derive_addresses).unwrap();
        let (unsealed, file) = store::unlock(home, PW, &chains::derive_addresses).unwrap();
        let me = chains::find("ethereum")
            .unwrap()
            .family
            .keys()
            .derive_address(&unsealed.seed, 0)
            .unwrap();
        let sock = protocol::socket_path(home).unwrap();
        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let meta = fs::symlink_metadata(&sock).unwrap();
        let shared = Arc::new(Shared {
            home: home.to_path_buf(),
            sock,
            sock_id: (meta.dev(), meta.ino()),
            session: Mutex::new(Some(Session {
                seed: unsealed.seed,
                expires_at: SystemTime::now() + ttl,
                sealed: file.crypto.ciphertext.clone(),
                // The reader ignores it: these tests decide what the read returns.
                anchor: Anchor::Sealed(WhitelistAnchor {
                    public_key: [7; 32],
                    generation: 1,
                    digest: [7; 32],
                }),
            })),
            stop: Notify::new(),
            reader,
            list_wait_ms: AtomicU64::new(list_wait.as_millis() as u64),
            readers: Arc::new(Semaphore::new(MAX_READERS)),
            inspector,
            inspect_wait_ms: AtomicU64::new(inspect_wait.as_millis() as u64),
            inspectors: Arc::new(Semaphore::new(MAX_INSPECTORS)),
        });
        Fixture {
            shared,
            me,
            _listener: listener,
            _dir: dir,
        }
    }

    /// A verified-looking list that pays BOB, with the default policy.
    fn list() -> Whitelist {
        let mut l = Whitelist::empty(&WHITELIST_RULES);
        l.generation = 1;
        l.entries.push(Entry::new_address("bob", BOB, &["*"]));
        l
    }

    /// A `sign_tx` request on Ethereum paying `to`, cheap enough for any cap.
    fn sign_tx(to: &str) -> Vec<u8> {
        let t = UnsignedEvmTx {
            chain_id: 1,
            nonce: 0,
            gas_limit: 21_000,
            fees: EvmFees::Eip1559 {
                max_fee_per_gas: U128::from(1_000_000_000u64),
                max_priority_fee_per_gas: U128::from(1u64),
            },
            to: to.parse().unwrap(),
            value: U256::from(1u64),
            input: Default::default(),
        };
        let payload = format!("0x{}", hex::encode(tx::encoded_for_signing(&t)));
        serde_json::to_vec(&Request::SignTx {
            chain: "ethereum".into(),
            index: 0,
            tx: json!(payload),
        })
        .unwrap()
    }

    fn lock_request() -> Vec<u8> {
        serde_json::to_vec(&Request::Lock).unwrap()
    }

    /// A list reader that blocks until released (or 30 s), counting the reads that entered,
    /// are in flight now (and the most ever at once), and have returned.
    struct Gate {
        open: Mutex<bool>,
        cv: std::sync::Condvar,
        entered: AtomicUsize,
        live: AtomicUsize,
        max_live: AtomicUsize,
        finished: AtomicUsize,
    }

    impl Gate {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                open: Mutex::new(false),
                cv: std::sync::Condvar::new(),
                entered: AtomicUsize::new(0),
                live: AtomicUsize::new(0),
                max_live: AtomicUsize::new(0),
                finished: AtomicUsize::new(0),
            })
        }

        fn block(&self) {
            self.entered.fetch_add(1, Ordering::SeqCst);
            let live = self.live.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_live.fetch_max(live, Ordering::SeqCst);
            let open = self.open.lock().unwrap();
            let _ = self
                .cv
                .wait_timeout_while(open, Duration::from_secs(30), |open| !*open)
                .unwrap();
            self.live.fetch_sub(1, Ordering::SeqCst);
            self.finished.fetch_add(1, Ordering::SeqCst);
        }

        fn reader(self: &Arc<Self>) -> ListReader {
            let g = self.clone();
            Arc::new(move |_, _| {
                g.block();
                Ok(list())
            })
        }

        fn inspector(self: &Arc<Self>) -> Inspector {
            let g = self.clone();
            Arc::new(move |home, sock, id, sealed| {
                g.block();
                inspect_files(home, sock, id, sealed)
            })
        }

        fn release(&self) {
            *self.open.lock().unwrap() = true;
            self.cv.notify_all();
        }

        fn count(&self, n: &AtomicUsize) -> usize {
            n.load(Ordering::SeqCst)
        }

        /// Waits (bounded at 30 s, blocking this thread) until `counter` reaches `n`.
        fn wait_blocking(&self, counter: &AtomicUsize, n: usize) {
            let start = Instant::now();
            while counter.load(Ordering::SeqCst) < n {
                assert!(start.elapsed() < Duration::from_secs(30), "stuck below {n}");
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        /// Waits (bounded, yielding to the runtime) until `counter` reaches `n`.
        async fn wait(&self, counter: &AtomicUsize, n: usize) {
            let start = Instant::now();
            while counter.load(Ordering::SeqCst) < n {
                assert!(start.elapsed() < Duration::from_secs(30), "stuck below {n}");
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }
    }

    fn spawn_handle(shared: &Arc<Shared>, bytes: Vec<u8>) -> tokio::task::JoinHandle<Response> {
        let shared = shared.clone();
        tokio::spawn(async move { handle(&shared, &bytes).await.0 })
    }

    fn assert_locked(resp: &Response) {
        assert!(!resp.ok, "signed: {resp:?}");
        assert_eq!(resp.error_code, Some(ErrorCode::WalletLocked), "{resp:?}");
        assert!(resp.result.is_none(), "{resp:?}");
    }

    /// A refusal because the list could not be read before the wait ran out.
    fn assert_late(resp: &Response) {
        assert!(!resp.ok, "signed: {resp:?}");
        assert_eq!(
            resp.error_code,
            Some(ErrorCode::WhitelistInvalid),
            "{resp:?}"
        );
        assert!(
            resp.error
                .as_deref()
                .unwrap()
                .contains("could not be read in time"),
            "{resp:?}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn control_a_released_read_signs() {
        let gate = Gate::new();
        let f = fixture(gate.reader(), GENEROUS, Duration::from_secs(60));
        let sign = spawn_handle(&f.shared, sign_tx(BOB));
        gate.wait(&gate.entered, 1).await;
        gate.release();
        let resp = sign.await.unwrap();
        assert!(resp.ok, "{resp:?}");
        assert!(resp.result.unwrap()["rawTx"].is_string());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn lock_is_answered_while_a_list_read_is_stalled_and_the_late_read_signs_nothing() {
        let gate = Gate::new();
        let f = fixture(gate.reader(), GENEROUS, Duration::from_secs(60));
        let start = Instant::now();
        let sign = spawn_handle(&f.shared, sign_tx(BOB));
        gate.wait(&gate.entered, 1).await;
        let (resp, stop) = handle(&f.shared, &lock_request()).await;
        let locked_after = start.elapsed();
        assert!(resp.ok && stop, "{resp:?}");
        assert_eq!(resp.result.unwrap()["wasUnlocked"], true);
        assert!(
            locked_after < Duration::from_secs(4),
            "lock waited {locked_after:?} for the stalled read"
        );
        assert!(f.shared.session().is_none(), "the seed outlived lock");
        gate.release();
        assert_locked(&sign.await.unwrap());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_read_that_ends_after_expiry_signs_nothing() {
        let gate = Gate::new();
        let f = fixture(gate.reader(), GENEROUS, Duration::from_millis(400));
        let start = Instant::now();
        let sign = spawn_handle(&f.shared, sign_tx(BOB));
        gate.wait(&gate.entered, 1).await;
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "the stalled read held the runtime: {:?}",
            start.elapsed()
        );
        gate.release();
        assert_locked(&sign.await.unwrap());
        assert!(f.shared.session().is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_read_during_which_wallet_json_is_resealed_signs_nothing() {
        // The reader itself re-seals wallet.json (what a whitelist edit does) before answering.
        let reader: ListReader = Arc::new(|home, _| {
            let lock = MetaLock::acquire(home).unwrap();
            let opened = lock.open(PW).unwrap();
            lock.reseal(&opened, &opened.unsealed().policy.clone())
                .unwrap();
            Ok(list())
        });
        let f = fixture(reader, GENEROUS, Duration::from_secs(60));
        let (resp, stop) = handle(&f.shared, &sign_tx(BOB)).await;
        assert_locked(&resp);
        assert!(stop);
        assert!(resp.error.unwrap().contains("wallet.json changed"));
    }

    #[test]
    fn dropping_the_runtime_does_not_wait_for_a_reader_still_blocked() {
        // After `lock` or expiry the daemon's runtime is dropped as the process ends. A reader
        // abandoned after the wait, and still blocked in its read, must not hold that up.
        let gate = Gate::new();
        let f = fixture(
            gate.reader(),
            Duration::from_millis(100),
            Duration::from_secs(60),
        );
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (resp, _) = rt.block_on(handle(&f.shared, &sign_tx(BOB)));
        assert_late(&resp);
        // The reader thread may be scheduled only after the request gave up on it: wait for it
        // to be inside its (blocked) read before dropping the runtime.
        gate.wait_blocking(&gate.entered, 1);
        assert_eq!(gate.count(&gate.entered), 1);
        assert_eq!(
            gate.count(&gate.finished),
            0,
            "the read was not still blocked"
        );
        let dropping = Instant::now();
        drop(rt);
        let dropped_in = dropping.elapsed();
        assert!(
            dropped_in < Duration::from_secs(3),
            "the runtime waited {dropped_in:?} for the blocked reader"
        );
        assert_eq!(
            gate.count(&gate.finished),
            0,
            "the reader finished before the runtime was dropped"
        );
        gate.release();
        gate.wait_blocking(&gate.finished, 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_read_past_the_wait_refuses_everything_until_the_list_reads_again() {
        let gate = Gate::new();
        let f = fixture(
            gate.reader(),
            Duration::from_millis(200),
            Duration::from_secs(60),
        );
        let (resp, stop) = handle(&f.shared, &sign_tx(BOB)).await;
        assert!(!stop);
        assert_late(&resp);
        // An unreadable list is an untrusted one: the key's own address too (P-1).
        let (resp, stop) = handle(&f.shared, &sign_tx(&f.me)).await;
        assert!(!stop);
        assert_late(&resp);
        // The session is untouched: once the stalled reads return, signing works again (with a
        // wait long enough for a fresh reader thread on a loaded host).
        gate.wait(&gate.entered, 2).await;
        gate.release();
        gate.wait(&gate.finished, 2).await;
        f.set_wait(GENEROUS);
        let (resp, _) = handle(&f.shared, &sign_tx(BOB)).await;
        assert!(resp.ok, "{resp:?}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn outstanding_list_readers_are_bounded_and_signing_resumes_after_release() {
        let gate = Gate::new();
        let f = fixture(
            gate.reader(),
            Duration::from_millis(300),
            Duration::from_secs(60),
        );
        // One after another, then all at once: every one is refused in time, and no more than
        // MAX_READERS reads are ever outstanding.
        for _ in 0..MAX_READERS + 2 {
            let (resp, stop) = handle(&f.shared, &sign_tx(BOB)).await;
            assert!(!stop);
            assert_late(&resp);
        }
        let parallel: Vec<_> = (0..MAX_READERS * 3)
            .map(|_| spawn_handle(&f.shared, sign_tx(BOB)))
            .collect();
        for h in parallel {
            assert_late(&h.await.unwrap());
        }
        // Each permit's thread may enter after its request gave up: wait for all of them.
        gate.wait(&gate.entered, MAX_READERS).await;
        assert_eq!(gate.count(&gate.entered), MAX_READERS, "readers started");
        assert!(gate.count(&gate.max_live) <= MAX_READERS);
        assert_eq!(gate.count(&gate.finished), 0);
        // Released: the stalled reads return and their permits come back; then signing works
        // (with a wait long enough for a fresh reader thread on a loaded host).
        gate.release();
        gate.wait(&gate.finished, MAX_READERS).await;
        f.set_wait(GENEROUS);
        let (resp, _) = handle(&f.shared, &sign_tx(BOB)).await;
        assert!(resp.ok, "{resp:?}");
        assert_eq!(gate.count(&gate.entered), MAX_READERS + 1);
        assert!(gate.count(&gate.max_live) <= MAX_READERS);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn lock_is_answered_while_every_reader_is_stalled() {
        // A long wait: nothing here is meant to time out; every permit is held by a stalled read
        // until release.
        let gate = Gate::new();
        let f = fixture(gate.reader(), GENEROUS, Duration::from_secs(60));
        let pending: Vec<_> = (0..MAX_READERS * 2)
            .map(|_| spawn_handle(&f.shared, sign_tx(BOB)))
            .collect();
        gate.wait(&gate.entered, MAX_READERS).await;
        let start = Instant::now();
        let (resp, stop) = handle(&f.shared, &lock_request()).await;
        assert!(resp.ok && stop, "{resp:?}");
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "lock waited {:?}",
            start.elapsed()
        );
        assert!(f.shared.session().is_none(), "the seed outlived lock");
        gate.release();
        for h in pending {
            assert_locked(&h.await.unwrap());
        }
        assert!(gate.count(&gate.max_live) <= MAX_READERS);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn lock_does_not_wait_for_wallet_or_socket_inspection() {
        let gate = Gate::new();
        let f = fixture_with_inspector(
            Arc::new(|_, _| Ok(list())),
            GENEROUS,
            Duration::from_secs(60),
            gate.inspector(),
            GENEROUS,
        );
        let pending: Vec<_> = (0..MAX_INSPECTORS * 2)
            .map(|_| spawn_handle(&f.shared, sign_tx(BOB)))
            .collect();
        gate.wait(&gate.entered, MAX_INSPECTORS).await;
        let start = Instant::now();
        let (response, stop) = handle(&f.shared, &lock_request()).await;
        assert!(response.ok && stop);
        assert!(start.elapsed() < Duration::from_millis(500));
        assert!(f.shared.session().is_none());
        assert_eq!(gate.count(&gate.finished), 0);
        gate.release();
        for request in pending {
            assert_locked(&request.await.unwrap());
        }
        assert!(gate.count(&gate.max_live) <= MAX_INSPECTORS);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn inspection_workers_stay_bounded_when_queued_requests_time_out() {
        let gate = Gate::new();
        let f = fixture_with_inspector(
            Arc::new(|_, _| Ok(list())),
            GENEROUS,
            Duration::from_secs(60),
            gate.inspector(),
            Duration::from_millis(300),
        );
        // Twelve requests compete for four workers. All four workers remain inside the
        // simulated syscall until after EVERY request, including queued ones, has returned.
        let pending: Vec<_> = (0..MAX_INSPECTORS * 3)
            .map(|_| spawn_handle(&f.shared, sign_tx(BOB)))
            .collect();
        gate.wait(&gate.entered, MAX_INSPECTORS).await;
        assert_eq!(f.shared.inspectors.available_permits(), 0);
        let responses = tokio::time::timeout(Duration::from_secs(3), async {
            let mut responses = Vec::new();
            for request in pending {
                responses.push(request.await.unwrap());
            }
            responses
        })
        .await
        .expect("queued requests did not respect the inspection deadline");
        assert!(
            responses.iter().any(|r| r
                .error
                .as_deref()
                .is_some_and(|e| e.contains("inspected in time"))),
            "no inspection timed out"
        );
        for response in &responses {
            assert_locked(response);
        }
        assert!(
            f.shared.session().is_none(),
            "inspection timeout retained the seed"
        );
        assert_eq!(
            gate.count(&gate.entered),
            MAX_INSPECTORS,
            "queued request started another worker"
        );
        assert_eq!(gate.count(&gate.max_live), MAX_INSPECTORS);
        assert_eq!(gate.count(&gate.finished), 0);
        assert_eq!(
            f.shared.inspectors.available_permits(),
            0,
            "timed-out syscall released its permit before returning"
        );
        gate.release();
        gate.wait(&gate.finished, MAX_INSPECTORS).await;
        // The counter is updated inside the inspector, just before its caller releases the
        // permit. Wait for that separate action, instead of racing the worker's final steps.
        tokio::time::timeout(Duration::from_secs(3), async {
            while f.shared.inspectors.available_permits() != MAX_INSPECTORS {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(gate.count(&gate.live), 0);
        assert_eq!(gate.count(&gate.entered), MAX_INSPECTORS);
        assert_locked(&handle(&f.shared, &sign_tx(BOB)).await.0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn expiry_does_not_wait_for_wallet_or_socket_inspection() {
        let gate = Gate::new();
        let f = fixture_with_inspector(
            Arc::new(|_, _| Ok(list())),
            GENEROUS,
            Duration::from_millis(500),
            gate.inspector(),
            GENEROUS,
        );
        f._listener.set_nonblocking(true).unwrap();
        let listener = UnixListener::from_std(f._listener.try_clone().unwrap()).unwrap();
        let shared = f.shared.clone();
        // Exercise the real server loop: this test never calls Shared::check directly.
        // Only its independent expiry path can wipe the seed while its monitor is blocked.
        let serving = tokio::spawn(async move { serve_until_done(listener, &shared).await });
        gate.wait(&gate.entered, 1).await;
        let request = spawn_handle(&f.shared, sign_tx(BOB));
        tokio::time::timeout(Duration::from_secs(3), serving)
            .await
            .unwrap()
            .unwrap();
        assert!(f.shared.session().is_none(), "the seed outlived expiry");
        assert_eq!(
            gate.count(&gate.finished),
            0,
            "the test did not stall the inspector"
        );
        gate.release();
        assert_locked(&request.await.unwrap());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn inspection_timeout_ends_session_and_late_success_cannot_sign() {
        let gate = Gate::new();
        let f = fixture_with_inspector(
            Arc::new(|_, _| Ok(list())),
            GENEROUS,
            Duration::from_secs(60),
            gate.inspector(),
            Duration::from_millis(100),
        );
        let start = Instant::now();
        let (response, stop) = handle(&f.shared, &sign_tx(BOB)).await;
        assert_locked(&response);
        assert!(stop);
        assert!(response.error.unwrap().contains("inspected in time"));
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(f.shared.session().is_none());
        gate.wait(&gate.entered, 1).await;
        gate.release();
        gate.wait(&gate.finished, 1).await;
        assert_locked(&handle(&f.shared, &sign_tx(BOB)).await.0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn socket_replacement_during_inspection_ends_session() {
        let gate = Gate::new();
        let f = fixture_with_inspector(
            Arc::new(|_, _| Ok(list())),
            GENEROUS,
            Duration::from_secs(60),
            gate.inspector(),
            GENEROUS,
        );
        let pending = spawn_handle(&f.shared, sign_tx(BOB));
        gate.wait(&gate.entered, 1).await;
        fs::remove_file(&f.shared.sock).unwrap();
        let _replacement = std::os::unix::net::UnixListener::bind(&f.shared.sock).unwrap();
        gate.release();
        let response = pending.await.unwrap();
        assert_locked(&response);
        assert!(response.error.unwrap().contains("socket was replaced"));
    }

    #[test]
    fn inspector_does_not_keep_runtime_alive_after_timeout() {
        let gate = Gate::new();
        let f = fixture_with_inspector(
            Arc::new(|_, _| Ok(list())),
            GENEROUS,
            Duration::from_secs(60),
            gate.inspector(),
            Duration::from_millis(100),
        );
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        assert_locked(&rt.block_on(handle(&f.shared, &sign_tx(BOB))).0);
        gate.wait_blocking(&gate.entered, 1);
        let start = Instant::now();
        drop(rt);
        assert!(start.elapsed() < Duration::from_secs(1));
        assert_eq!(gate.count(&gate.finished), 0);
        gate.release();
        gate.wait_blocking(&gate.finished, 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_session_keeps_the_anchor_and_never_the_whitelist_salt() {
        let unsealed = |policy: Value| store::Unsealed {
            seed: Seed::from_phrase(zeroize::Zeroizing::new(
                crate::core::seed::TEST_MNEMONIC.into(),
            ))
            .unwrap(),
            policy,
        };
        let sealed = json!({
            "whitelistPub": hex::encode([1u8; 32]),
            "whitelistGen": 3,
            "whitelistDigest": hex::encode([2u8; 32]),
            "whitelistSalt": hex::encode([9u8; 32]),
        });
        // The session's anchor is exactly the public key, the generation and the digest: the
        // pattern names every field, so a field added to it (the salt, key material) fails to
        // build here.
        let Anchor::Sealed(anchor) = Anchor::of(&unsealed(sealed.clone())) else {
            panic!("a sealed anchor with its salt is not Sealed");
        };
        let WhitelistAnchor {
            public_key,
            generation,
            digest,
        } = anchor;
        assert_eq!((public_key, generation, digest), ([1; 32], 3, [2; 32]));

        // zwallet 0.1.0 sealed no salt: the anchor cannot be used, so the list is Unusable and
        // nothing is signed, never NoList (which signs self-transfers).
        let mut old = sealed;
        old.as_object_mut().unwrap().remove("whitelistSalt");
        let anchor = Anchor::of(&unsealed(old));
        let Anchor::Malformed(reason) = &anchor else {
            panic!("a 0.1.0 anchor is not Malformed: {anchor:?}");
        };
        // The cause alone, as every daemon reason: a remedy is the editor's to give.
        assert_eq!(
            reason,
            "this whitelist was made by zwallet 0.1.0, whose key file is no longer used"
        );
        let never: ListReader = Arc::new(|_, _| unreachable!("no list is read"));
        let f = fixture(never, GENEROUS, GENEROUS);
        let loaded = f.shared.load(&anchor).await;
        assert!(
            matches!(loaded.view(), ListView::Unusable(r) if r == *reason),
            "not Unusable"
        );
    }
}
