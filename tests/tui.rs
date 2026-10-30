//! `zwallet tui` (`.prd/03-tui.md`): its screens drawn on ratatui's `TestBackend` through
//! `FilteredBackend`, the one write point, and its state machine driven by `Msg`s. Every frame a
//! test draws goes through [`checked_frame`], which asserts the three display-integrity rules
//! every later unit's frames keep too (03-tui.md §Display integrity rules 1-3). The binary itself
//! runs on a pty ([`tui_on_a_pty`]) where the behaviour needs a real terminal or a real daemon.

mod common;

use common::{Frame, Home, PTY_SIZE, PW, PtyChild};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::Rect;
use serde_json::Value;
use std::time::{Duration, Instant, SystemTime};
use zwallet::chains::{self, Net};
use zwallet::core::error::{ErrorCode, WalletError};
use zwallet::core::store;
use zwallet::core::terminal::terminal_ok;
use zwallet::core::time::{parse_rfc3339_utc, rfc3339_utc, unix_secs};
use zwallet::core::whitelist::ListFiles;
use zwallet::tui::app::{App, Effect, Key, Msg};
use zwallet::tui::style;
use zwallet::tui::term::FilteredBackend;
use zwallet::tui::text::{Outside, Source};
use zwallet::tui::ui::{self, Placed};
use zwallet::wallet::{self, KeyRow, Keys, Serving, StatusView, Trust};

fn family_address(chain: &chains::Chain, address: &str) -> String {
    if matches!(chain.family, chains::Family::Tron(_))
        && let Ok(body) = chains::evm::parse_address(address)
    {
        chains::tron::address::from_body(&body.into_array())
    } else {
        address.into()
    }
}

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
        Source::WhitelistName => 32,
        Source::Quantity => 78,
        Source::TransactionHash => 66,
        Source::KeyName | Source::RpcHost => 32,
        Source::Address => 42,
        Source::Expiry | Source::Home => 64,
        Source::Detail => 72,
        Source::Input => 512,
        Source::Error | Source::Warning => 512,
        Source::ListRow => 512,
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
        trust: Trust::Locked,
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
            status.address = Some((Some(B.into()), trust.clone()));
            app.update(Msg::Status(app.generation(), Ok(status)));
            // Status is primary-EVM metadata. Tron needs its own current-family observation.
            let address = family_address(chain, B);
            if matches!(chain.family, chains::Family::Tron(_)) {
                app.update(Msg::Address(
                    app.generation(),
                    name.clone(),
                    Ok((address.clone(), trust)),
                ));
            }
            let expected_address = if matches!(chain.family, chains::Family::Tron(_)) {
                format!("{}...{}", &address[..6], &address[address.len() - 4..])
            } else {
                "0x3C44...93BC".into()
            };
            let (rows, _) = checked_frame(&mut screen(80, 24), &app);
            let line = rows[22].trim_end();
            let case = format!("{} {words:?}", chain.key);
            // chain | name 0x3C44...93BC | trust words [warning]
            let rest = line
                .strip_prefix(&format!("{} | ", chain.key))
                .unwrap_or_else(|| panic!("{case}: {line:?}"));
            let (shown_name, rest) = rest
                .split_once(&format!(" {expected_address} | "))
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

/// A frame as ratatui's crossterm backend writes it at 79x24: the alternate screen, the cursor
/// hidden, the one line of [`ui::TOO_SMALL`] (its blank cells skipped), the styles reset, the
/// cursor hidden again; then the terminal given back: ratatui shows the cursor, the guard leaves
/// the alternate screen and shows it.
const SMALL_FRAME: &[u8] = b"\x1b[?1049h\x1b[?25l\x1b[1;1Hterminal\x1b[1;10Htoo\x1b[1;14Hsmall\
\x1b[1;20H(80x24\x1b[1;27Hneeded)\x1b[39m\x1b[49m\x1b[59m\x1b[0m\x1b[?25l\x1b[?25h\x1b[?1049l\x1b[?25h";

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
    let leave = b"\x1b[?25h\x1b[?1049l\x1b[?25h";
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
fn a_tui_that_exits_early_is_reported_before_the_wait_ends() {
    // `zwallet tui` refuses an unknown chain before it takes the terminal, and exits 1. A wait
    // for a frame it never draws reports the exit once the reader has every byte, not at the
    // wait's limit.
    let home = Home::new();
    let mut tui = PtyChild::spawn_full_screen(home.command(&["tui", "--chain", "nope"]), PTY_SIZE);
    let start = Instant::now();
    let waited = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tui.wait_output(PTY_WAIT, |_| false)
    }));
    let took = start.elapsed();
    let said = waited
        .expect_err("nothing shows what the wait looks for")
        .downcast::<String>()
        .map(|m| *m)
        .unwrap_or_default();
    assert!(said.contains("exited (exit status: 1)"), "{said}");
    assert!(
        took < PTY_WAIT,
        "reported only at the limit, after {took:?}"
    );
}

#[test]
fn t9_on_a_pty_q_gives_the_terminal_back_in_canonical_mode_with_echo() {
    let home = Home::new();
    home.create("main", &[]);
    // Loaded at 120x40 (the key list and the status drawn), then the window shrinks to 79x24:
    // the kernel tells the TUI (SIGWINCH), and it draws the warning alone.
    let mut tui = tui_on_a_pty(&home, PTY_SIZE);
    tui.wait_frame(PTY_WAIT, |f| {
        let (one, two) = bar(f);
        one.starts_with("ethereum | main 0x") && two.starts_with("locked | ") && two.contains(HOST)
    });
    // While the TUI holds the terminal: raw mode, no echo.
    let modes = tui.local_modes();
    assert_eq!(modes & (libc::ICANON | libc::ECHO), 0, "not raw");
    tui.resize((79, 24));
    tui.wait_frame(PTY_WAIT, |f| f.rows[0].trim_end() == ui::TOO_SMALL);
    let last = quit(&mut tui);
    assert!(only_the_warning(&last), "{last}");
    // Started at 79x24: the warning alone, judged on the last frame.
    let mut tui = tui_on_a_pty(&home, (79, 24));
    tui.wait_frame(PTY_WAIT, |f| f.rows[0].trim_end() == ui::TOO_SMALL);
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

#[test]
fn the_password_check_derives_the_named_keys_and_checks_the_list_it_reads_once() {
    let home = Home::new();
    home.create("a", &[]);
    home.create("b", &[]);
    let listed = home.allow("alice", B, &["ethereum"]);
    let files = ListFiles::read(home.path()).fingerprint();
    let opened = wallet::open_with_password(home.path(), PW, Some(&["a"])).unwrap();
    // The named key's address from the seed, as `list --verify` derives it.
    let stdin = format!("{PW}\n");
    let verify = ["list", "--verify", "--password-stdin", "--json"];
    let verify = home.run_stdin(&verify, &stdin).ok();
    assert_eq!(opened.keys.len(), 1);
    assert_eq!(opened.keys[0].name, "a");
    assert_eq!(
        opened.keys[0].address.as_deref(),
        verify["keys"][0]["address"].as_str()
    );
    assert_eq!((opened.active.as_str(), opened.salted), ("b", true));
    // The list the seal names, with the fingerprint of the bytes it verified.
    let checked = wallet::check_list(home.path(), &opened.anchor).unwrap();
    assert_eq!(checked.list, listed);
    assert_eq!(checked.list.entries[0].nickname, "alice");
    assert_eq!(checked.fingerprint, files);
    // `open_with_password` reads no list file: with both removed it still opens.
    for file in ["whitelist.json", "whitelist.next.json"] {
        let _ = std::fs::remove_file(home.path().join(file));
    }
    let opened = wallet::open_with_password(home.path(), PW, Some(&[])).unwrap();
    assert!(opened.keys.is_empty() && matches!(opened.anchor, Ok(Some(_))));
    assert!(wallet::check_list(home.path(), &opened.anchor).is_err());
    // A whitelist zwallet 0.1.0 sealed (no salt): `list --verify`'s message, and no list.
    let lock = store::MetaLock::acquire(home.path()).unwrap();
    let sealed = lock.open(PW).unwrap();
    let mut policy = sealed.unsealed().policy.clone();
    policy.as_object_mut().unwrap().remove("whitelistSalt");
    lock.reseal(&sealed, &policy).unwrap();
    drop(lock);
    let opened = wallet::open_with_password(home.path(), PW, Some(&[])).unwrap();
    assert!(!opened.salted);
    let reason = wallet::check_list(home.path(), &opened.anchor).unwrap_err();
    assert!(reason.contains("made by zwallet 0.1.0"), "{reason}");
    let verify = [
        "whitelist",
        "list",
        "--verify",
        "--password-stdin",
        "--json",
    ];
    let cli = home.run_stdin(&verify, &stdin).ok();
    assert_eq!(cli["reason"].as_str(), Some(reason.as_str()), "{cli}");
}

#[test]
fn t2b_on_a_pty_s_then_y_makes_the_selected_key_active() {
    let home = Home::new();
    home.create("a", &[]);
    home.create("b", &[]);
    let mut tui = tui_on_a_pty(&home, PTY_SIZE);
    tui.wait_frame(PTY_WAIT, |f| {
        f.rows.iter().any(|r| r.contains("* b ")) && bar(f).1.starts_with("locked | ")
    });
    // `s` on `a` (the first row) asks first; `y` confirms.
    tui.send(b"s");
    tui.wait_frame(PTY_WAIT, |f| {
        f.rows.iter().any(|r| r.contains("make a the active key"))
    });
    assert_eq!(home.run(&["list", "--json"]).ok()["active"], "b");
    tui.send(b"y");
    tui.wait_frame(PTY_WAIT, |f| f.rows.iter().any(|r| r.contains("* a ")));
    assert_eq!(home.run(&["list", "--json"]).ok()["active"], "a");
    // A later activation is accepted after the first finishes, including another row.
    tui.send(b"js");
    tui.wait_frame(PTY_WAIT, |f| {
        f.rows.iter().any(|r| r.contains("make b the active key"))
    });
    tui.send(b"y");
    tui.wait_frame(PTY_WAIT, |f| f.rows.iter().any(|r| r.contains("* b ")));
    assert_eq!(home.run(&["list", "--json"]).ok()["active"], "b");
    quit(&mut tui);
}

#[test]
fn activation_is_serial_and_quit_waits_for_the_confirmed_write() {
    for quit in [Key::Char('q'), Key::Interrupt] {
        let mut app = locked_ab();
        app.update(Msg::Key(Key::Char('s')));
        assert_eq!(
            app.update(Msg::Key(Key::Char('y'))),
            vec![Effect::SetActive("a".into())]
        );
        // No second write can race the first and restore the older selection last.
        app.update(Msg::Key(Key::Down));
        assert!(app.update(Msg::Key(Key::Char('s'))).is_empty());
        assert!(app.confirming().is_none());
        assert!(app.update(Msg::Key(Key::Char('y'))).is_empty());
        app.update(Msg::Key(quit));
        assert!(!app.quitting());
        assert!(app.quit_pending());
        let (rows, _) = checked_frame(&mut screen(80, 24), &app);
        assert!(rows[23].contains("setting active key... (quits when done)"));
        app.update(Msg::Activated(Ok((A.into(), Trust::Locked))));
        assert!(app.quitting());
        assert!(!app.quit_pending());
    }
}

#[test]
fn failed_activation_releases_the_gate_and_reports_failure_on_quit() {
    let mut app = locked_ab();
    app.update(Msg::Key(Key::Char('s')));
    app.update(Msg::Key(Key::Char('y')));
    app.update(Msg::Key(Key::Char('q')));
    app.update(Msg::Activated(Err(WalletError::validation(
        "write refused",
    ))));
    assert!(app.quitting());
    assert_eq!(app.take_quit_error().unwrap().message, "write refused");
}

#[test]
fn an_old_read_cannot_hide_an_activation_refusal() {
    let mut app = locked_ab();
    app.update(Msg::Key(Key::Char('r')));
    let old = app.generation();
    app.update(Msg::Key(Key::Char('s')));
    app.update(Msg::Key(Key::Char('y')));
    app.update(Msg::Activated(Err(WalletError::validation(
        "Do not send funds to either address",
    ))));
    app.update(Msg::Keys(old, Ok(keys_ab())));
    let (rows, _) = checked_frame(&mut screen(80, 24), &app);
    assert!(
        rows.join("\n")
            .contains("Do not send funds to either address")
    );
    assert!(!rows.join("\n").contains(A));
}

#[test]
fn activation_waiting_for_the_metadata_lock_keeps_the_async_loop_running() {
    let home = Home::new();
    home.create("a", &[]);
    let lock = store::MetaLock::acquire(home.path()).unwrap();
    let (release, receive) = std::sync::mpsc::channel();
    let owner = std::thread::spawn(move || {
        // A broken synchronous write eventually releases too; this test never hangs a suite.
        let _ = receive.recv_timeout(Duration::from_secs(3));
        drop(lock);
    });
    block_on(async {
        let work = wallet::set_active(home.path(), "a");
        tokio::pin!(work);
        let started = Instant::now();
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(30)) => {},
            _ = &mut work => panic!("the write finished while its lock was held"),
        }
        let elapsed = started.elapsed();
        let _ = release.send(());
        work.await.unwrap();
        assert!(
            elapsed < Duration::from_secs(2),
            "event loop blocked for {elapsed:?}"
        );
    });
    owner.join().unwrap();
}

