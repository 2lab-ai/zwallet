//! `Q` on 4 receive: the open key's address on the selected chain as zwallet's own QR code
//! (`crate::qr`, the CLI's encoder) with the CLI's caption below it, on the whole screen. Only an address
//! the password or this build's session verified is encoded, by the CLI's `with_qr` rule. This is
//! the one drawing in [`QR`], and its glyphs come from the encoder alone.
use super::*;
use crate::chains::Chain;
use crate::commands::{SCAN_TO_SEND, qr_networks};
use crate::tui::style::QR;

/// The hint row of the QR view.
const QR_HINT: &str = "Q/Esc addresses  V verify  c chain  r refresh  q quit  ? actions";

/// Whether the frame is the QR view: `Q` pressed on 4 receive, and nothing else on the screen.
pub(super) fn active(app: &App) -> bool {
    app.tab() == Tab::Receive
        && app.navigation().receive_qr
        && app.navigation().popup.is_none()
        && app.password_state() == PasswordState::Idle
        && !app.creation().stage.active()
}

/// The QR view on the whole screen: the title, the open key's code (or why there is none), the
/// hint.
pub(super) fn draw(frame: &mut Frame, app: &App, placed: &mut Vec<Placed>) {
    let area = frame.area();
    let chain = app.chain();
    let mut title = Row::default()
        .text("4 receive / ", TITLE)
        .text(chain.name, TITLE);
    if let Some(key) = app.open_key() {
        title = title
            .text(" / ", TITLE)
            .outside(&Outside::key_name(key), TITLE);
    }
    title.draw(frame, line(area, 0), placed);
    let hint = match app.notice() {
        Some(error) => Row::default()
            .outside(
                &error.cut(usize::from(area.width).saturating_sub(12)),
                ERROR,
            )
            .text("   ? actions", HINT),
        None => Row::default().text(QR_HINT, HINT),
    };
    hint.draw(frame, line(area, area.height - 1), placed);
    let body = Rect {
        y: area.y + 1,
        height: area.height - 2,
        ..area
    };
    let Some(key) = app.open_key() else {
        Row::default()
            .text(
                "Open a key with Enter on 1 keys, then Q shows its QR code.",
                HINT,
            )
            .draw(frame, line(body, 0), placed);
        return;
    };
    match &app.navigation().lines {
        Pane::Ready(_) => {}
        Pane::Loading => {
            Row::default()
                .text("reading...", HINT)
                .draw(frame, line(body, 0), placed);
            return;
        }
        Pane::Failed(error) => {
            error_view(
                frame,
                body,
                "No QR: the address read failed:",
                error,
                placed,
            );
            return;
        }
    }
    let Some((address, trust)) = app.displayed_address(key) else {
        Row::default()
            .text(
                "No QR: this key's address on this chain is not read yet.",
                WARNING,
            )
            .draw(frame, line(body, 0), placed);
        Row::default()
            .text("r reads it again; V verifies it from the seed.", HINT)
            .draw(frame, line(body, 1), placed);
        return;
    };
    // Why there is no code, as the CLI's `with_qr` decides: never for an address nothing verified.
    let mut rows = vec![Row::default().text(
        match &trust {
            TrustLine::Password => {
                return code(frame, body, chain, &address, "verified by password", placed);
            }
            TrustLine::Session => {
                return code(frame, body, chain, &address, "verified by session", placed);
            }
            TrustLine::Locked => {
                "No QR: the wallet is locked; this is the cached address, not verified."
            }
            TrustLine::NotVerified => "No QR: no session of this build verified this address.",
            TrustLine::Impostor(_) => "No QR: another process answered as the key daemon:",
        },
        WARNING,
    )];
    if let TrustLine::Impostor(warning) = &trust {
        rows.push(Row::default().outside(warning, WARNING));
    }
    rows.push(Row::default().text("V verifies it from the seed with the password.", HINT));
    rows.push(Row::default());
    rows.push(
        Row::default()
            .text("not verified: ", WARNING)
            .outside(&address, WARNING),
    );
    for (y, row) in (0..).zip(rows) {
        row.draw(frame, line(body, y), placed);
    }
}

