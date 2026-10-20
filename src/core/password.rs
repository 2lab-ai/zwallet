//! Reading the human's password: a no-echo prompt on the controlling terminal, or one line of
//! stdin with `--password-stdin`. There is deliberately no flag or env variable for it.

use crate::core::error::{Result, WalletError};
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, IsTerminal, Write};
use std::os::fd::AsRawFd;
use zeroize::Zeroizing;

pub fn read_password(from_stdin: bool, prompt: &str, confirm: bool) -> Result<Zeroizing<String>> {
    if from_stdin {
        let mut line = Zeroizing::new(String::new());
        std::io::stdin().lock().read_line(&mut line).map_err(|e| {
            WalletError::validation(format!("cannot read password from stdin: {e}"))
        })?;
        let pw = strip_newline(&line);
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

fn strip_newline(line: &str) -> Zeroizing<String> {
    let s = line
        .strip_suffix("\r\n")
        .or_else(|| line.strip_suffix('\n'))
        .unwrap_or(line);
    Zeroizing::new(s.to_string())
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
    (&tty).write_all(prompt.as_bytes()).map_err(|_| no_tty())?;
    // SAFETY: fd is open for the lifetime of `tty`; `term` came from tcgetattr.
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &term) } != 0 {
        return Err(no_tty());
    }
    let mut line = Zeroizing::new(String::new());
    let read = BufReader::new(&tty).read_line(&mut line);
    // SAFETY: restores the attributes captured above on the same fd.
    unsafe { libc::tcsetattr(fd, libc::TCSANOW, &original) };
    read.map_err(|_| no_tty())?;
    Ok(strip_newline(&line))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_one_trailing_newline_only() {
        assert_eq!(strip_newline("pw\n").as_str(), "pw");
        assert_eq!(strip_newline("pw\r\n").as_str(), "pw");
        assert_eq!(strip_newline(" pw \n").as_str(), " pw ");
        assert_eq!(strip_newline("pw").as_str(), "pw");
    }
}
