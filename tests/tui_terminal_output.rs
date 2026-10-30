//! Pure counter-examples for the final-output observer. Actual process cases are parent-run.
mod common;
use common::frame::restored_tail;
#[test]
fn final_output_observer_refuses_every_extra_byte_after_restore() {
    let prefix = b"\x1b[?1049h\x1b[1;1Hsafe\x1b[?2004l\x1b[?1049l";
    let expected = b"\x1b[?25h";
    let good = [prefix.as_slice(), expected.as_slice()].concat();
    assert_eq!(restored_tail(&good), Some(expected.as_slice()));
    for bad in [
        b"x".as_slice(),
        b"\x07",
        b"\x1b[2J",
        b"\x1b]0;hidden\x07",
        b"\x1b",
        b"\x1b[",
        b"\n",
        b"\x1b[?1049l",
    ] {
        let poisoned = [good.as_slice(), bad].concat();
        // The former frame-only observation falsely accepts every post-restore append.
        let old = common::Frame::last(&poisoned, (80, 24)).unwrap();
        assert!(old.stray.is_empty() && old.pending.is_empty());
        assert_ne!(
            restored_tail(&poisoned),
            Some(expected.as_slice()),
            "accepted tail: {bad:?}"
        );
    }
    for cut in 0..expected.len() {
        let partial = [prefix.as_slice(), &expected[..cut]].concat();
        assert_ne!(restored_tail(&partial), Some(expected.as_slice()));
    }
    assert!(restored_tail(b"no restore").is_none());
}
