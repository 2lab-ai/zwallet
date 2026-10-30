//! Test-only observations of the real runtime, terminal input and renderer. The resize test
//! disables refresh ticks, so only actual SIGWINCH events can redraw its settled screen.
use super::*;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::OnceLock;
use std::time::Instant;

#[path = "../../tests/common/frame.rs"]
pub(crate) mod frame;

pub(crate) const WAIT: Duration = Duration::from_secs(10);
const RESIZE_CHILD: &str = "ZWALLET_TEST_RESIZE_WITHOUT_TICKS_HOME";
static REFRESH_DISABLED: AtomicBool = AtomicBool::new(false);

pub(crate) fn refresh_disabled() -> bool {
    REFRESH_DISABLED.load(Ordering::SeqCst)
}

pub(crate) fn fixture_home() -> tempfile::TempDir {
    use crate::core::store::{self, KDF_MIN_LOG_N, KdfParams, MasterOptions};
    let home = tempfile::Builder::new()
        .prefix("zwt-term-")
        .tempdir_in("/tmp")
        .unwrap();
    store::create_master(
        home.path(),
        "temporary fixture password",
        "main",
        MasterOptions {
            words: 12,
            kdf: KdfParams {
                log_n: KDF_MIN_LOG_N,
                ..KdfParams::default()
            },
        },
        &crate::chains::derive_addresses,
    )
    .unwrap();
    home
}
static LOCK_RELEASE: OnceLock<PathBuf> = OnceLock::new();
pub(crate) const LOCK_ERROR: &str = "fixture lock failed; wallet may remain unlocked";

pub(crate) fn arm_lock_failure(release: PathBuf) {
    LOCK_RELEASE.set(release).unwrap();
}

