//! A QR code as terminal text, so a phone wallet can scan an address off the screen.

use qrcode::{Color, EcLevel, QrCode};

/// Light modules around the code (its quiet zone), as many as `qrencode -t UTF8` draws.
const QUIET: usize = 2;

/// Black on white (`30;107`): the code reads the same on a dark and on a light terminal.
const BLACK_ON_WHITE: &str = "\x1b[30;107m";
const RESET: &str = "\x1b[0m";

/// `text` as a QR code at error correction level M, in Unicode half blocks (two rows of modules
/// per line) with a quiet zone of [`QUIET`] modules. With `color`, each line paints black modules
/// on a white background. Without (`NO_COLOR`), plain characters for a dark terminal: the blocks
/// draw the light modules, as `qrencode -t UTF8` does. `None` when `text` does not fit.
pub fn render(text: &str, color: bool) -> Option<String> {
    let code = QrCode::with_error_correction_level(text, EcLevel::M).ok()?;
    let width = code.width();
    let modules = code.to_colors();
    let size = width + 2 * QUIET;
    // Light in the quiet zone, and below the last row when the size is odd.
    let dark = |x: usize, y: usize| {
        (QUIET..QUIET + width).contains(&x)
            && (QUIET..QUIET + width).contains(&y)
            && modules[(y - QUIET) * width + x - QUIET] == Color::Dark
    };
    let lines: Vec<String> = (0..size)
        .step_by(2)
        .map(|y| {
            let cells: String = (0..size)
                .map(|x| {
                    // The blocks draw the dark modules in colour, the light ones without.
                    match (dark(x, y) == color, dark(x, y + 1) == color) {
                        (true, true) => '█',
                        (true, false) => '▀',
                        (false, true) => '▄',
                        (false, false) => ' ',
                    }
                })
                .collect();
            if color {
                format!("{BLACK_ON_WHITE}{cells}{RESET}")
            } else {
                cells
            }
        })
        .collect();
    Some(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An EIP-55 address (mixed case, so the code holds it byte for byte).
    const ADDRESS: &str = "0x758a863FA27ffFc71686d4a67B7c45e79D0235FE";

    /// The modules `rendered` shows, `true` for dark, read back from its characters: a block is
    /// a dark module in colour, a light one without.
    fn modules(rendered: &str, color: bool) -> Vec<Vec<bool>> {
        let mut rows = Vec::new();
        for line in rendered.lines() {
            let cells = if color {
                line.strip_prefix(BLACK_ON_WHITE)
                    .and_then(|l| l.strip_suffix(RESET))
                    .unwrap_or_else(|| panic!("{line:?}"))
            } else {
                line
            };
            let (mut top, mut bottom) = (Vec::new(), Vec::new());
            for c in cells.chars() {
                let (t, b) = match c {
                    '█' => (true, true),
                    '▀' => (true, false),
                    '▄' => (false, true),
                    ' ' => (false, false),
                    other => panic!("{other:?} in {line:?}"),
                };
                top.push(t == color);
                bottom.push(b == color);
            }
            rows.push(top);
            rows.push(bottom);
        }
        rows
    }

    /// Decodes `rows` from a picture of them (4 px per module) on a background of `surround`
    /// (`true` for dark), where the code needs its own quiet zone.
    fn scan(rows: &[Vec<bool>], surround: bool) -> String {
        let (scale, pad) = (4, 8);
        let width = rows[0].len() + 2 * pad;
        let height = rows.len() + 2 * pad;
        let mut img =
            rqrr::PreparedImage::prepare_from_greyscale(width * scale, height * scale, |x, y| {
                let (x, y) = (x / scale, y / scale);
                let dark = if x < pad || y < pad {
                    surround
                } else {
                    rows.get(y - pad)
                        .and_then(|r| r.get(x - pad))
                        .copied()
                        .unwrap_or(surround)
                };
                if dark { 0 } else { 255 }
            });
        let grids = img.detect_grids();
        assert_eq!(grids.len(), 1);
        let (meta, text) = grids[0].decode().unwrap();
        assert_eq!(meta.ecc_level, 0, "level M");
        text
    }

    #[test]
    fn coloured_code_scans_to_the_address_on_any_background() {
        let rendered = render(ADDRESS, true).unwrap();
        let rows = modules(&rendered, true);
        // Version 3 (29 modules) plus a quiet zone of 4 modules on each side (ISO/IEC 18004),
        // in 19 lines.
        assert_eq!(rows[0].len(), 29 + 2 * 4);
        assert_eq!(rendered.lines().count(), 19);
        let light = |r: &Vec<bool>| r.iter().all(|&d| !d);
        assert!(rows[..4].iter().all(light) && rows[33..].iter().all(light));
        assert!(
            rows.iter()
                .all(|r| !r[..4].contains(&true) && !r[33..].contains(&true))
        );
        for surround in [true, false] {
            assert_eq!(scan(&rows, surround), ADDRESS, "surround dark {surround}");
        }
    }

    #[test]
    fn no_color_code_scans_to_the_address_on_a_dark_terminal() {
        let rendered = render(ADDRESS, false).unwrap();
        assert!(!rendered.contains('\x1b'), "{rendered}");
        let rows = modules(&rendered, false);
        assert_eq!(scan(&rows, true), ADDRESS);
        // The same modules as the coloured code.
        assert_eq!(rows, modules(&render(ADDRESS, true).unwrap(), true));
    }

    #[test]
    fn text_too_long_for_a_code_is_none() {
        assert_eq!(render(&"x".repeat(3000), true), None);
    }
}
