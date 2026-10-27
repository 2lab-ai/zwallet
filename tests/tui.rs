//! `zwallet tui` (`.prd/03-tui.md`): its screens drawn on ratatui's `TestBackend` through
//! `FilteredBackend`, the one write point, and its state machine driven by `Msg`s. Every frame a
//! test draws goes through [`checked_frame`], which asserts the three display-integrity rules
//! every later unit's frames keep too (03-tui.md §Display integrity rules 1-3).

mod common;

use common::Home;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::Rect;
use serde_json::Value;
use std::time::{Duration, SystemTime};
use zwallet::chains::{self, Net};
use zwallet::core::error::{ErrorCode, WalletError};
use zwallet::core::terminal::terminal_ok;
use zwallet::core::time::{parse_rfc3339_utc, rfc3339_utc, unix_secs};
use zwallet::tui::app::{App, Effect, Key, Msg};
use zwallet::tui::style;
use zwallet::tui::term::FilteredBackend;
use zwallet::tui::text::{Outside, Source};
use zwallet::tui::ui::{self, Placed};
use zwallet::wallet::{self, KeyRow, Keys, Serving, StatusView, Trust};

type Screen = Terminal<FilteredBackend<TestBackend>>;

/// A `width` x `height` terminal the TUI can only write to through the write point.
fn screen(width: u16, height: u16) -> Screen {
    Terminal::new(FilteredBackend::new(TestBackend::new(width, height))).unwrap()
}

fn row(buffer: &Buffer, y: u16) -> String {
    (0..buffer.area.width)
        .map(|x| buffer[(x, y)].symbol())
        .collect()
}

/// Draws `app` on `screen` and asserts what every frame holds, then returns its rows and the
/// outside values it drew:
/// 1. no cell was replaced by the write point, and every cell is exactly one `terminal_ok`
///    character, so no string from outside reached a cell raw;
/// 2. every cell's style is one of `style::ALL`;
/// 3. every outside value is printable ASCII within its source's budget, and is drawn where the
///    UI says, on one line (an error view: one block of at most 7 rows), and nowhere else.
fn checked_frame(screen: &mut Screen, app: &App) -> (Vec<String>, Vec<Placed>) {
    let mut placed = Vec::new();
    screen.draw(|f| placed = ui::draw(f, app)).unwrap();
    assert_eq!(
        screen.backend().replaced(),
        0,
        "rule 1: a cell was replaced"
    );
    let buffer = screen.backend().inner().buffer();
    let styles: Vec<_> = style::ALL
        .iter()
        .map(|s| {
            let mut cell = Cell::default();
            cell.set_style(*s);
            cell.style()
        })
        .collect();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            let cell = &buffer[(x, y)];
            let mut chars = cell.symbol().chars();
            assert!(
                matches!((chars.next(), chars.next()), (Some(c), None) if c != '\n' && terminal_ok(c)),
                "rule 1: cell ({x},{y}) holds {:?}",
                cell.symbol()
            );
            assert!(
                styles.contains(&cell.style()),
                "rule 2: cell ({x},{y}) has style {:?}",
                cell.style()
            );
        }
    }
    let rows: Vec<String> = (0..buffer.area.height).map(|y| row(buffer, y)).collect();
    for p in &placed {
        assert_placed(&rows, p, &placed);
    }
    (rows, placed)
}

/// Each source's budget (03-tui.md rule 3), written here rather than read from the code, so a
/// budget raised in the code fails the tests.
fn budget(source: Source) -> usize {
    match source {
        Source::KeyName | Source::RpcHost => 32,
        Source::Address => 42,
        Source::Expiry => 64,
        Source::Error | Source::Warning => 512,
    }
}

