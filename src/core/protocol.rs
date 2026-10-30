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

mod unlock_channel;

// Preserve the public protocol API; the private child channel owns framing, secret reads,
// and progress responses, while this module coordinates process identity and lifecycle.
pub use unlock_channel::{
    GO, STAGE_DECRYPTED, STAGE_INPUT, decode_unlock_header, encode_unlock_input, read_go,
    read_unlock_input,
};
use unlock_channel::{expect_stage, line_reader, next_response};

use crate::core::error::{ErrorCode, Result, WalletError};
use crate::core::store;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io::{ErrorKind, Write};
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
    /// The error's machine-readable fields ([`WalletError::details`], e.g. the whitelist command
    /// a human runs to allow a refused signature). Never secrets.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub details: Map<String, Value>,
}

impl Response {
    pub fn ok(result: Value) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            ok: true,
            result: Some(result),
            error: None,
            error_code: None,
            details: Map::new(),
        }
    }

    pub fn err(e: &WalletError) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            ok: false,
            result: None,
            error: Some(e.message.clone()),
            error_code: Some(e.code),
            details: e.details.clone(),
        }
    }

    pub fn into_result(self) -> Result<Value> {
        if self.ok {
            Ok(self.result.unwrap_or(Value::Null))
        } else {
            let mut e = WalletError::new(
                self.error_code.unwrap_or(ErrorCode::ValidationError),
                self.error.unwrap_or_else(|| "daemon error".into()),
            );
            for (k, v) in self.details {
                e = e.with_detail(&k, v);
            }
            Err(e)
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
    Ok(send_answered(home, req, timeout).await?.map(|a| a.value))
}

/// The process at the other end of a connection to `d.sock`, as the kernel reports it when the
/// connection is made. On macOS: the audit token of the process that last used the listening
/// socket (`LOCAL_PEERTOKEN`, its pid and pid version), and the UUID of the executable that
/// process ran when it did (`LOCAL_PEERUUID`). On Linux: the process that called listen(2)
/// (`SO_PEERCRED`). Asked at connect time: once the daemon has answered and closed its end, macOS
/// no longer says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Peer {
    pub pid: Option<i32>,
    token: Option<AuditToken>,
    exe_uuid: Option<[u8; 16]>,
}

/// A macOS `audit_token_t`: eight words, the pid in word 5 and the pid version in word 7.
type AuditToken = [u32; 8];

/// A daemon's answer to one request, and the [`Peer`] that gave it.
pub struct Answer {
    pub value: Value,
    pub peer: Peer,
}

/// [`send`], with the [`Peer`] that answered.
pub async fn send_answered(
    home: &Path,
    req: &Request,
    timeout: Duration,
) -> Result<Option<Answer>> {
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
        let token = peer_token(std::os::fd::AsRawFd::as_raw_fd(&stream));
        let who = Peer {
            pid: token.and_then(|t| i32::try_from(t[5]).ok()).or(peer.pid()),
            token,
            exe_uuid: peer_exe_uuid(&stream),
        };
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
            WalletError::validation(format!(
                "malformed reply from the key daemon: {}",
                crate::core::text::ascii_inert(&e.to_string(), crate::core::text::PROSE_MAX)
            ))
        })?;
        if !resp.ok {
            return Err(relayed_error(home, &who, resp));
        }
        Ok(Some(Answer {
            value: resp.result.unwrap_or(Value::Null),
            peer: who,
        }))
    };
    match tokio::time::timeout(timeout, exchange).await {
        Ok(r) => r,
        Err(_) => Err(WalletError::validation(format!(
            "the key daemon did not answer within {}s",
            timeout.as_secs()
        ))),
    }
}

/// The detail keys a key daemon's error carries (the signing policy's refusals): the only ones
/// relayed, and only when they are a string or a number. Any other key, and any array or object
/// value, is dropped, so nothing that answers on `d.sock` can put `address`, `verified` or a
/// nested blob into this CLI's error object.
const RELAYED_DETAILS: &[&str] = &["command", "counterparty", "chain", "feeWei", "capWei"];

/// The error a reply with `ok: false` carries, as this CLI shows it. Whatever answers on `d.sock`
/// writes that text, so it is made inert (printable ASCII only) where the error is built, and the
/// message with its kept details shares one [`PROSE_MAX`](crate::core::text::PROSE_MAX)-character
/// budget: the message first, then each [`RELAYED_DETAILS`] detail in order, each cut to what the
/// budget leaves, and the rest dropped. So the human output of one relayed error is at most that
/// many characters whatever the reply's size or nesting. Who answered is checked ([`check_peer`]):
/// - this build's key daemon: its message as is (inert);
/// - not known (`Unknown`; every genuine daemon on Linux): prefixed `the process answering on the
///   key daemon socket reported: `, so it never reads as this CLI's own output nor claims who sent
///   it;
/// - another process (`Other`): named (its executable path cut to [`RELAYED_EXE_SHOWN`]), its
///   message quoted after it, its details dropped (an impostor's `command` is no command to run),
///   with the `warning` detail. A `WALLET_LOCKED` reply from a process running this build's code
///   gets the neutral prefix instead ([`relayed_check`]), and still no details: a genuine
///   `WALLET_LOCKED` carries none.
///
/// When the check itself fails, the details are dropped too. The error code is kept: it is one of
/// a fixed set.
fn relayed_error(home: &Path, who: &Peer, resp: Response) -> WalletError {
    use crate::core::text::{PROSE_MAX, ascii_inert};
    let code = resp.error_code.unwrap_or(ErrorCode::ValidationError);
    let checked = check_peer_shown(home, who, RELAYED_EXE_SHOWN);
    let judged = checked.as_ref().ok().cloned();
    let check = relayed_check(who, resp.error_code, checked.unwrap_or(PeerCheck::Unknown));
    // Details only from a peer judged Unknown or this build's daemon as it is, never from one
    // judged another process and let through as Unknown by the `WALLET_LOCKED` exception.
    let details_ok = match judged {
        None => false,
        Some(PeerCheck::Other(_)) => false,
        Some(_) => true,
    };

    // The message, then (for a known key daemon or an Unknown peer) the scalar kept details in a
    // fixed order, sharing one budget.
    let raw_said = resp.error.as_deref().unwrap_or("daemon error");
    if let PeerCheck::Other(reason) = &check {
        let said = ascii_inert(raw_said, PROSE_MAX);
        let e = WalletError::new(code, format!("{reason}, and it reported: {said}; {RELOCK}"));
        return match check.warning() {
            Some(w) => e.with_detail("warning", w),
            None => e,
        };
    }
    let mut budget = PROSE_MAX;
    let said = ascii_inert(raw_said, budget);
    budget = budget.saturating_sub(said.chars().count());
    let message = match &check {
        PeerCheck::Unknown => {
            format!("the process answering on the key daemon socket reported: {said}")
        }
        _ => said,
    };
    let mut details = Map::new();
    if details_ok {
        for key in RELAYED_DETAILS {
            if budget == 0 {
                break;
            }
            match resp.details.get(*key) {
                Some(Value::String(s)) => {
                    let v = ascii_inert(s, budget);
                    budget = budget.saturating_sub(v.chars().count());
                    details.insert((*key).into(), Value::String(v));
                }
                // A number cannot be cut: kept only when it fits what is left of the budget.
                Some(n @ Value::Number(_)) => {
                    let len = n.to_string().len();
                    if len <= budget {
                        budget -= len;
                        details.insert((*key).into(), n.clone());
                    }
                }
                _ => {}
            }
        }
    }
    let mut e = WalletError::new(code, message);
    e.details = details;
    e
}

/// What [`relayed_error`] makes of a peer [`check_peer`] judged, given the reply's error `code`.
/// A daemon that ends its session (lock or expiry) answers `WALLET_LOCKED`, then drops the socket
/// and exits, so by the time its error is checked it no longer holds the socket nor shows in the
/// process table, and `check_peer` calls it another process. So a `WALLET_LOCKED` reply from any
/// process running this build's code (same cdhash, `CS_VALID`; not only a daemon whose session
/// just ended) is not named an impostor: `Unknown`, with no details ([`relayed_error`]). Any
/// other code keeps `Other` (named, details dropped). This never makes anything `ThisBuild`, and
/// it is used for relayed errors only; address verification keeps the strict check.
fn relayed_check(who: &Peer, code: Option<ErrorCode>, check: PeerCheck) -> PeerCheck {
    match check {
        PeerCheck::Other(_) if code == Some(ErrorCode::WalletLocked) && runs_this_build(who) => {
            PeerCheck::Unknown
        }
        other => other,
    }
}

/// Whether `peer` runs this build's code, by the kernel: on macOS its cdhash (through its audit
/// token) is this process's own and it is `CS_VALID`. Elsewhere this cannot be told: `false`.
fn runs_this_build(peer: &Peer) -> bool {
    #[cfg(target_os = "macos")]
    {
        let (Some(token), Some(own)) = (peer.token.as_ref(), own_cdhash()) else {
            return false;
        };
        peer_cdhash(token) == Some(own) && peer_cs_valid(token) == Some(true)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = peer;
        false
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
         password; set kernel.yama.ptrace_scope >= 1 (sysctl), or pass --allow-ptrace-scope-0 to \
         accept that risk"
    )))
}

/// The mount table of this process's mount namespace (Linux).
pub const MOUNTINFO: &str = "/proc/self/mountinfo";

/// Before a daemon is started (and before the password is asked for): on Linux, refuses where
/// `/proc` may hide this user's own key daemons ([`proc_hiding`]), since `lock` could not find
/// one there to stop it. There is no flag to accept that. Elsewhere this checks nothing.
pub fn check_daemons_findable() -> Result<()> {
    #[cfg(target_os = "linux")]
    if let Some(why) = proc_hiding_at(Path::new(MOUNTINFO)) {
        return Err(WalletError::validation(format!(
            "{why}: zwallet could not find its own key daemon to lock it; refusing to start one"
        )));
    }
    Ok(())
}

/// [`proc_hiding`] for the mount table at `path`; one that cannot be read counts as hiding.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn proc_hiding_at(path: &Path) -> Option<String> {
    match std::fs::read(path) {
        Ok(table) => proc_hiding(&table),
        Err(e) => Some(format!(
            "cannot read {}: {e}, so how /proc is mounted is unknown",
            path.display()
        )),
    }
}

