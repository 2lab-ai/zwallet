//! A verified receive identity, encoded locally, with its exact address beside the QR.
use super::*;
use qrcode::{Color, EcLevel, QrCode};

/// This is the only renderer that can introduce QR half-block glyphs. Outside text still
/// goes through `Outside`, and only a password/session-verified family address is encoded.
pub(super) fn draw(frame: &mut Frame, app: &App, placed: &mut Vec<Placed>) {
    let area = frame.area();
    let key = app.open_key().unwrap_or("none");
    Row::default()
        .text("Receive / ", TITLE)
        .outside(&Outside::detail(app.chain().name), TITLE)
        .text(" / ", TITLE)
        .outside(&Outside::key_name(key), TITLE)
        .draw(frame, line(area, 0), placed);
    Row::default()
        .text(
            "Q addresses  V verify  c chain  r refresh  q quit  ? actions",
            HINT,
        )
        .draw(frame, line(area, area.height - 1), placed);
    let content = Rect::new(area.x, area.y + 1, area.width, area.height - 2);
    if !matches!(app.navigation().lines, Pane::Ready(_)) {
        match &app.navigation().lines {
            Pane::Failed(error) => error_view(
                frame,
                content,
                "Receive address unavailable:",
                error,
                placed,
            ),
            _ => Row::default()
                .text("Reading the selected key's address...", HINT)
                .draw(frame, line(content, 0), placed),
        }
        return;
    }
    let Some((address, trust)) = app.displayed_address(key) else {
        Row::default()
            .text(
                "Open a key and verify its address with V before showing a QR.",
                WARNING,
            )
            .draw(frame, line(content, 0), placed);
        return;
    };
    if !matches!(trust, TrustLine::Password | TrustLine::Session) {
        Row::default()
            .text(
                "QR hidden: V verifies this address from the seed before funding.",
                WARNING,
            )
            .draw(frame, line(content, 0), placed);
        Row::default()
            .outside(&address, WARNING)
            .draw(frame, line(content, 2), placed);
        return;
    }
    let Ok(code) = QrCode::with_error_correction_level(address.text(), EcLevel::L) else {
        return;
    };
    let width = code.width();
    let quiet = 4;
    let size = width + 2 * quiet;
    let rows = size.div_ceil(2);
    if rows > usize::from(content.height) || size + 12 > usize::from(content.width) {
        Row::default()
            .text(
                "Enlarge this terminal to show the complete QR quiet zone.",
                WARNING,
            )
            .draw(frame, line(content, 0), placed);
        Row::default()
            .outside(&address, NORMAL)
            .draw(frame, line(content, 2), placed);
        return;
    }
    let modules = code.to_colors();
    let dark = |x: usize, y: usize| {
        (quiet..quiet + width).contains(&x)
            && (quiet..quiet + width).contains(&y)
            && modules[(y - quiet) * width + x - quiet] == Color::Dark
    };
    let qr = Rect::new(content.x, content.y, size as u16, rows as u16);
    for y in (0..size).step_by(2) {
        let symbols: String = (0..size)
            .map(|x| match (dark(x, y), dark(x, y + 1)) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            })
            .collect();
        frame.render_widget(
            Paragraph::new(symbols).style(crate::tui::style::QR),
            line(qr, (y / 2) as u16),
        );
    }
    let side = Rect::new(
        content.x + size as u16 + 2,
        content.y,
        content.width - size as u16 - 2,
        content.height,
    );
    Row::default()
        .text("Address", TITLE)
        .draw(frame, line(side, 2), placed);
    let chunks = address.text().as_bytes().chunks(usize::from(side.width));
    let count = chunks.len() as u16;
    for (i, chunk) in chunks.enumerate() {
        Row::default()
            .outside(
                &Outside::address(std::str::from_utf8(chunk).expect("validated ASCII address")),
                NORMAL,
            )
            .draw(frame, line(side, 3 + i as u16), placed);
    }
    Row::default()
        .text(
            if trust == TrustLine::Password {
                "Verified by password"
            } else {
                "Verified by session"
            },
            HINT,
        )
        .draw(frame, line(side, 4 + count), placed);
    Row::default()
        .text("Check chain and address", HINT)
        .draw(frame, line(side, 6 + count), placed);
    Row::default()
        .text("before sending funds.", HINT)
        .draw(frame, line(side, 7 + count), placed);
}
