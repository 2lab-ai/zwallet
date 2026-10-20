//! Daemon wire protocol and client.
//!
//! One JSON request line and one JSON response line per connection over `$ZWALLET_HOME/d.sock`
//! (0600 inside the 0700 home; peer uid checked on both ends). Requests are family-agnostic:
//! `chain` picks the family, `index` the key, and `tx` is the family's own unsigned-tx JSON.

use crate::core::error::{ErrorCode, Result, WalletError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::ErrorKind;
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use zeroize::{Zeroize, Zeroizing};

pub const PROTOCOL_VERSION: u32 = 1;
pub const SOCKET_NAME: &str = "d.sock";
pub const MAX_MESSAGE: u64 = 1 << 20;
/// `sizeof(sockaddr_un.sun_path)` including the terminating NUL.
#[cfg(target_os = "linux")]
pub const SUN_PATH_MAX: usize = 108;
#[cfg(not(target_os = "linux"))]
pub const SUN_PATH_MAX: usize = 104;
/// Upper bound on waiting for a freshly started daemon to accept connections.
const STARTUP_WAIT: Duration = Duration::from_secs(10);
const QUICK: Duration = Duration::from_secs(3);
/// Unlock runs scrypt and verifies every key, so it gets more time.
pub const UNLOCK_TIMEOUT: Duration = Duration::from_secs(60);
pub const SIGN_TIMEOUT: Duration = Duration::from_secs(15);

/// A daemon request. Not `Debug`; the unlock password is wiped on drop.
#[derive(Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum Request {
    Unlock {
        password: String,
        ttl_secs: u64,
    },
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

impl Drop for Request {
    fn drop(&mut self) {
        if let Request::Unlock { password, .. } = self {
            password.zeroize();
        }
    }
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

/// Starts `zwallet daemon` detached in its own session so it outlives the terminal. Only the
/// home path travels through env; the password and seed never do.
fn spawn_daemon(home: &Path) -> Result<()> {
    let exe = std::env::current_exe().map_err(io_err("cannot locate the zwallet binary"))?;
    let mut cmd = Command::new(exe);
    cmd.arg("daemon")
        .env_clear()
        .env("ZWALLET_HOME", home)
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
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
    cmd.spawn().map_err(io_err("cannot start the key daemon"))?;
    Ok(())
}

/// Ensures a daemon is accepting connections, starting one if needed. Polls with backoff up to
/// a bounded total wait.
pub async fn ensure_daemon(home: &Path) -> Result<()> {
    if status(home).await?.is_some() {
        return Ok(());
    }
    spawn_daemon(home)?;
    let deadline = Instant::now() + STARTUP_WAIT;
    let mut delay = Duration::from_millis(10);
    loop {
        tokio::time::sleep(delay).await;
        if status(home).await?.is_some() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(WalletError::validation(format!(
                "the key daemon did not start within {}s",
                STARTUP_WAIT.as_secs()
            )));
        }
        delay = (delay * 2).min(Duration::from_millis(200));
    }
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
        let u = serde_json::to_value(Request::Unlock {
            password: "x".into(),
            ttl_secs: 5,
        })
        .unwrap();
        assert_eq!(u["op"], "unlock");
        assert_eq!(u["ttlSecs"], 5);
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