/// Why the `/proc` of a mount table in `/proc/<pid>/mountinfo` form may hide this user's own key
/// daemons, or `None` when it cannot. A daemon is non-dumpable, and under `hidepid` (1 or
/// `noaccess`, 2 or `invisible`, 4 or `ptraceable`) the kernel hides a non-dumpable process from
/// the other processes of its own user too: its `/proc/<pid>` is unreadable or unlisted. Only
/// proc mounts at `/proc` without `hidepid` (or with 0 or `off`) are safe: no proc mount there,
/// another file system there, a line whose options cannot be read, or any other `hidepid` value
/// counts as hiding. `subset=pid` hides no process, and `gid=` is not looked at.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn proc_hiding(mountinfo: &[u8]) -> Option<String> {
    let mut found = false;
    for line in mountinfo.split(|b| *b == b'\n') {
        // `id parent major:minor root mount-point mount-options [optional...] - fstype source
        // super-options`; `hidepid` is a super option.
        let fields: Vec<&[u8]> = line.split(|b| *b == b' ').collect();
        if fields.get(4).copied() != Some(&b"/proc"[..]) {
            continue;
        }
        let after_dash = fields
            .iter()
            .skip(6)
            .position(|f| *f == b"-")
            .map(|i| &fields[i + 7..]);
        let Some([fstype, _source, options, ..]) = after_dash else {
            return Some("cannot read the options of the mount at /proc".into());
        };
        if *fstype != b"proc" {
            return Some(format!(
                "/proc is a {} mount, not proc",
                String::from_utf8_lossy(fstype)
            ));
        }
        for option in options.split(|b| *b == b',') {
            if let Some(value) = option.strip_prefix(b"hidepid=")
                && value != b"0"
                && value != b"off"
            {
                return Some(format!(
                    "/proc is mounted with hidepid={}",
                    String::from_utf8_lossy(value)
                ));
            }
        }
        found = true;
    }
    (!found).then(|| "no proc file system is mounted at /proc".into())
}

/// The binary `unlock` starts as the daemon, pinned before the password prompt. On Linux that is
/// the image this process runs (`/proc/self/exe`), so a binary swapped in at that path meanwhile
/// is never started. Elsewhere it is the canonical path of `current_exe()`, whose (dev, ino) is
/// recorded now and checked again right before the spawn: that refuses a binary swapped in while
/// the human types. One renamed in between that check and the exec is still started (macOS has
/// no `fexecve`). On macOS, once the child asks for input and before the password is written,
/// [`check_started_code`] refuses a child that does not run this build's code (its cdhash is not
/// this process's own, or it is not `CS_VALID`): it is killed and given nothing. A build that is
/// not code-signed cannot make that check, so [`Self::resolve`] refuses it before the prompt;
/// every macOS binary zwallet ships is ad-hoc signed, with the hardened runtime.
///
/// That check is partial hardening. It does not prove that no other process holds the channel:
/// the cdhash is read once the child asks for input, so a binary swapped in as it starts can fork
/// a process that keeps the channel and then exec this build, which passes, and that process can
/// read the password. It could also exec this build with `DYLD_INSERT_LIBRARIES`, which the
/// hardened runtime of the shipped binaries makes dyld ignore while System Integrity Protection
/// is on (a build from source has no hardened runtime, and with SIP off dyld loads it). A
/// binary swapped in before `unlock` started is this process, and gets the password on any OS.
/// The only real protection against a swapped binary is to install zwallet where an agent cannot
/// write it (the README's "Known limits").
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
            // Without a cdhash of its own this build cannot check the daemon it starts
            // ([`check_started_code`]): refused here, before the password is asked for.
            #[cfg(target_os = "macos")]
            if own_cdhash().is_none() {
                return Err(WalletError::validation(format!(
                    "cannot read this zwallet build's code-signing identity, so it cannot check \
                     that the key daemon it would start runs its code; not asking for the \
                     password. {}",
                    sign_this_build(&exe)
                )));
            }
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

/// Fails unless the daemon `child` that [`spawn_daemon`] started runs this build's code (macOS):
/// its cdhash is this process's own and it is `CS_VALID`. Asked by pid, which is safe here: the
/// child has not been reaped, so its pid names no other process. Every `csops` that fails refuses
/// it: unsigned code, a child that already exited, or this build's own cdhash (a build that is
/// not code-signed; [`DaemonExe::resolve`] refuses that one before the prompt).
///
/// This refuses a started process that does not run this build's code. It does not prove that no
/// other process holds the channel: a binary swapped in as it starts can fork a process that
/// keeps the channel, then exec this build, which passes; that process can read the password.
#[cfg(target_os = "macos")]
fn check_started_code(child: &Child) -> Result<()> {
    let pid = child.id() as libc::pid_t;
    // This binary's path, for the message only (`DaemonExe::resolve` names the pinned one).
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("zwallet"));
    started_code_verdict(pid, own_cdhash(), pid_cdhash(pid), pid_cs_valid(pid), &exe)
}

/// [`check_started_code`]'s decision on what the kernel said: `own` is this process's cdhash,
/// `child` and `child_valid` the started daemon's (pid `pid`) cdhash and `CS_VALID`, each `None`
/// when `csops` failed. `exe` (this binary) is only named in the message.
#[cfg(target_os = "macos")]
fn started_code_verdict(
    pid: libc::pid_t,
    own: Option<[u8; CDHASH_LEN]>,
    child: Option<[u8; CDHASH_LEN]>,
    child_valid: Option<bool>,
    exe: &Path,
) -> Result<()> {
    let Some(own) = own else {
        // `DaemonExe::resolve` refuses such a build before the prompt; this is in case the read
        // fails only now. Nothing is accepted for want of a cdhash.
        return Err(WalletError::validation(format!(
            "cannot read this zwallet build's code-signing identity, so it cannot check that the \
             key daemon it started (pid {pid}) runs its code: it was stopped and nothing was \
             given to it. {}",
            sign_this_build(exe)
        )));
    };
    if child == Some(own) && child_valid == Some(true) {
        return Ok(());
    }
    Err(WalletError::validation(format!(
        "the key daemon this command started (pid {pid}) does not run this zwallet build's code \
         (the zwallet binary may have been replaced as it was started), or exited before it could \
         be checked. It was stopped and nothing was given to it; no new session was started, and \
         this did not stop a session that was already running"
    )))
}

/// The fix when this build's own cdhash cannot be read because it is unsigned (an x86_64 build:
/// the linker signs only arm64 output): the `codesign` line for the binary at `path`, quoted for
/// the shell so it copies and runs as it is.
#[cfg(target_os = "macos")]
fn sign_this_build(path: &Path) -> String {
    format!(
        "If this is an unsigned build, sign it: `codesign --force --sign - {}` (zwallet's release \
         binaries are signed)",
        shell_quoted(&path.to_string_lossy())
    )
}

/// `s` as one single-quoted shell word: `'` becomes `'\''`.
#[cfg(target_os = "macos")]
fn shell_quoted(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

// ---- Who answered on `d.sock` ----
//
// The peer uid check lets any process of this user answer, so an address a daemon derives proves
// nothing by itself: a 20-line script on `d.sock` answers what it likes. [`check_peer`] asks the
// kernel instead which program answered: only this build's own key daemon for the home counts.

/// The identity of a file, as `stat` follows links to it: `(dev, ino)`.
pub fn file_id(path: &Path) -> Option<(u64, u64)> {
    std::fs::metadata(path).ok().map(|m| (m.dev(), m.ino()))
}

/// What [`check_peer`] establishes about a connection's [`Peer`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PeerCheck {
    /// This build's key daemon for the home. On macOS, for a signed build (every build zwallet
    /// ships): the kernel-validated code-signing hash (cdhash) of the code the peer is running
    /// equals this process's own, the peer is `CS_VALID`, it holds the listening `d.sock` of this
    /// home, its argv is in `daemon_pids`, and its socket-use executable UUID matches. Never on
    /// Linux, nor for a build that is not code-signed (it has no cdhash of its own).
    ThisBuild,
    /// Another program, another zwallet build (say the one before a `brew upgrade`), or not the
    /// key daemon of this home: why, naming the process (without advice: [`PeerCheck::warning`]
    /// adds [`RELOCK`] where it is shown).
    Other(String),
    /// Nothing can be established, so nothing is verified: a kernel query failed or the process
    /// is gone, or (on Linux) the peer cannot be the genuine key daemon's but may be this build.
    /// On Linux every zwallet process is non-dumpable (see `password::disable_core_dumps`), which
    /// hides `/proc/<pid>/exe` from its own user, so a key daemon's executable is always
    /// unreadable there; a readable one that runs this very build is no genuine daemon either.
    Unknown,
}

// ---- Linux: the peer's executable file ----
//
// A key daemon is non-dumpable, so its `/proc/<pid>/exe` is unreadable even to its own user:
// `check_peer` can never read it and returns `Unknown` for the genuine daemon. Nothing is
// verified by session on Linux. The code is here so that `address`, `addresses`, `change`,
// `status`, `balance` and `list` still name an impostor whose executable is readable (another
// program): in their `warning`, or in the error of a mismatch (`list` never claims verification
// without the password; who answered only words its error and its `warning`).

/// The executable of a running process, as the kernel names it.
#[cfg(target_os = "linux")]
enum ProcessExe {
    /// Its path, and the identity of the file there (`None` when it no longer exists).
    File(PathBuf, Option<(u64, u64)>),
    /// Hidden from this process, or the process is gone.
    Unreadable,
}

/// The executable `pid` runs: Linux `/proc/<pid>/exe`, stat'ed through the link (the very file,
/// whatever its path holds now). A target the kernel marks ` (deleted)` is no file at all.
#[cfg(target_os = "linux")]
fn process_exe(pid: i32) -> ProcessExe {
    let link = PathBuf::from(format!("/proc/{pid}/exe"));
    let Ok(meta) = std::fs::metadata(&link) else {
        // EACCES for a non-dumpable process (every zwallet daemon), ENOENT once it exited.
        return ProcessExe::Unreadable;
    };
    let Ok(path) = std::fs::read_link(&link) else {
        return ProcessExe::Unreadable;
    };
    let deleted = path.as_os_str().as_bytes().ends_with(b" (deleted)");
    let id = (!deleted).then(|| (meta.dev(), meta.ino()));
    ProcessExe::File(path, id)
}

// ---- macOS: the peer's code, by its code-signing hash ----
//
// `proc_pidpath` and `LOCAL_PEERUUID` both name a path or a value the Mach-O declares about
// itself: a same-user attacker can exec its own code from a path it then symlinks to the genuine
// binary (so dev/ino match) and embed the genuine `LC_UUID`, and it passes. So the decision rests
// on the cdhash the kernel validates against the code pages actually mapped (arm64 macOS runs only
// signed code; the linker ad-hoc signs arm64 output only, so CI signs every macOS binary zwallet
// ships, x86_64 included). `csops`/`csops_audittoken` live in libsystem.
#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn csops(
        pid: libc::pid_t,
        ops: u32,
        useraddr: *mut libc::c_void,
        usersize: libc::size_t,
    ) -> libc::c_int;
    fn csops_audittoken(
        pid: libc::pid_t,
        ops: u32,
        useraddr: *mut libc::c_void,
        usersize: libc::size_t,
        token: *const AuditToken,
    ) -> libc::c_int;
}

/// `CS_OPS_STATUS`: read the code-signing status word.
#[cfg(target_os = "macos")]
const CS_OPS_STATUS: u32 = 0;
/// `CS_OPS_CDHASH`: read the 20-byte code directory hash.
#[cfg(target_os = "macos")]
const CS_OPS_CDHASH: u32 = 5;
/// `CS_VALID`: the kernel has found nothing invalid about the process's signature.
#[cfg(target_os = "macos")]
const CS_VALID: u32 = 0x0000_0001;
#[cfg(target_os = "macos")]
const CDHASH_LEN: usize = 20;

/// This process's own cdhash (`csops` on `getpid`). `None` for a build the linker did not sign
/// (x86_64 macOS): `csops` has no cdhash for unsigned code.
#[cfg(target_os = "macos")]
fn own_cdhash() -> Option<[u8; CDHASH_LEN]> {
    pid_cdhash(std::process::id() as libc::pid_t)
}