/// Rule 3 for one outside value `p` of a frame's `placed` values: inert, within its budget, and
/// drawn whole where it was placed (a value one line long has its whole width there, never cut
/// by the edge; a value that has to be shorter is cut with `...` before it is placed).
fn assert_placed(rows: &[String], p: &Placed, placed: &[Placed]) {
    let (source, text) = (p.outside.source(), p.outside.text());
    assert!(
        text.bytes().all(|b| (0x20..=0x7e).contains(&b)),
        "{source:?} is not inert: {text:?}"
    );
    assert!(
        text.len() <= budget(source),
        "{source:?} is over its budget of {}: {} characters",
        budget(source),
        text.len()
    );
    let area = p.area;
    assert!(
        area.height == 1 || (source == Source::Error && area.height <= 7),
        "{source:?} spans {} rows",
        area.height
    );
    if area.height == 1 {
        assert_eq!(
            usize::from(area.width),
            text.len(),
            "{source:?} {text:?} is drawn cut, or not at all, at {area:?}"
        );
    }
    // The area shows the start of the text, then nothing.
    let shown: Vec<char> = (area.y..area.bottom())
        .flat_map(|y| {
            rows[y as usize]
                .chars()
                .skip(area.x as usize)
                .take(area.width as usize)
                .collect::<Vec<_>>()
        })
        .collect();
    let n = text.len().min(shown.len());
    let start: String = shown[..n].iter().collect();
    assert_eq!(start, text[..n], "{source:?} at {area:?}");
    assert!(
        shown[n..].iter().all(|c| *c == ' '),
        "{source:?} at {area:?}: {shown:?}"
    );
    // And no other place shows a copy of it, whole or cut (with `...`, at the edge, or before
    // blanks), but a placement of the same value: one of the two shown texts a `...`-cut of the
    // other, compared whole (two values that only share a prefix are not the same value).
    if text.len() >= 12 {
        let core: Vec<char> = text.trim_end_matches("...").chars().collect();
        for (y, row) in rows.iter().enumerate() {
            let row: Vec<char> = row.chars().collect();
            for x in 0..row.len() {
                let tail = &row[x..];
                let common = tail.iter().zip(&core).take_while(|(a, b)| a == b).count();
                if common < 12 {
                    continue;
                }
                let after = &tail[common..];
                let copy = common == core.len()
                    || after.is_empty()
                    || after.starts_with(&['.', '.', '.'])
                    || after.iter().all(|c| *c == ' ');
                if !copy {
                    continue;
                }
                let (cx, cy) = (x as u16, y as u16);
                let placed_there = placed.iter().any(|q| {
                    (q.area.x..q.area.right()).contains(&cx)
                        && (q.area.y..q.area.bottom()).contains(&cy)
                        && same_value(q.outside.text(), text)
                });
                assert!(
                    placed_there,
                    "{source:?} {text:?} also on row {y} at column {x}: {:?}",
                    rows[y]
                );
            }
        }
    }
}

/// Whether `a` and `b` show one value: one of them a `...`-cut of the other (or both whole).
fn same_value(a: &str, b: &str) -> bool {
    a.starts_with(b.trim_end_matches("...")) || b.starts_with(a.trim_end_matches("..."))
}

fn block_on<F: std::future::Future>(work: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(work)
}

fn now() -> u64 {
    unix_secs(SystemTime::now())
}

const A: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
const B: &str = "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC";

fn net() -> Net {
    Net {
        chain: chains::find("ethereum").unwrap(),
        rpc_url: "https://ethereum-rpc.publicnode.com".into(),
    }
}

fn app() -> App {
    App::new(&net(), Duration::from_secs(3600))
}

/// Keys `a` and `b` at [`A`] and [`B`], `b` active.
fn keys_ab() -> Keys {
    let row = |name: &str, index, address: &str| KeyRow {
        name: name.into(),
        index,
        address: Some(address.into()),
        active: name == "b",
    };
    Keys {
        active: "b".into(),
        rows: vec![row("a", 0, A), row("b", 1, B)],
        warning: None,
    }
}

fn locked_status() -> StatusView {
    StatusView {
        configured: true,
        locked: true,
        daemon_pids: vec![],
        other_daemon_pids: vec![],
        serving: None,
        active: Some("b".into()),
        address: Some((Some(B.into()), Trust::Locked)),
    }
}

