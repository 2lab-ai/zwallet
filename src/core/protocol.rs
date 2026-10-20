//! Daemon wire protocol and client.
//!
//! One JSON request line and one JSON response line per connection over `$ZWALLET_HOME/d.sock`
//! (0600 inside the 0700 home; peer uid checked on both ends). Requests are family-agnostic:
//! `chain` picks the family, `index` the key, and `tx` is the family's own unsigned-tx JSON.
//!
//! No request carries a secret. The password never travels over the socket: [`start_daemon`]
//! hands it to the daemon child it spawns, on that child's inherited stdin pipe, and reads the
//! child's readiness on its stdout pipe. Whatever else listens on `d.sock` (a stale socket, or a
//! squatter run by the same user) can learn nothing from a client and is replaced by the new
//! daemon, which binds under an exclusive lock on `d.lock`.

use crate::core::error::{ErrorCode, Result, WalletError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use zeroize::Zeroizing;

pub const PROTOCOL_VERSION: u32 = 2;
pub const SOCKET_NAME: &str = "d.sock";
/// Held (flock, exclusive) by the one daemon serving a home, for its whole life.
pub const LOCK_NAME: &str = "d.lock";
pub const MAX_MESSAGE: u64 = 1 << 20;
/// `sizeof(sockaddr_un.sun_path)` including the terminating NUL.
#[cfg(target_os = "linux")]
pub const SUN_PATH_MAX: usize = 108;
#[cfg(not(target_os = "linux"))]
pub const SUN_PATH_MAX: usize = 104;
const QUICK: Duration = Duration::from_secs(3);
/// How long `lock` waits for the old daemon's socket to disappear.
const LOCK_SETTLE: Duration = Duration::from_secs(2);
/// A new daemon waits for the previous one's lock, then decrypts (scrypt) and verifies every
/// key before it reports readiness, so it gets more time.
pub const UNLOCK_TIMEOUT: Duration = Duration::from_secs(60);
pub const SIGN_TIMEOUT: Duration = Duration::from_secs(15);

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

/// Asks whatever answers on `d.sock` to lock (the request carries nothing), then waits (bounded)
/// for the socket to disappear. Returns whether a daemon reported it was unlocked.
pub async fn lock(home: &Path) -> Result<bool> {
    let was_unlocked = send(home, &Request::Lock, QUICK)
        .await?
        .is_some_and(|v| v["wasUnlocked"] == serde_json::json!(true));
    let sock = socket_path(home)?;
    let deadline = tokio::time::Instant::now() + LOCK_SETTLE;
    while std::fs::symlink_metadata(&sock).is_ok() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Ok(was_unlocked)
}

/// What the CLI writes to the daemon child's stdin: the TTL in seconds on the first line, then
/// the password's raw bytes up to EOF.
pub fn encode_unlock_input(ttl: Duration, password: &str) -> Zeroizing<Vec<u8>> {
    let head = format!("{}\n", ttl.as_secs());
    let mut out = Zeroizing::new(Vec::with_capacity(head.len() + password.len()));
    out.extend_from_slice(head.as_bytes());
    out.extend_from_slice(password.as_bytes());
    out
}

/// The daemon's side of [`encode_unlock_input`]. Removes the TTL line in place, so the password
/// bytes stay in the one zeroized buffer they were read into.
pub fn decode_unlock_input(mut raw: Zeroizing<Vec<u8>>) -> Result<(Duration, Zeroizing<Vec<u8>>)> {
    let bad = || WalletError::validation("malformed unlock input");
    let nl = raw.iter().position(|b| *b == b'\n').ok_or_else(bad)?;
    let secs: u64 = std::str::from_utf8(&raw[..nl])
        .ok()
        .filter(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|d| d.parse().ok())
        .filter(|s| *s > 0)
        .ok_or_else(bad)?;
    raw.drain(..=nl);
    if raw.is_empty() {
        return Err(WalletError::validation(
            "no password given to the key daemon",
        ));
    }
    Ok((Duration::from_secs(secs), raw))
}

/// Starts a new key daemon for `home` holding the seed for `ttl`, and returns its readiness
/// report `{ locked, expiresAt, active, index, address, pid }` (the address derived by the
/// daemon from the decrypted seed, never read from the cache in wallet.json).
///
/// Any daemon already serving is first asked to lock. The new daemon is this process's own
/// child (`setsid`, so it outlives the terminal); it gets the TTL and password on its stdin pipe
/// and answers on its stdout pipe, so the password never touches the socket, argv or env. It
/// binds `d.sock` (replacing whatever is there) only after it decrypted the wallet and holds
/// `d.lock`. On any failure or timeout this kills and reaps the child.
pub async fn start_daemon(home: &Path, password: &str, ttl: Duration) -> Result<Value> {
    socket_path(home)?;
    // Best effort: a squatter that hangs or lies here only costs time; the new daemon replaces
    // its socket under d.lock either way.
    let _ = lock(home).await;
    let mut child = spawn_daemon(home)?;
    let ready = await_readiness(&mut child, password, ttl);
    if ready.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    ready
}

fn spawn_daemon(home: &Path) -> Result<Child> {
    let exe = std::env::current_exe().map_err(io_err("cannot locate the zwallet binary"))?;
    let mut cmd = Command::new(exe);
    cmd.arg("daemon")
        .env_clear()
        .env("ZWALLET_HOME", home)
        .current_dir("/")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // SAFETY: setsid(2) is async-signal-safe and only affects the child process.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    cmd.spawn().map_err(io_err("cannot start the key daemon"))
}

fn await_readiness(child: &mut Child, password: &str, ttl: Duration) -> Result<Value> {
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        return Err(WalletError::validation("the key daemon has no pipes"));
    };
    // Unbuffered pipe writes: no intermediate copy of the password in this process.
    stdin
        .write_all(&encode_unlock_input(ttl, password))
        .map_err(io_err("cannot hand the password to the key daemon"))?;
    drop(stdin);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let read = stdout.take(MAX_MESSAGE).read_to_end(&mut buf).map(|_| buf);
        let _ = tx.send(read);
    });
    let line = match rx.recv_timeout(UNLOCK_TIMEOUT) {
        Ok(Ok(line)) => line,
        Ok(Err(e)) => return Err(io_err("cannot read the key daemon's readiness")(e)),
        Err(_) => {
            return Err(WalletError::validation(format!(
                "the key daemon did not report readiness within {}s",
                UNLOCK_TIMEOUT.as_secs()
            )));
        }
    };
    if line.is_empty() {
        return Err(WalletError::validation(
            "the key daemon exited without reporting readiness",
        ));
    }
    let resp: Response = serde_json::from_slice(&line).map_err(|e| {
        WalletError::validation(format!("malformed readiness from the key daemon: {e}"))
    })?;
    resp.into_result()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let raw = encode_unlock_input(Duration::from_secs(86_400), "pa ss\nword");
        assert_eq!(raw.as_slice(), b"86400\npa ss\nword");
        let (ttl, pw) = decode_unlock_input(raw).unwrap();
        assert_eq!(ttl, Duration::from_secs(86_400));
        assert_eq!(pw.as_slice(), b"pa ss\nword");
        for bad in [
            &b"x\npw"[..],
            b"0\npw",
            b"\npw",
            b"10pw",
            b"10\n",
            b"-1\npw",
        ] {
            let err = decode_unlock_input(Zeroizing::new(bad.to_vec())).unwrap_err();
            assert_eq!(err.code, ErrorCode::ValidationError);
            assert!(!err.message.contains("pw"));
        }
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
    fn response_round_trip() {
        let e = WalletError::locked();
        let r: Response =
            serde_json::from_value(serde_json::to_value(Response::err(&e)).unwrap()).unwrap();
        assert_eq!(r.into_result().unwrap_err().code, ErrorCode::WalletLocked);
    }
}