/// The cdhash of the process `pid` (`csops`). Only for a pid that cannot name another process by
/// now: this process's own, or its child that has not been reaped.
#[cfg(target_os = "macos")]
fn pid_cdhash(pid: libc::pid_t) -> Option<[u8; CDHASH_LEN]> {
    let mut hash = [0u8; CDHASH_LEN];
    // SAFETY: csops writes at most CDHASH_LEN bytes into `hash` for `pid`.
    let rc = unsafe { csops(pid, CS_OPS_CDHASH, hash.as_mut_ptr().cast(), CDHASH_LEN) };
    (rc == 0).then_some(hash)
}

/// Whether the process `pid` is `CS_VALID` (`csops`), with the caveat of [`pid_cdhash`].
#[cfg(target_os = "macos")]
fn pid_cs_valid(pid: libc::pid_t) -> Option<bool> {
    let mut status: u32 = 0;
    // SAFETY: csops writes one u32 of status for `pid`.
    let rc = unsafe {
        csops(
            pid,
            CS_OPS_STATUS,
            (&mut status as *mut u32).cast(),
            std::mem::size_of::<u32>(),
        )
    };
    (rc == 0).then_some(status & CS_VALID != 0)
}

/// The cdhash of the process `token` names (`csops_audittoken`, so pid reuse cannot swap the
/// target between the check and now).
#[cfg(target_os = "macos")]
fn peer_cdhash(token: &AuditToken) -> Option<[u8; CDHASH_LEN]> {
    let mut hash = [0u8; CDHASH_LEN];
    // SAFETY: csops_audittoken writes at most CDHASH_LEN bytes into `hash` for `token`'s pid.
    let rc = unsafe {
        csops_audittoken(
            token[5] as libc::pid_t,
            CS_OPS_CDHASH,
            hash.as_mut_ptr().cast(),
            CDHASH_LEN,
            token,
        )
    };
    (rc == 0).then_some(hash)
}

/// Whether the process `token` names is `CS_VALID` (`csops_audittoken`).
#[cfg(target_os = "macos")]
fn peer_cs_valid(token: &AuditToken) -> Option<bool> {
    let mut status: u32 = 0;
    // SAFETY: csops_audittoken writes one u32 of status for `token`'s pid.
    let rc = unsafe {
        csops_audittoken(
            token[5] as libc::pid_t,
            CS_OPS_STATUS,
            (&mut status as *mut u32).cast(),
            std::mem::size_of::<u32>(),
            token,
        )
    };
    (rc == 0).then_some(status & CS_VALID != 0)
}

/// Offsets into `socket_fdinfo` (`<sys/proc_info.h>`), confirmed with `offsetof` on arm64 and
/// x86_64 macOS (the fields are fixed-width, so the offsets match on both). `soi_options` is a
/// `short`, `soi_kind` an `int`; `unsi_addr` is a `sockaddr_un` whose `sun_path` begins two bytes
/// in (`sun_len`, `sun_family`).
#[cfg(target_os = "macos")]
mod sock_fdinfo {
    pub const SIZE: usize = 792;
    pub const SOI_OPTIONS: usize = 188;
    pub const SOI_KIND: usize = 256;
    pub const SUN_PATH: usize = 282;
    /// `SOCKINFO_UN`.
    pub const KIND_UN: i32 = 3;
    /// `PROC_PIDFDSOCKETINFO` (`<sys/proc_info.h>`); libc does not re-export it.
    pub const FDSOCKETINFO: libc::c_int = 3;
}

/// Whether `pid` holds a listening (`SO_ACCEPTCONN`) `AF_UNIX` socket bound at `path`
/// (`proc_pidinfo`/`proc_pidfdinfo`). `None` if its descriptors cannot be listed: the genuine
/// daemon binds and holds `d.sock`, and closes every inherited descriptor at start-up, so an
/// attacker that exec'd this binary while squatting the socket on an inherited fd holds no such
/// socket. The bound path is compared in full, so a listener on another home does not count.
#[cfg(target_os = "macos")]
fn holds_listening_socket(pid: i32, path: &Path) -> Option<bool> {
    let want = path.as_os_str().as_bytes();
    // The descriptor table: an array of proc_fdinfo (fd: i32, type: u32).
    let entry = std::mem::size_of::<libc::proc_fdinfo>();
    // SAFETY: a null buffer asks only for the size the list needs now.
    let need =
        unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDLISTFDS, 0, std::ptr::null_mut(), 0) };
    if need <= 0 {
        return None;
    }
    let n = need as usize / entry + 32;
    let mut fds = vec![
        libc::proc_fdinfo {
            proc_fd: 0,
            proc_fdtype: 0,
        };
        n
    ];
    let size = libc::c_int::try_from(fds.len() * entry).ok()?;
    // SAFETY: the kernel writes at most `size` bytes of proc_fdinfo into `fds`.
    let got =
        unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDLISTFDS, 0, fds.as_mut_ptr().cast(), size) };
    if got <= 0 {
        return None;
    }
    let count = got as usize / entry;
    for info in &fds[..count.min(fds.len())] {
        if info.proc_fdtype != libc::PROX_FDTYPE_SOCKET as u32 {
            continue;
        }
        let mut buf = [0u8; sock_fdinfo::SIZE];
        // SAFETY: the kernel writes at most SIZE bytes of socket_fdinfo into `buf`.
        let r = unsafe {
            libc::proc_pidfdinfo(
                pid,
                info.proc_fd,
                sock_fdinfo::FDSOCKETINFO,
                buf.as_mut_ptr().cast(),
                sock_fdinfo::SIZE as libc::c_int,
            )
        };
        if r < sock_fdinfo::SIZE as libc::c_int {
            continue;
        }
        let kind = i32::from_ne_bytes(buf[sock_fdinfo::SOI_KIND..][..4].try_into().ok()?);
        let options = i16::from_ne_bytes(buf[sock_fdinfo::SOI_OPTIONS..][..2].try_into().ok()?);
        if kind != sock_fdinfo::KIND_UN || options as i32 & libc::SO_ACCEPTCONN == 0 {
            continue;
        }
        let tail = &buf[sock_fdinfo::SUN_PATH..];
        let end = tail.iter().position(|b| *b == 0).unwrap_or(tail.len());
        if &tail[..end] == want {
            return Some(true);
        }
    }
    Some(false)
}

/// The executable this process runs, in the same terms as a [`Peer`]: on Linux its `(dev, ino)`;
/// on macOS its cdhash and its socket-use UUID.
struct OwnExe {
    #[cfg(target_os = "linux")]
    id: (u64, u64),
    #[cfg(target_os = "macos")]
    cdhash: [u8; CDHASH_LEN],
    #[cfg(target_os = "macos")]
    uuid: Option<[u8; 16]>,
}

fn own_exe() -> Option<OwnExe> {
    #[cfg(target_os = "linux")]
    {
        Some(OwnExe {
            id: file_id(Path::new("/proc/self/exe"))?,
        })
    }
    #[cfg(target_os = "macos")]
    {
        let (a, b) = ChildChannel::pair().ok()?;
        (&a).write_all(&[0]).ok()?;
        Some(OwnExe {
            cdhash: own_cdhash()?,
            uuid: Some(local_peer_uuid(std::os::fd::AsRawFd::as_raw_fd(&b))?),
        })
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

/// `LOCAL_PEERUUID` of the connected unix socket `fd`: the UUID of the executable the peer
/// process ran when it last used its end. It does not follow a later `exec`, so it, with the
/// cdhash, pins the code that opened the socket.
#[cfg(target_os = "macos")]
fn local_peer_uuid(fd: RawFd) -> Option<[u8; 16]> {
    let mut uuid = [0u8; 16];
    let mut len = uuid.len() as libc::socklen_t;
    // SAFETY: the kernel writes at most `len` bytes into `uuid`.
    let rc = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_LOCAL,
            libc::LOCAL_PEERUUID,
            uuid.as_mut_ptr().cast(),
            &mut len,
        )
    };
    (rc == 0 && len as usize == uuid.len()).then_some(uuid)
}

/// `LOCAL_PEERTOKEN` of the connected unix socket `fd`: the audit token of the process that last
/// used the listening socket (its pid in word 5, pid version in word 7). None off macOS.
fn peer_token(fd: RawFd) -> Option<AuditToken> {
    #[cfg(target_os = "macos")]
    {
        let mut token: AuditToken = [0; 8];
        let mut len = std::mem::size_of::<AuditToken>() as libc::socklen_t;
        // SAFETY: the kernel writes at most `len` bytes of audit token into `token`.
        let rc = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_LOCAL,
                libc::LOCAL_PEERTOKEN,
                token.as_mut_ptr().cast(),
                &mut len,
            )
        };
        (rc == 0 && len as usize == std::mem::size_of::<AuditToken>()).then_some(token)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = fd;
        None
    }
}

fn peer_exe_uuid(stream: &UnixStream) -> Option<[u8; 16]> {
    #[cfg(target_os = "macos")]
    return local_peer_uuid(std::os::fd::AsRawFd::as_raw_fd(stream));
    #[cfg(not(target_os = "macos"))]
    {
        let _ = stream;
        None
    }
}

/// The advice that ends every [`PeerCheck::Other`] warning.
pub const RELOCK: &str = "run `zwallet lock` and unlock again";

impl PeerCheck {
    /// The warning shown for an [`PeerCheck::Other`] peer: its reason, then [`RELOCK`].
    pub fn warning(&self) -> Option<String> {
        match self {
            Self::Other(reason) => Some(format!("{reason}; {RELOCK}")),
            _ => None,
        }
    }
}

/// The most characters of an executable path a warning shows (escaped, see
/// [`crate::core::text::shown`]): the path is chosen by whoever started that process.
const EXE_SHOWN: usize = 200;

/// The most characters of an executable path a relayed error names, where it shares the line
/// with what that process said.
const RELAYED_EXE_SHOWN: usize = 64;

/// Why a [`Peer`] (pid `pid`) is not this build's key daemon: `what` it is. Any value from that
/// process in `what` must already be escaped.
fn peer_not_ours(pid: i32, what: &str) -> PeerCheck {
    PeerCheck::Other(format!(
        "the process answering on the key daemon socket (pid {pid}) {what}"
    ))
}

/// Whether `peer` is this build's key daemon for `home` ([`PeerCheck`]). Any kernel query that
/// fails gives `Unknown` (fail closed): nothing is verified by session.
///
/// On macOS, for a signed build (every build zwallet ships), the decision rests on the cdhash the
/// kernel validates against the code actually running, not on a path or a value the Mach-O
/// declares; see [`check_peer_macos`]. A build that is not code-signed has no cdhash of its own
/// and gets `Unknown` for every peer. On Linux the key daemon is non-dumpable, so its executable
/// is unreadable and this returns `Unknown`; see [`check_peer_linux`].
///
/// This cannot stop a process of this user that runs this very binary over its own `wallet.json`
/// with its own password, or injects code into it (`DYLD_INSERT_LIBRARIES` on a binary without the
/// hardened runtime, or on any binary while System Integrity Protection is off; `LD_PRELOAD`):
/// that is the same as replacing the binary, which nothing here can prevent.
pub fn check_peer(home: &Path, peer: &Peer) -> Result<PeerCheck> {
    check_peer_shown(home, peer, EXE_SHOWN)
}

