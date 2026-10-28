# v0.2 TUI — units and order

Status: **planned**, 2026-10-27, revised after review rounds 1 and 2 the same day; TUI-1 split
into three PRs on 2026-10-28 (§TUI-1 split); rewritten on 2026-10-28 (v2) for every CLI function
([`../03-tui.md`](../03-tui.md) §Coverage matrix, §Changes on 2026-10-28, v2). Fourteen units,
TUI-1a … TUI-12, each one PR, ordered by what the owner can do once it is merged. Each closes the
acceptance IDs of [`../03-tui.md`](../03-tui.md) §Acceptance listed for it; every ID T1–T41 (45
with T2b, T3b, T6b, T6c) is closed by exactly one unit. The library entry points are the ones
[`flow.md`](flow.md) names in its "Split" column and, for the functions added on 2026-10-28, its
§Functions added on 2026-10-28.

**Counting rule.** "Est. lines" is the changed lines of the PR (insertions plus deletions in
`git diff --stat`), tests included, with two exceptions: `Cargo.lock` is not counted (it is
generated; TUI-1's dependency change adds a few hundred lines there), and a block moved unchanged
from `src/commands.rs` into `src/wallet.rs` counts once (`git diff --color-moved` shows it as a
move). Test fixtures are single-line JSON files under `tests/fixtures/` and are counted. The cap
is about 800; TUI-1b, TUI-3, TUI-5, TUI-6, TUI-7 and TUI-11 are estimated at it. Every estimate
of a unit not yet started is an estimate, not a measurement. Line references in text moved from
2026-10-27 are to c7d3f13, as [`flow.md`](flow.md)'s are; those written on 2026-10-28 are to
24f4b5b.

**Every unit:**
- RED first: each test below is written and seen failing before the code it checks.
- Every `TestBackend` test draws through `FilteredBackend` and ends with three checks
  (03-tui.md §Display integrity): zero replaced cells (rule 1), every cell's style one of the
  `src/tui/style.rs` constants (rule 2), and each outside value within its source's budget and on
  its own line or block (rule 3). TUI-1 adds the helpers.
- At least one counter-example test: text or state from outside built to break the screen or to
  get an address shown as payable.
- "Nothing signed" is checked by counting `SignTx` requests where the daemon side records them
  (`FakeDaemon.received`, tests/common/mod.rs:503-505), not only by the absence of
  `eth_sendRawTransaction` at the mock node.
- Tests that reach `protocol::stop_all_daemons` (the `L` key, a whitelist change) take
  `common::exclusive()` (tests/common/mod.rs:91), as the CLI's do (v0-1/loop.md:15).
- No library entry point prints; the CLI command that used the code calls the new function and
  prints what it returns, so `tests/cli.rs` stays green unchanged.
- From TUI-3 on: every action the unit adds is a row of `src/tui/actions.rs` (03-tui.md
  §Architecture), so T12's test fails until its alias is in the menu; an action of a later unit
  is listed as `in a later version`.
- Every unit leaves the preview able to do something new end to end (the second column below);
  the dispatcher checks it on the merged preview with tmux frames at 120x40 and 80x24.

| Unit | The owner can, once it is merged | Scope | Closes | Depends on | Est. lines |
|---|---|---|---|---|---|
| TUI-1a | (nothing new on screen) | ratatui and crossterm, `FilteredBackend` and the terminal guard (`src/tui/term.rs`), `wallet::keys`, `wallet::status`, `wallet::net`, `list` and `status` on them | none | the agent-side v0.1 work merged (loop.md §Start condition) | 610 (measured) |
| TUI-1b | see the keys and the lock state; lock | `zwallet tui`, the screens (keys list, status bar, size rule), the cell/style/budget helpers | T1 | TUI-1a | 800 |
| TUI-1c | (the same, proven on a pty) | the pty runs (restore after `q`, `locked` within two refreshes), the argv-listed fake, the pty helpers | T9, T10 | TUI-1b | 350 |
| TUI-2 | unlock from the TUI, see the open key verified, make a key active | warm-up, password field, the password check (checked list, pinned addresses), make active, address trust | T2, T2b | TUI-1 | 750 |
| TUI-3 | see the balance on any chain and on every network; find every function from `?` | tab line, action menu, hint lines, popups and fields; 2 wallet balances, every network, the chain picker | T5, T8, T12, T13, T15 | TUI-2 | 800 |
| TUI-4 | receive: the address and its trust (and the QR, trade-off Q); verify and repair every key's address; unlock for a chosen time; see the session | 4 receive, `V` on every key, `A`, 7 session, `u`, `--allow-ptrace-scope-0` | T14, T16, T17, T18, T19, T20 | TUI-3 | 750 (+250 with Q3) |
| TUI-5 | send the coin or a registry token to a listed payee and see it mined | 3 send: asset, payee, amount, review with the overrides and the fee ceiling, exact-intent confirm, status | T6, T6b, T21, T22 | TUI-3 | 800 |
| TUI-6 | curate the whitelist: payees, contracts, fee caps, message signing, reset | 5 whitelist: view, `V`, every change; the pty run across T2, T6 and T7 | T7, T11, T23, T24, T25 | TUI-5 | 800 |
| TUI-7 | see the last 10 transactions, incoming ones included where an explorer answers | history: journal rows, explorer client, merge | T3, T3b | TUI-3; the explorer re-measure (loop.md §Entry conditions) | 800 |
| TUI-8 | open any transaction (a row, a send, a typed hash) with its fee and token movements | transaction detail, `dropped/replaced`, `/` lookup | T4, T6c, T26 | TUI-7 | 750 |
| TUI-9 | speed up or cancel a stuck send; retry an uncertain broadcast safely | replacements at nonce N | T27, T28, T29 | TUI-8 (and TUI-5) | 500 |
| TUI-10 | make the wallet or a new key without the shell (and see the phrase, trade-off B) | `n`, the first-run create, `B` | T30, T31, T32 | TUI-3 | 600 (+200 with B2) |
| TUI-11 | read and call contracts, sign a message, read any token | 6 tools: call, invoke, sign, token | T33, T34, T35, T36, T37 | TUI-5 | 800 |
| TUI-12 | everything above on Tron | the Tron rows of 03-tui.md §Coverage matrix | T38, T39, T40, T41 | TUI-5; 05-tron.md and its library units | per 05-tron.md |

**Old and new names.** Units TUI-3 … TUI-6 of 2026-10-27 never started; their text moved, with
its RED tests and counter-examples (reviewed in rounds 1–3 below), as follows. The review-round
tables below keep the old names.

| Old (2026-10-27) | New (2026-10-28, v2) | What moved |
|---|---|---|
| TUI-3 home: balance, journal, explorer, merge | TUI-7 (history) and TUI-3 (balance) | the history whole; the balance pane to TUI-3 |
| TUI-4 tx detail, `dropped/replaced`, chain switch | TUI-8 and TUI-3 | the detail and `tx_state` to TUI-8; the chain switch (T8) to TUI-3 |
| TUI-5 assets and transfer | TUI-5 and TUI-3 | the transfer stays; assets (T5) to TUI-3; the `dropped/replaced` rendering on the send status to TUI-8 |
| TUI-6 whitelist and the pty run | TUI-6 | all of it, plus T23–T25 |

## TUI-1 — `zwallet tui`, terminal setup, write point, keys, status bar, lock detection

- **Closes:** T1, T9, T10 (T1 in TUI-1b; T9 and T10 in TUI-1c, §TUI-1 split).
- **Adds the `ratatui` and `crossterm` dependencies** (Cargo.toml; the only unit that does).
- **Entry points added:** `wallet::keys`, `wallet::status` (with `Trust` made `pub`),
  `wallet::net`. The `L` key calls `protocol::stop_all_daemons` (exists).
- **Files:** `Cargo.toml`, `Cargo.lock`, `src/lib.rs`, `src/cli.rs` (`Command::Tui { warm, chain
  }`), `src/commands.rs` (`list` and `status` call `wallet::keys` and `wallet::status`; the `tui`
  dispatch), `src/wallet.rs` (new), `src/tui/mod.rs`, `app.rs`, `ui.rs`, `text.rs`, `style.rs`,
  `term.rs` (new), `tests/tui.rs` (new), `tests/common/mod.rs` (the argv-listed fake daemon below,
  moved from `build_attacker`, tests/cli.rs:4163).
- **RED tests:**
  - `FilteredBackend`, where it acts (unit): it wraps a recording inner backend (ratatui's
    `TestBackend`) and draws five bad cells, `█`, ESC, `┌` (box drawing), a two-character grapheme
    (`e` and U+0301) and the two-column glyph `漢`, among ASCII cells. The cells the inner backend
    receives: each bad cell is `?`, the column after `漢`'s `?` is a space, every ASCII cell is
    unchanged, and the replacement count is 5. An all-ASCII frame has count 0.
  - `FilteredBackend` over a real crossterm backend writing to a `Vec<u8>` (unit): cells whose
    content is `ESC[31m` and `ESC]0;x BEL`, all in the default style. The bytes written contain
    neither sequence and no BEL byte, and a `?` where each cell was; the backend's own cursor and
    style sequences (which contain ESC) are allowed and not asserted on (03-tui.md §Display
    integrity rule 1).
  - T1 (`TestBackend`): 2 keys, `b` active: both rows, `b` marked, `locked` appears once, in the
    bar.
  - T9 (`TestBackend`): at 80×24 the bar and the list fit; at 79×24 the frame is the one `terminal
    too small` line. (`Msg` sequence): `q` ends the app. (unit, `term.rs`): the restore guard
    writes the leave-alternate-screen and show-cursor sequences to a test writer on drop and from
    the panic hook (`catch_unwind`); the hook then writes its one fixed line, never the payload.
    (pty, `PtyChild`, tests/common/mod.rs:637): `zwallet tui`, then `q`; the pty is back in
    canonical mode with echo.
  - T10 (`Msg` sequence): a status refresh that reports locked makes the bar say `locked`. (pty):
    with a test daemon unlocked, `zwallet lock` from a second process; within two refreshes
    (10 s) the captured frame shows `locked`.
- **Counter-examples:**
  - `wallet.json` caches as an address `"\u{1b}[2J"` followed by 600 `█`: the whole key list
    fails, as `list` does (`primary_address(k)?`, commands.rs:845), so the keys pane shows the
    CLI's `VALIDATION_ERROR` in place of the rows, the value escaped and cut at 64 (`shown`,
    commands.rs:195-200); no row shows it as an address; zero replaced cells.
  - A process started with argv `daemon --home <canonical home>`, so that `daemon_pids` lists it
    and `status` uses its reply (commands.rs:1890-1900), as tests/cli.rs:3905-3912 starts one,
    answers `status` with `locked: false`, its own pid, and an `expiresAt` of 600 characters of
    ESC, `▀` and `▄`: the bar shows at most 64 escaped characters on one line (the budget
    assertion), and the address is not marked verified.