#[test]
fn a_key_the_bar_shows_verified_by_session_is_never_asked_to_be_verified() {
    // One key, verified by session: no row asks for a verify command.
    let mut keys = keys_ab();
    keys.rows.remove(0);
    let mut app = app();
    app.update(Msg::Keys(app.generation(), Ok(keys)));
    app.update(Msg::Status(app.generation(), Ok(open_status(now() + 3600))));
    let mut screen = screen(80, 24);
    let (rows, _) = checked_frame(&mut screen, &app);
    assert!(!rows.iter().any(|r| r.contains("--verify")), "{rows:#?}");
    assert!(
        rows[22].trim_end().ends_with(" | verified by session"),
        "{rows:#?}"
    );
    // Two keys: the other one stays flagged, with what checks it; the active one is not asked.
    app.update(Msg::Keys(app.generation(), Ok(keys_ab())));
    let (rows, _) = checked_frame(&mut screen, &app);
    let note = rows
        .iter()
        .find(|r| r.contains("not verified:"))
        .expect("the other key is flagged");
    assert!(
        note.contains("other key") && note.contains("V verifies every key"),
        "{note:?}"
    );
    assert!(
        !rows.iter().any(|r| r.contains("address --verify")),
        "{rows:#?}"
    );
    // Locked, or unverified (Linux, another process answering): the note stays as it was, and
    // the impostor's warning keeps its place above it.
    let other = "pid 7 runs other code".to_string();
    for trust in [
        Trust::Locked,
        Trust::Unverified(None),
        Trust::Unverified(Some(other.clone())),
    ] {
        let mut keys = keys_ab();
        keys.trust = trust.clone();
        app.update(Msg::Keys(app.generation(), Ok(keys)));
        let mut status = open_status(now() + 3600);
        status.address = Some((Some(B.into()), trust.clone()));
        app.update(Msg::Status(app.generation(), Ok(status)));
        let (rows, _) = checked_frame(&mut screen, &app);
        let note = "not verified: V checks addresses from the seed before funding";
        let at = rows
            .iter()
            .position(|r| r.contains(note))
            .expect("the note");
        if trust.warning().is_some() {
            let warned = format!("warning: {other}");
            assert!(rows[at - 1].contains(&warned), "{rows:#?}");
        }
    }
}

/// [`keys_ab`] read, and the wallet locked.
fn locked_ab() -> App {
    let mut app = app();
    app.update(Msg::Keys(app.generation(), Ok(keys_ab())));
    app.update(Msg::Status(app.generation(), Ok(locked_status())));
    app
}

#[test]
fn s_on_a_key_another_process_contradicts_stops_the_pane_and_makes_nothing_active() {
    // A process listed by argv answers `address` with its own address for every key, which
    // contradicts wallet.json's cache of `a`.
    let home = Home::new();
    home.create("a", &[]);
    home.create("b", &[]);
    let dir = tempfile::Builder::new()
        .prefix("zwt-")
        .tempdir_in("/tmp")
        .unwrap();
    let fake = common::daemon_argv_impostor(&home, dir.path(), &[]);
    let mut app = locked_ab();
    app.update(Msg::Key(Key::Char('s')));
    let work = app.update(Msg::Key(Key::Char('y')));
    assert_eq!(work, vec![Effect::SetActive("a".into())]);
    // The work that effect runs, through that process: refused before anything is switched.
    app.update(Msg::Activated(block_on(wallet::set_active(
        home.path(),
        "a",
    ))));
    let (_, placed) = checked_frame(&mut screen(80, 24), &app);
    let error = placed
        .iter()
        .find(|p| p.outside.source() == Source::Error)
        .expect("the refusal is drawn");
    // The whole error, wrapped in the pane: the address that process gave is never on a line of
    // its own without the warning.
    let text = error.outside.text();
    assert!(
        text.contains(A) && text.contains("Do not send funds to either address"),
        "{text}"
    );
    assert!(error.area.height > 1, "{:?}", error.area);
    drop(fake);
    assert_eq!(home.run(&["list", "--json"]).ok()["active"], "b");
}

#[test]
fn while_s_asks_any_key_but_y_says_no_and_ctrl_c_still_quits() {
    for (key, quits) in [(Key::Char('q'), false), (Key::Interrupt, true)] {
        let mut app = locked_ab();
        app.update(Msg::Key(Key::Char('s')));
        assert!(app.confirming().is_some());
        assert_eq!(app.update(Msg::Key(key)), vec![]);
        assert!(app.confirming().is_none(), "{key:?}");
        assert_eq!(app.quitting(), quits, "{key:?}");
    }
}

#[test]
fn enter_on_a_locked_key_starts_warmup_without_changing_global_active() {
    let mut app = locked_ab();
    let work = app.update(Msg::Key(Key::Enter));
    assert!(
        !work.is_empty(),
        "Enter must start the staged warm-up for a"
    );
    let (rows, _) = checked_frame(&mut screen(80, 24), &app);
    assert!(rows.iter().any(|r| r.contains("preparing...")));
}