/// Unlocked by a daemon of this build until `expires`.
fn open_status(expires: u64) -> StatusView {
    StatusView {
        locked: false,
        daemon_pids: vec![4242],
        serving: Some(Serving {
            pid: 4242,
            expires_at: Some(rfc3339_utc(expires)),
            whitelist: Value::Null,
        }),
        address: Some((Some(B.into()), Trust::Session)),
        ..locked_status()
    }
}

fn hhmm(secs: u64) -> String {
    format!("{}Z", &rfc3339_utc(secs)[11..16])
}

#[test]
fn t1_both_keys_listed_the_active_one_marked_and_locked_once_in_the_bar() {
    let home = Home::new();
    let a = home.create("a", &[])["address"]
        .as_str()
        .unwrap()
        .to_string();
    // The new key becomes active.
    let b = home.create("b", &[])["address"]
        .as_str()
        .unwrap()
        .to_string();
    let mut app = app();
    assert_eq!(
        app.start(),
        vec![Effect::LoadKeys(0), Effect::LoadStatus(0)]
    );
    app.update(Msg::Keys(
        app.generation(),
        block_on(wallet::keys(home.path())),
    ));
    app.update(Msg::Status(
        app.generation(),
        block_on(wallet::status(home.path())),
    ));
    let (rows, _) = checked_frame(&mut screen(80, 24), &app);
    let a_row = rows.iter().find(|r| r.contains(&a)).expect("a's row");
    let b_row = rows.iter().find(|r| r.contains(&b)).expect("b's row");
    assert!(b_row.contains("* b "), "{b_row:?}");
    assert!(!a_row.contains('*') && a_row.contains(" a "), "{a_row:?}");
    // The lock state once, in the bar (the last two rows), nowhere else.
    let all = rows.join("\n");
    assert_eq!(all.matches("locked").count(), 1, "{all}");
    assert!(rows[23].starts_with("locked"), "{all}");
    // A row address is never marked verified.
    assert!(!a_row.contains("verified") && !b_row.contains("verified"));
}

#[test]
fn t9_the_list_and_the_two_bar_lines_fit_80x24() {
    let expires = now() + 3600;
    let mut app = app();
    app.update(Msg::Keys(app.generation(), Ok(keys_ab())));
    app.update(Msg::Status(app.generation(), Ok(open_status(expires))));
    let (rows, _) = checked_frame(&mut screen(80, 24), &app);
    assert!(
        rows.iter()
            .any(|r| r.contains(&format!(" a {}", " ".repeat(15))))
    );
    assert!(rows.iter().any(|r| r.contains(A)));
    assert!(rows.iter().any(|r| r.contains(B)));
    assert_eq!(
        rows[22].trim_end(),
        "ethereum | b 0x3C44...93BC | verified by session"
    );
    assert_eq!(
        rows[23].trim_end(),
        format!(
            "signing (agent too) until {} | ethereum-rpc.publicnode.com",
            hhmm(expires)
        )
    );
}

#[test]
fn at_80x24_the_widest_values_are_drawn_whole() {
    // A session two days long (its date in front), a 32-character host and a 32-character key
    // name: the bar's two lines and the key rows keep every value whole, or cut it with `...`.
    let expires = now() + 2 * 86_400;
    let long_name = "k".repeat(32);
    let host = "eth-mainnet-rpc.provider-name.io".to_string();
    assert_eq!(host.len(), 32);
    let net = Net {
        chain: chains::find("avalanche").unwrap(),
        rpc_url: format!("https://{host}/v2/KEY"),
    };
    let mut app = App::new(&net, Duration::from_secs(3600));
    let mut keys = keys_ab();
    keys.rows[1].name = long_name.clone();
    keys.active = long_name.clone();
    app.update(Msg::Keys(app.generation(), Ok(keys)));
    let mut status = open_status(expires);
    status.active = Some(long_name.clone());
    status.address = Some((Some(B.into()), Trust::Unverified(None)));
    app.update(Msg::Status(app.generation(), Ok(status)));
    let (rows, _) = checked_frame(&mut screen(80, 24), &app);
    let date = rfc3339_utc(expires);
    let date = format!("{} {}Z", &date[..10], &date[11..16]);
    assert!(rows[23].contains(&date), "{:?}", rows[23]);
    assert!(rows[23].trim_end().ends_with(&host), "{:?}", rows[23]);
    // Both key rows show the whole address; the long name is cut with `...`.
    let b_row = rows.iter().find(|r| r.contains(B)).expect("b's row");
    assert!(
        b_row.contains(&format!("{}...", &long_name[..24])),
        "{b_row:?}"
    );
    assert!(rows.iter().any(|r| r.contains(A)));
    // Line 1 keeps the trust words whole, and the name whole where the platform's trust words
    // leave room for it (the Linux note does not).
    assert!(rows[22].trim_end().ends_with("not verified") || rows[22].contains("Linux)"));
    let name_shown = if cfg!(target_os = "linux") {
        format!("{}...", &long_name[..8])
    } else {
        long_name.clone()
    };
    assert!(rows[22].contains(&name_shown), "{:?}", rows[22]);
}