### TUI-1 split (2026-10-28)

The first coding pass measured about 610 changed lines for the dependencies, the write point and
the three entry points alone, and estimated about 1,100 more for the rest, so TUI-1 is three PRs
(the dispatcher's decision, 2026-10-28). The RED tests and counter-examples above are unchanged;
each lands in the PR named below.

| PR | Branch | Scope | Closes | State |
|---|---|---|---|---|
| TUI-1a | `feat/tui-1-shell` | `Cargo.toml`/`Cargo.lock` (ratatui 0.30.2, crossterm 0.29.0; kasuari 0.4.12 and num_threads 0.1.7 pinned to releases at least two weeks old); `src/tui/term.rs`: `FilteredBackend` and `Guard` with the panic hook (one guard per process; a panic while it holds the terminal restores it, writes the fixed line and ends the process with 101), with their RED tests, one of them a child process that panics inside `spawn_blocking`; `src/wallet.rs`: `keys`, `status`, `net`; `list` and `status` call them (no `list` or `status` assertion in `tests/cli.rs` changed; c14's lower bound tolerates a 2 s clock step, 5119e36). `Trust` is `pub`, re-exported as `wallet::Trust`; `Daemon` and its helpers stay in `src/commands.rs` as `pub(crate)` | none | in review |
| TUI-1b | `feat/tui-1b-screens` | `Command::Tui { warm, chain }` and its dispatch; `src/tui/mod.rs`, `app.rs`, `ui.rs`, `text.rs`, `style.rs`; `tests/tui.rs` with the cell, style and budget helpers, T1, the T9 frames and the `q` `Msg` test, the T10 `Msg` test, the cached-address counter-example; the README `zwallet tui` section; 03-tui.md §Status bar, the 80x24 layout and rule 3's RPC host row (decisions 2 and 4), rule 4's error view (decision 1) and the locked trust wording (§Changes on 2026-10-28); after review rounds 1 to 5, `q` waits for `L`, reads carry a generation, the bar and the key rows keep every value whole or `...`-cut at 80x24, and `assert_placed` finds any stray copy of a shown value | T1 | approved after review round 5, 2026-10-28 |
| TUI-1c | at dispatch | the T9 pty test (`q`, then canonical mode with echo), the T10 pty test (`zwallet lock` from a second process, `locked` within two refreshes), the argv-listed fake counter-example, the attacker moved into `tests/common`, the pty helpers (decision 5) | T9, T10 | planned |

Decisions (the dispatcher's, 2026-10-28):
1. The keys pane's error is a single-source error view, rule 4's exception: the CLI's
   `VALIDATION_ERROR`, its value escaped and cut at 64 as the CLI does, then wrapped inside the
   pane.
2. The status bar is two lines at the bottom: line 1 chain · key · short address · trust; line 2
   `locked` or `signing (agent too) until HH:MMZ` (UTC, through
   `core::time::parse_rfc3339_utc`) · RPC host. TUI-1b updates 03-tui.md §Status bar and the
   80x24 layout.
3. `TestBackend` tests build `StatusView` fixtures; behaviour against a real daemon is covered by
   the pty runs of the zwallet binary. (Called from a test binary, `check_peer` compares the
   test binary's own code, so a genuine daemon reads as another program there.)
4. The RPC host is `redact_url`'s origin, the host only, at most 32 characters, cut with `...`;
   TUI-1b adds it to rule 3's table.
5. TUI-1c extends `common::openpty` with a window size (default 120x40, settable), reading the
   output while the child runs, and the slave's termios for the echo check.
6. Moving the argv-listed attacker from `tests/cli.rs` into `tests/common` is allowed: helper
   lines change in `tests/cli.rs`, no assertion does.

## TUI-2 — warm-up, password field, password check, make active, address trust

- **Closes:** T2, T2b.
- **Entry points added:** `wallet::prepare_unlock`, `wallet::unlock`, `wallet::address`,
  `wallet::set_active`, `wallet::open_with_password` (the named keys' addresses from the seed,
  the sealed anchor and a `salted` flag, never the salt) and `wallet::check_list` (the checked
  list with its fingerprint and the list's own policy), 03-tui.md
  §Password check. The CLI's `unlock`, `change`, `list --verify` and `whitelist list --verify`
  call them (commands.rs:1912, :930, :825; commands/whitelist.rs:1343). `address --verify`
  (commands.rs:949-1022) does not move: it repairs the cache and reads no list file.
- **Files:** `src/wallet.rs`, `src/commands.rs`, `src/commands/whitelist.rs`, `src/tui/app.rs`,
  `ui.rs`, `password.rs` (new), `tests/tui.rs`.
- **Depends on:** TUI-1.
- **RED tests:**
  - T2 (pty): keys `a` and `b`, `b` active, locked. Enter on `a`, type the password: the frame's
    bar shows `signing (agent too) until` ≈ now+1h and `verified by password` for `a`;
    `zwallet status --json` has the same `expiresAt`; `zwallet list --json` still has `active:
    "b"`.
  - T2b (pty): `s` on `a`, confirm: `zwallet list --json` has `active: "a"`.
  - (unit): `open_with_password` then `check_list` on a seeded whitelist
    (`common::seed_whitelist`, tests/common/mod.rs:265) returns that list's entries, gen and the
    fingerprint of the files as read; with a zwallet 0.1.0 anchor `check_list` returns `list
    --verify`'s message and no list. `open_with_password` reads no list file (the files are
    removed before the call and it still succeeds).
  - (`Msg` sequence): while the password check runs (in `spawn_blocking`), the pane shows
    `checking...` and further frames are drawn; its result arrives as one `Msg`.
  - (`Msg` sequence): `prepare_unlock` runs before the field takes a key; the Enter that opens the
    field also switches the input thread to password mode, so keys typed right after it go into
    `PasswordBuf`, never into a `Msg::Key`; a `Trust::Unverified(Some(w))` status makes the bar
    show the `NOT this build's key daemon` line; on Linux the bar says `(no session check on
    Linux)`.
- **Counter-examples:**
  - A password in a frame: warm up with the password `pw-XYZ-0123456789`, typed and then pasted.
    No frame of the sequence and no pty capture contains the whole password or the marked piece
    `pw-XYZ` (shorter pieces are not searched: they collide with address hex); the field shows as
    many `*` as characters typed; the `Debug` text of every `Msg` holds neither; the paste
    event's string is all zero bytes after it is appended.
  - Pinning: after the warm-up, a process answering on `d.sock` gives another address for `a`:
    the pane stops with the mismatch error; the bar keeps the pinned address.
  - The exact order: `wallet::unlock` starts the session, then the test edits wallet.json's cached
    address for `a` (a process of this user racing the TUI), then the password check runs: the
    pane shows the CLI's `VALIDATION_ERROR` naming `zwallet address --verify`; `zwallet status
    --json` still has `locked: false`; there is no checked list (the picker offers nothing) and no
    pinned address (the bar does not say `verified by password`). The same edit made before the
    warm-up fails the warm-up itself (daemon/mod.rs:311): no session starts.
  - A process answers `address` with 600 `▀▄` characters: the pane shows the CLI's "not an
    address" error escaped; the bar never shows it as an address.

## TUI-3 — navigation, balances, every network, chain switch

- **Closes:** T5, T8, T12, T13, T15.
- **The owner can afterwards:** see the open key's balance on any chain (`c`) and on every
  network at once (`a`), and find every function of the TUI from `?` or `:`, including the ones
  a later unit brings (listed `in a later version`).
- **Entry points added:** `wallet::balances(home, key, chain) -> ChainBalances` (the native coin
  and every registry token, `Evm.wallet_balances`, chains/evm/mod.rs:259, with the owner's
  `Trust`) and `wallet::every_balance(home, key)`, moved from `every_balance` and `read_network`
  (commands.rs:1173-1350); `wallet::brief` made `pub` (commands.rs:1371). `balance` with and
  without `--chain` calls them. The chain switch uses `wallet::net` (wallet.rs:155).
- **Files:** `src/wallet.rs`, `src/commands.rs`, `src/tui/actions.rs` (new: the action table),
  `src/tui/form.rs` (new: the text field with its alphabet, the popup), `src/tui/app.rs`, `ui.rs`,
  `tests/tui.rs`.
- **Depends on:** TUI-2 (the open key).
- **RED tests:**
  - T12 (`TestBackend` and `Msg` sequence): `?`, then `whitelist add` typed: one row, key `a`,
    tab 5. The test holds the CLI column of 03-tui.md §Coverage matrix rows 1–52 as a list and
    asserts that the menu's aliases are exactly that list; Enter on an action that cannot run
    shows its reason and starts no `Effect`.
  - T13 (`Msg` sequence and `TestBackend`): `1` … `7`, `[` and `]` reach each tab; every frame's
    hint line ends in `? actions`; Esc closes a popup, then returns a sub-screen to its tab; in a
    text field `q`, `?` and `1` are typed, not acted on; every tab fits 80x24.
  - T5 (`TestBackend`): native and USDC non-zero, DAI zero: native and USDC rows only.
  - T15 (`TestBackend`, two mock nodes through `ZWALLET_RPC_<CHAIN>` and one that refuses): one
    line per network, the testnet last; the failed one's error cut to 72 (`brief`); `2 of 3
    networks have a balance; 1 did not answer`.
  - T8 (`Msg` sequence): `c` then base: every pane reloads for base, and a reply for ethereum that
    lands after the switch is dropped. (`TestBackend`): with `ZWALLET_RPC_BASE` set to a URL with
    a key in its path and in a host label, the bar shows the host with the label `<redacted>`
    and nothing of the path (rule 3's RPC host row).
- **Counter-examples:**
  - A token whose `balanceOf` fails with an error of 600 characters of ESC and `█`: its row says
    `<SYMBOL> error: …` within 72, zero replaced cells, and the other rows still show.
  - A network whose RPC error quotes the URL's key: the line shows `<redacted>` (`Net::redact`,
    chains/mod.rs:282).
  - The menu filter typed with ESC and `█`: neither reaches the field (its alphabet is printable
    ASCII), the filter is unchanged.

## TUI-4 — receive, verify and repair every key, the session, unlock for a duration

- **Closes:** T14, T16, T17, T18, T19, T20.
- **The owner can afterwards:** receive (the full address and its trust, every family named),
  check every key's address with the password and repair a modified cache, unlock for a duration
  he types, and read the session in full.
- **Entry points added:** `wallet::addresses(home, key)` from `addresses` (commands.rs:1054);
  `wallet::repair(home, password) -> Repaired`, the `address --verify` path
  (`store::unlock_repairing`, commands.rs:965-1052, :973), returning the `repaired` list; `V` on
  1 keys calls TUI-2's `wallet::open_with_password` with every key, as `list --verify` does
  (commands.rs:846-908). `Command::Tui` gains `--allow-ptrace-scope-0` (cli.rs:83-93), passed to
  TUI-2's `wallet::prepare_unlock`. The session tab reads `wallet::status` (wallet.rs:103).
  Trade-off Q: Q1 is a line of text; Q3 adds `QR_DARK` and `QR_LIGHT` to `src/tui/style.rs` and a
  QR widget over `qrcode` (src/qr.rs:3), which the CLI already depends on.
- **Files:** `src/cli.rs`, `src/wallet.rs`, `src/commands.rs`, `src/tui/actions.rs`, `app.rs`,
  `ui.rs`, `style.rs` (Q3), `tests/tui.rs`.
- **Depends on:** TUI-3.
- **RED tests:**
  - T14 (pty): 7 session, `u`, `10m`, the password: the bar's expiry ≈ now+10m; `zwallet status
    --json` has the same `expiresAt`.
  - T16 (`TestBackend`, `StatusView` fixtures as TUI-1b's decision 3): this home's daemon, two of
    other homes, the session's whitelist view: each on its labelled line; the view as `reports
    gen N, M entries (not checked here)`.
  - T17 (`TestBackend`): warm: the full EIP-55 address and `verified by password`; locked:
    `not verified` and the `V` hint; every chain of `zwallet chains` named.
  - T18 (`TestBackend`): Q1: `Q` draws the one line naming `zwallet address`. Q3 (only if picked):
    at 120x40 the overlay's `QR_DARK` cells decode with `rqrr` (as src/qr.rs's tests decode) to
    the address; at 80x24 the size line; for `not verified` and for an impostor's trust no code
    and the reason; no cell outside the overlay has a QR style.
  - T19 (pty): `V` on 1 keys: every row `verified`; the test edits `beta`'s cached address, `V`
    shows the CLI's `VALIDATION_ERROR` naming `A`; `A` and the password: the `repaired` list and
    every row `verified`; `zwallet list --verify --password-stdin` agrees.
  - (`TestBackend`): at 80x24 a 32-character key name, an index of 1000 and the word `verified`:
    the address and the word are whole, the name is cut with `...` (03-tui.md §Status bar, the
    key list's widths; TUI-1b's `an_index_of_1000_and_a_32_character_name_never_cut_the_address`,
    tests/tui.rs:362, keeps passing for rows without the word).
  - T20 (unit, `ptrace_scope_check` on a fixture file, core/protocol.rs:464): the TUI's
    `prepare_unlock` refuses scope 0 and names `zwallet tui --allow-ptrace-scope-0`; with the
    flag it passes. (pty, Linux, where `common::ptrace_scope_is_open`, tests/cli.rs:1025): the
    warm-up without the flag stops before the password field.
- **Counter-examples:**
  - wallet.json's cache for `beta` is `"\u{1b}[2J"` and 600 `█`: `A` shows `was` escaped and cut
    at 64; no row shows it as an address.
  - A `StatusView` with 300 pids of other homes: 8 shown, then `292 more`; a home path holding ESC:
    escaped, at most 64.
  - A process answering `address` for the open key with another address after the password check:
    4 receive stops with the mismatch error and the bar keeps the pinned address (TUI-2's pinning).

## TUI-5 — transfer

- **Closes:** T6, T6b, T21, T22.
- **The owner can afterwards:** send the coin or a registry token to a listed payee, override
  the pending and repeat guards knowingly, set a fee ceiling, and follow the send to mined. The
  status shows pending, mined (success or reverted) and `unknown`; `dropped/replaced` comes with
  TUI-8.
- **Moved from the old TUI-5 (2026-10-27)**, unchanged except: T5 and its test went to TUI-3; the
  `Msg`-sequence test of `dropped/replaced` on the send-status screen went to TUI-8, which adds
  `rpc::tx_state`; `Depends on` is TUI-3 (tabs, forms, assets) and TUI-2 (the checked list, a
  warm session). Added:
  - `wallet::review_transfer` takes the overrides (`allow_pending`, `allow_duplicate`) and a fee
    ceiling (`max_fee_wei`, as `max_fee_wei` reads `--max-fee-gwei`, commands.rs:1532); the review
    holds them, and confirm uses exactly those.
  - T21 (mock node): a pending nonce: the review shows it and `p`; confirm without `p` makes zero
    `SignTx`, with `p` one. A journal entry from 5 minutes ago with the same chain, kind, `to` and
    `amountRaw`: the same with `d`.
  - T22 (mock node): `g` and `5`: the review runs again with a 5 gwei ceiling; a node fee above
    the automatic ceiling shows the CLI's `VALIDATION_ERROR` until `g` raises it.
  - Counter-example: `g` with `0`, `0.5.1` or 30 digits: the field refuses the keys outside its
    alphabet and the CLI's parse refuses the rest; the review is unchanged.

The moved text follows.

- **Entry points added:** `wallet::review_transfer` (prepare as a dry-run, duplicates, the
  whitelist verdict and fee cap on the checked list; no lock, no signature; `Review` holds the
  exact intent),
  `wallet::send_transfer` (fingerprint check, lock, prepare again with the held payee address,
  pending and duplicate guards again, `Review::matches` on the decoded transaction (the transfer
  intent exactly equal, the most the fee can cost, `max_gas_cost` + `l1_fee` as U256, at most the
  reviewed one), then sign,
  check the signer, journal, broadcast), `SendRecord::write` returning its line instead of
  printing it. The CLI's `transfer` calls the send path; `invoke` keeps the signing tail they are
  cut from (commands.rs:1530, :1421, :1631); both print the `broadcasting` line as before. Assets
  use `wallet::balances` (TUI-3).
- **Files:** `src/wallet.rs`, `src/commands.rs`, `src/chains/mod.rs`, `src/chains/evm/mod.rs`,
  `src/tui/app.rs`, `ui.rs`, `tests/evm_offline.rs`, `tests/tui.rs`.
- **Depends on:** TUI-3, TUI-2 (the checked list, a warm session).
- **RED tests:**
  - T6 (pty, mock node through `ZWALLET_RPC_ETHEREUM`): asset → `alice` → amount → review → type
    `alice`: the hash is in the next frame; the status reaches mined; the journal file has the
    line.
  - T6b (`TestBackend`): the picker lists `alice` (address entry on ethereum) and not a contract
    entry nor an entry on base only; (`Msg` sequence): typed `0x...` characters in the picker
    change nothing.
  - (mock node): the fee rises between review and confirm: a new review, and zero `SignTx`
    requests reached the daemon side. The fee falls instead: confirm signs (one `SignTx`), and
    the test decodes the transaction in that request (`tx::decode_unsigned`,
    chains/evm/tx.rs:378) and asserts its `chain_id`, `to`, `value`, `input` and `nonce` equal the
    fields the review held. The same two cases on base (OP-stack): the fixture's
    `GasPriceOracle` answer (`op_l1_fee`, chains/evm/rpc.rs:1040) moves `l1_fee` up, then down,
    with `max_gas_cost` unchanged: up refuses (zero `SignTx`), down signs. The comparison reads
    U256 integers, never the formatted fields.
  - (`Msg` sequence): the polling bound turns the send's status into `unknown — run zwallet tx`.
  - (unit): `wallet::review_transfer` takes the `CheckedList` and computes the verdict and the fee
    cap on it: with both list files removed from the home after the check, it still reports
    `alice` allowed under the checked list's cap (it reads no list file; the fingerprint check is
    the TUI's own step before it).
  - (pty): no `broadcasting` line appears outside the frame; `zwallet transfer` still prints it
    (tests/cli.rs unchanged).
- **Counter-examples:**
  - whitelist.json rewritten at the same gen with `alice` at a lookalike address, nickname and
    everything else unchanged, while the TUI holds a checked list: the picker drops the checked
    list and offers nothing; started without a password check, the picker offers nothing either.
  - A process listed by argv answers `status` with `whitelist: { gen: N+1, verified: true,
    entries: ["alice"] }` (a forged session view): nothing is offered and nothing is labelled
    verified.
  - The list is re-signed with the password from another shell after the review, `alice` now at
    another address, nonce, fee, funds and verdict unchanged: confirm finds the files changed,
    signs nothing (zero `SignTx`), and shows `not verified`.
  - A journal line whose `to` is 600 characters with ESC and `█`: the duplicate guard shows
    `<invalid>` for it, within 512.

## TUI-6 — whitelist screens and edits; the pty run across T2, T6 and T7

- **Closes:** T7, T11, T23, T24, T25.
- **The owner can afterwards:** curate the whitelist from the TUI: payees, contracts with their
  selectors, fee caps, message signing, reset, each with its summary and the password.
- **Entry points added:** `wallet::whitelist_view`, `wallet::whitelist_plan`,
  `wallet::whitelist_apply`, and `Change` made `pub` with `Change::add`, `Change::remove`,
  `Change::fee_cap`, `Change::message_signing` (`Change::MessageSigning`, commands/whitelist.rs:112;
  added 2026-10-28), `Change::reset`. The CLI's `whitelist` commands call them
  (commands/whitelist.rs:51) and keep their own prompt and printing. T7's verified state and `V`
  use `wallet::open_with_password` and `wallet::check_list` (TUI-2).
- **Files:** `src/wallet.rs`, `src/commands/whitelist.rs`, `src/tui/app.rs`, `ui.rs`,
  `tests/tui.rs`.
- **Depends on:** TUI-5 (the end-to-end run goes through a send), TUI-3 (forms), TUI-2 (the
  password field).
- **RED tests:**
  - T7 (pty, `common::exclusive()`): warm, then `5`, `a`, `bob` and an address, no duration: the
    frame shows the summary above the password field; after the password, `zwallet whitelist list
    --verify --password-stdin` shows `bob` at gen+1, `zwallet status --json` has `locked: true`,
    and T7 shows the new list as verified (the password check ran again). The same with a
    duration of 10m: unlocked until about now+10m.
  - (pty): started warm with no password typed, T7 shows `not verified`; `V` and the password
    show it verified; `zwallet status --json` and the list files are the same before and after
    `V`.
  - T11 (pty, the run of 03-tui.md §Tests: T2 → T6 → T7): no echo; the password is in no captured
    frame; Esc in the middle of the password, then a retry, works.
  - T23 (pty, `common::exclusive()`): `d` on `bob`, `f` base `0.001`, `R`: each shows its
    summary above the password (the reset's cost as `whitelist reset` spells it, README.md:404-408);
    after each, `whitelist list --verify --password-stdin` agrees; locked afterwards.
  - T24 (pty): `a`, kind contract, `router`, ethereum, `0x414bf389`: the outcome has the CLI's
    contract warning (README.md:281-282); `whitelist list --verify` shows the entry and its
    selector. A registry token's address as a contract: the CLI's refusal, nothing stopped.
  - T25 (pty): `m`: the summary shows the old and the new value; `whitelist list --verify` has
    `allowMessageSigning: true`.
- **Counter-examples:**
  - `whitelist.json` with a 600-character nickname, or a nickname of `▀▄█`, or one holding ESC:
    not a list at parse (`check_shapes`, core/whitelist.rs:689); T7 shows the "cannot be read"
    line with the parser's message cut at 80; zero replaced cells.
  - A forged session view (as in TUI-5) says `verified: true` at gen N+1: T7 shows "the running
    session reports gen N+1, 1 entry; this TUI did not check it", draws no entry name from it,
    and draws no "verified".
  - The file changes after the password check: T7 drops the checked list and shows `not
    verified`.
  - The password in a frame, as in TUI-2, across the edit's summary, password and outcome frames.

## TUI-7 — history: journal rows, explorer, merge (the old TUI-3)

- **Closes:** T3, T3b.
- **The owner can afterwards:** see the last 10 transactions on the wallet tab, this machine's
  sends and, on ethereum, optimism and sepolia, incoming ones from the explorer, with the
  coverage note.
- **Moved from the old TUI-3**, except the balance pane (`wallet::balances`, `wallet::brief`),
  which TUI-3 adds.
- **Entry points added:** `SendJournal::recent` with `JournalRow` (the checks of `EntryText::of`, chains/mod.rs:756),
  `chains::evm::explorer::fetch` (new module, the shape check of 03-tui.md §History),
  `chains::EXPLORERS`, `history::merge` (new module).
- **Files:** `src/chains/mod.rs`, `src/chains/evm/mod.rs`, `src/chains/evm/explorer.rs` (new),
  `src/history.rs` (new), `src/lib.rs`, `src/wallet.rs`, `src/tui/app.rs`, `ui.rs`,
  `tests/fixtures/explorer/*.json` (new, single-line), `tests/tui.rs`.
- **Depends on:** TUI-3 (the wallet tab); the explorer re-measure (loop.md §Entry conditions).
- **RED tests:**
  - Explorer fixture: a local HTTP server in the test, built like `tiny_rpc_reply`
    (tests/common/mod.rs:926), serving single-line Blockscout bodies cut from the 2026-10-27
    captures to the kept fields plus a few dropped ones.
  - T3 (`TestBackend`, the `App` built with the fixture explorer and a fixture journal file): 14
    transactions, one hash in both the journal and the explorer, 2 pending: exactly 10 rows,
    pending first, that hash once with both sources, the coverage note shown.
  - T3b (`TestBackend`): the fixture explorer answers 403 `text/html`: journal rows only, the
    note says `explorer unavailable (HTTP 403)`, no banner over the list.
  - (unit, `explorer.rs`): each kept field at its shape's edges (64 and 65 hex digits, 78 and 79
    decimal digits, `null` block number); an item about another address is dropped; a body over
    1 MiB and an answer after 8 s count as unavailable.
  - (unit, `history.rs`): the merge's order (pending, block descending, position) and
    de-duplication by `(chainId, txHash)`; a reply for chain A that arrives after the switch to
    chain B is dropped.
  - (live, `--ignored`): `eth.blockscout.com` for a known address parses with no item dropped
    for shape.
- **Counter-examples (a forged explorer page):** an item whose `from.name` is `zwallet: verified`,
  a token transfer of a non-registry contract whose `token.symbol` is `USDC`, a 0-value
  transaction from a lookalike address to the wallet, a 63-digit `hash`, a `method` of 600
  characters with ESC and `█`, an item about another address, and a 403 body made of `▀▄`. None
  of their text is in any cell; the note counts the dropped items; zero replaced cells.

## TUI-8 — transaction detail, `dropped/replaced`, lookup by hash (the old TUI-4)

- **Closes:** T4, T6c, T26.
- **The owner can afterwards:** open any transaction (a history row, his own send, or a hash
  typed with `/`) with its fee, confirmations and token movements, and see a send that another
  transaction replaced.
- **Moved from the old TUI-4**, except the chain switch (T8), which TUI-3 adds.
- **Entry points added:** `rpc::tx_detail` (`effectiveGasPrice`, the receipt's `l1Fee` on base and
  optimism, `eth_blockNumber` for confirmations, and the receipt's registry-token `Transfer` logs
  that involve the wallet, 03-tui.md §History), `rpc::tx_state` (`dropped/replaced`: the account's
  `latest` nonce past the transaction's, no receipt). The detail's status uses `tx_state`; TUI-5's
  send-status screen uses it from this unit on. `/` reads a hash in a field (`0x` and 64 hex
  digits) and opens the detail, as `tx` looks one up (commands.rs:1783).
- **Files:** `src/chains/evm/rpc.rs`, `src/chains/evm/mod.rs`, `src/tui/app.rs`, `ui.rs`,
  `tests/evm_offline.rs`, `tests/tui.rs`.
- **Depends on:** TUI-7 (the rows), TUI-5 (the send-status screen).
- **RED tests:**
  - T4 (mock node, `MockRpc`, tests/evm_offline.rs:40): a fixture transaction and receipt with
    `gasUsed`, `effectiveGasPrice` and, on base, `l1Fee`, and a fixture head block: every detail
    field equals the value written in the fixture. Receipt fixtures: a USDC transfer routed
    through a router contract (the transaction's `to` is the router; the USDC `Transfer` log pays
    the wallet): the detail shows the router as `to` and one movement with the USDC amount and
    the real recipient; a receipt with two movements: two lines; a reverted receipt: "no token
    moved"; a pending transaction: "token movements unknown until it is mined"; a USDC
    `Transfer` log of amount 0 from a lookalike address: not shown, counted.
  - T6c (mock node): a journal row whose hash has no receipt while `eth_getTransactionCount` at
    `latest` is past its nonce: the detail's status is `dropped/replaced`.
  - T26 (mock node): `/` and a fixture hash: the detail; a hash the node does not know: `not
    found` with the CLI's hint (README.md:507-522); 63 hex digits cannot be entered.
  - (`Msg` sequence): a `tx_state` result of `dropped/replaced` on the send-status screen renders
    `dropped/replaced` with the hash and the nonce (moved from the old TUI-5).
- **Counter-examples:**
  - The mock node answers `eth_getTransactionReceipt` with an error of 600 characters of ESC and
    `█`: one error line within the pane's width; the detail view shows at most 512 escaped
    characters in at most 7 rows; zero replaced cells.
  - A `Transfer` log from a contract outside the registry that pays the wallet 1,000,000 "USDC":
    no movement is shown for it; the detail counts it as not shown.
  - Nickname labels: no password check in this session, and whitelist.json lists `alice` at the
    transaction's `to` address: the detail shows the address with no nickname. After the password
    check, a whitelist.json rewritten at the same gen with `alice` moved to a lookalike address
    drops the checked list, and the lookalike gets no nickname.

## TUI-9 — speed up, cancel, retry at the same nonce

- **Closes:** T27, T28, T29.
- **The owner can afterwards:** speed up or cancel a send of this machine that is stuck pending,
  and retry a broadcast that came back `BROADCAST_UNCERTAIN` without risking a second payment.
- **Entry points added:** `wallet::review_transfer` takes a `Replacement { nonce, replace, bump }`
  (the CLI's `--nonce`, `--replace`, `--bump`, cli.rs:256-266, read by `replacement`,
  commands.rs:1521-1531); the cancel is a review of 0 of the chain's coin to the key's pinned
  address with that replacement. `Review::matches` holds the nonce and the replaced hash with
  the transfer intent. The send-status and detail screens offer `s` and `x` only for a journal
  send whose nonce `rpc::tx_state` (TUI-8) reports pending, and `R` only after
  `BROADCAST_UNCERTAIN`.
- **Files:** `src/wallet.rs`, `src/commands.rs`, `src/tui/actions.rs`, `app.rs`, `ui.rs`,
  `tests/evm_offline.rs`, `tests/tui.rs`.
- **Depends on:** TUI-8 (`tx_state`, the detail screen), TUI-5 (review and confirm).
- **RED tests:**
  - T27 (mock node): a pending send at nonce N, `s`, the bump 1.25, confirm: one `SignTx`; the
    test decodes it (`tx::decode_unsigned`, chains/evm/tx.rs:378): `nonce` N, `to`, `value` and
    `input` as the replaced send's, fees at least 1.25 times the replaced transaction's as the
    mock reports them; the journal has a second line at nonce N.
  - T28 (mock node): `x`, type `cancel`: one `SignTx` with nonce N, `to` the key's pinned address,
    `value` 0, empty `input`.
  - T29 (mock node that times out `eth_sendRawTransaction` after accepting it): the status says
    `BROADCAST_UNCERTAIN` and offers `R`; `R` opens the review at nonce N with `--replace` the
    first hash; no action on that screen reaches a send without a nonce (the `Effect`s are
    listed).
- **Counter-examples:**
  - The node reports nonce N mined by another hash before confirm: confirm signs nothing (zero
    `SignTx`) and shows the CLI's "already mined (nonce consumed)" message (README.md:594-602).
  - wallet.json's cached address is edited after the password check: the cancel still pays the
    pinned address; the pane stops with the mismatch error if a session read disagrees (TUI-2).
  - A journal line at nonce N whose `to` is 600 characters of ESC and `█`: `s` is not offered for
    it (an `<invalid>` field), and its text is within 512.

## TUI-10 — the wallet and new keys from the TUI; the recovery phrase

- **Closes:** T30, T31, T32.
- **The owner can afterwards:** create the wallet on a new machine and further keys without the
  shell. The phrase: B1 (a line naming `zwallet backup`) until the owner picks trade-off B; B2
  is a delta of this unit.
- **Entry points added:** `wallet::create(home, name, words, password, ttl) -> Created`, from
  `create` and `add_subkey` (commands.rs:683-845), returning the reminder to run the backup
  instead of printing it (commands.rs:773-776; 03-tui.md rule 6). The CLI's `create` calls it and
  prints the reminder as before. B2 only: `wallet::recovery_phrase(home, password) ->
  Zeroizing<Vec<String>>`, from `backup` (commands.rs:1970-1995) without its terminal check,
  which `zwallet tui`'s own check (src/tui/mod.rs:38-43) stands in for.
- **Files:** `src/wallet.rs`, `src/commands.rs`, `src/tui/actions.rs`, `app.rs`, `ui.rs`,
  `password.rs`, `tests/tui.rs`.
- **Depends on:** TUI-3 (forms), TUI-2 (the password field).
- **RED tests:**
  - T30 (pty): `n`, `trading`, the password: `zwallet list --json` has `trading` active; the row
    says `verified`. With `10m`: `zwallet status --json` unlocked until about now+10m.
  - T31 (pty, an empty home): the create form; two passwords that differ: nothing is created
    (no `wallet.json`); then the same twice: `wallet.json` exists, `main` is key 0; the frame
    shows the reminder and no word of the phrase (the test reads the phrase with `zwallet
    backup` on its own pty and searches every frame for each word).
  - T32 (`TestBackend`): B1: `B` draws the one line naming `zwallet backup`. B2 (only if picked,
    pty): the password, then the words; any key hides them; 60 s hide them; no other frame of the
    run and no capture after `q` holds any of them; `Msg`'s `Debug` holds none.
- **Counter-examples:**
  - The name field takes only `validate_key_name`'s alphabet; a name that exists: the CLI's
    `KEY_EXISTS`, nothing written.
  - Both password buffers of the first-run form are zero after a mismatch, a cancel and a success.
  - B2: a panic while the words are on screen: the fixed panic line only, never a word (the
    hook's line is a constant, 03-tui.md §Password handling).

## TUI-11 — contracts, messages, any token

- **Closes:** T33, T34, T35, T36, T37.
- **The owner can afterwards:** read any contract, call a listed contract with a listed
  selector, sign a message when the list allows it, and read the balance of any ERC-20.
- **Entry points added:** `wallet::call` from `call` (commands.rs:1388-1436), returning the
  rendered result and the raw hex cut to 512; `wallet::review_invoke` and `wallet::send_invoke`
  from `invoke` (commands.rs:1437-1508: `abi::parse_function`, `abi::encode_call`,
  `abi::parse_calldata`), on the review and confirm of TUI-5 with the invoke intent (03-tui.md
  §Status of a send); `wallet::sign_message` from `sign_message` (commands.rs:1792-1818);
  `wallet::token_balance` from `balance --token` (commands.rs:1106-1172; `erc20_meta`,
  chains/evm/rpc.rs:209).
- **Files:** `src/wallet.rs`, `src/commands.rs`, `src/tui/actions.rs`, `app.rs`, `ui.rs`,
  `tests/evm_offline.rs`, `tests/tui.rs`.
- **Depends on:** TUI-5 (review and confirm), TUI-2 (the checked list's contract entries and
  policy).
- **RED tests:**
  - T33 (mock node): `balanceOf(address)(uint256)` on a fixture token: the result as the CLI
    renders it.
  - T34 (mock node): `router`, selector `0x414bf389`, signature and arguments, type `router`:
    the review shows zwallet's decode (contract-call, contract, selector, value); one `SignTx`
    whose decoded `to` and `input` equal the review's; the journal line has kind `invoke`. A
    signature whose selector `router` does not list stays in the form, which names the listed
    ones.
  - T35 (mock node): raw calldata with a listed first 4 bytes: as T34; an unlisted one: refused at
    the form.
  - T36 (fake daemon): message signing on in the checked list: the signature, checked against the
    address (`verify_message`, chains/evm/mod.rs:193); off: the review shows `WHITELIST_DENIED`
    and points to `m`; a paste holding a non-ASCII character, and 513 characters: refused at the
    field, which names `zwallet sign-message`.
  - T37 (mock node): a token whose `symbol()` is 40 characters with ESC: the balance, the symbol
    escaped and cut at 32; no send offered.
- **Counter-examples:**
  - `call`'s result is a `string` of 600 characters of ESC and `█`: at most 512 escaped
    characters in at most 7 rows, zero replaced cells.
  - whitelist.json rewritten after the check with `router` at another address: the invoke form
    drops the checked list and offers nothing; at confirm, zero `SignTx`.
  - A forged session view says message signing is on while the checked list says off: the review
    follows the checked list.

## TUI-12 — Tron

- **Closes:** T38, T39, T40, T41.
- **The owner can afterwards:** receive on Tron, see TRX and TRC20 USDT, send to a listed Tron
  payee, and look a Tron transaction up, on the same tabs as EVM.
- **Entry points, files, RED tests and counter-examples:** from [`../05-tron.md`](../05-tron.md)
  (branch `docs/tron-spec`, being written on 2026-10-28): its inputs, outputs and errors per
  capability, and how a Tron payee is listed (03-tui.md §Coverage matrix, Tron payees). The TUI
  side adds no screen: `tron` in the chain picker, its address on 4 receive, its rows on 2 wallet
  and 3 send, its lookup on `/`, each through rules 1–7.
- **Depends on:** TUI-5; 05-tron.md and the Tron library units it plans.

## Review round 1 (2026-10-27)

Three panelists (grok-4.7, gpt-6-astra, fable) rejected the first draft. Each finding, the
decision, and where it now lives.

| Finding | Decision | Where |
|---|---|---|
| B1: the picker's gate (session `verified`, same gen, nickname) authenticates no address; the session view has no addresses and its `status` reply has no peer check, so a same-gen rewrite or a forged view puts a lookalike on the review screen; T7's verified state and T4's labels had the same hole | payees, T7's verified state and T4 labels come only from the list the TUI checked with the password typed in this session (warm-up, a change, or `V`); the session view is information only; the file is checked against the verified bytes before the review and at confirm | 03-tui.md §Password check, §Display integrity rule 7, T2/T4/T6/T7; flow.md §Password check, §T4, §T6, §T7; TUI-2, TUI-4, TUI-5, TUI-6 counter-examples |
| B2: confirm compared nonce, fee, funds and verdict only, so a changed payee address could be signed | the review holds the transfer intent (chainId, from, payee, asset, amount, calldata, nonce), `sufficientFunds`, the verdict and the most the fee can cost; confirm re-prepares under the lock with the held payee, runs both guards again, and signs only when the transfer intent is exactly equal and the most the fee can cost is at most the reviewed one (the fee rule as amended in round 2); "nothing signed" counts `SignTx` requests | 03-tui.md §Status of a send; flow.md §T6 (`Review::matches`); TUI-5 |
| B3: history keeps token movements the explorer reports, but T4 decoded only a direct `transfer` call | T4 decodes the selected transaction's receipt `Transfer` logs of registry tokens that involve the wallet (one receipt, no scan); outer from/to apart from each movement; several movements, reverted, pending | 03-tui.md T4, §History; flow.md §T4; TUI-4 fixtures |
| B4: the cell rule was enforced only in tests | `FilteredBackend` is the one write point at run time; tests assert zero replacements | 03-tui.md §Display integrity rule 1; flow.md §T9; TUI-1 |
| 1: rule 5's "no block glyph, so no QR" was wrong | the defence is styles, placement and budgets together, each tested | 03-tui.md rule 5 |
| 2: the cell rule does not prove budgets or provenance | `Outside { source, text }` and per-source budget assertions | 03-tui.md rule 3; every unit |
| 3: an outside newline could break the 7-row bound | the error view wraps the escaped text | 03-tui.md rule 4 |
| 4: "within 5 s" with a 5 s refresh is flaky | within two refreshes (10 s) | 03-tui.md §Acceptance T10; TUI-1; loop.md |
| 5: paste copy, password type, panic hook, typeahead | paste copy zeroized; `PasswordBuf` without `Debug`/`Display`/`Clone`; fixed panic line through the filter; password mode set with the Enter | 03-tui.md §Password handling; TUI-1, TUI-2 |
| 6: `record_receipt(None)` says `pending` | the TUI's send-status state maps its 90 s bound to `unknown` | 03-tui.md §Status of a send; flow.md §T6; TUI-5 |
| 7: TUI-1's fake status came from an unlisted pid, so it tested the "ignored" rule | the fake runs with the daemon's argv, so `daemon_pids` lists it | TUI-1 counter-examples |
| 8: sources missing from rule 3 | `ZWALLET_RPC_<CHAIN>`, the session view's entries (never drawn), the input fields' alphabets | 03-tui.md rule 3 |
| 9: a later session read could change the shown address | the password-checked addresses are pinned; a different one stops the pane | 03-tui.md §Address trust; TUI-2 |
| 10: 0-value incoming native transfers are the poisoning pattern too | dropped like 0-value token transfers | 03-tui.md §History; TUI-3 |
| 11: TUI-3's fixtures could break the cap | single-line JSON, counted | counting rule above; TUI-3 |
| 12: Routescan for the 403 chains | re-measure before TUI-3 merges, Routescan's paths for base, arbitrum and polygon included; decide from that data | loop.md §Entry conditions; 03-tui.md §History |
| 13: U1–U6 clash with the epic's own units | renamed TUI-1 … TUI-6 | all four files |
| 14: the start condition | TUI-1 starts once the agent-side v0.1 work is merged; G3 and G4 wait on owner actions only | loop.md §Start condition; 03-tui.md Status |
| (rebalance) TUI-5 grew with B1 and B2 | T6c and `rpc::tx_state` move to TUI-4 (the detail's status needs them first); T5 moves to TUI-5 (assets open the transfer) | the unit table above; loop.md |

## Review round 2 (2026-10-27)

fable and grok-4.7 approved; gpt-6-astra rejected on one item. Each finding, the decision, and
where it now lives.

| Finding | Decision | Where |
|---|---|---|
| R2-1 (blocking): TUI-1's test said a crossterm backend's bytes hold no ESC, which no correct backend passes (it writes its own cursor and style sequences) | test the filter where it acts, on the cells a recording inner backend receives (five bad cells, `漢`'s continuation is a space, count 5); one real crossterm test asserts only that sequences from cell content (`ESC[31m`, `ESC]0;x BEL`) never reach the writer; rule 1 states the line between data and the backend's own control | 03-tui.md §Display integrity rule 1; flow.md §T9; TUI-1 |
| 1: an exact fee match makes confirm fail on any fee drop | confirm signs when `maxGasCost` is equal or lower; all else exactly equal | 03-tui.md §Status of a send; flow.md §T6; TUI-5 |
| 2: where the fingerprint is checked was named only for the review and confirm | pane open and refresh, picker open, before the review, at confirm | 03-tui.md §Password check; flow.md §Password check |
| 3: no TUI-5 test renders `dropped/replaced` on the send-status screen | a `Msg`-sequence test reusing TUI-4's `tx_state` | TUI-5 |
| 4: moving `address --verify` onto the check would make it read the list | two entry points, `open_with_password` and `check_list`; `address --verify` keeps its repairing path | 03-tui.md §Password check; flow.md §Password check; TUI-2, TUI-6 |
| 5: a 0-amount movement in T4 | counted, not shown | 03-tui.md §History (T4 paragraph); flow.md §T4; TUI-4 |
| 6: wrong text.rs reference | `escape` core/text.rs:40-50, `ascii_inert` :69; the other references touched in round 1 rechecked | 03-tui.md rule 4 |
| 7: scrypt freezes the frame | `spawn_blocking` with a `checking...` state | 03-tui.md §Password check; flow.md §Password check; TUI-2 |
| 8: the verdict could be computed on the file as it is at that moment | verdict and fee cap on the checked list only | 03-tui.md §Status of a send; flow.md §T6; TUI-5 |
| 9: "any 4-character piece" of the password collides with address hex | search the whole password and the marked piece `pw-XYZ` | TUI-2 |
| 10: the order "session starts, then the check finds a modified cache" had no test | a TUI-2 counter-example in that order; the same edit before the warm-up fails the warm-up (daemon/mod.rs:311) | TUI-2; 03-tui.md §Password check |

## Review round 3 (2026-10-27)

Trinity primary panel (fable, grok-4.7, gpt-6-astra): round 1 0/3 approve, round 2 2:1, round 3
unanimous APPROVE on e7d896d, no must-fix. Four non-blocking points were applied after consensus:

| Point | Applied | Where |
|---|---|---|
| (fable) the fee compared at confirm is the number the review showed | `max_gas_cost` + `l1_fee` as U256 (chains/evm/rpc.rs:736-744), never formatted strings; `l1_fee` is set by the sequencer at inclusion and cannot be signed; a rise that leaves the balance short is caught by `sufficientFunds`; a base variant of the fee tests | 03-tui.md §Status of a send; flow.md §T6 (`Review::matches`); TUI-5 |
| (fable) the salt does not travel in the snapshot | `Opened { addresses, anchor: Option<WhitelistAnchor>, salted: bool }` (core/store.rs:419), never `SealedWhitelist` with its salt (:461-467); `CheckedList.policy` is the list's own policy, not `Unsealed.policy` | 03-tui.md §Password check; flow.md §Password check; TUI-2 |
| (grok) the "fee falls, so it signs once" test also checks the signed body | the `SignTx` request's transaction is decoded and its `chain_id`, `to`, `value`, `input`, `nonce` must equal the held fields | TUI-5 |
| (gpt-6-astra) one wording for the confirm rule | "the transfer intent is exactly equal, and the most the fee can cost is at most the reviewed one" | 03-tui.md §Changes item 8; this file, Review round 1 row B2 |