#[test]
fn password_check_is_one_message_and_pinned_addresses_survive_a_later_impostor() {
    use zwallet::tui::app::PasswordState;
    let mut app = locked_ab();
    assert_eq!(
        app.update(Msg::Key(Key::Enter)),
        vec![Effect::PrepareUnlock]
    );
    assert_eq!(app.password_state(), PasswordState::Preparing);
    app.update(Msg::Prepared(Ok(())));
    for n in 1..=17 {
        let msg = Msg::PasswordLength(n, false);
        assert!(!format!("{msg:?}").contains("pw-XYZ"));
        app.update(msg);
        let (rows, _) = checked_frame(&mut screen(80, 24), &app);
        assert_eq!(
            rows.iter().map(|r| r.matches('*').count()).sum::<usize>(),
            n + 1
        );
        assert!(!rows.iter().any(|r| r.contains("pw-XYZ")));
    }
    assert_eq!(
        app.update(Msg::PasswordSubmitted),
        vec![Effect::CheckPassword {
            key: "a".into(),
            start_session: true
        }]
    );
    let (rows, _) = checked_frame(&mut screen(80, 24), &app);
    assert!(rows.iter().any(|r| r.contains("checking...")));
    // Resize/tick draw while the worker owns the password; no secret is in the state.
    app.update(Msg::Resize);
    let opened = wallet::Opened {
        keys: keys_ab().rows,
        active: "b".into(),
        warning: None,
        anchor: Ok(None),
        salted: false,
    };
    app.update(Msg::Checked(Ok((opened, Err("no list".into())))));
    app.update(Msg::Status(app.generation(), Ok(open_status(now() + 3600))));
    let (rows, _) = checked_frame(&mut screen(80, 24), &app);
    assert!(
        rows[22].contains("a 0x7099...79C8 | verified by password"),
        "{rows:#?}"
    );
    assert!(!rows.iter().any(|r| r.contains("not verified:")));
    app.update(Msg::Address(
        app.generation(),
        "a".into(),
        Ok((B.into(), Trust::Unverified(None))),
    ));
    let (rows, placed) = checked_frame(&mut screen(80, 24), &app);
    assert!(placed.iter().any(
        |p| p.outside.source() == Source::Error && p.outside.text().contains("does not match")
    ));
    assert!(rows[22].contains("0x7099...79C8 | verified by password"));
}

#[test]
fn failed_password_check_never_pins_or_checks_a_list_and_quit_waits_for_it() {
    let mut app = locked_ab();
    app.update(Msg::Key(Key::Enter));
    app.update(Msg::Prepared(Ok(())));
    app.update(Msg::PasswordSubmitted);
    app.update(Msg::Key(Key::Char('q')));
    assert!(!app.quitting());
    app.update(Msg::Checked(Err(WalletError::validation(
        "cache changed; run zwallet address --verify",
    ))));
    assert!(app.quitting());
    assert!(!app.pinned("a") && app.checked_list().is_none());
    let (_, placed) = checked_frame(&mut screen(80, 24), &app);
    assert!(
        placed
            .iter()
            .any(|p| p.outside.text().contains("address --verify"))
    );
}

#[test]
fn t2_warmup_on_a_pty_accepts_typeahead_and_paste_without_echo_or_active_change() {
    let home = Home::new();
    let password = "pw-XYZ-0123456789";
    for name in ["a", "b"] {
        home.run_stdin(
            &["create", name, "--password-stdin", "--json"],
            &format!("{password}\n"),
        )
        .ok();
    }
    let mut tui = PtyChild::spawn_full_screen(tui_without_public_rpc(&home), PTY_SIZE);
    tui.wait_frame(PTY_WAIT, |f| {
        f.rows.iter().any(|r| r.contains("* b ")) && bar(f).1.starts_with("locked | ")
    });
    // Typeahead in the same write as Enter must already belong to the secret buffer.
    tui.send(b"\rpw-XYZ-");
    tui.wait_frame(PTY_WAIT, |f| f.rows.iter().any(|r| r.contains("*******")));
    tui.send(b"\x1b[200~0123456789\x1b[201~");
    tui.wait_frame(PTY_WAIT, |f| {
        f.rows.iter().any(|r| r.contains("*****************"))
    });
    let before = now();
    tui.send(b"\r");
    let warmed = tui.wait_frame(PTY_WAIT, |f| {
        bar(f).0.contains("a ")
            && bar(f).0.contains("verified by password")
            && bar(f).1.starts_with("signing (agent too) until ")
    });
    let status = home.run(&["status", "--json"]).ok();
    let expiry = parse_rfc3339_utc(status["expiresAt"].as_str().unwrap()).unwrap();
    assert!((before + 3590..=now() + 3610).contains(&expiry));
    let shown = Outside::expiry(status["expiresAt"].as_str().unwrap(), now());
    assert!(bar(&warmed).1.contains(shown.cut(17).text()));
    assert_eq!(home.run(&["list", "--json"]).ok()["active"], "b");
    let frame = quit(&mut tui);
    let output = String::from_utf8_lossy(&tui.output()).into_owned();
    assert!(
        !output.contains(password) && !output.contains("pw-XYZ"),
        "secret leaked"
    );
    assert!(bar(&frame).0.contains("verified by password"));
}

#[test]
fn a_cache_changed_after_unlock_fails_the_password_check_but_keeps_the_session() {
    let home = Home::new();
    home.create("a", &[]);
    home.create("b", &[]);
    home.run_stdin(
        &[
            "unlock",
            "1h",
            "--allow-ptrace-scope-0",
            "--password-stdin",
            "--json",
        ],
        &format!("{PW}\n"),
    )
    .ok();
    // Exact stage order: unlock succeeded; metadata changes before the local password check.
    home.edit_wallet(|v| v["keys"][0]["addresses"]["ethereum"] = serde_json::json!(B));
    let mut app = locked_ab();
    app.update(Msg::Key(Key::Enter));
    app.update(Msg::Prepared(Ok(())));
    app.update(Msg::PasswordSubmitted);
    let opened = wallet::open_with_password(home.path(), PW, Some(&["a", "b"]));
    let e = opened.unwrap_err();
    assert_eq!(e.code, ErrorCode::ValidationError);
    assert!(e.message.contains("zwallet address --verify"));
    app.update(Msg::Checked(Err(e)));
    assert!(app.checked_list().is_none() && !app.pinned("a"));
    home.assert_unlocked();
    let (_, placed) = checked_frame(&mut screen(80, 24), &app);
    assert!(
        placed
            .iter()
            .any(|p| p.outside.text().contains("zwallet address --verify"))
    );
}

/// Probabilistic regression coverage for `edit_wallet`, not a deterministic reproduction. The
/// test above (and every other `edit_wallet` under a live session) relies on it replacing
/// wallet.json at once: a running daemon reads the file every second (`store::read_wallet_file`)
/// and ends its session on a file it cannot parse. Written in place (truncate, then write), an
/// edit could show it an empty file, and `status` said `locked: true` (issue #52). Here a reader
/// polls the file throughout 2,000 edits; against the in-place write it saw torn files in every
/// run measured, but a run that sees none proves nothing by itself.
#[test]
fn edit_wallet_never_lets_a_reader_see_wallet_json_half_written() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    /// Stops the reader however the edits end, a panic included.
    struct StopReader(Arc<AtomicBool>);
    impl Drop for StopReader {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    let home = Home::new();
    home.create("a", &[]);
    let (done, reads) = (
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicU64::new(0)),
    );
    let (ready, reading) = std::sync::mpsc::channel();
    let reader = {
        let (path, done, reads) = (home.path().to_path_buf(), done.clone(), reads.clone());
        std::thread::spawn(move || {
            let (mut ready, mut torn, mut first) = (Some(ready), 0u64, None);
            while !done.load(Ordering::SeqCst) {
                let read = store::read_wallet_file(&path);
                // Counted before `ready` goes out: the writer's `before` includes the first read,
                // so only reads made while the edits run count as progress.
                reads.fetch_add(1, Ordering::SeqCst);
                match read {
                    // The writer starts once a read has succeeded.
                    Ok(_) => {
                        if let Some(ready) = ready.take() {
                            let _ = ready.send(());
                        }
                    }
                    Err(e) => {
                        torn += 1;
                        first.get_or_insert(e.message);
                    }
                }
            }
            (torn, first)
        })
    };
    let stop = StopReader(done);
    reading
        .recv_timeout(Duration::from_secs(30))
        .expect("the reader never read wallet.json whole before the edits");
    let before = reads.load(Ordering::SeqCst);
    for _ in 0..2000 {
        home.edit_wallet(|_| {});
    }
    let during = reads.load(Ordering::SeqCst) - before;
    drop(stop);
    let (torn, first) = reader.join().unwrap();
    assert!(during > 0, "the reader made no read while the edits ran");
    assert_eq!(
        torn, 0,
        "{torn} reads saw a half-written wallet.json ({during} while the edits ran), e.g. {first:?}"
    );
}