#[test]
fn an_index_of_1000_and_a_32_character_name_never_cut_the_address() {
    let mut keys = keys_ab();
    keys.rows[1].index = 1000;
    keys.rows[1].name = "n".repeat(32);
    let mut app = app();
    app.update(Msg::Keys(app.generation(), Ok(keys)));
    let (rows, _) = checked_frame(&mut screen(80, 24), &app);
    let b_row = rows.iter().find(|r| r.contains(" 1000 ")).expect("b's row");
    assert!(b_row.contains(B), "{b_row:?}");
    assert!(b_row.contains("..."), "the name is cut: {b_row:?}");
}

/// The shortest a key name is cut to on bar line 1, `...` included; written here, not read from
/// the code.
const BAR_NAME_MIN: usize = 8;

/// `shown` is `whole`, or a cut of it that ends in `...` (and is at least `min` long).
fn whole_or_cut(shown: &str, whole: &str, min: usize) -> bool {
    shown == whole
        || (shown.len() >= min
            && shown.ends_with("...")
            && whole.starts_with(shown.trim_end_matches("...")))
}

#[test]
fn line_1_fits_80_columns_for_every_chain_and_every_trust_with_a_32_character_key_name() {
    let name = "n".repeat(32);
    // The longest warning `PeerCheck::warning` writes: its words, a pid, and the executable path
    // (an outside value) it quotes; and one past every budget.
    let warning = format!(
        "the process answering on the key daemon socket (pid 4294967295) runs other code than \
         this zwallet build ({}); run `zwallet lock` and unlock again",
        "/p".repeat(32)
    );
    let flood = "w".repeat(600);
    let not_verified = if cfg!(target_os = "linux") {
        "not verified (no session check on Linux)"
    } else {
        "not verified"
    };
    let impostor = "NOT this build's key daemon: ";
    for chain in zwallet::chains::CHAINS {
        for (trust, words, w) in [
            (Trust::Session, "verified by session", None),
            (Trust::Unverified(None), not_verified, None),
            (Trust::Locked, "not verified", None),
            (
                Trust::Unverified(Some(warning.clone())),
                impostor,
                Some(warning.as_str()),
            ),
            (
                Trust::Unverified(Some(flood.clone())),
                impostor,
                Some(flood.as_str()),
            ),
        ] {
            let net = Net {
                chain,
                rpc_url: chain.default_rpc().to_string(),
            };
            let mut app = App::new(&net, Duration::from_secs(3600));
            app.update(Msg::Keys(app.generation(), Ok(keys_ab())));
            let mut status = open_status(now() + 3600);
            status.active = Some(name.clone());
            status.address = Some((Some(B.into()), trust));
            app.update(Msg::Status(app.generation(), Ok(status)));
            let (rows, _) = checked_frame(&mut screen(80, 24), &app);
            let line = rows[22].trim_end();
            let case = format!("{} {words:?}", chain.key);
            // chain | name 0x3C44...93BC | trust words [warning]
            let rest = line
                .strip_prefix(&format!("{} | ", chain.key))
                .unwrap_or_else(|| panic!("{case}: {line:?}"));
            let (shown_name, rest) = rest
                .split_once(" 0x3C44...93BC | ")
                .unwrap_or_else(|| panic!("{case}: {line:?}"));
            assert!(
                whole_or_cut(shown_name, &name, BAR_NAME_MIN),
                "{case}: the name {shown_name:?} in {line:?}"
            );
            match w {
                // At 80 columns no state's words are cut.
                None => assert_eq!(rest, words, "{case}: in {line:?}"),
                Some(w) => {
                    // The words whole, then the warning whole or cut with `...`, never left out.
                    let shown = rest
                        .strip_prefix(impostor)
                        .unwrap_or_else(|| panic!("{case}: the words cut in {line:?}"));
                    let whole = Outside::warning(w).text().to_string();
                    assert!(
                        whole_or_cut(shown, &whole, 4),
                        "{case}: the warning {shown:?} in {line:?}"
                    );
                }
            }
        }
    }
}

