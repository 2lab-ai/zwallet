//! The key holder (`zwallet daemon`, hidden).
//!
//! Started detached by `zwallet unlock`. It decrypts `wallet.json` itself, keeps only the master
//! seed in memory, derives keys and signs on request, and nothing else: it makes no network
//! request. On `lock`, on expiry (wall clock, so system sleep counts), or on SIGTERM/SIGINT it
//! drops the seed (zeroized) and exits. A fresh daemon that is not unlocked within a short grace
//! period exits too, so a daemon process exists only while the wallet is unlocked.

use crate::chains;
use crate::core::error::{Result, WalletError};
use crate::core::protocol::{self, MAX_MESSAGE, Request, Response};
use crate::core::seed::Seed;
use crate::core::store;
use crate::core::time::{rfc3339_utc, unix_secs};
use alloy::hex;
use serde_json::{Value, json};
use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::signal::unix::{SignalKind, signal};
use zeroize::Zeroizing;

/// A daemon that receives no successful unlock within this window exits.
const STARTUP_GRACE: Duration = Duration::from_secs(30);
/// Longest sleep between expiry checks.
const MAX_TICK: Duration = Duration::from_secs(30);
const IO_TIMEOUT: Duration = Duration::from_secs(10);

struct Session {
    seed: Seed,
    expires_at: SystemTime,
    /// Ciphertext of the wallet.json this seed came from; if the file is replaced, lock.
    sealed: String,
}

struct State {
    home: PathBuf,
    session: Option<Session>,
    started: SystemTime,
    done: bool,
}

impl State {
    fn expire_if_due(&mut self) {
        let now = SystemTime::now();
        match &self.session {
            Some(s) if now >= s.expires_at => {
                self.session = None;
                self.done = true;
            }
            None if now >= self.started + STARTUP_GRACE => self.done = true,
            _ => {}
        }
    }

    fn next_tick(&self) -> Duration {
        let target = self
            .session
            .as_ref()
            .map(|s| s.expires_at)
            .unwrap_or(self.started + STARTUP_GRACE);
        target
            .duration_since(SystemTime::now())
            .unwrap_or(Duration::ZERO)
            .clamp(Duration::from_millis(5), MAX_TICK)
    }

    fn status(&self) -> Value {
        match &self.session {
            Some(s) => json!({
                "locked": false,
                "expiresAt": rfc3339_utc(unix_secs(s.expires_at)),
                "pid": std::process::id(),
            }),
            None => json!({ "locked": true, "pid": std::process::id() }),
        }
    }

    /// The seed, provided the session is live and wallet.json is still the file it came from.
    fn seed(&mut self) -> Result<&Seed> {
        let current = store::read_wallet_file(&self.home)
            .ok()
            .map(|f| f.crypto.ciphertext);
        let unchanged = matches!(
            (&self.session, current),
            (Some(s), Some(c)) if s.sealed == c
        );
        if self.session.is_some() && !unchanged {
            self.session = None;
            self.done = true;
            return Err(WalletError::new(
                crate::core::error::ErrorCode::WalletLocked,
                "wallet.json changed since unlock; locked. A human must run `zwallet unlock 24h`",
            ));
        }
        self.session
            .as_ref()
            .map(|s| &s.seed)
            .ok_or_else(WalletError::locked)
    }
}

fn disable_core_dumps() {
    let zero = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: setrlimit reads a valid rlimit struct; failure is harmless (best effort).
    unsafe {
        libc::setrlimit(libc::RLIMIT_CORE, &zero);
    }
}