#[test]
fn an_invalid_socket_address_never_replaces_a_password_pinned_address() {
    let home = Home::new();
    home.create("a", &[]);
    let dir = tempfile::Builder::new()
        .prefix("zwt-")
        .tempdir_in("/tmp")
        .unwrap();
    let reply = serde_json::json!({"v": 2, "ok": true, "result": {"address": "▀▄".repeat(600)}})
        .to_string();
    let fake = common::daemon_argv_impostor(&home, dir.path(), &[("ATTACKER_REPLY", &reply)]);
    let mut app = locked_ab();
    app.update(Msg::Key(Key::Enter));
    app.update(Msg::Prepared(Ok(())));
    app.update(Msg::PasswordSubmitted);
    app.update(Msg::Checked(Ok((
        wallet::Opened {
            keys: keys_ab().rows,
            active: "b".into(),
            warning: None,
            anchor: Ok(None),
            salted: false,
        },
        Err("no list".into()),
    ))));
    app.update(Msg::Status(app.generation(), Ok(open_status(now() + 3600))));
    let result = block_on(wallet::address(
        home.path(),
        "a",
        chains::find("ethereum").unwrap(),
    ));
    assert!(
        result
            .as_ref()
            .unwrap_err()
            .message
            .contains("not an address")
    );
    app.update(Msg::Address(app.generation(), "a".into(), result));
    let (rows, placed) = checked_frame(&mut screen(80, 24), &app);
    assert!(rows[22].contains("0x7099...79C8 | verified by password"));
    assert!(
        placed
            .iter()
            .any(|p| p.outside.source() == Source::Error && p.outside.text().contains("\\u{2580}"))
    );
    drop(fake);
}

#[test]
fn lock_during_password_check_runs_after_it_so_warmup_cannot_undo_the_lock() {
    let mut app = locked_ab();
    app.update(Msg::Key(Key::Enter));
    app.update(Msg::Prepared(Ok(())));
    app.update(Msg::PasswordSubmitted);
    assert!(app.update(Msg::Key(Key::Char('L'))).is_empty());
    assert!(app.locking());
    app.update(Msg::Key(Key::Char('q')));
    assert!(!app.quitting());
    let work = app.update(Msg::Checked(Err(WalletError::validation("check failed"))));
    assert!(work.contains(&Effect::LockAll));
    assert!(!app.quitting());
    app.update(Msg::Locked(Ok(vec![])));
    assert!(app.quitting());
}

#[test]
fn failed_preparation_dismisses_typeahead_without_opening_a_password_or_signing_task() {
    use zwallet::tui::app::PasswordState;
    let mut app = locked_ab();
    app.update(Msg::Key(Key::Enter));
    app.update(Msg::Prepared(Err(WalletError::validation(
        "session start refused",
    ))));
    assert_eq!(app.password_state(), PasswordState::Failed);
    assert!(app.update(Msg::PasswordSubmitted).is_empty());
    let (rows, _) = checked_frame(&mut screen(80, 24), &app);
    assert!(rows.iter().any(|r| r.contains("session start refused")));
    assert!(rows.iter().any(|r| r.contains("Enter/Esc dismiss")));
    app.update(Msg::PasswordCancelled);
    assert_eq!(app.password_state(), PasswordState::Idle);
}

fn pinned_app_with_checked_list() -> App {
    use zwallet::chains::evm::registry::WHITELIST_RULES;
    use zwallet::core::whitelist::Whitelist;
    let directory = tempfile::Builder::new()
        .prefix("zwt-")
        .tempdir_in("/tmp")
        .unwrap();
    let checked = wallet::CheckedList {
        list: Whitelist::empty(&WHITELIST_RULES),
        source: "whitelist.json",
        fingerprint: ListFiles::read(directory.path()).fingerprint(),
    };
    let mut app = locked_ab();
    app.update(Msg::Key(Key::Enter));
    app.update(Msg::Prepared(Ok(())));
    app.update(Msg::PasswordSubmitted);
    app.update(Msg::Checked(Ok((
        wallet::Opened {
            keys: keys_ab().rows,
            active: "b".into(),
            warning: None,
            anchor: Ok(None),
            salted: false,
        },
        Ok(checked),
    ))));
    app.update(Msg::Status(app.generation(), Ok(locked_status())));
    app
}

#[test]
fn a_second_password_check_cannot_replace_a_lifetime_pin_or_partially_insert_new_pins() {
    let mut app = pinned_app_with_checked_list();
    let mut opened = wallet::Opened {
        keys: keys_ab().rows,
        active: "b".into(),
        warning: None,
        anchor: Ok(None),
        salted: false,
    };
    opened.keys.insert(
        0,
        KeyRow {
            name: "c".into(),
            index: 2,
            address: Some(B.into()),
            active: false,
        },
    );
    opened.keys[1].address = Some(B.into());
    app.update(Msg::Checked(Ok((opened, Err("no list".into())))));
    assert!(
        !app.pinned("c"),
        "validation must finish before inserting any pin"
    );
    assert!(app.checked_list().is_none());
    let (rows, placed) = checked_frame(&mut screen(80, 24), &app);
    assert!(
        placed
            .iter()
            .any(|p| p.outside.text().contains("does not match"))
    );
    assert!(rows[22].contains("0x7099...79C8"));
}

#[test]
fn bad_keys_drop_the_checked_list_and_invalidate_older_status_and_address_reads() {
    for malformed in [false, true] {
        let mut app = pinned_app_with_checked_list();
        let old = app.generation();
        assert!(app.checked_list().is_some());
        let mut keys = keys_ab();
        keys.rows[0].address = Some(B.into());
        let result = if malformed {
            Err(WalletError::validation("keys could not be read"))
        } else {
            Ok(keys)
        };
        app.update(Msg::Keys(old, result));
        assert!(
            app.checked_list().is_none(),
            "a failed key pane cannot retain an actionable checked list"
        );
        assert_ne!(old, app.generation());
        app.update(Msg::Status(old, Ok(open_status(now() + 3600))));
        app.update(Msg::Address(
            old,
            "a".into(),
            Ok((B.into(), Trust::Session)),
        ));
        let (rows, placed) = checked_frame(&mut screen(80, 24), &app);
        assert!(placed.iter().any(|p| p.outside.source() == Source::Error));
        assert!(rows[22].contains("0x7099...79C8"));
        assert!(rows[23].starts_with("locked | "));
    }
}

#[test]
fn an_address_refusal_preserves_the_pin_but_never_stale_lock_certainty() {
    let mut app = pinned_app_with_checked_list();
    let mut changed = open_status(now() + 3600);
    changed.address = Some((Some(A.into()), Trust::Unverified(None)));
    app.update(Msg::Status(app.generation(), Ok(changed)));
    let (rows, _) = checked_frame(&mut screen(80, 24), &app);
    assert!(rows[22].contains("0x7099...79C8"));
    assert!(
        rows[23].starts_with("signing (agent too) until "),
        "{rows:#?}"
    );
    app.update(Msg::Status(
        app.generation(),
        Err(WalletError::validation("status read failed")),
    ));
    let (rows, _) = checked_frame(&mut screen(80, 24), &app);
    assert!(rows[22].contains("0x7099...79C8"));
    assert!(rows[23].starts_with("status unknown: "), "{rows:#?}");
}

// TUI-3+4: pure Msg sequences and the same bounded/placed TestBackend gate as TUI-1.
mod navigation {
    use super::*;
    use alloy::primitives::U256;
    use zeroize::Zeroizing;
    use zwallet::tui::actions::Tab;
    use zwallet::tui::app::{FieldPurpose, PasswordState, Popup, UserText};
    use zwallet::tui::form::Field;

    fn paste(app: &mut App, value: &str) -> Vec<Effect> {
        app.update(Msg::Paste(UserText(Zeroizing::new(value.into()))))
    }
    fn view(app: &App) -> String {
        checked_frame(&mut screen(80, 24), app).0.join("\n")
    }
    fn native(raw: u64) -> chains::Balance {
        chains::Balance {
            symbol: "ETH".into(),
            decimals: 18,
            raw: U256::from(raw),
        }
    }
    fn address_rows(address: &str) -> wallet::Addresses {
        wallet::Addresses {
            key: "a".into(),
            trust: Trust::Locked,
            rows: chains::CHAINS
                .iter()
                .map(|chain| wallet::AddressRow {
                    chain,
                    error: None,
                    address: family_address(chain, address),
                })
                .collect(),
        }
    }
    fn field(app: &App) -> &Field {
        match app.navigation().popup.as_ref().unwrap() {
            Popup::Field { field, .. } => field,
            _ => panic!("expected field"),
        }
    }

    #[test]
    fn nav_all_tabs_hints_and_future_work_fit_one_frame() {
        let mut app = locked_ab();
        for number in '1'..='7' {
            let work = app.update(Msg::Key(Key::Char(number)));
            assert!(work.iter().any(|e| matches!(e, Effect::LoadKeys(_))));
            assert_eq!(app.tab().number(), number as u8 - b'0');
            let text = view(&app);
            assert!(
                text.lines()
                    .nth(21)
                    .unwrap()
                    .trim_end()
                    .ends_with("? actions")
            );
            // Whitelist (#47) and tools (#48) are both implemented: no tab is a placeholder.
            assert!(!text.contains("in a later version"), "{text}");
        }
        app.update(Msg::Key(Key::Char(']')));
        assert_eq!(app.tab(), Tab::Keys);
        app.update(Msg::Key(Key::Char('[')));
        assert_eq!(app.tab(), Tab::Session);
    }