#[test]
#[should_panic(expected = "also on row 1 at column 20")]
fn assert_placed_catches_a_stray_copy_of_a_name_that_shares_another_names_prefix() {
    // Two key names with the same first 12 characters, each placed on its own row; a stray copy
    // of the first one on the second one's row must not pass for the second one.
    let (a, b) = ("treasury-main-01", "treasury-main-02");
    let rows = vec![format!("{a:<40}"), format!("{b:<20}{a:<20}")];
    let at = |text: &str, y: u16| Placed {
        outside: Outside::key_name(text),
        area: Rect::new(0, y, text.len() as u16, 1),
    };
    let placed = vec![at(a, 0), at(b, 1)];
    for p in &placed {
        assert_placed(&rows, p, &placed);
    }
}

#[test]
#[should_panic(expected = "also on row 2 at column 1")]
fn assert_placed_catches_a_stray_copy_that_a_border_cuts_short() {
    // A warning placed on row 0, and drawn raw again in a bordered pane on row 2, where the
    // border cuts it after 30 characters: fewer than the placed value, followed by `|`.
    let text = "the process answering on the key daemon socket (pid 4242) is no";
    let warning = Outside::warning(text);
    let rows = vec![
        format!("{text:<80}"),
        " ".repeat(80),
        format!("|{}|{}", &text[..30], " ".repeat(48)),
    ];
    let placed = vec![Placed {
        area: Rect::new(0, 0, warning.text().len() as u16, 1),
        outside: warning,
    }];
    assert_placed(&rows, &placed[0], &placed);
}

#[test]
#[should_panic(expected = "also on row 1 at column 0")]
fn assert_placed_catches_a_verbatim_copy_of_a_short_cut_value() {
    // A key name cut to `abcdefghi...` (12 characters, a core of 9), placed on row 0, and copied
    // verbatim on row 1 where nothing is placed.
    let name = Outside::key_name("abcdefghijklmnop").cut(12);
    assert_eq!(name.text(), "abcdefghi...");
    let rows = vec![
        format!("{:<40}", name.text()),
        format!("{:<40}", name.text()),
    ];
    let placed = vec![Placed {
        area: Rect::new(0, 0, 12, 1),
        outside: name,
    }];
    assert_placed(&rows, &placed[0], &placed);
}

#[test]
fn a_hostile_status_reply_is_drawn_inert_and_bounded() {
    // What a process answering on d.sock wrote, as it would reach the TUI if nothing escaped it
    // on the way: an expiry of 600 characters of ESC and half blocks, and a warning holding a
    // terminal escape and a line break.
    let expiry = "\u{1b}[2J\u{2580}\u{2584}".repeat(150);
    let warning = format!("pid 7 \u{1b}]0;x\u{7}\n{}", "\u{2588}".repeat(600));
    let mut status = open_status(now() + 3600);
    status.serving.as_mut().unwrap().expires_at = Some(expiry);
    status.address = Some((Some(B.into()), Trust::Unverified(Some(warning))));
    let mut app = app();
    app.update(Msg::Keys(app.generation(), Ok(keys_ab())));
    app.update(Msg::Status(app.generation(), Ok(status)));
    let (rows, placed) = checked_frame(&mut screen(80, 24), &app);
    assert!(
        rows[22].contains("NOT this build's key daemon: "),
        "{:?}",
        rows[22]
    );
    assert!(rows[23].contains("\\u{1b}"), "{:?}", rows[23]);
    assert!(placed.iter().any(|p| p.outside.source() == Source::Expiry));
    assert!(placed.iter().any(|p| p.outside.source() == Source::Warning));
}

