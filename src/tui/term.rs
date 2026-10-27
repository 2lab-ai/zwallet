#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::{Backend, CrosstermBackend, TestBackend};
    use ratatui::buffer::Cell;
    use std::sync::{Arc, Mutex};

    /// A writer the test reads back after the code under test wrote to it.
    #[derive(Clone, Default)]
    struct Shared(Arc<Mutex<Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Shared {
        fn bytes(&self) -> Vec<u8> {
            self.0.lock().unwrap().clone()
        }
    }

    fn holds(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    #[test]
    fn the_filter_replaces_every_cell_outside_the_terminal_set_where_the_backend_receives_it() {
        let mut terminal = Terminal::new(FilteredBackend::new(TestBackend::new(12, 2))).unwrap();
        // Frame 1, all ASCII: nothing replaced. `X` at column 9 is what the column after the wide
        // glyph of frame 2 shows unless the filter writes a space there (ratatui's diff does not
        // send that column).
        terminal
            .draw(|f| {
                f.buffer_mut()
                    .set_string(0, 0, "ab       X", Default::default())
            })
            .unwrap();
        assert_eq!(terminal.backend().replaced(), 0);
        // Frame 2: five bad cells among ASCII ones.
        terminal
            .draw(|f| {
                let buf = f.buffer_mut();
                buf.set_string(0, 0, "ab", Default::default());
                for (x, symbol) in [
                    (2, "\u{2588}"),
                    (3, "\u{1b}"),
                    (4, "\u{250c}"),
                    (5, "e\u{301}"),
                    (8, "\u{6f22}"),
                ] {
                    buf[(x, 0)].set_symbol(symbol);
                }
                buf.set_string(6, 0, "cd", Default::default());
                buf.set_string(0, 1, "all ASCII ~", Default::default());
            })
            .unwrap();
        let backend = terminal.backend();
        assert_eq!(backend.replaced(), 5);
        let got = backend.inner().buffer();
        let row = |y: u16| -> String { (0..12).map(|x| got[(x, y)].symbol()).collect() };
        // `漢` is `?`, and the column it covered (9) a space: the `X` is gone.
        assert_eq!(row(0), "ab????cd?   ");
        assert_eq!(row(1), "all ASCII ~ ");
    }

    #[test]
    fn terminal_control_from_a_cell_never_reaches_a_crossterm_writer() {
        let out = Shared::default();
        let mut backend = FilteredBackend::new(CrosstermBackend::new(out.clone()));
        let mut csi = Cell::default();
        csi.set_symbol("\u{1b}[31m");
        let mut osc = Cell::default();
        osc.set_symbol("\u{1b}]0;x\u{7}");
        let mut plain = Cell::default();
        plain.set_symbol("k");
        backend
            .draw([(0, 0, &csi), (1, 0, &plain), (2, 0, &osc)].into_iter())
            .unwrap();
        backend.flush().unwrap();
        assert_eq!(backend.replaced(), 2);
        let bytes = out.bytes();
        assert!(!holds(&bytes, b"\x1b[31m"), "{bytes:?}");
        assert!(!holds(&bytes, b"\x1b]0;x"), "{bytes:?}");
        assert!(!bytes.contains(&0x07), "{bytes:?}");
        assert!(holds(&bytes, b"?k?"), "{bytes:?}");
    }

    /// What crossterm writes to leave the alternate screen and to show the cursor.
    const LEAVE_ALTERNATE_SCREEN: &[u8] = b"\x1b[?1049l";
    const SHOW_CURSOR: &[u8] = b"\x1b[?25h";

    #[test]
    fn the_guard_restores_the_terminal_on_drop_and_from_the_panic_hook() {
        let out = Shared::default();
        drop(Guard::on(out.clone()).unwrap());
        let written = out.bytes();
        assert!(holds(&written, LEAVE_ALTERNATE_SCREEN), "{written:?}");
        assert!(holds(&written, SHOW_CURSOR), "{written:?}");

        // A panic while the TUI holds the terminal: the hook restores it, then writes its one
        // fixed line, never the payload.
        let (out, err) = (Shared::default(), Shared::default());
        let (hook_out, hook_err) = (out.clone(), err.clone());
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |_| {
            on_panic(&mut hook_out.clone(), &mut hook_err.clone())
        }));
        let caught = std::panic::catch_unwind(|| panic!("payload pw-XYZ-0123456789 \u{1b}[2J"));
        std::panic::set_hook(previous);
        assert!(caught.is_err());
        let written = out.bytes();
        assert!(holds(&written, LEAVE_ALTERNATE_SCREEN), "{written:?}");
        assert!(holds(&written, SHOW_CURSOR), "{written:?}");
        assert_eq!(
            String::from_utf8(err.bytes()).unwrap(),
            "zwallet tui stopped unexpectedly; the terminal was restored\n"
        );
    }
}