    /// D1: the chain picker and a read-only lines popup are lists, not text fields: the global
    /// tab keys close them and switch tabs.
    #[test]
    fn nav_chain_picker_and_lines_popup_pass_the_global_tab_keys() {
        let mut app = locked_ab();
        app.update(Msg::Key(Key::Char('c')));
        assert!(matches!(app.navigation().popup, Some(Popup::Chains { .. })));
        app.update(Msg::Key(Key::Char('2')));
        assert_eq!(app.tab(), Tab::Wallet, "{}", view(&app));
        assert!(app.navigation().popup.is_none());
        app.update(Msg::Key(Key::Char('4')));
        app.update(Msg::Key(Key::Char('Q')));
        assert!(matches!(app.navigation().popup, Some(Popup::Lines { .. })));
        app.update(Msg::Key(Key::Char(']')));
        assert_eq!(app.tab(), Tab::Whitelist, "{}", view(&app));
        assert!(app.navigation().popup.is_none());
    }

    /// The Keys tab's hint names every key it acts on, `u unlock` included (#43's merge dropped
    /// it), within 80 columns, and `u` there opens the duration field.
    #[test]
    fn nav_keys_idle_hint_names_unlock_and_u_opens_the_duration_field() {
        let mut app = locked_ab();
        app.update(Msg::Key(Key::Char('1')));
        assert_eq!(app.tab(), Tab::Keys);
        let text = view(&app);
        let hint = text.lines().nth(21).unwrap().trim_end();
        assert!(hint.len() <= 80, "{hint}");
        for control in [
            "Enter open",
            "n new",
            "B backup",
            "V verify",
            "A repair",
            "s active",
            "u unlock",
        ] {
            assert!(hint.contains(control), "missing {control}: {hint}");
        }
        assert!(hint.ends_with("? actions"), "{hint}");
        app.update(Msg::Key(Key::Char('u')));
        assert!(matches!(
            app.navigation().popup,
            Some(Popup::Field {
                purpose: FieldPurpose::Duration,
                ..
            })
        ));
    }

    #[test]
    fn nav_tools_idle_hint_fits_and_opens_the_action_menu() {
        let mut app = locked_ab();
        app.update(Msg::Key(Key::Char('6')));
        assert_eq!(app.tools().stage, zwallet::tui::app::tools::Stage::Menu);
        let text = view(&app);
        let hint = text.lines().nth(21).unwrap().trim_end();
        assert!(hint.len() <= 80, "{hint}");
        for control in [
            "Enter open",
            "C call",
            "i invoke",
            "d raw",
            "m message",
            "t token",
        ] {
            assert!(hint.contains(control), "missing {control}: {hint}");
        }
        assert!(hint.ends_with("? actions"), "{hint}");
        assert!(!text.contains("in a later version"), "{text}");
        app.update(Msg::Key(Key::Char('?')));
        assert!(matches!(app.navigation().popup, Some(Popup::Menu(_))));
        assert!(view(&app).contains("actions (55)"));
    }

    #[test]
    fn nav_menu_keeps_input_focused_and_disabled_actions_inert() {
        let mut app = locked_ab();
        app.update(Msg::Key(Key::Char('?')));
        let text = view(&app);
        assert!(text.contains("actions (55)"));
        assert!(text.contains(&zwallet::version_line()));
        assert!(!text.lines().nth(21).unwrap().contains("? actions"));
        // Every action is implemented in this tree; a Tron row on an EVM chain stays disabled.
        paste(&mut app, "tron tx");
        assert!(app.update(Msg::Key(Key::Enter)).is_empty());
        assert!(view(&app).contains("select Tron with c"));
        for c in ['q', '?', '1'] {
            app.update(Msg::Key(Key::Char(c)));
        }
        match app.navigation().popup.as_ref().unwrap() {
            Popup::Menu(menu) => assert_eq!(menu.filter().value(), "tron txq?1"),
            _ => panic!("menu should stay open"),
        }
        paste(&mut app, "\u{1b}evil\u{2588}");
        assert!(!view(&app).contains("evil"));
        assert!(!app.quitting());
        app.update(Msg::Key(Key::Esc));
        assert!(app.navigation().popup.is_none());
    }

    #[test]
    fn nav_balance_rows_omit_zero_tokens_and_bound_hostile_errors() {
        let mut app = pinned_app_with_checked_list();
        let tokens = vec![
            chains::TokenBalance {
                address: A.into(),
                symbol: "USDC".into(),
                decimals: 6,
                raw: Ok(U256::from(2_000_000)),
            },
            chains::TokenBalance {
                address: A.into(),
                symbol: "DAI".into(),
                decimals: 18,
                raw: Ok(U256::ZERO),
            },
            chains::TokenBalance {
                address: A.into(),
                symbol: "BAD".into(),
                decimals: 6,
                raw: Err(WalletError::rpc("\u{1b}\u{2588}".repeat(600))),
            },
        ];
        app.update(Msg::Balances(
            app.generation(),
            "a".into(),
            Ok(wallet::ChainBalances {
                activated: None,
                warning: None,
                chain: chains::find("ethereum").unwrap(),
                key: "a".into(),
                address: A.into(),
                trust: Trust::Locked,
                native: native(1_000_000_000_000_000_000),
                tokens,
            }),
        ));
        let text = view(&app);
        assert!(text.contains("1 ETH"));
        assert!(text.contains("2 USDC"));
        assert!(!text.contains("DAI"));
        assert!(text.contains("BAD error:"));
    }

    #[test]
    fn nav_all_networks_render_one_row_each_and_failure_summary() {
        let mut app = pinned_app_with_checked_list();
        app.update(Msg::Key(Key::Char('a')));
        let mut chains: Vec<_> = chains::CHAINS.iter().collect();
        chains.sort_by_key(|chain| chain.is_testnet());
        let networks = chains
            .iter()
            .enumerate()
            .map(|(i, chain)| wallet::NetworkBalance {
                chain,
                address: Some(family_address(chain, A)),
                balances: if i < 2 {
                    Ok(wallet::read::NetworkRead {
                        native: native(1),
                        tokens: vec![],
                        activated: None,
                        warning: None,
                    })
                } else {
                    Err(WalletError::rpc("unavailable"))
                },
            })
            .collect();
        app.update(Msg::EveryBalances(
            app.generation(),
            "a".into(),
            Ok(wallet::EveryBalance {
                key: "a".into(),
                address: A.into(),
                trust: Trust::Locked,
                networks,
            }),
        ));
        let text = view(&app);
        for chain in &chains {
            assert!(text.contains(&format!("{}:", chain.key)));
        }
        assert!(text.contains(&format!(
            "2 of {} networks have a balance; {} did not answer",
            chains.len(),
            chains.len() - 2
        )));
        assert!(text.find("sepolia:").unwrap() > text.find("base:").unwrap());
    }

