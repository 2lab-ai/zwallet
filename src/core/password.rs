//! Reading the human's password: a no-echo prompt on the controlling terminal, or one line of
//! stdin with `--password-stdin`. There is deliberately no flag or env variable for it.
//!
//! Secrets are read with raw read(2) into one zeroized buffer allocated once at its final size:
//! no `BufReader` and no growing `Vec`, so no freed buffer keeps an unwiped copy.

use crate::core::error::{Result, WalletError};
use std::fs::OpenOptions;
use std::io::{ErrorKind, IsTerminal};
use std::os::fd::{AsRawFd, RawFd};
use zeroize::{Zeroize, Zeroizing};

/// Longest password accepted on stdin or the terminal, in bytes, including its newline.
pub const MAX_SECRET: usize = 1024;

/// Best effort: no core dump of a process that holds the password or seed. Called once, as the
/// first line of the process entry (`cli::main_entry`), for every command. On Linux this also
/// clears the dumpable flag, which from this call on keeps other processes of the same user out
/// of `/proc/<pid>/mem`, `/proc/<pid>/fd` and new ptrace attachments. Not before it: exec makes a
/// process dumpable again, and a tracer attached in that window stays attached. That is why the
/// daemon's channel is a socket (not reopenable through `/proc/<pid>/fd`) and why `unlock`
/// requires Yama `ptrace_scope` ≥ 1 on Linux.
pub fn disable_core_dumps() {
    let zero = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: setrlimit reads a valid rlimit struct; failure is harmless (best effort).
    unsafe {
        libc::setrlimit(libc::RLIMIT_CORE, &zero);
    }
    #[cfg(target_os = "linux")]
    // SAFETY: PR_SET_DUMPABLE takes a plain integer argument; failure is harmless.
    unsafe {
        libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0);
    }
}

/// Reads `fd` with raw read(2) into a buffer of exactly `max` bytes, until EOF or (with `line`)
/// the first newline, which is kept. More than `max` bytes is an error.
pub fn read_secret(fd: RawFd, line: bool, max: usize) -> std::io::Result<Zeroizing<Vec<u8>>> {
    let mut buf = Zeroizing::new(vec![0u8; max]);
    let mut len = 0;
    loop {
        if len == buf.len() {
            return Err(std::io::Error::other("input too long"));
        }
        let free = &mut buf[len..];
        // SAFETY: `free` is a valid writable region of `free.len()` bytes for the whole call.
        let n = unsafe { libc::read(fd, free.as_mut_ptr().cast(), free.len()) };
        if n < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        if n == 0 {
            break;
        }
        let start = len;
        len += n as usize;
        if line && let Some(pos) = buf[start..len].iter().position(|b| *b == b'\n') {
            len = start + pos + 1;
            break;
        }
    }
    // Shrinks the length only; the capacity (and the tail, wiped on drop) stays in place.
    buf.truncate(len);
    Ok(buf)
}

/// Reads exactly `len` bytes from `fd` with raw read(2) into one zeroized buffer of that size
/// (the daemon's password, whose length the unlocking CLI sends first). EOF before that is an
/// error.
pub fn read_exact_secret(fd: RawFd, len: usize) -> std::io::Result<Zeroizing<Vec<u8>>> {
    let mut buf = Zeroizing::new(vec![0u8; len]);
    let mut got = 0;
    while got < len {
        let free = &mut buf[got..];
        // SAFETY: `free` is a valid writable region of `free.len()` bytes for the whole call.
        let n = unsafe { libc::read(fd, free.as_mut_ptr().cast(), free.len()) };
        if n < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        if n == 0 {
            return Err(std::io::Error::from(ErrorKind::UnexpectedEof));
        }
        got += n as usize;
    }
    Ok(buf)
}

/// Turns raw secret bytes into a password without copying them: one trailing `\n` or `\r\n` is
/// dropped, and invalid UTF-8 is wiped before the error is returned.
pub fn into_password(mut raw: Zeroizing<Vec<u8>>) -> Result<Zeroizing<String>> {
    if raw.last() == Some(&b'\n') {
        raw.pop();
        if raw.last() == Some(&b'\r') {
            raw.pop();
        }
    }
    match String::from_utf8(std::mem::take(&mut *raw)) {
        Ok(s) => Ok(Zeroizing::new(s)),
        Err(e) => {
            e.into_bytes().zeroize();
            Err(WalletError::validation("the password is not valid UTF-8"))
        }
    }
}