/// [`check_peer`], with at most `exe_max` characters of the peer's executable path in the reason.
fn check_peer_shown(home: &Path, peer: &Peer, exe_max: usize) -> Result<PeerCheck> {
    let (Some(pid), Some(own)) = (peer.pid, own_exe()) else {
        return Ok(PeerCheck::Unknown);
    };
    #[cfg(target_os = "macos")]
    return check_peer_macos(home, peer, pid, &own, exe_max);
    #[cfg(target_os = "linux")]
    return check_peer_linux(home, pid, &own, exe_max);
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (home, peer, pid, &own, exe_max);
        Ok(PeerCheck::Unknown)
    }
}

/// [`check_peer`] on macOS. The legs, in order: the peer's cdhash equals this process's own; the
/// peer is `CS_VALID`; its pid is in `daemon_pids`; it holds the listening `d.sock` of this home;
/// the executable UUID it used the socket with matches; and the token still resolves (pid reuse).
/// Everything after the cdhash guards the gap that `csops_audittoken` reports the cdhash of the
/// listen-socket owner, which an attacker can turn into this binary's by exec'ing it while its
/// own child keeps answering.
#[cfg(target_os = "macos")]
fn check_peer_macos(
    home: &Path,
    peer: &Peer,
    pid: i32,
    own: &OwnExe,
    exe_max: usize,
) -> Result<PeerCheck> {
    let (Some(token), Some(uuid)) = (peer.token.as_ref(), own.uuid) else {
        return Ok(PeerCheck::Unknown);
    };
    let (Some(cdhash), Some(valid)) = (peer_cdhash(token), peer_cs_valid(token)) else {
        return Ok(PeerCheck::Unknown);
    };
    if cdhash != own.cdhash {
        // Its path, for the message only (never for the decision).
        let path = macos_exe_path(pid, exe_max);
        return Ok(peer_not_ours(
            pid,
            &format!("runs other code than this zwallet build ({path})"),
        ));
    }
    if !valid {
        return Ok(peer_not_ours(
            pid,
            "runs this zwallet build but its signature is invalid",
        ));
    }
    if !daemon_pids(home, None)?.contains(&pid) {
        return Ok(peer_not_ours(
            pid,
            "is not a key daemon of this wallet home",
        ));
    }
    let sock = daemon_home(home)?.join(SOCKET_NAME);
    match holds_listening_socket(pid, &sock) {
        None => return Ok(PeerCheck::Unknown),
        Some(false) => {
            return Ok(peer_not_ours(
                pid,
                "does not hold the key daemon socket of this home",
            ));
        }
        Some(true) => {}
    }
    if peer.exe_uuid != Some(uuid) {
        return Ok(peer_not_ours(
            pid,
            "opened the key daemon socket as another program",
        ));
    }
    // Once more, so the token the whole check rested on still names a live process.
    if peer_cdhash(token) != Some(own.cdhash) {
        return Ok(PeerCheck::Unknown);
    }
    Ok(PeerCheck::ThisBuild)
}

/// [`check_peer`] on Linux. Nothing is ever verified by session here: the genuine key daemon is
/// non-dumpable, so its `/proc/<pid>/exe` is unreadable to its own user (`process_exe` returns
/// `Unreadable`) — this never returns `ThisBuild`. A *readable* executable therefore belongs to
/// some other process; if it runs other code it is named in the commands' `warning`, and if it
/// happens to run this very build it is still not a genuine daemon (which would be unreadable),
/// so `Unknown`. `home` is unused; the socket/daemon-table legs that would need it only matter where
/// a session can verify.
#[cfg(target_os = "linux")]
fn check_peer_linux(_home: &Path, pid: i32, own: &OwnExe, exe_max: usize) -> Result<PeerCheck> {
    match process_exe(pid) {
        ProcessExe::File(path, id) if id != Some(own.id) => Ok(peer_not_ours(
            pid,
            &format!(
                "is not this zwallet build ({})",
                crate::core::text::shown(&path.to_string_lossy(), exe_max)
            ),
        )),
        _ => Ok(PeerCheck::Unknown),
    }
}

/// The path of `pid`'s executable (`proc_pidpath`), for a warning message only, escaped (the
/// path is chosen by whoever started that process, and may carry a terminal escape or a line
/// break). `?` if the kernel will not say.
#[cfg(target_os = "macos")]
fn macos_exe_path(pid: i32, max: usize) -> String {
    let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: the kernel writes at most `buf.len()` bytes into `buf`.
    let n = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    match usize::try_from(n) {
        Ok(n) if n > 0 && n <= buf.len() => {
            crate::core::text::shown(&String::from_utf8_lossy(&buf[..n]), max)
        }
        _ => "?".to_string(),
    }
}

/// Starts a new key daemon for `home` holding the seed for `ttl`, and returns its readiness
/// report `{ locked, expiresAt, active, index, address, pid }` (the address derived by the
/// daemon from the seed it decrypted with `password`, never read from the cache in wallet.json).
///
/// The new daemon is this process's own child (`setsid`, argv `zwallet daemon --home <canonical
/// home>`, no environment, no descriptor but its channel), started from `exe`. Its fds 0 and 1
/// are one end of a socketpair whose other end only this process holds. Over that channel:
/// 1. it asks for input only once core dumps are off; on macOS it must then run this build's
///    code ([`check_started_code`]); it then gets the TTL and the password,
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

/// Closes every descriptor this process holds from fd 3 up, so the key daemon never inherits a
/// listening socket a parent left open (an attacker that `exec`s this binary while its own child
/// squats `d.sock`): without this the daemon's pid would pass [`check_peer`]'s socket-holder leg.
/// Called at the very start of `zwallet daemon`, before the runtime opens any descriptor of its
/// own. The genuine spawn path already marks fds ≥ 3 close-on-exec, so for it this closes nothing.
/// A listing that cannot be read refuses to start (fail closed).
pub fn close_inherited_fds() -> Result<()> {
    let mut fds = OpenFds::prepare()?;
    fds.for_each(|fd| {
        if fd >= 3 {
            // SAFETY: close(2) of a descriptor number; a bad one is harmless here.
            unsafe {
                libc::close(fd);
            }
        }
        Ok(())
    })
    .map_err(io_err("cannot close inherited descriptors"))
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
    // What started may not be the file `DaemonExe::check` saw (no `fexecve`): a child that does
    // not run this build's code is refused before the password is written.
    #[cfg(target_os = "macos")]
    check_started_code(child)?;
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

/// Test-only, not a user feature: a home-path prefix that confines every lookup below to the
/// daemons whose `--home` starts with it, and so the stop-all of `zwallet lock`, of a whitelist
/// edit and of the `create --unlock` mismatch. The test suite sets it on every zwallet it starts,
/// so that `cargo test` stops only the daemons of its own test homes, never the user's session.
/// Only builds with `debug_assertions` (what `cargo test` builds) read it; it is compiled out of
/// release builds, which neither read it nor carry its name.
pub const TEST_SCOPE_ENV: &str = "ZWALLET_TEST_DAEMON_SCOPE";

/// The `--home`s under a path prefix, compared byte by byte (the prefix may end inside a name,
/// as `/tmp/zwt-` does). A daemon's `--home` is canonical, so the prefix counts as written and
/// with its directory canonical (`/tmp/zwt-` is `/private/tmp/zwt-` there on macOS).
struct HomePrefix(Vec<Vec<u8>>);

impl HomePrefix {
    fn new(prefix: &OsStr) -> Self {
        let raw = prefix.as_bytes();
        let mut forms = vec![raw.to_vec()];
        if let Some(slash) = raw.iter().rposition(|b| *b == b'/') {
            let (dir, name) = raw.split_at(slash + 1);
            if let Ok(dir) = std::fs::canonicalize(OsStr::from_bytes(dir)) {
                let mut form = dir.as_os_str().as_bytes().to_vec();
                if !form.ends_with(b"/") {
                    form.push(b'/');
                }
                form.extend_from_slice(name);
                forms.push(form);
            }
        }
        Self(forms)
    }

    fn holds(&self, home: &OsStr) -> bool {
        self.0.iter().any(|form| home.as_bytes().starts_with(form))
    }
}

/// The [`TEST_SCOPE_ENV`] prefix of a debug build.
#[cfg(debug_assertions)]
fn test_scope() -> Option<HomePrefix> {
    std::env::var_os(TEST_SCOPE_ENV).map(|prefix| HomePrefix::new(&prefix))
}

/// A release build has no test scope.
#[cfg(not(debug_assertions))]
fn test_scope() -> Option<HomePrefix> {
    None
}

/// `(pid, --home)` of every zwallet daemon of this user, other than this process and `keep`:
/// any process whose argv is `<any> daemon --home <absolute path>` (in a debug build given
/// [`TEST_SCOPE_ENV`], only those under it).
fn daemon_processes(keep: Option<i32>) -> Result<Vec<(i32, OsString)>> {
    daemon_processes_in(test_scope().as_ref(), keep)
}

/// [`daemon_processes`] under `scope` as given (`None`: every home), whatever the environment
/// says: the one place the process table is filtered by home.
fn daemon_processes_in(
    scope: Option<&HomePrefix>,
    keep: Option<i32>,
) -> Result<Vec<(i32, OsString)>> {
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
        .filter(|(_, home)| scope.is_none_or(|scope| scope.holds(home)))
        .collect())
}

fn sorted(mut pids: Vec<i32>) -> Vec<i32> {
    pids.sort_unstable();
    pids.dedup();
    pids
}

fn pids_of(daemons: Vec<(i32, OsString)>) -> Vec<i32> {
    sorted(daemons.into_iter().map(|(pid, _)| pid).collect())
}

/// Pids of every zwallet daemon of this user, whatever home it serves (in a debug build given
/// [`TEST_SCOPE_ENV`], only those under it).
pub fn all_daemon_pids() -> Result<Vec<i32>> {
    Ok(pids_of(daemon_processes(None)?))
}

/// Test harness only: pids of every zwallet daemon of this user whose `--home` starts with
/// `prefix`, matched as [`TEST_SCOPE_ENV`] is. The prefix is given, so a [`TEST_SCOPE_ENV`] in
/// the environment changes nothing here.
#[doc(hidden)]
pub fn daemon_pids_under(prefix: &Path) -> Result<Vec<i32>> {
    let scope = HomePrefix::new(prefix.as_os_str());
    Ok(pids_of(daemon_processes_in(Some(&scope), None)?))
}

/// Pids of this user's zwallet daemons for `home`, other than this process and `keep`.
pub fn daemon_pids(home: &Path, keep: Option<i32>) -> Result<Vec<i32>> {
    home_daemons(test_scope().as_ref(), &HomeMatch::new(home), keep)
}

/// Test harness only: [`daemon_pids`] among the daemons under `prefix` (as in
/// [`daemon_pids_under`]), whatever the environment says.
#[doc(hidden)]
pub fn home_daemon_pids_under(prefix: &Path, home: &Path) -> Result<Vec<i32>> {
    let scope = HomePrefix::new(prefix.as_os_str());
    home_daemons(Some(&scope), &HomeMatch::new(home), None)
}

