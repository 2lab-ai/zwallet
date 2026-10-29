//! No daemon or wallet: hold a duplicate writer to reproduce the fork/exec fixture race.

#[path = "common/executable.rs"]
mod executable;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

#[test]
fn executable_copy_waits_for_the_last_inherited_writer() {
    let dir = tempfile::Builder::new()
        .prefix("zwt-copy-")
        .tempdir_in("/tmp")
        .unwrap();
    let copy = dir.path().join("true");
    let worker_path = copy.clone();
    let (writer_tx, writer_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let result =
            executable::copy_before_close("/usr/bin/true".as_ref(), &worker_path, |writer| {
                writer_tx.send(writer.try_clone().unwrap()).unwrap();
            });
        done_tx.send(result).unwrap();
    });
    // dup and fork share the writable open-file description, including its flock. Holding
    // this fd makes the race persistent; no probabilistic parallel-spawn loop is needed.
    let inherited = writer_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    #[cfg(target_os = "linux")]
    {
        let error = Command::new(&copy).env_clear().status().unwrap_err();
        assert_eq!(error.raw_os_error(), Some(libc::ETXTBSY));
    }
    let early = done_rx.recv_timeout(Duration::from_millis(100));
    drop(inherited);
    // Always release and join before assertions, including a RED result, so no worker leaks.
    let waited = early.is_err();
    let result = match early {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            done_rx.recv_timeout(Duration::from_secs(5)).unwrap()
        }
        Err(error) => panic!("copy worker disconnected: {error}"),
    };
    worker.join().unwrap();
    result.unwrap();
    assert!(
        waited,
        "copy returned while an inherited writer was still open"
    );
    assert_eq!(fs::read(&copy).unwrap(), fs::read("/usr/bin/true").unwrap());
    assert_eq!(
        fs::metadata(&copy).unwrap().permissions().mode(),
        fs::metadata("/usr/bin/true").unwrap().permissions().mode()
    );
    assert!(Command::new(&copy).env_clear().status().unwrap().success());
}

#[test]
fn executable_copy_refuses_to_replace_an_existing_fixture() {
    let dir = tempfile::Builder::new()
        .prefix("zwt-copy-")
        .tempdir_in("/tmp")
        .unwrap();
    let copy = dir.path().join("existing");
    fs::write(&copy, b"keep this fixture").unwrap();
    let error = executable::copy_executable("/usr/bin/true", &copy).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read(&copy).unwrap(), b"keep this fixture");
}