    #[test]
    fn nav_chain_switch_reloads_every_pane_and_discards_old_response() {
        let mut app = pinned_app_with_checked_list();
        let old = app.generation();
        app.update(Msg::Key(Key::Char('c')));
        let base = chains::CHAINS
            .iter()
            .position(|chain| chain.key == "base")
            .unwrap();
        for _ in 0..base {
            app.update(Msg::Key(Key::Down));
        }
        let effects = app.update(Msg::Key(Key::Enter));
        assert_eq!(app.net().chain.key, "base");
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::LoadBalances(_, _)))
        );
        assert!(effects.iter().any(|e| matches!(e, Effect::LoadKeys(_))));
        assert!(effects.iter().any(|e| matches!(e, Effect::LoadStatus(_))));
        app.update(Msg::Balances(
            old,
            "a".into(),
            Err(WalletError::rpc("stale reply")),
        ));
        assert!(!view(&app).contains("stale reply"));
    }

    #[test]
    fn nav_rpc_is_redacted_checked_before_adoption_and_cancel_discards_reply() {
        let mut app = locked_ab();
        let original = app.net().rpc_url.clone();
        app.update(Msg::Key(Key::Char('c')));
        app.update(Msg::Key(Key::Char('e')));
        let url = "https://user:password@rpc.example.com/private-secret?q=query-secret";
        paste(&mut app, url);
        let text = view(&app);
        for secret in ["password", "private-secret", "query-secret"] {
            assert!(!text.contains(secret));
        }
        assert!(text.contains("rpc.example.com"));
        let effects = app.update(Msg::Key(Key::Enter));
        let (request, choice) = match effects.into_iter().next().unwrap() {
            Effect::CheckRpc(request, choice) => (request, choice),
            _ => panic!("check first"),
        };
        assert_eq!(app.net().rpc_url, original);
        app.update(Msg::RpcChecked(
            request,
            Err(WalletError::rpc("wrong chain")),
        ));
        assert_eq!(app.net().rpc_url, original);
        assert!(view(&app).contains("wrong chain"));
        app.update(Msg::Key(Key::Esc));
        app.update(Msg::Key(Key::Char('c')));
        app.update(Msg::Key(Key::Char('e')));
        paste(&mut app, url);
        let request = match app.update(Msg::Key(Key::Enter)).remove(0) {
            Effect::CheckRpc(r, _) => r,
            _ => unreachable!(),
        };
        app.update(Msg::Key(Key::Esc));
        app.update(Msg::RpcChecked(request, Ok(choice.clone())));
        assert_eq!(app.net().rpc_url, original);
        app.update(Msg::Key(Key::Char('c')));
        app.update(Msg::Key(Key::Char('e')));
        paste(&mut app, url);
        let request = match app.update(Msg::Key(Key::Enter)).remove(0) {
            Effect::CheckRpc(r, _) => r,
            _ => unreachable!(),
        };
        app.update(Msg::RpcChecked(request, Ok(choice)));
        assert_eq!(app.net().rpc_url, url);
        assert_eq!(super::app().net().rpc_url, original);
    }

    #[test]
    fn nav_receive_shows_full_pinned_address_and_rejects_later_mismatch() {
        let mut app = pinned_app_with_checked_list();
        app.update(Msg::Key(Key::Char('4')));
        app.update(Msg::Addresses(
            app.generation(),
            "a".into(),
            Ok(address_rows(A)),
        ));
        let text = view(&app);
        assert!(text.contains(A));
        assert!(text.contains("verified by password"));
        for chain in chains::CHAINS {
            assert!(text.contains(&format!("{}:", chain.key)));
        }
        app.update(Msg::Key(Key::Char('Q')));
        assert!(view(&app).contains("zwallet address shows it as a QR code"));
        app.update(Msg::Key(Key::Esc));
        app.update(Msg::Addresses(
            app.generation(),
            "a".into(),
            Ok(address_rows(B)),
        ));
        assert!(app.checked_list().is_none());
        let text = view(&app);
        assert!(text.contains("does not match"));
        assert!(text.contains("0x7099...79C8"));
    }

    #[test]
    fn nav_verify_is_read_only_and_duration_is_typed_before_preparing() {
        let mut app = locked_ab();
        assert!(matches!(
            app.update(Msg::Key(Key::Char('V'))).as_slice(),
            [Effect::PrepareRead]
        ));
        app.update(Msg::Prepared(Ok(())));
        assert!(view(&app).contains("Verify every key"));
        assert!(matches!(
            app.update(Msg::PasswordSubmitted).as_slice(),
            [Effect::CheckReadPassword {
                all: true,
                repair: false,
                ..
            }]
        ));
        app.update(Msg::Checked(Err(WalletError::validation("test refusal"))));
        app.update(Msg::Key(Key::Char('u')));
        paste(&mut app, "10m");
        assert_eq!(field(&app).value(), "10m");
        assert!(
            matches!(app.update(Msg::Key(Key::Enter)).as_slice(), [Effect::PrepareDuration(ttl)] if *ttl == Duration::from_secs(600))
        );
        assert_eq!(app.password_state(), PasswordState::Preparing);
    }

    #[test]
    fn nav_rpc_field_q_question_digits_are_input_and_start_flag_is_immutable() {
        let mut app = locked_ab();
        app.update(Msg::Key(Key::Char('c')));
        app.update(Msg::Key(Key::Char('e')));
        for c in ['q', '?', '1'] {
            app.update(Msg::Key(Key::Char(c)));
        }
        assert_eq!(field(&app).value(), "q?1");
        assert!(matches!(
            app.navigation().popup,
            Some(Popup::Field {
                purpose: FieldPurpose::Rpc(_),
                ..
            })
        ));
        assert!(!app.quitting());
        app.update(Msg::Key(Key::Esc));
        app.update(Msg::Key(Key::Char('?')));
        paste(&mut app, "unlock --allow-ptrace-scope-0");
        assert!(app.update(Msg::Key(Key::Enter)).is_empty());
        assert!(!app.navigation().allow_ptrace_scope_0);
        assert!(view(&app).contains("zwallet tui --allow-ptrace-scope-0"));
    }

    #[test]
    fn nav_session_reports_only_untrusted_metadata_and_bounds_pid_home_values() {
        let mut app = locked_ab();
        app.configure(std::path::Path::new("/tmp/zwt-public\u{1b}home"), true);
        app.update(Msg::Key(Key::Char('7')));
        let mut status = open_status(now() + 600);
        status.other_daemon_pids = (1000..1300).collect();
        status.serving.as_mut().unwrap().whitelist =
            serde_json::json!({"gen":9,"verified":true,"entries":["SECRET_ENTRY_NAME"]});
        app.update(Msg::Status(app.generation(), Ok(status)));
        let text = view(&app);
        assert!(text.contains("ptrace_scope 0 accepted"));
        assert!(text.contains("reports gen 9, 1 entries (not checked here)"));
        assert!(text.contains("292 more"));
        assert!(text.contains("1007"));
        assert!(!text.contains("1008"));
        assert!(!text.contains("SECRET_ENTRY_NAME"));
        assert!(text.contains("\\u{1b}"));
    }

    #[test]
    fn nav_verified_large_index_keeps_full_address_and_label() {
        let mut app = locked_ab();
        let mut keys = keys_ab();
        keys.rows[1].name = "n".repeat(32);
        keys.rows[1].index = 1000;
        keys.active = keys.rows[1].name.clone();
        app.update(Msg::Key(Key::Char('V')));
        app.update(Msg::Prepared(Ok(())));
        app.update(Msg::PasswordSubmitted);
        app.update(Msg::Checked(Ok((
            wallet::Opened {
                keys: keys.rows.clone(),
                active: keys.active.clone(),
                warning: None,
                anchor: Ok(None),
                salted: false,
            },
            Err("no checked list".into()),
        ))));
        app.update(Msg::Keys(app.generation(), Ok(keys)));
        let text = view(&app);
        let row = text.lines().find(|line| line.contains("1000")).unwrap();
        assert!(row.contains(B) && row.contains(" verified"));
        assert!(row.contains("..."));
    }

    #[test]
    fn nav_inconsistent_password_family_result_pins_nothing() {
        let mut app = locked_ab();
        app.update(Msg::Key(Key::Char('V')));
        app.update(Msg::Prepared(Ok(())));
        app.update(Msg::PasswordSubmitted);
        let checked = wallet::PasswordCheck {
            opened: wallet::Opened {
                keys: keys_ab().rows,
                active: "b".into(),
                warning: None,
                anchor: Ok(None),
                salted: false,
            },
            addresses: vec![
                wallet::VerifiedAddress {
                    key: "a".into(),
                    chain: "ethereum",
                    family: "evm",
                    address: A.into(),
                },
                wallet::VerifiedAddress {
                    key: "a".into(),
                    chain: "base",
                    family: "evm",
                    address: B.into(),
                },
                wallet::VerifiedAddress {
                    key: "b".into(),
                    chain: "ethereum",
                    family: "evm",
                    address: B.into(),
                },
            ],
            repairs: vec![],
        };
        app.update(Msg::Verified(Ok((checked, Err("no checked list".into())))));
        assert!(!app.pinned("a") && !app.pinned("b"));
        assert!(!app.family_pinned("a", chains::find("base").unwrap()));
        assert!(view(&app).contains("family addresses disagree"));
    }

    #[test]
    fn nav_repair_lists_hostile_old_cache_as_text_and_scrolls() {
        let mut app = locked_ab();
        app.update(Msg::Key(Key::Char('A')));
        app.update(Msg::Prepared(Ok(())));
        app.update(Msg::PasswordSubmitted);
        let checked = wallet::PasswordCheck {
            opened: wallet::Opened {
                keys: keys_ab().rows,
                active: "b".into(),
                warning: None,
                anchor: Ok(None),
                salted: false,
            },
            addresses: vec![
                wallet::VerifiedAddress {
                    key: "a".into(),
                    chain: "ethereum",
                    family: "evm",
                    address: A.into(),
                },
                wallet::VerifiedAddress {
                    key: "b".into(),
                    chain: "ethereum",
                    family: "evm",
                    address: B.into(),
                },
            ],
            repairs: (0..10)
                .map(|_| store::CacheRepair {
                    key: "a".into(),
                    chain: "ethereum".into(),
                    was: "\u{1b}[2J".to_string() + &"\u{2588}".repeat(600),
                    now: A.into(),
                })
                .collect(),
        };
        app.update(Msg::Verified(Ok((checked, Err("no checked list".into())))));
        assert!(view(&app).contains("was (do not fund):"));
        for _ in 0..27 {
            app.update(Msg::Key(Key::Down));
        }
        assert_eq!(app.navigation().popup_scroll, 27);
        assert!(view(&app).contains(A));
        app.update(Msg::Key(Key::Esc));
        assert_eq!(app.navigation().popup_scroll, 0);
    }
}

/// Read panes must never make a process test contact a public endpoint.
fn tui_without_public_rpc(home: &Home) -> std::process::Command {
    let mut command = home.command(&["tui", "--allow-ptrace-scope-0"]);
    for chain in chains::CHAINS {
        command.env(chain.rpc_env_var(), "http://127.0.0.1:1");
    }
    command
}

#[test]
fn t14_on_a_pty_duration_unlock_matches_status_expiry() {
    let home = Home::new();
    home.create("a", &[]);
    let mut tui = PtyChild::spawn_full_screen(tui_without_public_rpc(&home), PTY_SIZE);
    tui.wait_frame(PTY_WAIT, |f| bar(f).1.starts_with("locked | "));
    tui.send(b"7u10m\r");
    tui.wait_frame(PTY_WAIT, |f| {
        f.rows.iter().any(|r| r.contains("Wallet password:"))
    });
    let before = now();
    tui.send(format!("{PW}\r").as_bytes());
    let unlocked = tui.wait_frame(PTY_WAIT, |f| {
        bar(f).1.starts_with("signing (agent too) until ")
            && bar(f).0.contains("verified by password")
    });
    let status = home.run(&["status", "--json"]).ok();
    let expires = parse_rfc3339_utc(status["expiresAt"].as_str().unwrap()).unwrap();
    assert!((before + 590..=now() + 610).contains(&expires));
    assert!(
        bar(&unlocked).1.contains(
            Outside::expiry(status["expiresAt"].as_str().unwrap(), now())
                .cut(17)
                .text()
        )
    );
    quit(&mut tui);
}

