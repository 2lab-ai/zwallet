//! Executable fixtures must have no writable descriptor left in a forked child before exec.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

/// Copy a fixture's bytes and permissions to a new path, then wait for inherited writers.
/// `O_CLOEXEC` alone is insufficient: a parallel fork can keep the writer until its exec.
/// See https://github.com/rust-lang/rust/issues/114554.
pub fn copy_executable(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<u64> {
    copy_before_close(from.as_ref(), to.as_ref(), |_| {})
}

// The observer lets the regression hold the same open-file description as an inherited fd.
// This module is test-only; no wallet command or production copy operation uses this hook.
pub(crate) fn copy_before_close(
    from: &Path,
    to: &Path,
    before_close: impl FnOnce(&File),
) -> io::Result<u64> {
    let mut source = File::open(from)?;
    let mut target = OpenOptions::new().write(true).create_new(true).open(to)?;
    // flock belongs to the open-file description, so a fork or dup shares this lock.
    // Never explicitly unlock it: only the last writer closing may release the lock.
    target.lock()?;
    let copied = io::copy(&mut source, &mut target)?;
    target.set_permissions(source.metadata()?.permissions())?;
    before_close(&target);
    drop(target);
    // This independent, read-only description cannot acquire a shared lock until every
    // inherited copy of the locked writer has closed. Later forks inherit only readers.
    let ready = File::open(to)?;
    ready.lock_shared()?;
    Ok(copied)
}