pub fn read_password(from_stdin: bool, prompt: &str, confirm: bool) -> Result<Zeroizing<String>> {
    if from_stdin {
        let raw = read_secret(std::io::stdin().as_raw_fd(), true, MAX_SECRET).map_err(|e| {
            WalletError::validation(format!("cannot read password from stdin: {e}"))
        })?;
        let pw = into_password(raw)?;
        if pw.is_empty() {
            return Err(WalletError::validation(
                "--password-stdin: no password on stdin",
            ));
        }
        return Ok(pw);
    }
    // Prompt only when a human is plausibly at the keyboard; otherwise fail fast instead of
    // blocking an agent's shell on a hidden /dev/tty prompt.
    if !std::io::stdin().is_terminal() {
        return Err(no_tty());
    }
    let first = prompt_tty(prompt)?;
    if confirm {
        let second = prompt_tty("Repeat password: ")?;
        if *first != *second {
            return Err(WalletError::validation("passwords do not match"));
        }
    }
    Ok(first)
}

fn no_tty() -> WalletError {
    WalletError::validation(
        "no terminal to prompt for the password; a human must run this at a terminal, or pipe \
         it with --password-stdin",
    )
}

/// Prompts on `/dev/tty` with echo off.
fn prompt_tty(prompt: &str) -> Result<Zeroizing<String>> {
    let tty = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .map_err(|_| no_tty())?;
    let fd = tty.as_raw_fd();
    // SAFETY: termios is plain old data; tcgetattr fills it for a valid fd.
    let mut term: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(fd, &mut term) } != 0 {
        return Err(no_tty());
    }
    let original = term;
    term.c_lflag &= !libc::ECHO;
    term.c_lflag |= libc::ECHONL;
    // The prompt can carry a whitelist change summary, which quotes files: through the write point.
    crate::core::terminal::write_human(&mut &tty, prompt).map_err(|_| no_tty())?;
    // SAFETY: fd is open for the lifetime of `tty`; `term` came from tcgetattr.
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &term) } != 0 {
        return Err(no_tty());
    }
    let read = read_secret(fd, true, MAX_SECRET);
    // SAFETY: restores the attributes captured above on the same fd.
    unsafe { libc::tcsetattr(fd, libc::TCSANOW, &original) };
    into_password(read.map_err(|_| no_tty())?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::fd::{FromRawFd, OwnedFd};

    fn pipe_with(bytes: &[u8]) -> OwnedFd {
        let mut fds = [0; 2];
        // SAFETY: pipe(2) fills two fds we then own.
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let (r, w) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
        std::fs::File::from(w).write_all(bytes).unwrap();
        r
    }

    fn pw(bytes: &[u8]) -> String {
        into_password(Zeroizing::new(bytes.to_vec()))
            .unwrap()
            .to_string()
    }

    #[test]
    fn strips_one_trailing_newline_only() {
        assert_eq!(pw(b"pw\n"), "pw");
        assert_eq!(pw(b"pw\r\n"), "pw");
        assert_eq!(pw(b" pw \n"), " pw ");
        assert_eq!(pw(b"pw"), "pw");
        assert_eq!(pw(b"pw\n\n"), "pw\n");
        assert!(into_password(Zeroizing::new(vec![0xff, 0xfe])).is_err());
    }

    #[test]
    fn raw_reads_stop_at_the_first_line_or_eof_without_reallocating() {
        let fd = pipe_with(b"first line\nsecond\n");
        let got = read_secret(fd.as_raw_fd(), true, MAX_SECRET).unwrap();
        assert_eq!(got.as_slice(), b"first line\n");
        assert_eq!(got.capacity(), MAX_SECRET, "one allocation, never grown");

        let fd = pipe_with(b"30\nall of it\nincluding this");
        let got = read_secret(fd.as_raw_fd(), false, MAX_SECRET).unwrap();
        assert_eq!(got.as_slice(), b"30\nall of it\nincluding this");

        let fd = pipe_with(&[b'x'; MAX_SECRET + 1]);
        assert!(read_secret(fd.as_raw_fd(), false, MAX_SECRET).is_err());
    }
}
