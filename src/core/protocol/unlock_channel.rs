//! Private CLI-to-child unlock channel framing and progress responses.
//!
//! The public daemon socket never carries a password. These helpers are used only on the
//! socketpair owned by the unlocking CLI and its child. Identity checks and the decision to
//! send a password or GO remain in the parent module's lifecycle coordinator.

use super::{MAX_MESSAGE, Response};
use crate::core::error::{Result, WalletError};
use crate::core::password::{MAX_SECRET, read_exact_secret};
use serde_json::json;
use std::io::{BufRead, BufReader, ErrorKind, Read};
use std::os::fd::RawFd;
use std::os::unix::net::UnixStream as ChildChannel;
use std::sync::mpsc;
use std::time::Duration;
use zeroize::Zeroizing;

/// Longest header line before the password: `<ttl secs> <password length>`.
const MAX_HEADER: usize = 48;
/// Child → CLI progress, the `stage` of an ok [`Response`] line: ready for the password (core
/// dumps are off), then the wallet decrypted with it. The third line is the readiness itself.
pub const STAGE_INPUT: &str = "input";
pub const STAGE_DECRYPTED: &str = "decrypted";
/// CLI → child once the old daemons are gone: take `d.lock`, bind and serve.
pub const GO: u8 = b'G';

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

/// Lines from the daemon's end of the channel, read on a thread so each can be awaited with a
/// timeout. `None` once it closed.
pub(super) fn line_reader(chan: ChildChannel) -> mpsc::Receiver<Option<Vec<u8>>> {
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

pub(super) fn next_response(
    lines: &mpsc::Receiver<Option<Vec<u8>>>,
    wait: Duration,
) -> Result<Response> {
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

pub(super) fn expect_stage(
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