/// `address` (on `chain`) as its QR code at the top left of `body`, under it the CLI's caption and
/// `verified`. When `body` cannot hold all of it, no code at all: how big the terminal must be,
/// and that `zwallet address` prints it. An EVM or Tron address is a version 3 code, 37 columns
/// and 19 lines with its quiet zone: at 80x24 it is whole.
fn code(
    frame: &mut Frame,
    body: Rect,
    chain: &Chain,
    address: &Outside,
    verified: &str,
    placed: &mut Vec<Placed>,
) {
    let networks = qr_networks(chain);
    // No address of a registered family is too long for a code.
    let Some(lines) = crate::qr::lines(address.text(), false) else {
        Row::default()
            .text("No QR: this address does not fit a QR code.", WARNING)
            .draw(frame, line(body, 0), placed);
        return;
    };
    let size = lines.first().map_or(0, |l| l.chars().count());
    let width = size
        .max(SCAN_TO_SEND.len() + address.text().len())
        .max(networks.len());
    let height = lines.len() + 3;
    if width > usize::from(body.width) || height > usize::from(body.height) {
        Row::default()
            .text(
                &format!("No QR: the code needs a {width}x{} terminal.", height + 2),
                WARNING,
            )
            .draw(frame, line(body, 0), placed);
        Row::default()
            .text("`zwallet address` prints it.", HINT)
            .draw(frame, line(body, 1), placed);
        Row::default()
            .text(verified, HINT)
            .text(": ", HINT)
            .outside(address, NORMAL)
            .draw(frame, line(body, 3), placed);
        return;
    }
    for (y, cells) in (0..).zip(&lines) {
        let row = Rect::new(body.x, body.y + y, size as u16, 1);
        frame.render_widget(Paragraph::new(cells.as_str()).style(QR), row);
    }
    let below = lines.len() as u16;
    Row::default()
        .text(SCAN_TO_SEND, NORMAL)
        .outside(address, NORMAL)
        .draw(frame, line(body, below), placed);
    Row::default()
        .text(networks, NORMAL)
        .draw(frame, line(body, below + 1), placed);
    Row::default()
        .text(verified, HINT)
        .draw(frame, line(body, below + 2), placed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// `address` on `chain` drawn by [`code`] into a body of `width` x `height`: its rows, and
    /// how many cells have the QR style.
    fn drawn(chain: &str, address: &str, width: u16, height: u16) -> (Vec<String>, usize) {
        let chain = crate::chains::find(chain).unwrap();
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut placed = Vec::new();
        terminal
            .draw(|f| {
                let area = f.area();
                code(
                    f,
                    area,
                    chain,
                    &Outside::address(address),
                    "verified by password",
                    &mut placed,
                )
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let rows = (0..height)
            .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect())
            .collect();
        let qr = buffer
            .content
            .iter()
            .filter(|c| crate::tui::style::is_qr(c.style()))
            .count();
        (rows, qr)
    }

    #[test]
    fn a_body_that_cannot_hold_the_whole_code_and_caption_says_how_big_and_draws_no_code() {
        const TRON: &str = "TWer2Ygk5TEheHp3TPuYeqxmB6SsGZmaL6";
        const EVM: &str = "0x758a863FA27ffFc71686d4a67B7c45e79D0235FE";
        // The body of an 80x24 screen (title and hint rows aside) holds either whole.
        for (chain, address) in [("tron", TRON), ("ethereum", EVM)] {
            let (_, qr) = drawn(chain, address, 80, 22);
            assert_eq!(qr, 37 * 19, "{chain}");
        }
        // Tron's caption is 53 columns wide, an EVM one 59; the code and caption 22 rows.
        for (chain, address, needs, width, height) in [
            ("tron", TRON, "53x24", 52, 22),
            ("tron", TRON, "53x24", 80, 21),
            ("ethereum", EVM, "59x24", 58, 22),
        ] {
            let (rows, qr) = drawn(chain, address, width, height);
            assert_eq!(qr, 0, "{chain} at {width}x{height}");
            assert!(
                rows[0].contains(&format!("No QR: the code needs a {needs} terminal.")),
                "{rows:#?}"
            );
            assert!(
                rows[1].contains("`zwallet address` prints it."),
                "{rows:#?}"
            );
            assert!(
                rows.iter().all(|r| !r.contains(['█', '▀', '▄'])),
                "{rows:#?}"
            );
        }
    }
}