pub async fn run(home: PathBuf) -> Result<()> {
    let path = protocol::socket_path(&home)?;
    if protocol::status(&home).await?.is_some() {
        return Ok(()); // another daemon already serves this home
    }
    disable_core_dumps();
    let fail = |what: &'static str| {
        move |e: std::io::Error| WalletError::validation(format!("{what}: {e}"))
    };
    match fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(fail("cannot remove stale socket")(e)),
    }
    let listener = UnixListener::bind(&path).map_err(fail("cannot bind daemon socket"))?;
    fs::set_permissions(&path, fs::Permissions::from_mode(store::FILE_MODE))
        .map_err(fail("cannot restrict daemon socket"))?;
    let inode = fs::metadata(&path).map(|m| m.ino()).ok();

    let mut term = signal(SignalKind::terminate()).map_err(fail("signal setup"))?;
    let mut int = signal(SignalKind::interrupt()).map_err(fail("signal setup"))?;
    // No controlling terminal after setsid, but ignore a stray SIGHUP rather than die.
    let _hup = signal(SignalKind::hangup()).map_err(fail("signal setup"))?;

    let uid = protocol::current_uid();
    let mut state = State {
        home,
        session: None,
        started: SystemTime::now(),
        done: false,
    };
    while !state.done {
        tokio::select! {
            accepted = listener.accept() => {
                if let Ok((stream, _)) = accepted {
                    serve(&mut state, stream, uid).await;
                }
            }
            _ = tokio::time::sleep(state.next_tick()) => {}
            _ = term.recv() => state.done = true,
            _ = int.recv() => state.done = true,
        }
        state.expire_if_due();
    }
    state.session = None; // zeroizes the seed
    drop(listener);
    if fs::metadata(&path).map(|m| m.ino()).ok() == inode {
        let _ = fs::remove_file(&path);
    }
    Ok(())
}

async fn serve(state: &mut State, stream: UnixStream, uid: u32) {
    match stream.peer_cred() {
        Ok(cred) if cred.uid() == uid => {}
        _ => return,
    }
    let (reader, mut writer) = stream.into_split();
    let mut buf = Zeroizing::new(Vec::new());
    let mut reader = BufReader::new(reader.take(MAX_MESSAGE));
    match tokio::time::timeout(IO_TIMEOUT, reader.read_until(b'\n', &mut buf)).await {
        Ok(Ok(n)) if n > 0 => {}
        _ => return,
    }
    let response = match serde_json::from_slice::<Request>(&buf) {
        Ok(req) => match handle(state, &req) {
            Ok(v) => Response::ok(v),
            Err(e) => Response::err(&e),
        },
        Err(_) => Response::err(&WalletError::validation("malformed daemon request")),
    };
    let mut out = serde_json::to_vec(&response).unwrap_or_default();
    out.push(b'\n');
    let _ = tokio::time::timeout(IO_TIMEOUT, writer.write_all(&out)).await;
    let _ = writer.shutdown().await;
}

fn handle(state: &mut State, req: &Request) -> Result<Value> {
    state.expire_if_due();
    match req {
        Request::Unlock { password, ttl_secs } => {
            let expires_at = (*ttl_secs > 0)
                .then(|| SystemTime::now().checked_add(Duration::from_secs(*ttl_secs)))
                .flatten()
                .ok_or_else(|| WalletError::validation("invalid unlock duration"))?;
            match store::unlock(&state.home, password, &chains::derive_addresses) {
                Ok((unsealed, file)) => {
                    state.session = Some(Session {
                        seed: unsealed.seed,
                        expires_at,
                        sealed: file.crypto.ciphertext.clone(),
                    });
                    state.done = false;
                    let mut status = state.status();
                    status["active"] = json!(file.active);
                    Ok(status)
                }
                Err(e) => {
                    // A daemon that never unlocked has no reason to stay.
                    if state.session.is_none() {
                        state.done = true;
                    }
                    Err(e)
                }
            }
        }
        Request::Lock => {
            let was_unlocked = state.session.take().is_some();
            state.done = true;
            Ok(json!({ "locked": true, "wasUnlocked": was_unlocked }))
        }
        Request::Status => Ok(state.status()),
        Request::Address { chain, index } => {
            let chain = chains::find(chain)?;
            let seed = state.seed()?;
            let address = chain.family.keys().derive_address(seed, *index)?;
            Ok(json!({ "chain": chain.key, "index": index, "address": address }))
        }
        Request::SignMessage {
            chain,
            index,
            message_hex,
        } => {
            let chain = chains::find(chain)?;
            let message = hex::decode(message_hex)
                .map_err(|_| WalletError::validation("message_hex is not hex"))?;
            let seed = state.seed()?;
            chain
                .family
                .keys()
                .sign_message(seed, *index, chain, &message)
        }
        Request::SignTx { chain, index, tx } => {
            let chain = chains::find(chain)?;
            let seed = state.seed()?;
            chain.family.keys().sign_tx(seed, *index, chain, tx)
        }
    }
}