/// Exists only in a cfg(test) library. This branch never calls a real stop action.
/// The driver releases it only after observing the real runtime's quit-pending frame.
pub(crate) fn injected_lock_failure() -> Option<WalletError> {
    LOCK_RELEASE.get().map(|release| {
        let start = Instant::now();
        while !release.exists() {
            assert!(
                start.elapsed() < WAIT,
                "fixture did not release failed lock"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        WalletError::validation(LOCK_ERROR)
    })
}

pub(crate) struct Pty {
    child: Child,
    master: std::fs::File,
    bytes: Arc<Mutex<Vec<u8>>>,
    reader: Option<std::thread::JoinHandle<()>>,
    errors: Option<std::thread::JoinHandle<Vec<u8>>>,
}
impl Pty {
    pub(crate) fn spawn(mut command: Command, merged_stderr: bool, size: (u16, u16)) -> Self {
        let (mut master, mut slave) = (-1, -1);
        let mut window = libc::winsize {
            ws_row: size.1,
            ws_col: size.0,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: openpty initializes two exclusively owned fds; winsize is valid.
        // Its libc type is *mut on macOS and *const on Linux; a raw pointer fits both.
        assert_eq!(
            unsafe {
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &raw mut window,
                )
            },
            0
        );
        // SAFETY: openpty just returned these fds; each is transferred to exactly one owner.
        let (master, slave) = unsafe {
            (
                std::fs::File::from_raw_fd(master),
                OwnedFd::from_raw_fd(slave),
            )
        };
        for fd in [master.as_raw_fd(), slave.as_raw_fd()] {
            // SAFETY: fcntl operates on a live owned fd, before a child is started.
            assert_eq!(
                unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) },
                0
            );
        }
        command
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(if merged_stderr {
                Stdio::from(slave.try_clone().unwrap())
            } else {
                Stdio::piped()
            });
        // SAFETY: setsid/ioctl are async-signal-safe and stdin is the child's pty slave.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 || libc::ioctl(0, libc::TIOCSCTTY as libc::c_ulong, 0) == -1
                {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command.spawn().unwrap();
        drop(slave); // EOF must include every final write and end with this child.
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let sink = bytes.clone();
        let mut source = master.try_clone().unwrap();
        let reader = std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match source.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => sink.lock().unwrap().extend_from_slice(&buf[..n]),
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) if e.raw_os_error() == Some(libc::EIO) => break, // PTY EOF.
                    Err(e) => panic!("PTY reader failed before EOF: {e}"),
                }
            }
        });
        let errors = child.stderr.take().map(|mut error| {
            std::thread::spawn(move || {
                let mut b = Vec::new();
                error.read_to_end(&mut b).unwrap();
                b
            })
        });
        Self {
            child,
            master,
            bytes,
            reader: Some(reader),
            errors,
        }
    }
    pub(crate) fn output(&self) -> Vec<u8> {
        self.bytes.lock().unwrap().clone()
    }
    pub(crate) fn send(&self, bytes: &[u8]) {
        (&self.master).write_all(bytes).unwrap();
    }
    pub(crate) fn wait(&mut self, condition: impl Fn(&[u8]) -> bool) {
        let start = Instant::now();
        loop {
            let bytes = self.output();
            if condition(&bytes) {
                return;
            }
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "child exited before observation: {bytes:?}"
            );
            assert!(
                start.elapsed() < WAIT,
                "terminal observation deadline: {bytes:?}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    pub(crate) fn resize(&self, cols: u16, rows: u16) {
        let window = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: ioctl reads one valid winsize on the test's owned terminal.
        assert_eq!(
            unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ, &window) },
            0
        );
    }
    pub(crate) fn finish(&mut self) -> (ExitStatus, Vec<u8>, Vec<u8>) {
        let start = Instant::now();
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            assert!(start.elapsed() < WAIT, "child did not exit");
            std::thread::sleep(Duration::from_millis(10));
        };
        while self.reader.as_ref().is_some_and(|r| !r.is_finished())
            || self.errors.as_ref().is_some_and(|r| !r.is_finished())
        {
            assert!(
                start.elapsed() < WAIT,
                "child output did not end after exit"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        self.reader.take().unwrap().join().unwrap();
        let err = self
            .errors
            .take()
            .map(|t| t.join().unwrap())
            .unwrap_or_default();
        // SAFETY: tcgetattr initializes termios for the still-owned PTY master.
        let modes = unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            assert_eq!(libc::tcgetattr(self.master.as_raw_fd(), &mut t), 0);
            t.c_lflag
        };
        assert_ne!(modes & libc::ICANON, 0);
        assert_ne!(modes & libc::ECHO, 0);
        (status, self.output(), err)
    }
}
impl Drop for Pty {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn resize_without_ticks_child() {
    let Some(home) = std::env::var_os(RESIZE_CHILD) else {
        return;
    };
    REFRESH_DISABLED.store(true, Ordering::SeqCst);
    let net = Net {
        chain: crate::chains::find("ethereum").unwrap(),
        rpc_url: "http://127.0.0.1:1".into(),
    };
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(run(home.into(), net, Duration::from_secs(60), false))
        .unwrap();
    std::process::exit(0);
}

#[test]
fn actual_sigwinch_redraws_shrink_and_grow_without_refresh_ticks() {
    let home = fixture_home();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .env_clear()
        .env(RESIZE_CHILD, home.path())
        .env(crate::core::protocol::TEST_SCOPE_ENV, home.path())
        .args([
            "tui::terminal_observation_tests::resize_without_ticks_child",
            "--exact",
            "--nocapture",
        ]);
    let mut child = Pty::spawn(command, false, (120, 40));
    // Both initial workers have completed once the key list and locked status are visible.
    // After this there is no message producer except actual terminal input: ticks are disabled.
    child.wait(|bytes| {
        let f = frame::Frame::parse(bytes, (120, 40));
        f.rows[38].starts_with("ethereum | main 0x")
            && f.rows[39].starts_with("locked | ")
            && f.rows[..38].iter().any(|r| r.contains("* main"))
    });
    // A real menu key crosses event::poll/read first, proving SIGWINCH registration exists.
    // Neither opening nor dismissing this menu starts a worker.
    child.send(b"?");
    child.wait(|bytes| {
        frame::Frame::parse(bytes, (120, 40))
            .rows
            .iter()
            .any(|r| r.contains("filter: "))
    });
    child.send(b"\x1b");
    child.wait(|bytes| {
        let f = frame::Frame::parse(bytes, (120, 40));
        !f.rows.iter().any(|r| r.contains("filter: "))
            && f.rows[..38].iter().any(|r| r.contains("* main"))
    });
    child.resize(79, 24);
    child.wait(|bytes| {
        let f = frame::Frame::parse(bytes, (120, 40));
        f.rows[0].trim_end() == ui::TOO_SMALL && f.rows[1..].iter().all(|r| r.trim().is_empty())
    });
    child.resize(80, 24);
    child.wait(|bytes| {
        let f = frame::Frame::parse(bytes, (120, 40));
        f.rows[22].starts_with("ethereum | main 0x")
            && f.rows[23].starts_with("locked | ")
            && !f.rows[0].contains(ui::TOO_SMALL)
    });
    child.send(b"q");
    let (status, bytes, err) = child.finish();
    assert!(status.success());
    assert!(err.is_empty(), "{err:?}");
    let f = frame::Frame::last(&bytes, (120, 40)).unwrap();
    assert!(f.stray.is_empty() && f.pending.is_empty(), "{f}");
    assert!(f.rows[22].starts_with("ethereum | main 0x"), "{f}");
    assert!(f.rows[23].starts_with("locked | "), "{f}");
    assert!(
        f.rows[..24].iter().all(|r| r[80..].trim().is_empty()),
        "{f}"
    );
    assert!(f.rows[24..].iter().all(|r| r.trim().is_empty()), "{f}");
    assert_eq!(frame::restored_tail(&bytes), Some(&b"\x1b[?25h"[..]));
}