#[test]
fn t9_below_80x24_the_frame_is_one_line() {
    for (width, height) in [(79, 24), (80, 23), (40, 10)] {
        let mut app = app();
        app.update(Msg::Keys(app.generation(), Ok(keys_ab())));
        app.update(Msg::Status(app.generation(), Ok(locked_status())));
        let (rows, _) = checked_frame(&mut screen(width, height), &app);
        assert_eq!(
            rows[0].trim_end(),
            "terminal too small (80x24 needed)",
            "{width}x{height}"
        );
        assert!(
            rows[1..].iter().all(|r| r.trim().is_empty()),
            "{width}x{height}: {rows:#?}"
        );
    }
}

#[test]
fn t9_q_ends_the_app_and_l_asks_to_lock_every_daemon() {
    for key in [Key::Char('q'), Key::Interrupt] {
        let mut app = app();
        assert!(!app.quitting());
        app.update(Msg::Key(key));
        assert!(app.quitting());
    }
    // `L` asks for `protocol::stop_all_daemons`; this test does not run it.
    let mut app = app();
    assert_eq!(app.update(Msg::Key(Key::Char('L'))), vec![Effect::LockAll]);
    assert!(!app.quitting());
}

#[test]
fn t10_a_status_refresh_that_reports_locked_makes_the_bar_say_locked() {
    assert_eq!(zwallet::tui::REFRESH, Duration::from_secs(5));
    let mut app = app();
    app.update(Msg::Keys(app.generation(), Ok(keys_ab())));
    app.update(Msg::Status(app.generation(), Ok(open_status(now() + 3600))));
    let mut screen = screen(80, 24);
    let (rows, _) = checked_frame(&mut screen, &app);
    assert!(rows[23].starts_with("signing (agent too) until "));
    assert!(!rows.join("\n").contains("locked"));
    // The refresh timer asks for the status once; a second tick while it runs asks nothing.
    assert_eq!(app.update(Msg::Tick), vec![Effect::LoadStatus(0)]);
    assert_eq!(app.update(Msg::Tick), vec![]);
    app.update(Msg::Status(app.generation(), Ok(locked_status())));
    let (rows, _) = checked_frame(&mut screen, &app);
    assert!(rows[23].starts_with("locked | "), "{:?}", rows[23]);
    assert!(!rows.join("\n").contains("signing"));
    assert_eq!(app.update(Msg::Tick), vec![Effect::LoadStatus(0)]);
}

#[test]
fn q_while_l_is_locking_quits_once_the_lock_is_done_and_its_error_comes_out() {
    let ready = |app: &mut App| {
        app.update(Msg::Keys(app.generation(), Ok(keys_ab())));
        app.update(Msg::Status(app.generation(), Ok(open_status(now() + 3600))));
    };
    let mut app = app();
    ready(&mut app);
    assert_eq!(app.update(Msg::Key(Key::Char('L'))), vec![Effect::LockAll]);
    app.update(Msg::Key(Key::Char('q')));
    assert!(!app.quitting(), "quit while every daemon is being stopped");
    let (rows, _) = checked_frame(&mut screen(80, 24), &app);
    assert!(
        rows[23].starts_with("locking... (quits when done)"),
        "{:?}",
        rows[23]
    );
    app.update(Msg::Locked(Ok(vec![4242])));
    assert!(app.quitting());
    assert!(app.take_quit_error().is_none());
    // A lock that failed: the TUI still quits, and the error comes out for the CLI to print.
    let mut app = self::app();
    ready(&mut app);
    app.update(Msg::Key(Key::Char('L')));
    app.update(Msg::Key(Key::Interrupt));
    assert!(!app.quitting());
    let failed = WalletError::validation("could not stop zwallet daemon(s) 4242");
    app.update(Msg::Locked(Err(failed.clone())));
    assert!(app.quitting());
    assert_eq!(app.take_quit_error(), Some(failed));
}