#[test]
fn t19_on_a_pty_verify_all_refuses_bad_cache_then_repair_agrees_with_cli() {
    let home = Home::new();
    home.create("a", &[]);
    home.create("b", &[]);
    let mut tui = PtyChild::spawn_full_screen(tui_without_public_rpc(&home), PTY_SIZE);
    tui.wait_frame(PTY_WAIT, |f| f.rows.iter().any(|r| r.contains("* b ")));
    tui.send(b"V");
    tui.wait_frame(PTY_WAIT, |f| {
        f.rows.iter().any(|r| r.contains("Wallet password:"))
    });
    tui.send(format!("{PW}\r").as_bytes());
    tui.wait_frame(PTY_WAIT, |f| {
        f.rows
            .iter()
            .filter(|r| r.contains(" verified") && r.contains("0x"))
            .count()
            >= 2
    });
    assert_eq!(home.run(&["status", "--json"]).ok()["locked"], true);
    home.edit_wallet(|v| v["keys"][1]["addresses"]["ethereum"] = serde_json::json!(A));
    tui.send(b"V");
    tui.wait_frame(PTY_WAIT, |f| {
        f.rows.iter().any(|r| r.contains("Wallet password:"))
    });
    tui.send(format!("{PW}\r").as_bytes());
    tui.wait_frame(PTY_WAIT, |f| {
        f.rows.iter().any(|r| r.contains("A on 1 keys repairs"))
    });
    tui.send(b"A");
    tui.wait_frame(PTY_WAIT, |f| {
        f.rows.iter().any(|r| r.contains("Wallet password:"))
    });
    tui.send(format!("{PW}\r").as_bytes());
    tui.wait_frame(PTY_WAIT, |f| {
        f.rows.iter().any(|r| r.contains("was (do not fund):"))
    });
    tui.send(b"\x1b");
    let repaired_frame = tui.wait_frame(PTY_WAIT, |f| {
        f.rows
            .iter()
            .filter(|r| r.contains(" verified") && r.contains("0x"))
            .count()
            >= 2
    });
    let checked = home
        .run_stdin(
            &["list", "--verify", "--password-stdin", "--json"],
            &format!("{PW}\n"),
        )
        .ok();
    assert_eq!(checked["verified"], true);
    for row in checked["keys"].as_array().unwrap() {
        let address = row["address"].as_str().unwrap();
        assert!(
            repaired_frame
                .rows
                .iter()
                .any(|line| line.contains(address))
        );
    }
    assert_eq!(home.run(&["status", "--json"]).ok()["locked"], true);
    quit(&mut tui);
}

mod creation {
    use super::*;
    use zeroize::Zeroizing;
    use zwallet::tui::app::create::{CreateEffect, CreateMsg, CreateStage};
    use zwallet::tui::app::{PasswordState, UserText};
    fn paste(app: &mut App, value: &str) {
        app.update(Msg::Paste(UserText(Zeroizing::new(value.into()))));
    }
    fn frame(app: &App) -> String {
        checked_frame(&mut screen(80, 24), app).0.join("\n")
    }
    fn form(app: &mut App, name: &str, first: bool, ttl: &str) -> u64 {
        paste(app, name);
        app.update(Msg::Key(Key::Enter));
        if first {
            app.update(Msg::Key(Key::Enter));
        }
        paste(app, ttl);
        app.update(Msg::Key(Key::Enter));
        paste(app, name);
        match app.update(Msg::Key(Key::Enter)).remove(0) {
            Effect::Create(CreateEffect::Prepare {
                id,
                name: actual,
                words,
                ttl: duration,
            }) => {
                assert_eq!(actual, name);
                assert_eq!(words, first.then_some(24));
                assert_eq!(
                    duration,
                    if ttl.is_empty() {
                        None
                    } else {
                        Some(Duration::from_secs(600))
                    }
                );
                id
            }
            _ => panic!("must prepare before password"),
        }
    }
    fn created(name: &str, first: bool) -> wallet::Created {
        wallet::Created {
            name: name.into(),
            index: if first { 0 } else { 2 },
            addresses: chains::CHAINS
                .iter()
                .map(|c| (c.key.to_string(), family_address(c, A)))
                .collect(),
            active: name.into(),
            master_created: first,
            verified: true,
            session: None,
            backup_reminder: first.then_some("Back up the new wallet before funding it."),
        }
    }
    #[test]
    fn create_empty_home_opens_first_form_and_password_confirmation_precedes_submission() {
        let mut app = app();
        app.update(Msg::Keys(
            app.generation(),
            Err(WalletError::new(ErrorCode::WalletNotFound, "no wallet")),
        ));
        assert!(matches!(app.creation().stage, CreateStage::Form));
        assert!(frame(&app).contains("Create the first wallet and key 0"));
        let id = form(&mut app, "main", true, "");
        assert_eq!(app.password_state(), PasswordState::Preparing);
        app.update(Msg::Create(CreateMsg::Prepared(id, Ok(true))));
        assert!(frame(&app).contains("New wallet password"));
        assert!(app.update(Msg::PasswordSubmitted).is_empty());
        app.update(Msg::Create(CreateMsg::PasswordAgain));
        assert!(frame(&app).contains("Repeat the new wallet password"));
        app.update(Msg::Create(CreateMsg::PasswordMismatch));
        assert!(matches!(app.creation().stage, CreateStage::PasswordFirst));
        assert!(frame(&app).contains("Both cleared"));
        app.update(Msg::Create(CreateMsg::PasswordAgain));
        assert_eq!(
            app.update(Msg::PasswordSubmitted),
            vec![Effect::Create(CreateEffect::Submit { id })]
        );
    }
    #[test]
    fn create_additional_key_names_active_change_and_asks_once_with_typed_unlock() {
        let mut app = locked_ab();
        app.update(Msg::Key(Key::Char('n')));
        assert!(frame(&app).contains("new key becomes active"));
        let id = form(&mut app, "trading", false, "10m");
        app.update(Msg::Create(CreateMsg::Prepared(id, Ok(false))));
        assert!(matches!(
            app.creation().stage,
            CreateStage::PasswordExisting
        ));
        assert!(frame(&app).contains("Wallet password"));
        assert_eq!(
            app.update(Msg::PasswordSubmitted),
            vec![Effect::Create(CreateEffect::Submit { id })]
        );
        app.update(Msg::Create(CreateMsg::Finished(
            id,
            Ok(created("trading", false)),
        )));
        assert_eq!(app.open_key(), Some("trading"));
        assert!(app.pinned("trading"));
        for chain in chains::CHAINS {
            assert!(app.family_pinned("trading", chain));
        }
        assert!(frame(&app).contains("active, verified by password"));
    }
    #[test]
    fn create_submission_cannot_be_cancelled_and_quit_waits_for_failure_and_refresh() {
        let mut app = locked_ab();
        app.update(Msg::Key(Key::Char('n')));
        let id = form(&mut app, "trading", false, "");
        app.update(Msg::Create(CreateMsg::Prepared(id, Ok(false))));
        app.update(Msg::PasswordSubmitted);
        app.update(Msg::PasswordCancelled);
        app.update(Msg::Key(Key::Esc));
        assert!(matches!(app.creation().stage, CreateStage::Submitting));
        app.update(Msg::Key(Key::Char('q')));
        assert!(!app.quitting());
        let work = app.update(Msg::Create(CreateMsg::Finished(
            id,
            Err(WalletError::validation("unlock failed after creation")),
        )));
        assert!(work.iter().any(|e| matches!(e, Effect::LoadKeys(_))));
        assert!(work.iter().any(|e| matches!(e, Effect::LoadStatus(_))));
        assert!(app.quitting());
        assert!(app.take_quit_error().is_some());
    }
    #[test]
    fn create_lock_waits_until_optional_unlock_and_creation_finish() {
        let mut app = locked_ab();
        app.update(Msg::Key(Key::Char('n')));
        let id = form(&mut app, "trading", false, "10m");
        app.update(Msg::Create(CreateMsg::Prepared(id, Ok(false))));
        app.update(Msg::PasswordSubmitted);
        assert!(app.update(Msg::Key(Key::Char('L'))).is_empty());
        assert!(app.locking());
        let work = app.update(Msg::Create(CreateMsg::Finished(
            id,
            Ok(created("trading", false)),
        )));
        assert!(work.contains(&Effect::LockAll));
    }
    /// Round-2 audit: creation is a modal. Its fields keep digits as input, and its result
    /// stays (it is not a dead end) until Enter or Esc closes it; then the tab keys act.
    #[test]
    fn create_cannot_be_stranded_by_a_tab_key() {
        let mut app = locked_ab();
        app.update(Msg::Key(Key::Char('n')));
        app.update(Msg::Key(Key::Char('2')));
        assert_eq!(app.tab(), zwallet::tui::actions::Tab::Keys);
        assert!(app.creation().stage.active());
        app.update(Msg::Key(Key::Backspace));
        let id = form(&mut app, "trading", false, "10m");
        app.update(Msg::Create(CreateMsg::Prepared(id, Ok(false))));
        app.update(Msg::PasswordSubmitted);
        app.update(Msg::Create(CreateMsg::Finished(
            id,
            Ok(created("trading", false)),
        )));
        assert!(matches!(app.creation().stage, CreateStage::Done(_)));
        app.update(Msg::Key(Key::Char('2')));
        assert!(matches!(app.creation().stage, CreateStage::Done(_)));
        assert!(frame(&app).contains("trading"));
        app.update(Msg::Key(Key::Enter));
        assert!(!app.creation().stage.active());
        app.update(Msg::Key(Key::Char('2')));
        assert_eq!(app.tab(), zwallet::tui::actions::Tab::Wallet);
    }
    #[test]
    fn create_completed_view_keeps_emergency_lock_available() {
        let mut app = locked_ab();
        app.update(Msg::Key(Key::Char('n')));
        let id = form(&mut app, "trading", false, "10m");
        app.update(Msg::Create(CreateMsg::Prepared(id, Ok(false))));
        app.update(Msg::PasswordSubmitted);
        app.update(Msg::Create(CreateMsg::Finished(
            id,
            Ok(created("trading", false)),
        )));
        assert!(matches!(app.creation().stage, CreateStage::Done(_)));
        assert_eq!(app.update(Msg::Key(Key::Char('L'))), vec![Effect::LockAll]);
        assert!(app.locking());
        assert!(matches!(app.creation().stage, CreateStage::Done(_)));
        frame(&app);
        app.update(Msg::Key(Key::Char('q')));
        assert!(!app.quitting());
        app.update(Msg::Locked(Ok(vec![])));
        assert!(app.quitting());
    }
    #[test]
    fn create_form_bounds_input_name_confirmation_and_all_five_word_counts() {
        let mut app = app();
        app.update(Msg::Key(Key::Char('n')));
        paste(&mut app, "\u{1b}[2J\u{2588}");
        assert_eq!(app.creation().form.as_ref().unwrap().fields[0].value(), "");
        paste(&mut app, "main");
        app.update(Msg::Key(Key::Enter));
        for expected in [12, 15, 18, 21, 24] {
            app.update(Msg::Key(Key::Right));
            assert_eq!(
                app.creation().form.as_ref().unwrap().fields[1].value(),
                expected.to_string()
            );
            frame(&app);
        }
        app.update(Msg::Key(Key::Enter));
        app.update(Msg::Key(Key::Enter));
        paste(&mut app, "wrong");
        assert!(app.update(Msg::Key(Key::Enter)).is_empty());
        assert!(frame(&app).contains("exact new key name"));
        app.update(Msg::Key(Key::Esc));
        assert!(!app.creation().stage.active());
    }
    #[test]
    fn create_success_reminds_backup_without_secret_carriers_and_backup_has_no_effect() {
        let mut app = app();
        app.update(Msg::Key(Key::Char('n')));
        let id = form(&mut app, "main", true, "");
        app.update(Msg::Create(CreateMsg::Prepared(id, Ok(true))));
        app.update(Msg::Create(CreateMsg::PasswordAgain));
        app.update(Msg::PasswordSubmitted);
        let public = created("main", true);
        let msg = Msg::Create(CreateMsg::Finished(id, Ok(public.clone())));
        let debug = format!("{public:?} {msg:?}");
        assert!(!debug.contains(PW));
        assert!(!debug.contains("mnemonic"));
        app.update(msg);
        assert!(frame(&app).contains("Back up this new wallet before funding it"));
        assert!(app.update(Msg::Key(Key::Char('B'))).is_empty());
        assert!(frame(&app).contains("run zwallet backup in a shell to see the recovery phrase"));
    }
    #[test]
    fn create_cannot_replace_an_existing_verified_derivation_index() {
        let mut app = pinned_app_with_checked_list();
        app.update(Msg::Key(Key::Char('1')));
        app.update(Msg::Key(Key::Char('n')));
        let id = form(&mut app, "trading", false, "");
        app.update(Msg::Create(CreateMsg::Prepared(id, Ok(false))));
        app.update(Msg::PasswordSubmitted);
        app.update(Msg::Create(CreateMsg::Finished(
            id,
            Ok(created("a", false)),
        )));
        assert!(frame(&app).contains("index conflicts"));
        assert_eq!(app.verified_key().unwrap().index, 0);
    }
    #[test]
    fn create_existing_wallet_disappearance_never_silently_creates_a_new_master() {
        let mut app = locked_ab();
        app.update(Msg::Keys(
            app.generation(),
            Err(WalletError::new(
                ErrorCode::WalletNotFound,
                "missing wallet",
            )),
        ));
        assert!(!app.creation().stage.active());
        app.update(Msg::Key(Key::Char('n')));
        let id = form(&mut app, "trading", false, "");
        app.update(Msg::Create(CreateMsg::Prepared(id, Ok(true))));
        assert_eq!(app.password_state(), PasswordState::Failed);
        assert!(frame(&app).contains("appeared or disappeared"));
        assert!(app.update(Msg::PasswordSubmitted).is_empty());
    }
}