fn home_daemons(
    scope: Option<&HomePrefix>,
    wanted: &HomeMatch,
    keep: Option<i32>,
) -> Result<Vec<i32>> {
    Ok(sorted(wanted.select(&daemon_processes_in(scope, keep)?)))
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
    stop_home_daemons(test_scope().as_ref(), home, keep)
}

/// Test harness only: [`stop_daemons`] among the daemons under `prefix` (as in
/// [`daemon_pids_under`]), whatever the environment says.
#[doc(hidden)]
pub fn stop_daemons_under(prefix: &Path, home: &Path) -> Result<Vec<i32>> {
    let scope = HomePrefix::new(prefix.as_os_str());
    stop_home_daemons(Some(&scope), home, None)
}

fn stop_home_daemons(
    scope: Option<&HomePrefix>,
    home: &Path,
    keep: Option<i32>,
) -> Result<Vec<i32>> {
    let wanted = HomeMatch::new(home);
    stop_until_gone(
        || home_daemons(scope, &wanted, keep),
        &format!(" for {}", home.display()),
    )
}

/// Stops every zwallet daemon of this user, whatever home it serves (`zwallet lock`; in a debug
/// build given [`TEST_SCOPE_ENV`], only those under it); see [`stop_until_gone`].
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

/// `(pid, argv)` of every process of this effective uid whose argv can be read. Fails where
/// `/proc` may hide this user's key daemons ([`proc_hiding`]): a list without them would let
/// `lock` report the wallet locked while one still holds the key.
#[cfg(target_os = "linux")]
fn process_argvs() -> Result<Vec<(i32, Vec<OsString>)>> {
    if let Some(why) = proc_hiding_at(Path::new(MOUNTINFO)) {
        return Err(WalletError::validation(format!(
            "{why}: zwallet cannot see key daemons on this host, so it can neither stop them nor \
             tell whether the wallet is locked"
        )));
    }
    let proc = Path::new("/proc");
    let dir = std::fs::read_dir(proc).map_err(io_err("cannot list /proc"))?;
    proc_argvs(
        proc,
        dir.map(|entry| entry.map(|e| e.file_name())),
        current_uid(),
        |path: &Path| std::fs::read(path),
        may_signal,
    )
}

/// `kill(pid, 0)`: `Ok` when this process may signal `pid`, EPERM when it may not, ESRCH when it
/// is gone. Sends nothing. Permission is not ownership: `Ok` also comes for a process whose real
/// or saved uid is this user's while its effective uid is another, and for any process under
/// root or CAP_KILL; EPERM also comes from a security module (LSM) refusing the signal.
#[cfg(target_os = "linux")]
fn may_signal(pid: i32) -> std::io::Result<()> {
    // SAFETY: kill(2) with signal 0 only checks that `pid` exists and may be signalled; `pid` is
    // a /proc entry, so it is > 0 (never a process group).
    match unsafe { libc::kill(pid, 0) } {
        0 => Ok(()),
        _ => Err(std::io::Error::last_os_error()),
    }
}

/// [`process_argvs`] over the entry `names` of a `/proc` at `root`, reading its files with
/// `read`. An error from the listing itself fails the whole lookup, as a full pid buffer does on
/// macOS (`list_pids`): a listing cut short may miss a daemon, which `lock` would then leave
/// running and `status` would not report. A pid whose files are gone (ENOENT, ESRCH) exited and
/// is skipped. A denied read (EACCES, EPERM) is not proof the process is another user's. Where
/// `status` is denied, `signal` (`kill(pid, 0)`) decides by signal permission, not ownership: a
/// process this user may signal (its own, one whose real or saved uid is this user's, or any
/// under root or CAP_KILL) fails the lookup, while EPERM (this user may not signal it: another
/// user's, or one a security module shields) or ESRCH (gone) skips it. A denied `cmdline` of a
/// process whose `status` shows this user's effective uid fails it too. Any other error reading
/// them fails the lookup as well.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn proc_argvs(
    root: &Path,
    names: impl Iterator<Item = std::io::Result<OsString>>,
    uid: u32,
    read: impl Fn(&Path) -> std::io::Result<Vec<u8>>,
    signal: impl Fn(i32) -> std::io::Result<()>,
) -> Result<Vec<(i32, Vec<OsString>)>> {
    let unreadable = |path: &Path, e: &std::io::Error| {
        WalletError::validation(format!(
            "cannot read {}: {e}; not trusting a partial list of key daemons",
            path.display()
        ))
    };
    // `whose` says what the lookup established about the process, and no more.
    let denied = |path: &Path, whose: &str, e: &std::io::Error| {
        WalletError::validation(format!(
            "cannot read {} of a process {whose} ({e}); refusing to trust a partial daemon list",
            path.display()
        ))
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
        let status_path = root.join(&name).join("status");
        let status = match read(&status_path) {
            Ok(bytes) => bytes,
            Err(e) if pid_gone(&e) => continue,
            Err(e) if access_denied(&e) => match signal(pid) {
                Ok(()) => return Err(denied(&status_path, "this user may signal", &e)),
                Err(k) if matches!(k.raw_os_error(), Some(libc::EPERM | libc::ESRCH)) => continue,
                Err(k) => {
                    return Err(WalletError::validation(format!(
                        "cannot read {} ({e}) and cannot tell whether this user may signal \
                         process {pid} (kill: {k}); refusing to trust a partial daemon list",
                        status_path.display()
                    )));
                }
            },
            Err(e) => return Err(unreadable(&status_path, &e)),
        };
        if status_euid(&status) != Some(uid) {
            continue;
        }
        let cmdline_path = root.join(&name).join("cmdline");
        let cmdline = match read(&cmdline_path) {
            Ok(bytes) => bytes,
            Err(e) if pid_gone(&e) => continue,
            Err(e) if access_denied(&e) => {
                return Err(denied(
                    &cmdline_path,
                    "whose effective uid is this user's",
                    &e,
                ));
            }
            Err(e) => return Err(unreadable(&cmdline_path, &e)),
        };
        out.push((pid, split_nul(&cmdline, usize::MAX)));
    }
    Ok(out)
}

/// Whether reading a `/proc/<pid>` file failed because the process is gone.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn pid_gone(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(libc::ENOENT | libc::ESRCH))
}