#[test]
fn a_status_read_from_before_the_lock_is_dropped_and_a_fresh_one_follows() {
    let mut app = app();
    app.update(Msg::Keys(app.generation(), Ok(keys_ab())));
    let before = app.generation();
    assert_eq!(app.update(Msg::Tick), vec![Effect::LoadStatus(before)]);
    app.update(Msg::Status(before, Ok(open_status(now() + 3600))));
    // A read in flight when `L` is pressed.
    assert_eq!(app.update(Msg::Tick), vec![Effect::LoadStatus(before)]);
    assert_eq!(app.update(Msg::Key(Key::Char('L'))), vec![Effect::LockAll]);
    let work = app.update(Msg::Locked(Ok(vec![4242])));
    let after = app.generation();
    assert_ne!(before, after);
    assert!(work.contains(&Effect::LoadStatus(after)), "{work:?}");
    // Until the fresh read lands, the bar says it is checking, not what it showed before.
    let (rows, _) = checked_frame(&mut screen(80, 24), &app);
    assert!(rows[23].starts_with("checking..."), "{:?}", rows[23]);
    // The read started before the lock lands after it: dropped, not drawn.
    app.update(Msg::Status(before, Ok(open_status(now() + 3600))));
    let mut screen = screen(80, 24);
    let (rows, _) = checked_frame(&mut screen, &app);
    assert!(!rows.join("\n").contains("signing"), "{:?}", rows[23]);
    app.update(Msg::Status(after, Ok(locked_status())));
    let (rows, _) = checked_frame(&mut screen, &app);
    assert!(rows[23].starts_with("locked | "), "{:?}", rows[23]);
}

#[test]
fn a_key_read_from_before_the_lock_is_dropped_and_the_reload_after_it_always_runs() {
    let mut app = app();
    let before = app.generation();
    // A key read in flight when `L` is pressed (the one `start` asks for).
    assert_eq!(
        app.start(),
        vec![Effect::LoadKeys(before), Effect::LoadStatus(before)]
    );
    app.update(Msg::Key(Key::Char('L')));
    let work = app.update(Msg::Locked(Ok(vec![4242])));
    let after = app.generation();
    assert!(work.contains(&Effect::LoadKeys(after)), "{work:?}");
    // The key read from before the lock lands: dropped (its rows are not drawn).
    let mut stale = keys_ab();
    stale.rows[0].name = "stale".into();
    app.update(Msg::Keys(before, Ok(stale)));
    let mut screen = screen(80, 24);
    let (rows, _) = checked_frame(&mut screen, &app);
    assert!(!rows.join("\n").contains("stale"));
    app.update(Msg::Keys(after, Ok(keys_ab())));
    let (rows, _) = checked_frame(&mut screen, &app);
    assert!(rows.iter().any(|r| r.contains(A)));
}

#[test]
fn a_cached_address_that_is_no_address_fails_the_keys_pane_with_the_clis_error() {
    // wallet.json caches a terminal escape and 600 block glyphs as key a's address: the whole
    // list fails, as `zwallet list` does, and the pane shows the CLI's error with the value
    // escaped and cut at 64, wrapped in the pane (one source alone); no row shows it as an
    // address, and no cell holds a byte of it raw.
    let home = Home::new();
    home.create("a", &[]);
    home.create("b", &[]);
    let hostile = format!("\u{1b}[2J{}", "\u{2588}".repeat(600));
    home.edit_wallet(|v| v["keys"][0]["addresses"]["ethereum"] = serde_json::json!(hostile));
    let keys = block_on(wallet::keys(home.path()));
    let e = keys.as_ref().unwrap_err();
    assert_eq!(e.code, ErrorCode::ValidationError);
    let shown = zwallet::core::text::shown(&hostile, 64);
    assert!(e.message.contains(&shown), "{}", e.message);
    let mut app = app();
    app.update(Msg::Keys(app.generation(), keys));
    app.update(Msg::Status(
        app.generation(),
        block_on(wallet::status(home.path())),
    ));
    let (rows, placed) = checked_frame(&mut screen(80, 24), &app);
    let error = placed
        .iter()
        .find(|p| p.outside.source() == Source::Error)
        .expect("the error is drawn");
    assert!(error.outside.text().contains(&shown));
    assert!(error.outside.text().ends_with("[VALIDATION_ERROR]"));
    assert!(
        error.area.height > 1,
        "wrapped in the pane: {:?}",
        error.area
    );
    // No key row: the pane holds the error alone.
    assert!(!rows.iter().any(|r| r.contains(" a ") || r.contains("* b ")));
}

