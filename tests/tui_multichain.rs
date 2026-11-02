//! Observed QR pixels and native-family UI capability boundaries, using public fixtures only.
use ratatui::{Terminal, backend::TestBackend};
use std::time::Duration;
use zwallet::{
    chains::{self, Net},
    tui::{
        app::{App, Key, Msg},
        style,
        term::FilteredBackend,
        ui,
    },
    wallet::{self, Trust},
};
const EVM: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
const TRON: &str = "TWer2Ygk5TEheHp3TPuYeqxmB6SsGZmaL6";
fn fixture(chain: &str, address: &str, trust: Trust) -> App {
    let chain = chains::find(chain).unwrap();
    let mut app = App::new(
        &Net {
            chain,
            rpc_url: "http://127.0.0.1:9".into(),
        },
        Duration::from_secs(60),
    );
    app.start();
    app.update(Msg::Keys(
        app.generation(),
        Ok(wallet::Keys {
            active: "b".into(),
            trust: Trust::Locked,
            rows: vec![
                wallet::KeyRow {
                    name: "a".into(),
                    index: 0,
                    address: Some(EVM.into()),
                    active: false,
                },
                wallet::KeyRow {
                    name: "b".into(),
                    index: 1,
                    address: Some(EVM.into()),
                    active: true,
                },
            ],
        }),
    ));
    app.update(Msg::Key(Key::Enter));
    app.update(Msg::Key(Key::Char('4')));
    app.update(Msg::Addresses(
        app.generation(),
        "a".into(),
        Ok(wallet::Addresses {
            key: "a".into(),
            trust,
            rows: vec![wallet::AddressRow {
                chain,
                address: address.into(),
                error: None,
            }],
        }),
    ));
    app.update(Msg::Key(Key::Char('Q')));
    app
}
fn render(app: &App, size: (u16, u16)) -> (ratatui::buffer::Buffer, String) {
    let mut term = Terminal::new(FilteredBackend::new(TestBackend::new(size.0, size.1))).unwrap();
    term.draw(|frame| {
        ui::draw(frame, app);
    })
    .unwrap();
    assert_eq!(term.backend().replaced(), 0);
    let buffer = term.backend().inner().buffer().clone();
    let text = (0..size.1)
        .map(|y| {
            (0..size.0)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    (buffer, text)
}
#[test]
fn receive_qr_decodes_exact_identity_beside_full_address_at_both_sizes() {
    for (chain, address) in [
        ("tron", TRON.to_string()),
        ("solana", bs58::encode([7u8; 32]).into_string()),
        (
            "ton",
            "0:2d527ea485fe0cc67001e1a40d7039543cd3ac2ed53c20e5596c869b173c3d29".to_string(),
        ),
    ] {
        for size in [(80, 24), (120, 40)] {
            let app = fixture(chain, &address, Trust::Session);
            let (buffer, text) = render(&app, size);
            assert!(text.contains("Verified by session"), "{text}");
            assert!(text.lines().next().unwrap().contains(" / a"), "{text}");
            let cells: Vec<_> = (0..size.1)
                .flat_map(|y| (0..size.0).map(move |x| (x, y)))
                .filter(|&(x, y)| style::is_qr(buffer[(x, y)].style()))
                .collect();
            assert!(!cells.is_empty(), "{text}");
            let x0 = cells.iter().map(|p| p.0).min().unwrap();
            let x1 = cells.iter().map(|p| p.0).max().unwrap();
            let y0 = cells.iter().map(|p| p.1).min().unwrap();
            let y1 = cells.iter().map(|p| p.1).max().unwrap();
            let (width, height) = (usize::from(x1 - x0 + 1), usize::from(y1 - y0 + 1) * 2);
            let mut pixels = vec![vec![false; width]; height];
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let (top, bottom) = match buffer[(x, y)].symbol() {
                        "█" => (true, true),
                        "▀" => (true, false),
                        "▄" => (false, true),
                        " " => (false, false),
                        other => panic!("unexpected QR cell {other}"),
                    };
                    pixels[usize::from(y - y0) * 2][usize::from(x - x0)] = top;
                    pixels[usize::from(y - y0) * 2 + 1][usize::from(x - x0)] = bottom;
                }
            }
            // Check all four quiet-zone modules independently of the decoder.
            assert!(
                pixels
                    .iter()
                    .all(|row| row[..4].iter().all(|v| !v) && row[width - 4..].iter().all(|v| !v))
            );
            assert!(pixels[..4].iter().flatten().all(|v| !v));
            assert!(pixels[height - 4..].iter().flatten().all(|v| !v));
            let mut image =
                rqrr::PreparedImage::prepare_from_greyscale(width * 4, height * 4, |x, y| {
                    if pixels[y / 4][x / 4] { 0 } else { 255 }
                });
            let grids = image.detect_grids();
            assert_eq!(grids.len(), 1, "{text}");
            assert_eq!(grids[0].decode().unwrap().1, address);
            let side_x = x1 + 3;
            let side_text = (y0..=y1)
                .map(|y| {
                    (side_x..size.0)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect::<String>()
                        .trim_end()
                        .to_string()
                })
                .collect::<Vec<_>>();
            let start = side_text.iter().position(|s| s == "Address").unwrap() + 1;
            let mut shown = String::new();
            for line in side_text.iter().skip(start) {
                if line.is_empty() {
                    break;
                }
                shown.push_str(line);
            }
            assert_eq!(shown, address, "{text}");
            if chain == "tron" {
                assert!(
                    side_text.iter().any(|line| line == TRON),
                    "Tron address must fit one line: {text}"
                );
            }
            let proof = format!(
                "chain={chain}; viewport={}x{}; QR=({x0},{y0})..({x1},{y1}); address column={side_x}; address first row={}\n{text}\n",
                size.0,
                size.1,
                y0 + start as u16
            );
            std::fs::write(
                format!("/tmp/zwallet-receive-{chain}-{}x{}.txt", size.0, size.1),
                proof,
            )
            .unwrap();
            std::fs::write(format!("/tmp/zwallet-receive-{chain}-{}x{}.json",size.0,size.1), serde_json::to_vec_pretty(&serde_json::json!({"chain":chain,"width":size.0,"height":size.1,"rows":text.split('\n').collect::<Vec<_>>(),"qr":{"x":x0,"y":y0,"width":x1-x0+1,"height":y1-y0+1},"address":{"x":side_x,"y":y0+start as u16,"text":address}})).unwrap()).unwrap();
        }
    }
}
#[test]
fn receive_never_encodes_unverified_or_impostor_address() {
    for trust in [
        Trust::Locked,
        Trust::Unverified(None),
        Trust::Unverified(Some("fixture impostor".into())),
    ] {
        let app = fixture("tron", TRON, trust);
        let (buffer, text) = render(&app, (80, 24));
        assert!(text.contains("QR hidden"), "{text}");
        assert!(
            buffer
                .content()
                .iter()
                .all(|cell| !style::is_qr(cell.style()))
        );
    }
}
