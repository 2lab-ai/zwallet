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
//! trusts it only if it verifies against that anchor (`.prd/04-whitelist.md`). Each connection is
//! served in its own task
//! with I/O timeouts, while the main loop keeps the clock: on `lock`, on expiry (wall clock, so
//! system sleep counts), on SIGTERM/SIGINT, or as soon as wallet.json's ciphertext changes or
//! `d.sock` is no longer its socket (checked every tick and on every request), it drops the seed
//! (zeroized) and exits, even while connections are open. A daemon process exists only while the
//! wallet is unlocked.

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
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::unix::OwnedReadHalf;
use tokio::net::{UnixListener, UnixStream};
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::Notify;

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
    /// The whitelist anchor sealed with the seed, or why there is none to trust (no whitelist
    /// yet, or a malformed anchor: the session still serves, and signs only self-transfers).
    anchor: std::result::Result<WhitelistAnchor, String>,
}

impl Session {
    /// The list signatures are checked against right now: read on every call, never cached.
    fn list(&self, home: &Path) -> std::result::Result<Whitelist, String> {
        load_list(home, &self.anchor)
    }
}

/// The sealed anchor as the session keeps it.
fn session_anchor(unsealed: &store::Unsealed) -> std::result::Result<WhitelistAnchor, String> {
    match unsealed.whitelist_anchor() {
        Ok(Some(anchor)) => Ok(anchor),
        Ok(None) => Err(policy::NO_WHITELIST.to_string()),
        Err(e) => Err(e.message),
    }
}

/// `whitelist.json`, else a staged `whitelist.next.json`, that verifies against `anchor`.
fn load_list(
    home: &Path,
    anchor: &std::result::Result<WhitelistAnchor, String>,
) -> std::result::Result<Whitelist, String> {
    let anchor = anchor.as_ref().map_err(Clone::clone)?;
    whitelist::load_verified(home, anchor).map_err(|e| e.message)
}

fn view(list: &std::result::Result<Whitelist, String>) -> ListView<'_> {
    match list {
        Ok(list) => ListView::Usable(list),
        Err(reason) => ListView::Unusable(reason.clone()),
    }
}

/// What `unlock` and `status` show of the list: `{ gen, verified, entries: [nicknames] }`, or
/// `{ gen (the sealed one, if any), verified: false, entries: [], reason }`.
fn list_summary(
    anchor: &std::result::Result<WhitelistAnchor, String>,
    list: &std::result::Result<Whitelist, String>,
) -> Value {
    match list {
        Ok(list) => json!({
            "gen": list.generation,
            "verified": true,
            "entries": list.entries.iter().map(|e| e.nickname.as_str()).collect::<Vec<_>>(),
        }),
        Err(reason) => json!({
            "gen": anchor.as_ref().ok().map(|a| a.generation),
            "verified": false,
            "entries": [],
            "reason": reason,
        }),
    }
}

/// What the main loop and the connection tasks share.
struct Shared {
    home: PathBuf,
    sock: PathBuf,
    /// `(dev, ino)` of the socket this daemon bound.
    sock_id: (u64, u64),
    session: Mutex<Option<Session>>,
    /// A connection task asks the main loop to stop (after `lock`, or after it saw tampering).
    stop: Notify,
}

impl Shared {
    /// Whether the session must end now, and why. Ends it (zeroizing the seed) if so.
    fn check(&self, session: &mut Option<Session>) -> Option<&'static str> {
        let reason = match session.as_ref() {
            None => Some("the wallet is locked"),
            Some(s) if SystemTime::now() >= s.expires_at => Some("the unlock expired"),
            Some(_) if !self.socket_is_ours() => Some("the daemon socket was replaced or removed"),
            Some(s) if !self.wallet_unchanged(&s.sealed) => {
                Some("wallet.json changed since unlock")
            }
            Some(_) => None,
        };
        if reason.is_some() {
            *session = None;
        }
        reason
    }

    fn socket_is_ours(&self) -> bool {
        fs::symlink_metadata(&self.sock)
            .is_ok_and(|m| m.file_type().is_socket() && (m.dev(), m.ino()) == self.sock_id)
    }

    fn wallet_unchanged(&self, sealed: &str) -> bool {
        store::read_wallet_file(&self.home).is_ok_and(|f| f.crypto.ciphertext == sealed)
    }

    fn session(&self) -> std::sync::MutexGuard<'_, Option<Session>> {
        self.session.lock().unwrap_or_else(PoisonError::into_inner)
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
        remove_own_socket(&shared);
        return Err(WalletError::validation("the unlocking process went away"));
    }
    serve_until_done(listener, &shared).await;
    *shared.session() = None; // zeroizes the seed
    remove_own_socket(&shared);
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
    let anchor = session_anchor(&unsealed);
    let ready = json!({
        "locked": false,
        "expiresAt": rfc3339_utc(unix_secs(expires_at)),
        "active": file.active,
        "index": key.index,
        "address": address,
        "pid": std::process::id(),
        "whitelist": list_summary(&anchor, &load_list(home, &anchor)),
    });
    let shared = Arc::new(Shared {
        home: home.to_path_buf(),
        sock,
        sock_id: (meta.dev(), meta.ino()),
        session: Mutex::new(Some(Session {
            seed: unsealed.seed,
            expires_at,
            sealed: file.crypto.ciphertext.clone(),
            anchor,
        })),
        stop: Notify::new(),
    });
    // wallet.json may have changed while this daemon waited for the go byte.
    if let Some(reason) = shared.check(&mut shared.session()) {
        remove_own_socket(&shared);
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

fn remove_own_socket(shared: &Shared) {
    if shared.socket_is_ours() {
        let _ = fs::remove_file(&shared.sock);
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
    let (response, stop) = handle(&shared, &request);
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
fn handle(shared: &Shared, bytes: &[u8]) -> (Response, bool) {
    let Ok(req) = serde_json::from_slice::<Request>(bytes) else {
        let e = WalletError::validation("malformed daemon request");
        return (Response::err(&e), false);
    };
    let pid = std::process::id();
    let mut session = shared.session();
    if let Some(reason) = shared.check(&mut session) {
        let resp = match req {
            Request::Status => {
                Response::ok(json!({ "locked": true, "pid": pid, "reason": reason }))
            }
            Request::Lock => Response::ok(json!({ "locked": true, "wasUnlocked": false })),
            _ => Response::err(&WalletError::new(
                ErrorCode::WalletLocked,
                format!("{reason}; locked. A human must run `zwallet unlock 24h` (any duration)"),
            )),
        };
        return (resp, true);
    }
    let Some(s) = session.as_ref() else {
        return (Response::err(&WalletError::locked()), true);
    };
    let result = match &req {
        Request::Lock => {
            *session = None;
            return (
                Response::ok(json!({ "locked": true, "wasUnlocked": true })),
                true,
            );
        }
        Request::Status => Ok(json!({
            "locked": false,
            "expiresAt": rfc3339_utc(unix_secs(s.expires_at)),
            "pid": pid,
            "whitelist": list_summary(&s.anchor, &s.list(&shared.home)),
        })),
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
            let list = s.list(&shared.home);
            chain
                .family
                .keys()
                .sign_message(&s.seed, *index, chain, &message, &view(&list))
        }),
        Request::SignTx { chain, index, tx } => chains::find(chain).and_then(|chain| {
            let list = s.list(&shared.home);
            chain
                .family
                .keys()
                .sign_tx(&s.seed, *index, chain, tx, &view(&list))
        }),
    };
    let resp = match result {
        Ok(v) => Response::ok(v),
        Err(e) => Response::err(&e),
    };
    (resp, false)
}