#[test]
fn every_outside_value_is_made_inert_and_bounded_where_it_comes_in() {
    let hostile = format!(
        "\u{1b}]0;x\u{7}\u{1b}[31m{}\n\u{202e}\\",
        "\u{2580}\u{2584}".repeat(300)
    );
    let values = [
        Outside::key_name(&hostile),
        Outside::address(&hostile),
        Outside::short_address(&hostile),
        Outside::expiry(&hostile, now()),
        Outside::rpc_host(&hostile),
        Outside::warning(&hostile),
        Outside::error(&WalletError::validation(hostile.clone())),
    ];
    for o in &values {
        let text = o.text();
        assert!(text.bytes().all(|b| (0x20..=0x7e).contains(&b)), "{o:?}");
        assert_eq!(o.source().budget(), budget(o.source()), "{o:?}");
        assert!(text.len() <= budget(o.source()), "{o:?}");
        assert!(!text.is_empty(), "{o:?}");
    }
    assert!(values[0].text().starts_with("\\u{1b}]0;x\\u{7}"));
    // The RPC host: the origin's host, at most 32 characters, cut with `...`; no path, no key.
    let host = |url: &str| Outside::rpc_host(url).text().to_string();
    assert_eq!(
        host("https://eth-mainnet.g.alchemy.com/v2/SECRETKEY"),
        "eth-mainnet.g.alchemy.com"
    );
    assert_eq!(host("http://127.0.0.1:8545"), "127.0.0.1:8545");
    let long = host("https://a-very-long-host-name-of-an-rpc-provider.example.com/v1");
    assert_eq!(long, "a-very-long-host-name-of-an-r...");
    assert_eq!(host("not a url"), "<redacted>");
    // An expiry zwallet can read is drawn by zwallet: HH:MMZ, with the date a day or more away.
    let at = parse_rfc3339_utc("2026-10-28T14:32:00Z").unwrap();
    assert_eq!(
        Outside::expiry("2026-10-28T14:32:00Z", at - 3600).text(),
        "14:32Z"
    );
    assert_eq!(
        Outside::expiry("2026-10-28T14:32:00Z", at - 86_400).text(),
        "2026-10-28 14:32Z"
    );
    // One it cannot read is shown as it came: escaped by `wallet::status`, at most 64.
    assert_eq!(Outside::expiry("soon", at).text(), "soon");
    assert_eq!(Outside::short_address(B).text(), "0x3C44...93BC");
}

#[test]
fn zwallet_tui_needs_a_terminal_and_has_no_json() {
    let home = Home::new();
    let out = home.run(&["tui"]);
    assert_eq!(out.code, 1, "{}", out.stdout);
    assert!(out.stderr.contains("needs a terminal"), "{}", out.stderr);
    let msg = home.run(&["tui", "--json"]).err("VALIDATION_ERROR");
    assert!(msg.contains("--json"), "{msg}");
    let msg = home
        .run(&["tui", "--warm", "soon", "--json"])
        .err("VALIDATION_ERROR");
    assert!(msg.contains("duration"), "{msg}");
    let msg = home
        .run(&["tui", "--chain", "nope", "--json"])
        .err("CHAIN_UNSUPPORTED");
    assert!(msg.contains("unsupported chain"), "{msg}");
}
