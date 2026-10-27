//! `zwallet tui` (`.prd/03-tui.md`): its screens drawn on ratatui's `TestBackend` through
//! `FilteredBackend`, the one write point, and its state machine driven by `Msg`s. Every frame a
//! test draws goes through [`checked_frame`], which asserts the three display-integrity rules
//! every later unit's frames keep too (03-tui.md §Display integrity rules 1-3). The binary itself
//! runs on a pty ([`tui_on_a_pty`]) where the behaviour needs a real terminal or a real daemon.

mod common;

use common::{Frame, Home, PTY_SIZE, PtyChild};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::Rect;
use serde_json::Value;
use std::time::{Duration, Instant, SystemTime};
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
/// by the edge; a value that has to be shorter is cut with `...` before it is placed), and
/// nowhere else but inside placed areas.
///
/// Fixtures: a value of 4 or more characters that also occurs in zwallet's own text (`lock`,
/// `keys`, `quit`, `status`, `verified`, `warning`, `zwallet`, ...) fails the frame, and so does
/// one that is a substring of some other text drawn where nothing is placed. Pick fixture names
/// and values that occur in neither.
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
    // And no other place shows it: a verbatim copy of it (or of its core, without `...`) of 4
    // characters or more, or a part of 12 or more, is a stray unless it lies wholly inside a
    // placed area. Each placed area holds that value's own text (checked for it as for this one),
    // so a match there is part of it, whatever its source: an error quoting a key name the list
    // shows too.
    let full: Vec<char> = text.chars().collect();
    let core: Vec<char> = text.trim_end_matches("...").chars().collect();
    for (y, row) in rows.iter().enumerate() {
        let row: Vec<char> = row.chars().collect();
        for x in 0..row.len() {
            let tail = &row[x..];
            let common = tail.iter().zip(&core).take_while(|(a, b)| a == b).count();
            let whole = full.len() >= 4 && tail.starts_with(&full);
            let exact = whole || (core.len() >= 4 && common == core.len());
            if !exact && common < 12 {
                continue;
            }
            let end = x + if whole { full.len() } else { common };
            let (cy, from, to) = (y as u16, x as u16, end as u16);
            let inside = placed.iter().any(|q| {
                (q.area.y..q.area.bottom()).contains(&cy)
                    && q.area.x <= from
                    && to <= q.area.right()
            });
            assert!(
                inside,
                "{source:?} {text:?} also on row {y} at column {x}: {:?}",
                rows[y]
            );
        }
    }
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
fn an_error_that_quotes_a_listed_key_name_is_drawn_beside_it() {
    // A lock that failed, its error naming the key `treasury`, while the key list shows that key:
    // the name is placed twice, once on its own and once inside the error.
    let mut keys = keys_ab();
    keys.rows[0].name = "treasury".into();
    let mut app = app();
    app.update(Msg::Keys(app.generation(), Ok(keys)));
    app.update(Msg::Status(app.generation(), Ok(open_status(now() + 3600))));
    app.update(Msg::Key(Key::Char('L')));
    let failed = WalletError::validation("could not stop the daemon of key 'treasury'");
    app.update(Msg::Locked(Err(failed)));
    let (rows, placed) = checked_frame(&mut screen(80, 24), &app);
    assert!(
        rows.iter()
            .any(|r| r.contains(" treasury ") && r.contains(A))
    );
    assert!(placed
        .iter()
        .any(|p| p.outside.source() == Source::Error && p.outside.text().contains("'treasury'")));
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

/// How long a pty test waits for a frame or an exit, on a loaded machine too. A bound the
/// acceptance sets (T10's two refreshes) is asserted on its own.
const PTY_WAIT: Duration = Duration::from_secs(60);

/// The end of the bar's second line on every pty frame (the default RPC, the environment being
/// empty): once it is drawn, the frame above it is too.
const HOST: &str = " | ethereum-rpc.publicnode.com";

/// `zwallet tui` for `home` on a pty of `size`, its controlling terminal, as a human starts it.
fn tui_on_a_pty(home: &Home, size: (u16, u16)) -> PtyChild {
    PtyChild::spawn_full_screen(home.command(&["tui"]), size)
}

/// The bar's two lines of a [`PTY_SIZE`] frame.
fn bar(frame: &Frame) -> (&str, &str) {
    let rows = usize::from(PTY_SIZE.1);
    (&frame.rows[rows - 2], &frame.rows[rows - 1])
}

/// `q`, then the exit: status 0, nothing on stderr, and the pty back in canonical mode with echo.
/// Returns the last frame, the TUI's whole output up to giving the terminal back: no stray byte
/// in all of it, and no sequence left unfinished.
fn quit(tui: &mut PtyChild) -> Frame {
    tui.send(b"q");
    let (status, stderr) = tui.wait_exit(PTY_WAIT);
    assert!(status.success(), "{status}: {stderr}");
    assert!(stderr.is_empty(), "{stderr:?}");
    let modes = tui.local_modes();
    assert_ne!(modes & libc::ICANON, 0, "canonical mode is off");
    assert_ne!(modes & libc::ECHO, 0, "echo is off");
    let last = tui.final_frame(PTY_WAIT);
    assert!(last.stray.is_empty() && last.pending.is_empty(), "{last}");
    last
}

/// The draws in a TUI's output so far: ratatui hides the cursor after every draw, and the guard
/// once before the first.
fn draws(output: &[u8]) -> usize {
    let hide = b"\x1b[?25l";
    output
        .windows(hide.len())
        .filter(|w| w == hide)
        .count()
        .saturating_sub(1)
}

/// A frame as ratatui's crossterm backend writes it at 79x24: the alternate screen, the cursor
/// hidden, the one line of [`ui::TOO_SMALL`] (its blank cells skipped), the styles reset, the
/// cursor hidden again; then the terminal given back.
const SMALL_FRAME: &[u8] = b"\x1b[?1049h\x1b[?25l\x1b[1;1Hterminal\x1b[1;10Htoo\x1b[1;14Hsmall\
\x1b[1;20H(80x24\x1b[1;27Hneeded)\x1b[39m\x1b[49m\x1b[59m\x1b[0m\x1b[?25l\x1b[?1049l\x1b[?25h";

#[test]
fn a_valid_stream_cut_anywhere_holds_no_stray_byte() {
    // A pty read ends anywhere, so a snapshot of the output can end inside a sequence: after a
    // lone ESC, or inside a CSI before its final byte. That is not a stray byte.
    let stray: Vec<_> = (0..=SMALL_FRAME.len())
        .filter_map(|end| {
            let frame = Frame::parse(&SMALL_FRAME[..end], (79, 24));
            let tail = String::from_utf8_lossy(&SMALL_FRAME[end.saturating_sub(8)..end]);
            (!frame.stray.is_empty()).then(|| format!("cut after ...{tail:?}: {:?}", frame.stray))
        })
        .collect();
    assert!(stray.is_empty(), "{stray:#?}");
    // The unfinished sequence is pending, and only at the end.
    assert_eq!(Frame::parse(b"x\x1b", (79, 24)).pending, "\u{1b}");
    assert_eq!(Frame::parse(b"x\x1b[1;2", (79, 24)).pending, "\u{1b}[1;2");
    assert!(Frame::parse(SMALL_FRAME, (79, 24)).pending.is_empty());
    // A stray byte before the end still counts.
    for bad in [&b"x\x1b\x1b[0m"[..], b"x\x1bQ", b"\x1b[1\x07m", b"x\x07"] {
        assert!(!Frame::parse(bad, (79, 24)).stray.is_empty(), "{bad:?}");
    }
}

/// Whether `frame` is the 79x24 frame: the one `terminal too small` line and nothing else.
fn only_the_warning(frame: &Frame) -> bool {
    frame.rows[0].trim_end() == ui::TOO_SMALL && frame.rows[1..].iter().all(|r| r.trim().is_empty())
}

#[test]
fn the_79x24_check_runs_on_the_last_frame_not_on_a_live_snapshot() {
    // The warning is drawn, then a later draw (a later pty read) writes on row 5. A snapshot
    // taken once row 0 shows the warning passes the check; the last frame before the terminal
    // is given back does not.
    let leave = b"\x1b[?1049l\x1b[?25h";
    let (warning, given_back) = SMALL_FRAME.split_at(SMALL_FRAME.len() - leave.len());
    let more = b"\x1b[5;1Hmore\x1b[0m\x1b[?25l";
    let stream = [warning, &more[..], given_back].concat();
    assert!(only_the_warning(&Frame::parse(warning, (79, 24))));
    let last = Frame::last(&stream, (79, 24)).expect("the terminal is given back");
    assert!(!only_the_warning(&last), "{last}");
    // Without the later draw the last frame is the warning alone; before the terminal is given
    // back there is no last frame.
    assert!(only_the_warning(
        &Frame::last(SMALL_FRAME, (79, 24)).unwrap()
    ));
    assert!(Frame::last(warning, (79, 24)).is_none());
}

#[test]
fn t9_on_a_pty_q_gives_the_terminal_back_in_canonical_mode_with_echo() {
    let home = Home::new();
    home.create("main", &[]);
    // At 120x40, once the key list and the status are drawn.
    let mut tui = tui_on_a_pty(&home, PTY_SIZE);
    tui.wait_frame(PTY_WAIT, |f| {
        let (one, two) = bar(f);
        one.starts_with("ethereum | main 0x") && two.starts_with("locked | ") && two.contains(HOST)
    });
    // While the TUI holds the terminal: raw mode, no echo.
    let modes = tui.local_modes();
    assert_eq!(modes & (libc::ICANON | libc::ECHO), 0, "not raw");
    let last = quit(&mut tui);
    assert!(last.rows[0].starts_with("zwallet tui | keys"), "{last}");
    assert!(last.rows.iter().any(|r| r.contains("* main ")), "{last}");
    // At 79x24, once both reads are drawn: the TUI draws once at the start and once per
    // message, and before any key the messages are the two reads and the refresh ticks, the
    // first of which comes a refresh after the start. Three draws within a refresh of the spawn
    // are the first one and the two reads'.
    let spawned = Instant::now();
    let mut tui = tui_on_a_pty(&home, (79, 24));
    tui.wait_output(PTY_WAIT, |output| draws(output) >= 3);
    let took = spawned.elapsed();
    eprintln!("t9: at 79x24 both reads were drawn {took:?} after the spawn");
    assert!(
        took < zwallet::tui::REFRESH,
        "three draws only {took:?} after the spawn: a tick's cannot be told from a read's"
    );
    let modes = tui.local_modes();
    assert_eq!(modes & (libc::ICANON | libc::ECHO), 0, "not raw at 79x24");
    let last = quit(&mut tui);
    assert!(only_the_warning(&last), "{last}");
}

#[test]
fn t10_on_a_pty_zwallet_lock_elsewhere_shows_locked_within_two_refreshes() {
    // `zwallet lock` stops every daemon of a test home.
    let _serial = common::exclusive();
    let home = Home::new();
    home.create("main", &[]);
    home.unlock("1h");
    let mut tui = tui_on_a_pty(&home, PTY_SIZE);
    let frame = tui.wait_frame(PTY_WAIT, |f| {
        let (_, two) = bar(f);
        two.starts_with("signing (agent too) until ") && two.contains(HOST)
    });
    if cfg!(target_os = "macos") {
        // This build's daemon, checked from the zwallet binary (decision 3 of units.md).
        assert!(
            bar(&frame).0.trim_end().ends_with(" | verified by session"),
            "{frame}"
        );
    }
    home.run(&["lock", "--json"]).ok();
    let locked = Instant::now();
    let frame = tui.wait_frame(PTY_WAIT, |f| bar(f).1.starts_with("locked | "));
    let took = locked.elapsed();
    eprintln!("t10: the bar said locked {took:?} after zwallet lock returned");
    // Two refreshes, 10 s (03-tui.md T10), written here rather than read from the code.
    assert!(
        took <= Duration::from_secs(10),
        "locked shown {took:?} after the lock, over two refreshes"
    );
    let last = quit(&mut tui);
    assert!(bar(&last).1.starts_with("locked | "), "{frame}\n{last}");
}

#[test]
fn an_argv_listed_fake_daemon_s_expiry_reaches_the_terminal_escaped_and_cut() {
    // A process started with the daemon's argv, so the process table lists it for this home and
    // `status` takes its reply: unlocked, its own pid, an expiry of 600 characters of ESC and
    // half blocks (JSON escapes, as the reply carries them).
    let home = Home::new();
    home.create("main", &[]);
    // The cache holds the fake's address, so the status relays its session rather than stopping
    // at a mismatch.
    home.edit_wallet(|v| {
        for (_, a) in v["keys"][0]["addresses"].as_object_mut().unwrap() {
            *a = serde_json::json!(A);
        }
    });
    let dir = tempfile::Builder::new()
        .prefix("zwt-")
        .tempdir_in("/tmp")
        .unwrap();
    let expires = "\\u001b\\u2580\\u2584".repeat(200);
    let fake = common::daemon_argv_impostor(&home, dir.path(), &[("ATTACKER_EXPIRES", &expires)]);
    assert!(home.daemon_pids().contains(&(fake.0.id() as i32)));
    let mut tui = tui_on_a_pty(&home, PTY_SIZE);
    // The fake's status is drawn once its expiry and the separator after it are; then the checks
    // run on the last frame.
    let until = "signing (agent too) until ";
    tui.wait_frame(PTY_WAIT, |f| {
        let (one, two) = bar(f);
        one.contains("NOT this build's key daemon: ")
            && two
                .strip_prefix(until)
                .is_some_and(|rest| rest.contains(" | "))
    });
    let frame = quit(&mut tui);
    // Line 2 shows the expiry escaped, cut within its budget, on that line alone.
    let shown = bar(&frame)
        .1
        .strip_prefix(until)
        .and_then(|rest| rest.split(" | ").next())
        .unwrap_or_else(|| panic!("{frame}"));
    assert!(shown.starts_with("\\u{1b}\\u{2580}"), "{shown:?}");
    assert!(
        shown.len() <= 64 && common::ascii_printable(shown),
        "{shown:?}"
    );
    let rows = frame.rows.iter().filter(|r| r.contains("\\u{1b}")).count();
    assert_eq!(rows, 1, "{frame}");
    // No raw ESC from the value reached the terminal (`quit`: no stray byte in the whole run),
    // and no half block.
    let output = String::from_utf8_lossy(&tui.output()).into_owned();
    assert!(!output.contains(['\u{2580}', '\u{2584}']), "{output:?}");
    // The address is not marked verified anywhere.
    assert!(
        !frame.rows.iter().any(|r| r.contains("verified by")),
        "{frame}"
    );
    drop(fake);
}