#[test]
fn t30_on_a_pty_create_key_activates_it_and_optional_unlock_uses_typed_duration() {
    let home = Home::new();
    home.create("main", &[]);
    let mut tui = PtyChild::spawn_full_screen(tui_without_public_rpc(&home), PTY_SIZE);
    tui.wait_frame(PTY_WAIT, |f| f.rows.iter().any(|r| r.contains("* main ")));
    tui.send(b"ntrading\r10m\rtrading\r");
    tui.wait_frame(PTY_WAIT, |f| {
        f.rows
            .iter()
            .any(|r| r.contains("Wallet password: verify and create"))
    });
    let before = now();
    tui.send(format!("{PW}\r").as_bytes());
    let created = tui.wait_frame(PTY_WAIT, |f| {
        f.rows
            .iter()
            .any(|r| r.contains("Created trading") && r.contains("verified by password"))
    });
    assert!(created.rows.iter().any(|r| r.contains("active")));
    let list = home.run(&["list", "--json"]).ok();
    assert_eq!(list["active"], "trading");
    let status = home.run(&["status", "--json"]).ok();
    let expires = parse_rfc3339_utc(status["expiresAt"].as_str().unwrap()).unwrap();
    assert!((before + 590..=now() + 610).contains(&expires));
    quit(&mut tui);
    assert!(
        !String::from_utf8_lossy(&tui.output()).contains(PW),
        "password reached a TUI capture"
    );
}

#[test]
fn t31_on_a_pty_first_wallet_confirms_password_and_never_displays_recovery_words() {
    let home = Home::new();
    let mut tui = PtyChild::spawn_full_screen(tui_without_public_rpc(&home), PTY_SIZE);
    tui.wait_frame(PTY_WAIT, |f| {
        f.rows
            .iter()
            .any(|r| r.contains("Create the first wallet and key 0"))
    });
    // Name, default 24 words, no unlock, then the explicit name confirmation.
    tui.send(b"main\r\r\rmain\r");
    tui.wait_frame(PTY_WAIT, |f| {
        f.rows.iter().any(|r| r.contains("New wallet password"))
    });
    tui.send(format!("{PW}\r").as_bytes());
    tui.wait_frame(PTY_WAIT, |f| {
        f.rows
            .iter()
            .any(|r| r.contains("Repeat the new wallet password"))
    });
    tui.send(b"different-fixture-password\r");
    tui.wait_frame(PTY_WAIT, |f| {
        f.rows.iter().any(|r| r.contains("Both cleared"))
    });
    assert!(!home.wallet_json().exists());
    tui.send(format!("{PW}\r").as_bytes());
    tui.wait_frame(PTY_WAIT, |f| {
        f.rows
            .iter()
            .any(|r| r.contains("Repeat the new wallet password"))
    });
    tui.send(format!("{PW}\r").as_bytes());
    tui.wait_frame(PTY_WAIT, |f| {
        f.rows
            .iter()
            .any(|r| r.contains("Back up this new wallet before funding it"))
    });
    let list = home.run(&["list", "--json"]).ok();
    assert_eq!(list["keys"][0]["name"], "main");
    assert_eq!(list["keys"][0]["index"], 0);
    quit(&mut tui);
    let captured = zeroize::Zeroizing::new(String::from_utf8_lossy(&tui.output()).into_owned());
    assert!(!captured.contains(PW), "password reached a TUI capture");
    // Read the generated phrase only through the existing terminal-only CLI after TUI exit.
    // No assertion formats this private capture or a phrase word.
    let mut backup = PtyChild::spawn_on_terminal(home.command(&["backup"]));
    backup.wait_for("Wallet password:", PTY_WAIT);
    backup.type_line(PW);
    let (output, text) = backup.finish_terminal();
    assert!(output.status.success(), "fixture backup failed");
    let text = zeroize::Zeroizing::new(text);
    let words: Vec<&str> = text
        .lines()
        .filter_map(|line| {
            let (number, word) = line.trim().split_once(". ")?;
            number.parse::<usize>().ok()?;
            Some(word.trim())
        })
        .collect();
    assert_eq!(
        words.len(),
        24,
        "fixture backup did not yield the requested word count"
    );
    let tokens: Vec<&str> = captured
        .split(|c: char| !c.is_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    for three in words.windows(3) {
        assert!(
            !tokens.windows(3).any(|shown| shown == three),
            "recovery words reached the TUI"
        );
    }
}