/// Whether reading a `/proc/<pid>` file was denied: another user's process, or one of this user's
/// that `/proc` will not show (see [`proc_argvs`]).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn access_denied(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(libc::EACCES | libc::EPERM))
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
    use serde_json::json;
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
    fn error_details_cross_the_socket() {
        let e = WalletError::new(ErrorCode::WhitelistDenied, "refusing to sign: no")
            .with_detail("command", "zwallet whitelist add <nick> 0x1 --chains base")
            .with_detail("chain", "base");
        let wire = serde_json::to_string(&Response::err(&e)).unwrap();
        let back: Response = serde_json::from_str(&wire).unwrap();
        assert_eq!(back.into_result().unwrap_err(), e);
        // No details: no field on the wire, and none after.
        let plain = WalletError::validation("x");
        let wire = serde_json::to_value(Response::err(&plain)).unwrap();
        assert!(wire.get("details").is_none(), "{wire}");
        let back: Response = serde_json::from_value(wire).unwrap();
        assert_eq!(back.into_result().unwrap_err(), plain);
        let ok = serde_json::to_value(Response::ok(json!({ "locked": true }))).unwrap();
        assert!(ok.get("details").is_none(), "{ok}");
    }

    #[test]
    fn w12_no_request_carries_a_hash_typed_data_or_an_authorization() {
        // Every request there is, by its wire name: the only signing paths are an EIP-191
        // message and an unsigned transaction's bytes (which the daemon decodes and checks).
        let all = [
            Request::Lock,
            Request::Status,
            Request::Address {
                chain: "base".into(),
                index: 0,
            },
            Request::SignMessage {
                chain: "base".into(),
                index: 0,
                message_hex: "00".into(),
            },
            Request::SignTx {
                chain: "base".into(),
                index: 0,
                tx: json!("0x02"),
            },
        ];
        for r in &all {
            // Exhaustive: a new request kind does not compile until it is listed above.
            match r {
                Request::Lock
                | Request::Status
                | Request::Address { .. }
                | Request::SignMessage { .. }
                | Request::SignTx { .. } => {}
            }
        }
        let ops: Vec<String> = all
            .iter()
            .map(|r| {
                serde_json::to_value(r).unwrap()["op"]
                    .as_str()
                    .unwrap()
                    .into()
            })
            .collect();
        assert_eq!(
            ops,
            ["lock", "status", "address", "sign_message", "sign_tx"]
        );
        let hash = format!("0x{}", "ab".repeat(32));
        for op in [
            json!({ "op": "sign_hash", "chain": "base", "index": 0, "hash": hash }),
            json!({ "op": "sign_typed_data", "chain": "base", "index": 0, "typedData": {} }),
            json!({ "op": "sign_authorization", "chain": "base", "index": 0, "address": hash }),
            json!({ "op": "sign_raw", "chain": "base", "index": 0, "messageHex": "00" }),
        ] {
            assert!(
                serde_json::from_value::<Request>(op.clone()).is_err(),
                "{op}"
            );
        }
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

    /// Every byte of `s` is printable ASCII (0x20..=0x7E).
    fn ascii_printable(s: &str) -> bool {
        s.bytes().all(|b| (0x20..=0x7e).contains(&b))
    }

    #[test]
    fn a_relayed_error_is_escaped_and_never_reads_as_this_clis_own() {
        use crate::core::text::PROSE_MAX;
        let home = tempfile::Builder::new()
            .prefix("zwt-")
            .tempdir_in("/tmp")
            .unwrap();
        let reply = |error: &str, command: &str| -> Response {
            serde_json::from_value(json!({
                "v": 2, "ok": false, "errorCode": "WHITELIST_DENIED",
                "error": error,
                "details": {
                    "command": command, "n\u{1b}": 1, "address": "0xdEaD", "verified": true,
                    "verifiedBy": "session", "warning": "fake", "list": ["a", "b"],
                },
            }))
            .unwrap()
        };
        let resp = reply(
            "refusing\r\u{1b}[2Kaddress: 0xdEaD\nverified: true\u{9b}\u{202e}",
            "zwallet whitelist add x 0xdEaD\n\u{1b}[8m",
        );
        // Who answered is not known (no pid): prefixed with only what is known, and escaped to
        // printable ASCII (every byte 0x20..=0x7E), so `\r` and `\n` are `\u{d}`/`\u{a}`.
        let unknown = Peer {
            pid: None,
            token: None,
            exe_uuid: None,
        };
        let e = relayed_error(home.path(), &unknown, resp);
        assert_eq!(e.code, ErrorCode::WhitelistDenied);
        assert!(
            e.message.starts_with(
                "the process answering on the key daemon socket reported: \
                 refusing\\u{d}\\u{1b}[2Kaddress"
            ),
            "{}",
            e.message
        );
        assert!(ascii_printable(&e.message), "{:?}", e.message);
        // Only allowlisted scalar detail keys come through; the array `list`, and the squatter's
        // own `address`/`verified`/`warning`, are dropped.
        assert_eq!(e.details.keys().collect::<Vec<_>>(), vec!["command"]);
        assert_eq!(
            e.details["command"],
            "zwallet whitelist add x 0xdEaD\\u{a}\\u{1b}[8m"
        );
        // A long block/padding payload: message plus details share one PROSE_MAX budget, so the
        // whole human output is bounded, printable ASCII, and draws nothing.
        let padded = format!(
            "{}{}scan to send to 0xdEaD",
            " ".repeat(4000),
            "\u{2588}\u{2580}\u{2584} ".repeat(2000)
        );
        let e = relayed_error(home.path(), &unknown, reply(&padded, &padded));
        let prefix = "the process answering on the key daemon socket reported: ";
        let total: usize = e.message.chars().count()
            + e.details
                .values()
                .map(|v| v.as_str().map_or(0, |s| s.chars().count()))
                .sum::<usize>();
        assert!(total <= prefix.chars().count() + PROSE_MAX, "{total}");
        assert!(ascii_printable(&e.message), "{:?}", &e.message[..80]);
    }

    #[test]
    fn relayed_details_drop_nesting_and_escape_every_solid_glyph() {
        use crate::core::text::PROSE_MAX;
        let home = tempfile::Builder::new()
            .prefix("zwt-")
            .tempdir_in("/tmp")
            .unwrap();
        // Solid glyphs an earlier blacklist let through — ⬛⬜ (U+2B1B/2B1C), a sextant (U+1FB00)
        // and an octant (U+1CD00) — tile a QR as well as the old block elements do. A short
        // message keeps budget for the one allowlisted scalar detail; the array and object details
        // are not relayable at any size.
        let resp: Response = serde_json::from_value(json!({
            "v": 2, "ok": false, "errorCode": "WHITELIST_DENIED",
            "error": "refused \u{2b1b}\u{2b1c} and \u{1fb00}\u{1cd00}",
            "details": {
                "command": "zwallet whitelist add \u{2b1b} 0xdEaD",
                "counterparty": vec!["0xdEaD"; 64],
                "chain": { "evil": "\u{1cd00}".repeat(5000) },
            },
        }))
        .unwrap();
        let unknown = Peer {
            pid: None,
            token: None,
            exe_uuid: None,
        };
        let e = relayed_error(home.path(), &unknown, resp);
        assert_eq!(e.code, ErrorCode::WhitelistDenied);
        // Every byte of the message is printable ASCII: each glyph is a `\u{...}` escape.
        assert!(ascii_printable(&e.message), "{:?}", e.message);
        assert!(
            e.message
                .contains("\\u{2b1b}\\u{2b1c} and \\u{1fb00}\\u{1cd00}"),
            "{}",
            e.message
        );
        // Only the allowlisted scalar `command` survives, escaped; the array and the object detail
        // (whatever their size) are dropped, so the whole stays bounded printable ASCII.
        assert_eq!(e.details.keys().collect::<Vec<_>>(), vec!["command"]);
        let command = e.details["command"].as_str().unwrap();
        assert!(
            ascii_printable(command) && command.contains("\\u{2b1b}"),
            "{command:?}"
        );
        let total: usize = e.message.chars().count()
            + e.details
                .values()
                .map(|v| v.as_str().map_or(0, |s| s.chars().count()))
                .sum::<usize>();
        let prefix = "the process answering on the key daemon socket reported: ";
        assert!(total <= prefix.chars().count() + PROSE_MAX, "{total}");
    }

    #[test]
    fn a_relayed_number_detail_counts_toward_the_budget() {
        use crate::core::text::PROSE_MAX;
        let home = tempfile::Builder::new()
            .prefix("zwt-")
            .tempdir_in("/tmp")
            .unwrap();
        let unknown = Peer {
            pid: None,
            token: None,
            exe_uuid: None,
        };
        let reply = |error: String| -> Response {
            serde_json::from_value(json!({
                "v": 2, "ok": false, "errorCode": "FEE_CAP_EXCEEDED", "error": error,
                "details": { "feeWei": u64::MAX, "capWei": 7 },
            }))
            .unwrap()
        };
        // 500 characters of message leave 12: `capWei` (1) fits, `feeWei` (20 digits) does not.
        let e = relayed_error(home.path(), &unknown, reply("m".repeat(500)));
        let prefix = "the process answering on the key daemon socket reported: ";
        let total = e.message.len() - prefix.len()
            + e.details
                .values()
                .map(|v| v.to_string().len())
                .sum::<usize>();
        assert!(total <= PROSE_MAX, "{total}");
        assert!(e.details.get("feeWei").is_none(), "{:?}", e.details);
        assert_eq!(e.details["capWei"], 7);
        // With room, both are kept.
        let e = relayed_error(home.path(), &unknown, reply("short".into()));
        assert_eq!(e.details["feeWei"], u64::MAX);
    }

    /// A session-ending reply from this build's own daemon (it answers, then drops the socket
    /// and exits) is not named an impostor: same code, but no longer a listed daemon holding the
    /// socket, is `Unknown` for a relayed error. Modelled with this test process, which runs this
    /// build's code (its own cdhash) and is no key daemon of the home. arm64 only, as every
    /// test that needs this test binary's own cdhash ([`own_token`]).
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[test]
    fn a_relayed_error_from_this_builds_code_after_its_session_ended_is_not_named() {
        let home = tempfile::Builder::new()
            .prefix("zwt-")
            .tempdir_in("/tmp")
            .unwrap();
        let me = Peer {
            pid: Some(std::process::id() as i32),
            token: Some(own_token()),
            exe_uuid: own_exe().unwrap().uuid,
        };
        // The strict check (address verification) still calls it another process.
        assert!(matches!(
            check_peer(home.path(), &me).unwrap(),
            PeerCheck::Other(_)
        ));
        // The reply carries details a genuine `WALLET_LOCKED` never has: none of them comes
        // through, though the message keeps the neutral prefix.
        let resp: Response = serde_json::from_value(json!({
            "v": 2, "ok": false, "errorCode": "WALLET_LOCKED",
            "error": "the unlock expired; locked. A human must run `zwallet unlock 24h` (any duration)",
            "details": {
                "command": "zwallet whitelist add savings 0x000000000000000000000000000000000000dEaD",
                "counterparty": "0x000000000000000000000000000000000000dEaD",
                "chain": "ethereum",
            },
        }))
        .unwrap();
        let e = relayed_error(home.path(), &me, resp);
        assert_eq!(e.code, ErrorCode::WalletLocked);
        assert!(
            e.message
                .starts_with("the process answering on the key daemon socket reported: the unlock"),
            "{}",
            e.message
        );
        assert!(!e.message.contains("not a key daemon"), "{}", e.message);
        assert!(e.details.is_empty(), "{:?}", e.details);
        // Any other code from the same process keeps it named, details dropped.
        let resp: Response = serde_json::from_value(json!({
            "v": 2, "ok": false, "errorCode": "WHITELIST_DENIED", "error": "refused",
            "details": { "command": "zwallet whitelist add x 0xdEaD" },
        }))
        .unwrap();
        let e = relayed_error(home.path(), &me, resp);
        assert!(e.message.contains("is not a key daemon"), "{}", e.message);
        assert!(e.details.get("command").is_none(), "{:?}", e.details);
    }

    #[test]
    fn file_id_is_the_file_not_its_name_or_its_bytes() {
        let dir = tempfile::Builder::new()
            .prefix("zwt-")
            .tempdir_in("/tmp")
            .unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        std::fs::write(&a, b"zwallet").unwrap();
        std::fs::write(&b, b"zwallet").unwrap();
        // The same bytes in another file: another file.
        assert!(file_id(&a).is_some());
        assert_ne!(file_id(&a), file_id(&b));
        // Another name for the same file: the same file.
        let hard = dir.path().join("hard");
        std::fs::hard_link(&a, &hard).unwrap();
        assert_eq!(file_id(&hard), file_id(&a));
        let soft = dir.path().join("soft");
        std::os::unix::fs::symlink(&a, &soft).unwrap();
        assert_eq!(file_id(&soft), file_id(&a));
        assert_eq!(file_id(&dir.path().join("missing")), None);
    }

    /// This process's own audit token (macOS), read over a socketpair to itself: the peer of
    /// either end is this process, so `LOCAL_PEERTOKEN` names us.
    ///
    /// The unit tests that need this test binary's own cdhash run on arm64 macOS only. The linker
    /// signs every arm64 build; an x86_64 test binary is unsigned, so it has no cdhash and every
    /// check refuses for that reason. The integration tests' harness signs the zwallet binary
    /// (`tests/common`), but a running test binary cannot sign itself.
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn own_token() -> AuditToken {
        let (a, b) = ChildChannel::pair().unwrap();
        (&a).write_all(&[0]).unwrap();
        peer_token(std::os::fd::AsRawFd::as_raw_fd(&b)).expect("own audit token")
    }

    #[test]
    fn check_peer_names_what_answered() {
        let home = tempfile::Builder::new()
            .prefix("zwt-")
            .tempdir_in("/tmp")
            .unwrap();
        // This test process runs this build's code. On macOS that gets past the cdhash leg but is
        // no key daemon of the home, so it is named (arm64 only: see `own_token`). On Linux its
        // executable is readable, so it is not the non-dumpable genuine daemon: Unknown (nothing
        // is verified by session there).
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            let me = std::process::id() as i32;
            let this = Peer {
                pid: Some(me),
                token: Some(own_token()),
                exe_uuid: own_exe().unwrap().uuid,
            };
            let check = check_peer(home.path(), &this).unwrap();
            let reason = format!(
                "the process answering on the key daemon socket (pid {me}) is not a key daemon \
                 of this wallet home"
            );
            assert_eq!(check, PeerCheck::Other(reason.clone()));
            // The reason carries no advice; the shown warning adds it once.
            assert_eq!(check.warning(), Some(format!("{reason}; {RELOCK}")));
            // No token means the kernel will not say, so Unknown (fail closed).
            let no_token = Peer {
                token: None,
                ..this
            };
            assert_eq!(
                check_peer(home.path(), &no_token).unwrap(),
                PeerCheck::Unknown
            );
        }
        #[cfg(target_os = "linux")]
        {
            let this = Peer {
                pid: Some(std::process::id() as i32),
                token: None,
                exe_uuid: None,
            };
            assert_eq!(check_peer(home.path(), &this).unwrap(), PeerCheck::Unknown);
        }
        // No pid at all: nothing is known, nothing is claimed.
        let none = Peer {
            pid: None,
            token: None,
            exe_uuid: None,
        };
        assert_eq!(check_peer(home.path(), &none).unwrap(), PeerCheck::Unknown);
        // Linux: another program's readable executable is not this build's. `spawn()` returns
        // before the child has exec'd `sleep`, so poll until `/proc/<pid>/exe` is no longer this
        // test binary; until then it still reads as own's id.
        #[cfg(target_os = "linux")]
        {
            let own_id = file_id(Path::new("/proc/self/exe")).unwrap();
            let sleep = Reaped(Command::new("sleep").arg("30").spawn().unwrap());
            let spid = sleep.0.id() as i32;
            let exe = PathBuf::from(format!("/proc/{spid}/exe"));
            let start = Instant::now();
            while file_id(&exe) == Some(own_id) && start.elapsed() < Duration::from_secs(5) {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_ne!(file_id(&exe), Some(own_id), "sleep did not exec within 5s");
            let other = Peer {
                pid: Some(spid),
                token: None,
                exe_uuid: None,
            };
            let w = match check_peer(home.path(), &other).unwrap() {
                PeerCheck::Other(w) => w,
                PeerCheck::ThisBuild => panic!("sleep passed for this build's key daemon"),
                PeerCheck::Unknown => panic!("sleep's readable executable gave Unknown, not Other"),
            };
            assert!(
                w.contains("is not this zwallet build (") && w.contains("sleep)"),
                "{w}"
            );
            drop(sleep);
            // Gone: unreadable, so Unknown.
            assert_eq!(check_peer(home.path(), &other).unwrap(), PeerCheck::Unknown);
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn holds_listening_socket_sees_a_bound_listener() {
        let dir = tempfile::Builder::new()
            .prefix("zwt-")
            .tempdir_in("/tmp")
            .unwrap();
        let path = dir.path().join(SOCKET_NAME);
        let me = std::process::id() as i32;
        assert_eq!(holds_listening_socket(me, &path), Some(false));
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        assert_eq!(holds_listening_socket(me, &path), Some(true));
        // Another path is not this one.
        assert_eq!(
            holds_listening_socket(me, &dir.path().join("other.sock")),
            Some(false)
        );
        drop(listener);
        assert_eq!(holds_listening_socket(me, &path), Some(false));
    }

    /// A child process killed and reaped on drop, so a failing assertion or panic never leaves
    /// it behind. (x86_64 macOS has no test that starts one.)
    #[cfg(any(target_os = "linux", all(target_os = "macos", target_arch = "aarch64")))]
    struct Reaped(Child);

    #[cfg(any(target_os = "linux", all(target_os = "macos", target_arch = "aarch64")))]
    impl Drop for Reaped {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    /// A started daemon that does not run this build's code gets no password. A binary renamed
    /// into the pinned path after [`DaemonExe::check`] (macOS has no `fexecve`) is started in its
    /// place; that is modelled here by starting another program as the child directly. It asks
    /// for input as the daemon does, then records whatever it is sent: it is refused, and gets
    /// nothing. arm64 only: it needs this test binary's own cdhash (see `own_token`).
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[tokio::test(flavor = "current_thread")]
    async fn a_started_daemon_that_runs_other_code_is_given_nothing() {
        let dir = tempfile::Builder::new()
            .prefix("zwt-")
            .tempdir_in("/tmp")
            .unwrap();
        let got = dir.path().join("got");
        let ask = serde_json::to_string(&Response::ok(json!({ "stage": STAGE_INPUT }))).unwrap();
        let (ours, theirs) = ChildChannel::pair().unwrap();
        let theirs_out = theirs.try_clone().unwrap();
        let mut child = Reaped(
            Command::new("/bin/sh")
                .args([
                    "-c",
                    r#"printf '%s\n' "$1"; exec /bin/dd bs=4096 count=1 of="$2" 2>/dev/null"#,
                    "sh",
                ])
                .arg(&ask)
                .arg(&got)
                .env_clear()
                .stdin(Stdio::from(OwnedFd::from(theirs)))
                .stdout(Stdio::from(OwnedFd::from(theirs_out)))
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let pid = child.0.id();
        let started = drive_daemon(
            dir.path(),
            &mut child.0,
            ours,
            "not the password",
            Duration::from_secs(60),
        )
        .await;
        // As `start_daemon` does on any failure.
        let _ = child.0.kill();
        let _ = child.0.wait();
        let sent = std::fs::metadata(&got).map_or(0, |m| m.len());
        assert_eq!(
            sent, 0,
            "the started process read {sent} bytes of the unlock input"
        );
        let e = started.expect_err("a daemon that runs other code was driven to readiness");
        assert_eq!(e.code, ErrorCode::ValidationError);
        for part in [
            format!("the key daemon this command started (pid {pid}) does not run this zwallet"),
            "nothing was given to it".to_string(),
            "no new session was started".to_string(),
        ] {
            assert!(
                e.message.contains(&part),
                "missing {part:?} in {}",
                e.message
            );
        }
    }

    /// [`started_code_verdict`] accepts only this build's cdhash on a `CS_VALID` child, and reads
    /// a `csops` that failed as a refusal. A live child with this build's cdhash but `CS_VALID`
    /// clear cannot be built as a process: arm64 macOS runs code with `CS_KILL` set (the status
    /// words of a linker-signed binary and of `/bin/sleep` both carry 0x200), so a process whose
    /// signature turns invalid is killed instead of left running. So that leg is tested here.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_started_code_verdict_needs_the_same_cdhash_and_cs_valid() {
        let (ours, other) = ([7u8; CDHASH_LEN], [8u8; CDHASH_LEN]);
        let exe = Path::new("/z/zwallet");
        assert!(started_code_verdict(9, Some(ours), Some(ours), Some(true), exe).is_ok());
        for (child, valid) in [
            (Some(ours), Some(false)),
            (Some(ours), None),
            (Some(other), Some(true)),
            (None, Some(true)),
            (None, None),
        ] {
            let e = started_code_verdict(9, Some(ours), child, valid, exe)
                .expect_err(&format!("accepted child {child:?} valid {valid:?}"));
            assert_eq!(e.code, ErrorCode::ValidationError);
            assert!(
                e.message
                    .contains("(pid 9) does not run this zwallet build's code")
            );
        }
    }

    /// A build whose own cdhash cannot be read has nothing to compare the child with: it refuses
    /// (fail closed), says the identity cannot be read, and gives the signing line as the fix for
    /// an unsigned build, with the path quoted for the shell.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_started_code_verdict_refuses_when_this_build_has_no_cdhash() {
        let h = [7u8; CDHASH_LEN];
        let exe = Path::new("/Applications/My Tools/zwallet");
        for (child, valid) in [(Some(h), Some(true)), (None, None)] {
            let e = started_code_verdict(9, None, child, valid, exe)
                .expect_err("a build without a cdhash of its own accepted the started daemon");
            assert_eq!(e.code, ErrorCode::ValidationError);
            for part in [
                "cannot read this zwallet build's code-signing identity",
                "nothing was given to it",
                "If this is an unsigned build, sign it: `codesign --force --sign - \
                 '/Applications/My Tools/zwallet'`",
                "release binaries are signed",
            ] {
                assert!(
                    e.message.contains(part),
                    "missing {part:?} in {}",
                    e.message
                );
            }
        }
    }

    /// The suggested `codesign` line names the binary as one shell word, whatever its path holds
    /// (a space, a quote, a `$`): `sh` reads it back as the path itself.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_signing_line_quotes_the_path_for_the_shell() {
        for path in ["/usr/local/bin/zwallet", "/a b/it's $HOME/zw`x`"] {
            let word = shell_quoted(path);
            let out = Command::new("/bin/sh")
                .args(["-c", &format!("printf %s {word}")])
                .env_clear()
                .output()
                .unwrap();
            assert_eq!(String::from_utf8_lossy(&out.stdout), path, "{word}");
            assert!(sign_this_build(Path::new(path)).contains(&format!("--sign - {word}`")));
        }
    }

    /// The refusal says what this start did, not what state the wallet is in: `unlock` refuses
    /// before it stops a running session, and `whitelist ... --unlock` stopped every daemon
    /// before. A genuine child that exited after asking for input is refused the same way (its
    /// `csops` fails), so the cause names that too.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_started_daemon_refusal_claims_no_lock_state() {
        let e = started_code_verdict(
            42,
            Some([7; CDHASH_LEN]),
            Some([8; CDHASH_LEN]),
            Some(true),
            Path::new("/z/zwallet"),
        )
        .unwrap_err();
        let m = &e.message;
        for part in [
            "the key daemon this command started (pid 42) does not run this zwallet build's code",
            "or exited before it could be checked",
            "It was stopped and nothing was given to it; no new session was started, and this did \
             not stop a session that was already running",
        ] {
            assert!(m.contains(part), "missing {part:?} in {m}");
        }
        assert!(
            !m.contains("stays locked") && !m.contains("untouched"),
            "{m}"
        );
    }

    /// Set only for the child [`the_started_daemon_check_accepts_only_this_builds_code`] starts.
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    const CHILD_WAITS: &str = "ZWT_CHILD_WAITS_FOR_STDIN";

    /// No check of its own: [`the_started_daemon_check_accepts_only_this_builds_code`] runs this
    /// test binary as a child (so it runs this build's code) through this test, which then waits
    /// until its stdin closes. Run any other way it returns at once.
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[test]
    fn child_running_this_build_waits_for_stdin() {
        use std::io::Read;

        if std::env::var_os(CHILD_WAITS).is_some() {
            let _ = std::io::stdin().read_to_end(&mut Vec::new());
        }
    }

    /// [`check_started_code`] refuses a child that runs other code, and one that already exited,
    /// and accepts one that runs this build's code. The test binary stands for the build here: it
    /// is linker-signed like zwallet, and `own_cdhash` is its own (arm64 only: see `own_token`).
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[test]
    fn the_started_daemon_check_accepts_only_this_builds_code() {
        assert!(own_cdhash().is_some(), "this test binary is linker-signed");
        // Other code: `sleep` is Apple-signed and CS_VALID, so only the cdhash refuses it.
        let sleep = Reaped(
            Command::new("/bin/sleep")
                .arg("30")
                .env_clear()
                .stdin(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let spid = sleep.0.id();
        assert_eq!(pid_cs_valid(spid as libc::pid_t), Some(true));
        let e = check_started_code(&sleep.0).expect_err("sleep passed as this build's code");
        assert_eq!(e.code, ErrorCode::ValidationError);
        assert!(
            e.message.contains(&format!(
                "(pid {spid}) does not run this zwallet build's code"
            )),
            "{}",
            e.message
        );
        drop(sleep);

        // This build's code: this test binary again, kept alive by the helper test above.
        let mut this = Reaped(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "core::protocol::tests::child_running_this_build_waits_for_stdin",
                    "--exact",
                    "--test-threads=1",
                    "-q",
                ])
                .env_clear()
                .env(CHILD_WAITS, "1")
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        check_started_code(&this.0).expect("a child that runs this build's code is refused");

        // Exited but not reaped (a zombie): the kernel has no code for it any more, so refused.
        drop(this.0.stdin.take());
        let tpid = this.0.id();
        // SAFETY: `info` is a valid siginfo_t; waits, without reaping it (WNOWAIT), for this
        // process's own child to exit.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let rc =
            unsafe { libc::waitid(libc::P_PID, tpid, &mut info, libc::WEXITED | libc::WNOWAIT) };
        assert_eq!(rc, 0, "{}", std::io::Error::last_os_error());
        assert!(
            check_started_code(&this.0).is_err(),
            "an exited child passed as this build's code"
        );
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
    fn a_test_scope_holds_the_homes_under_its_prefix_as_written_or_canonical() {
        let tmp = std::fs::canonicalize("/tmp").unwrap();
        let scope = HomePrefix::new(OsStr::new("/tmp/zwt-"));
        for home in [
            PathBuf::from("/tmp/zwt-a1"),
            tmp.join("zwt-a1"),
            tmp.join("zwt-a1").join("nested"),
        ] {
            assert!(scope.holds(home.as_os_str()), "{home:?}");
        }
        for home in [
            PathBuf::from("/tmp/zwx-a1"),
            tmp.join("zwx-a1"),
            tmp.clone(),
            PathBuf::from("/Users/u/.zwallet"),
        ] {
            assert!(!scope.holds(home.as_os_str()), "{home:?}");
        }
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

    /// A `kill(pid, 0)` for [`proc_argvs`]: `pid` to `Ok` (`None`: this user may signal it) or an
    /// errno. This user may not signal any other pid (EPERM).
    fn fake_kill(answers: Vec<(i32, Option<i32>)>) -> impl Fn(i32) -> std::io::Result<()> {
        let answers: BTreeMap<i32, Option<i32>> = answers.into_iter().collect();
        move |pid| match answers.get(&pid) {
            Some(None) => Ok(()),
            Some(Some(errno)) => Err(std::io::Error::from_raw_os_error(*errno)),
            None => Err(std::io::Error::from_raw_os_error(libc::EPERM)),
        }
    }

    /// A `kill(pid, 0)` that must not be called: every status in the test is readable or gone.
    fn no_kill(pid: i32) -> std::io::Result<()> {
        panic!("kill({pid}, 0) called, but only a denied status read needs it")
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
        // A host where every status is readable never asks kill(2).
        let got = proc_argvs(Path::new(FAKE_PROC), names, ME, read, no_kill).unwrap();
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
            // Closed to this user (EPERM is what `hidepid` answers), and kill(2) says this user
            // may not signal it (another user's, or shielded by a security module)...
            ("64/status", Err(libc::EACCES)),
            ("65/status", Err(libc::EPERM)),
            // ...or that it exited in between.
            ("66/status", Err(libc::EACCES)),
            ("70/status", Ok(proc_status(ME))),
            ("70/cmdline", Ok(DAEMON_A.to_vec())),
        ]);
        let kill = fake_kill(vec![(64, Some(libc::EPERM)), (66, Some(libc::ESRCH))]);
        let names = listing(&["60", "61", "62", "63", "64", "65", "66", "70"]);
        let got = proc_argvs(Path::new(FAKE_PROC), names, ME, read, kill).unwrap();
        assert_eq!(got, vec![(70, split_nul(DAEMON_A, usize::MAX))]);
    }

    #[test]
    fn a_denied_read_of_this_users_process_fails_the_lookup() {
        let daemon = |files: Vec<(&'static str, std::result::Result<Vec<u8>, i32>)>| {
            let mut all = vec![
                ("70/status", Ok(proc_status(ME))),
                ("70/cmdline", Ok(DAEMON_A.to_vec())),
            ];
            all.retain(|(rel, _)| files.iter().all(|(r, _)| r != rel));
            all.extend(files);
            fake_proc(all)
        };
        let lookup =
            |read, kill| proc_argvs(Path::new(FAKE_PROC), listing(&["70"]), ME, read, kill);
        // The message says what was established about the process (`whose`), never ownership.
        let refused = |err: WalletError, rel: &str, whose: &str, errno: i32| {
            assert_eq!(err.code, ErrorCode::ValidationError);
            assert_eq!(
                err.message,
                format!(
                    "cannot read /fake-proc/{rel} of a process {whose} ({}); refusing to trust a \
                     partial daemon list",
                    std::io::Error::from_raw_os_error(errno)
                )
            );
        };
        for errno in [libc::EACCES, libc::EPERM] {
            // Its status shows this user's effective uid, but its argv is closed: it may be a
            // daemon.
            let read = daemon(vec![("70/cmdline", Err(errno))]);
            refused(
                lookup(read, fake_kill(vec![])).unwrap_err(),
                "70/cmdline",
                "whose effective uid is this user's",
                errno,
            );
            // Its status is closed, but kill(2) says this user may signal it (its own, one whose
            // real or saved uid is this user's, or any under root or CAP_KILL), so it may be a
            // daemon too.
            let read = daemon(vec![("70/status", Err(errno))]);
            refused(
                lookup(read, fake_kill(vec![(70, None)])).unwrap_err(),
                "70/status",
                "this user may signal",
                errno,
            );
            // kill(2) failing in an unexpected way cannot tell whether it may be signalled.
            let read = daemon(vec![("70/status", Err(errno))]);
            let err = lookup(read, fake_kill(vec![(70, Some(libc::EINVAL))])).unwrap_err();
            assert!(
                err.message.contains("cannot read /fake-proc/70/status")
                    && err
                        .message
                        .contains("cannot tell whether this user may signal process 70")
                    && err
                        .message
                        .ends_with("refusing to trust a partial daemon list"),
                "{}",
                err.message
            );
            // Closed, and this user may not signal it (kill(2) answers EPERM): skipped.
            let read = daemon(vec![("70/status", Err(errno))]);
            assert!(lookup(read, fake_kill(vec![])).unwrap().is_empty());
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn may_signal_tells_this_users_processes_from_gone_ones() {
        assert!(may_signal(std::process::id() as i32).is_ok());
        // Above any pid_max (at most 2^22): no such process.
        let gone = may_signal(i32::MAX).unwrap_err();
        assert_eq!(gone.raw_os_error(), Some(libc::ESRCH), "{gone}");
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
            let err = proc_argvs(Path::new(FAKE_PROC), names.into_iter(), ME, &read, no_kill)
                .unwrap_err();
            assert_eq!(err.code, ErrorCode::ValidationError);
            assert!(
                err.message.contains("cannot list /fake-proc")
                    && err.message.contains("not trusting a partial list"),
                "{at}: {}",
                err.message
            );
        }
        assert_eq!(
            proc_argvs(
                Path::new(FAKE_PROC),
                listing(&["70", "71"]),
                ME,
                &read,
                no_kill
            )
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
            let err =
                proc_argvs(Path::new(FAKE_PROC), names, ME, fake_proc(files), no_kill).unwrap_err();
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
            missing.message.contains("missing") && missing.message.contains("ptrace_scope >= 1"),
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

    /// `/proc/self/mountinfo` of a `rust:1.97-bookworm` container (the overlay root's options
    /// shortened), with the proc line at `/proc` as `{proc}`. The masked `/proc/<x>` lines are
    /// proc and tmpfs mounts below `/proc`, which must not count as `/proc` itself.
    const MOUNTINFO_DOCKER: &str = "\
272 144 0:69 / / rw,relatime - overlay overlay rw,lowerdir=/l,upperdir=/u,workdir=/w
{proc}
275 272 0:102 / /dev rw,nosuid - tmpfs tmpfs rw,size=65536k,mode=755
280 275 0:105 / /dev/shm rw,nosuid,nodev,noexec,relatime - tmpfs shm rw,size=65536k
145 274 0:101 /bus /proc/bus ro,nosuid,nodev,noexec,relatime - proc proc rw
146 274 0:101 /fs /proc/fs ro,nosuid,nodev,noexec,relatime - proc proc rw
159 274 0:101 /sys /proc/sys ro,nosuid,nodev,noexec,relatime - proc proc rw
175 274 0:102 /null /proc/kcore rw,nosuid - tmpfs tmpfs rw,size=65536k,mode=755
177 274 0:106 / /proc/scsi ro,relatime - tmpfs tmpfs ro,size=4k,nr_inodes=1
";

    fn mountinfo_with(proc_line: &str) -> Vec<u8> {
        MOUNTINFO_DOCKER.replace("{proc}", proc_line).into_bytes()
    }

    #[test]
    fn a_proc_mounted_without_hidepid_hides_no_daemon() {
        const PROC: &str = "274 272 0:101 / /proc rw,nosuid,nodev,noexec,relatime";
        for proc_line in [
            format!("{PROC} - proc proc rw"),
            // Measured: subset=pid lists and opens a same-user non-dumpable process.
            format!("{PROC} - proc proc rw,subset=pid"),
            format!("{PROC} - proc proc rw,hidepid=0"),
            format!("{PROC} - proc proc rw,hidepid=off"),
            // An optional field (shared:N) before the separator.
            format!("{PROC} shared:12 - proc proc rw"),
        ] {
            assert_eq!(
                proc_hiding(&mountinfo_with(&proc_line)),
                None,
                "{proc_line}"
            );
        }
        // The host this runs on lists its own daemons (or it is not Linux).
        #[cfg(target_os = "linux")]
        assert_eq!(proc_hiding_at(Path::new(MOUNTINFO)), None);
        check_daemons_findable().unwrap();
    }

    #[test]
    fn a_proc_mounted_with_hidepid_or_unreadable_hides_daemons() {
        const PROC: &str = "274 272 0:101 / /proc rw,nosuid,nodev,noexec,relatime";
        for (proc_line, why) in [
            // Older kernels show the number, 5.8 and later the name.
            (
                format!("{PROC} - proc proc rw,hidepid=2"),
                "/proc is mounted with hidepid=2",
            ),
            (
                format!("{PROC} - proc proc rw,hidepid=invisible"),
                "/proc is mounted with hidepid=invisible",
            ),
            (
                format!("{PROC} - proc proc rw,hidepid=noaccess"),
                "/proc is mounted with hidepid=noaccess",
            ),
            (
                format!("{PROC} - proc proc rw,hidepid=1"),
                "/proc is mounted with hidepid=1",
            ),
            (
                format!("{PROC} - proc proc rw,hidepid=ptraceable"),
                "/proc is mounted with hidepid=ptraceable",
            ),
            // A group exempt from hidepid is not looked at.
            (
                format!("{PROC} - proc proc rw,gid=1001,hidepid=invisible"),
                "/proc is mounted with hidepid=invisible",
            ),
            (
                format!("{PROC} - proc proc rw,hidepid=7"),
                "/proc is mounted with hidepid=7",
            ),
            // The options are missing: no separator, or nothing after it.
            (
                PROC.to_string(),
                "cannot read the options of the mount at /proc",
            ),
            (
                format!("{PROC} - proc proc"),
                "cannot read the options of the mount at /proc",
            ),
            (
                format!("{PROC} - tmpfs tmpfs rw"),
                "/proc is a tmpfs mount, not proc",
            ),
            // No mount at /proc at all.
            (
                "999 272 0:101 / /procs rw - proc proc rw".to_string(),
                "no proc file system is mounted at /proc",
            ),
        ] {
            assert_eq!(
                proc_hiding(&mountinfo_with(&proc_line)).as_deref(),
                Some(why),
                "{proc_line}"
            );
        }
        // Two proc mounts stacked at /proc: one with hidepid is enough.
        let stacked = format!(
            "{PROC} - proc proc rw\n300 274 0:110 / /proc rw - proc proc rw,hidepid=invisible"
        );
        assert_eq!(
            proc_hiding(&mountinfo_with(&stacked)).as_deref(),
            Some("/proc is mounted with hidepid=invisible")
        );
        assert_eq!(
            proc_hiding(b"").as_deref(),
            Some("no proc file system is mounted at /proc")
        );
        // A mount table that cannot be read.
        let tmp = tempfile::tempdir().unwrap();
        let missing = proc_hiding_at(&tmp.path().join("mountinfo")).unwrap();
        assert!(
            missing.starts_with("cannot read ")
                && missing.ends_with("how /proc is mounted is unknown"),
            "{missing}"
        );
        let unreadable = proc_hiding_at(tmp.path()).unwrap();
        assert!(unreadable.starts_with("cannot read "), "{unreadable}");
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
