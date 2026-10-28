# v0.2 TUI — units and order

Status: **planned**, 2026-10-27, revised after review rounds 1 and 2 the same day; TUI-1 split
into three PRs on 2026-10-28 (§TUI-1 split). Six units, TUI-1 … TUI-6, each one PR (TUI-1 three:
TUI-1a, TUI-1b, TUI-1c), in dependency order. Each closes the acceptance IDs of
[`../03-tui.md`](../03-tui.md) §Acceptance listed for it; every ID T1–T11 (15 with T2b, T3b, T6b,
T6c) is closed by exactly one unit. The library entry points are the ones [`flow.md`](flow.md)
names in its "Split" column.

**Counting rule.** "Est. lines" is the changed lines of the PR (insertions plus deletions in
`git diff --stat`), tests included, with two exceptions: `Cargo.lock` is not counted (it is
generated; TUI-1's dependency change adds a few hundred lines there), and a block moved unchanged
from `src/commands.rs` into `src/wallet.rs` counts once (`git diff --color-moved` shows it as a
move). Test fixtures are single-line JSON files under `tests/fixtures/` and are counted. The cap
is about 800; TUI-1b, TUI-3, TUI-4 and TUI-5 are estimated at it.

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

| Unit | Scope | Closes | Depends on | Est. lines |
|---|---|---|---|---|
| TUI-1a | ratatui and crossterm, `FilteredBackend` and the terminal guard (`src/tui/term.rs`), `wallet::keys`, `wallet::status`, `wallet::net`, `list` and `status` on them | none | the agent-side v0.1 work merged (loop.md §Start condition) | 610 (measured) |
| TUI-1b | `zwallet tui`, the screens (keys list, status bar, size rule), the cell/style/budget helpers | T1 | TUI-1a | 800 |
| TUI-1c | the pty runs (restore after `q`, `locked` within two refreshes), the argv-listed fake, the pty helpers | T9, T10 | TUI-1b | 350 |
| TUI-2 | warm-up, password field, the password check (checked list, pinned addresses), make active, address trust | T2, T2b | TUI-1 | 750 |
| TUI-3 | home: balance, send journal rows, explorer client, merge | T3, T3b | TUI-2; the explorer re-measure (loop.md §Entry conditions) | 800 |
| TUI-4 | tx detail with receipt token movements, `dropped/replaced`, chain switch | T4, T6c, T8 | TUI-3 | 800 |
| TUI-5 | assets, transfer: picker from the checked list, amount, review, exact-intent confirm, send status | T5, T6, T6b | TUI-4 | 800 |
| TUI-6 | whitelist screens and edits; the pty run across T2, T6 and T7 | T7, T11 | TUI-5 | 750 |

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
| TUI-1b | at dispatch | `Command::Tui { warm, chain }` and its dispatch; `src/tui/mod.rs`, `app.rs`, `ui.rs`, `text.rs`, `style.rs`; `tests/tui.rs` with the cell, style and budget helpers, T1, the T9 frames and the `q` `Msg` test, the T10 `Msg` test, the cached-address counter-example; the README `zwallet tui` section; 03-tui.md §Status bar, the 80x24 layout and rule 3's RPC host row (decisions 2 and 4) | T1 | planned |
| TUI-1c | at dispatch | the T9 pty test (`q`, then canonical mode with echo), the T10 pty test (`zwallet lock` from a second process, `locked` within two refreshes), the argv-listed fake counter-example, the attacker moved into `tests/common`, the pty helpers (decision 5) | T9, T10 | planned |

Decisions (the dispatcher's, 2026-10-28):
1. The keys pane's error is a single-source error view, rule 4's exception: the CLI's
   `VALIDATION_ERROR`, its value escaped and cut at 64 as the CLI does, then wrapped inside the
   pane.
2. The status bar is two lines at the bottom: line 1 chain · key · short address · trust; line 2
   `locked` or `signing open (agent too) until HH:MMZ` (UTC, through
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
    bar shows `signing open (agent too) until` ≈ now+1h and `verified by password` for `a`;
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

## TUI-3 — home: balance, journal rows, explorer, merge

- **Closes:** T3, T3b.
- **Entry points added:** `wallet::balances`, `wallet::brief` (made `pub`),
  `SendJournal::recent` with `JournalRow` (the checks of `EntryText::of`, chains/mod.rs:756),
  `chains::evm::explorer::fetch` (new module, the shape check of 03-tui.md §History),
  `chains::EXPLORERS`, `history::merge` (new module).
- **Files:** `src/chains/mod.rs`, `src/chains/evm/mod.rs`, `src/chains/evm/explorer.rs` (new),
  `src/history.rs` (new), `src/lib.rs`, `src/wallet.rs`, `src/tui/app.rs`, `ui.rs`,
  `tests/fixtures/explorer/*.json` (new, single-line), `tests/tui.rs`.
- **Depends on:** TUI-2 (Home opens after the warm-up); the explorer re-measure (loop.md §Entry
  conditions).
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

## TUI-4 — tx detail, `dropped/replaced`, chain switch

- **Closes:** T4, T6c, T8.
- **Entry points added:** `rpc::tx_detail` (`effectiveGasPrice`, the receipt's `l1Fee` on base and
  optimism, `eth_blockNumber` for confirmations, and the receipt's registry-token `Transfer` logs
  that involve the wallet, 03-tui.md §History), `rpc::tx_state` (`dropped/replaced`: the account's
  `latest` nonce past the transaction's, no receipt). The detail's status uses `tx_state`; TUI-5's
  send-status screen reuses it. The chain switch uses `wallet::net` (TUI-1).
- **Files:** `src/chains/evm/rpc.rs`, `src/chains/evm/mod.rs`, `src/tui/app.rs`, `ui.rs`,
  `tests/evm_offline.rs`, `tests/tui.rs`.
- **Depends on:** TUI-3.
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
  - T8 (`Msg` sequence): `c` then base: every pane reloads for base. (`TestBackend`): with
    `ZWALLET_RPC_BASE` set to a URL with a key in its path, the bar shows
    `https://host/<redacted>`.
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

## TUI-5 — assets and transfer

- **Closes:** T5, T6, T6b.
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
- **Depends on:** TUI-4 (`tx_state`), TUI-2 (the checked list, a warm session).
- **RED tests:**
  - T5 (`TestBackend`): native and USDC non-zero, DAI zero: native and USDC rows only.
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
  - (`Msg` sequence): a `tx_state` result of `dropped/replaced` (TUI-4) on the send-status screen
    renders `dropped/replaced` with the hash and the nonce.
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

- **Closes:** T7, T11.
- **Entry points added:** `wallet::whitelist_view`, `wallet::whitelist_plan`,
  `wallet::whitelist_apply`, and `Change` made `pub` with `Change::add`, `Change::remove`,
  `Change::fee_cap`, `Change::reset`. The CLI's `whitelist` commands call them
  (commands/whitelist.rs:51) and keep their own prompt and printing. T7's verified state and `V`
  use `wallet::open_with_password` and `wallet::check_list` (TUI-2).
- **Files:** `src/wallet.rs`, `src/commands/whitelist.rs`, `src/tui/app.rs`, `ui.rs`,
  `tests/tui.rs`.
- **Depends on:** TUI-5 (the end-to-end run goes through a send), TUI-2 (the password field).
- **RED tests:**
  - T7 (pty, `common::exclusive()`): warm, then `w`, `a`, `bob` and an address, no duration: the
    frame shows the summary above the password field; after the password, `zwallet whitelist list
    --verify --password-stdin` shows `bob` at gen+1, `zwallet status --json` has `locked: true`,
    and T7 shows the new list as verified (the password check ran again). The same with a
    duration of 10m: unlocked until about now+10m.
  - (pty): started warm with no password typed, T7 shows `not verified`; `V` and the password
    show it verified; `zwallet status --json` and the list files are the same before and after
    `V`.
  - T11 (pty, the run of 03-tui.md §Tests: T2 → T6 → T7): no echo; the password is in no captured
    frame; Esc in the middle of the password, then a retry, works.
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
