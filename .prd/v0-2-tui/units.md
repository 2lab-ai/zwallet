# v0.2 TUI — units and order

Status: **planned**, 2026-10-27. Six units, each one PR, in dependency order. Each closes the
acceptance IDs of [`../03-tui.md`](../03-tui.md) §Acceptance listed for it; every ID T1–T11 (15
with T2b, T3b, T6b, T6c) is closed by exactly one unit. The library entry points are the ones
[`flow.md`](flow.md) names in its "Split" column.

**Counting rule.** "Est. lines" is the changed lines of the PR (insertions plus deletions in
`git diff --stat`), tests included, `Cargo.lock` excluded (it is generated; U1's dependency change
adds a few hundred lines there). The cap is about 800.

**Every unit:**
- RED first: each test below is written and seen failing before the code it checks.
- Every `TestBackend` test ends with the cell rule: every cell of every frame passes
  `core::terminal::terminal_ok` (03-tui.md §Display integrity rule 1). U1 adds the helper.
- At least one counter-example test: text from outside built to break the screen.
- Tests that reach `protocol::stop_all_daemons` (the `L` key, a whitelist change) take
  `common::exclusive()` (tests/common/mod.rs:91), as the CLI's do (v0-1/loop.md:15).
- No library entry point prints; the CLI command that used the code calls the new function and
  prints what it returns, so `tests/cli.rs` stays green unchanged.

| Unit | Scope | Closes | Depends on | Est. lines |
|---|---|---|---|---|
| U1 | `zwallet tui`, terminal setup, cell rules, keys list, status bar, lock detection | T1, T9, T10 | v0.1 complete | 750 |
| U2 | warm-up, password field, make active, address trust in the bar | T2, T2b | U1 | 600 |
| U3 | home: balance, send journal rows, explorer client, merge | T3, T3b | U2 | 800 |
| U4 | tx detail, assets, chain switch | T4, T5, T8 | U3 | 650 |
| U5 | transfer: picker, amount, review, confirm, send status | T6, T6b, T6c | U4 | 800 |
| U6 | whitelist screens and edits; the pty run across T2, T6 and T7 | T7, T11 | U5 | 700 |

## U1 — `zwallet tui`, terminal setup, cell rules, keys, status bar, lock detection

- **Closes:** T1, T9, T10.
- **Adds the `ratatui` and `crossterm` dependencies** (Cargo.toml; the only unit that does).
- **Entry points added:** `wallet::keys`, `wallet::status` (with `Trust` made `pub`),
  `wallet::net`. The `L` key calls `protocol::stop_all_daemons` (exists).
- **Files:** `Cargo.toml`, `Cargo.lock`, `src/lib.rs`, `src/cli.rs` (`Command::Tui { warm, chain
  }`), `src/commands.rs` (`list` and `status` call `wallet::keys` and `wallet::status`; the `tui`
  dispatch), `src/wallet.rs` (new), `src/tui/mod.rs`, `app.rs`, `ui.rs`, `text.rs`, `term.rs`
  (new), `tests/tui.rs` (new).
- **RED tests:**
  - T1 (`TestBackend`): 2 keys, `b` active: both rows, `b` marked, `locked` appears once, in the
    bar.
  - T9 (`TestBackend`): at 80×24 the bar and the list fit; at 79×24 the frame is the one `terminal
    too small` line. (`Msg` sequence): `q` ends the app. (unit, `term.rs`): the restore guard
    writes the leave-alternate-screen and show-cursor sequences to a test writer on drop and from
    the panic hook (`catch_unwind`). (pty, `PtyChild`, tests/common/mod.rs:637): `zwallet tui`,
    then `q`; the pty is back in canonical mode with echo.
  - T10 (`Msg` sequence): a status refresh that reports locked makes the bar say `locked`. (pty):
    with a test daemon unlocked, `zwallet lock` from a second process; within 5 s the captured
    frame shows `locked`.
- **Counter-examples:**
  - `wallet.json` caches as an address `"\u{1b}[2J"` followed by 600 `█`: the T1 row shows the
    CLI's `VALIDATION_ERROR` with the value escaped and cut at 64; the cell rule holds.
  - A `FakeDaemon` (tests/common/mod.rs:503) on `d.sock` answers `status` with an `expiresAt` of
    600 characters of ESC, `▀` and `▄`: the bar shows at most 64 escaped characters on one line.

## U2 — warm-up, password field, make active, address trust

- **Closes:** T2, T2b.
- **Entry points added:** `wallet::prepare_unlock`, `wallet::unlock`, `wallet::address`,
  `wallet::set_active`. The CLI's `unlock` and `change` call them (commands.rs:1912, :930).
- **Files:** `src/wallet.rs`, `src/commands.rs`, `src/tui/app.rs`, `ui.rs`, `password.rs` (new),
  `tests/tui.rs`.
- **Depends on:** U1.
- **RED tests:**
  - T2 (pty): keys `a` and `b`, `b` active, locked. Enter on `a`, type the password: the frame's
    bar shows `signing open (agent too) until` ≈ now+1h; `zwallet status --json` has the same
    `expiresAt`; `zwallet list --json` still has `active: "b"`; the bar's trust for `a` is not
    `verified by password` (readiness verified `b`'s address, 03-tui.md §Address trust).
  - T2b (pty): `s` on `a`, confirm: `zwallet list --json` has `active: "a"`.
  - (`Msg` sequence): `prepare_unlock` runs before the field takes a key; a `Trust::Unverified(Some
    (w))` status makes the bar show the `NOT this build's key daemon` line; on Linux the bar says
    `(no session check on Linux)`.
- **Counter-examples:**
  - A password in a frame: warm up with the password `pw-XYZ-0123456789`. No `TestBackend` frame
    of the sequence and no pty capture contains it or any 4-character piece of it; the field
    shows as many `*` as characters typed. The `Debug` text of every `Msg` sent holds no part of
    it.
  - A `FakeDaemon` answers `address` with 600 `▀▄` characters: the pane shows the CLI's "not an
    address" error escaped; the bar never shows it as an address.

## U3 — home: balance, journal rows, explorer, merge

- **Closes:** T3, T3b.
- **Entry points added:** `wallet::balances`, `wallet::brief` (made `pub`),
  `SendJournal::recent` with `JournalRow` (the checks of `EntryText::of`, chains/mod.rs:756),
  `chains::evm::explorer::fetch` (new module, the shape check of 03-tui.md §History),
  `chains::EXPLORERS`, `history::merge` (new module).
- **Files:** `src/chains/mod.rs`, `src/chains/evm/mod.rs`, `src/chains/evm/explorer.rs` (new),
  `src/history.rs` (new), `src/lib.rs`, `src/wallet.rs`, `src/tui/app.rs`, `ui.rs`,
  `tests/tui.rs`.
- **Depends on:** U2 (Home opens after the warm-up).
- **RED tests:**
  - Explorer fixture: a local HTTP server in the test, built like `tiny_rpc_reply`
    (tests/common/mod.rs:926) but serving Blockscout bodies cut from the 2026-10-27 captures.
  - T3 (`TestBackend`, the `App` built with the fixture explorer and a fixture journal file): 14
    transactions, one hash in both the journal and the explorer, 2 pending: exactly 10 rows,
    pending first, that hash once with both sources, the coverage note shown.
  - T3b (`TestBackend`): the fixture explorer answers 403 `text/html`: journal rows only, the
    note says `explorer unavailable (HTTP 403)`, no banner over the list.
  - (unit, `explorer.rs`): each kept field at its shape's edges (64 and 65 hex digits, 78 and 79
    decimal digits, `null` block number); an item about another address is dropped.
  - (unit, `history.rs`): the merge's order (pending, block descending, position) and
    de-duplication by `(chainId, txHash)`.
  - (live, `--ignored`): `eth.blockscout.com` for a known address parses with no item dropped
    for shape.
- **Counter-examples (a forged explorer page):** an item whose `from.name` is `zwallet: verified`,
  a token transfer of a non-registry contract whose `token.symbol` is `USDC`, a 63-digit `hash`, a
  `method` of 600 characters with ESC and `█`, an item about another address, and a 403 body made
  of `▀▄`. None of their text is in any cell; the note counts the dropped items; the cell rule
  holds.

## U4 — tx detail, assets, chain switch

- **Closes:** T4, T5, T8.
- **Entry points added:** `rpc::tx_detail` (`effectiveGasPrice`, the receipt's `l1Fee` on base and
  optimism, `eth_blockNumber` for confirmations, a registry token's `transfer` decoded). Assets
  use `wallet::balances` (U3); the chain switch uses `wallet::net` (U1).
- **Files:** `src/chains/evm/rpc.rs`, `src/chains/evm/mod.rs`, `src/tui/app.rs`, `ui.rs`,
  `tests/evm_offline.rs`, `tests/tui.rs`.
- **Depends on:** U3.
- **RED tests:**
  - T4 (mock node, `MockRpc`, tests/evm_offline.rs:40): a fixture transaction and receipt with
    `gasUsed`, `effectiveGasPrice` and, on base, `l1Fee`, and a fixture head block: every detail
    field equals the value written in the fixture; a USDC `transfer` shows its USDC amount.
  - T5 (`TestBackend`): native and USDC non-zero, DAI zero: native and USDC rows only.
  - T8 (`Msg` sequence): `c` then base: every pane reloads for base. (`TestBackend`): with
    `ZWALLET_RPC_BASE` set to a URL with a key in its path, the bar shows
    `https://host/<redacted>`.
- **Counter-examples:**
  - The mock node answers `eth_getTransactionReceipt` with an error of 600 characters of ESC and
    `█`: one error line within the pane's width; the detail view shows at most 512 escaped
    characters; the cell rule holds.
  - A `transfer` call to a contract outside the registry: no amount is decoded; the detail says
    it is not a registry token.

## U5 — transfer and send status

- **Closes:** T6, T6b, T6c.
- **Entry points added:** `wallet::review_transfer` (prepare as a dry-run, duplicates, the
  whitelist check on disk; no lock, no signature), `wallet::send_transfer` (lock, prepare again,
  sign, check the signer, journal, broadcast; a changed review signs nothing), `SendRecord::write`
  returning its line instead of printing it, `rpc::tx_state` (`dropped/replaced`). The CLI's
  `transfer` calls them, `invoke` keeps the signing tail they are cut from (commands.rs:1530,
  :1421, :1631), and both print the `broadcasting` line as before.
- **Files:** `src/wallet.rs`, `src/commands.rs`, `src/chains/mod.rs`, `src/chains/evm/rpc.rs`,
  `src/chains/evm/mod.rs`, `src/tui/app.rs`, `ui.rs`, `tests/evm_offline.rs`, `tests/tui.rs`.
- **Depends on:** U4 (Assets opens the transfer), U2 (a warm session).
- **RED tests:**
  - T6 (pty, mock node through `ZWALLET_RPC_ETHEREUM`): asset → `alice` → amount → review → type
    `alice`: the hash is in the next frame; the status reaches mined; the journal file has the
    line.
  - T6b (`TestBackend`): the picker lists `alice` (address entry on ethereum) and not a contract
    entry nor an entry on base only; (`Msg` sequence): typed `0x...` characters in the picker
    change nothing.
  - T6c (mock node): no receipt for the hash and `eth_getTransactionCount` at `latest` past its
    nonce: status `dropped/replaced`.
  - (mock node): the fee rises between review and confirm: a new review, nothing signed (the mock
    saw no `eth_sendRawTransaction`).
  - (pty): no `broadcasting` line appears outside the frame; `zwallet transfer` still prints it
    (tests/cli.rs unchanged).
- **Counter-examples:**
  - A journal line whose `to` is 600 characters with ESC and `█`: the duplicate guard shows
    `<invalid>` for it, within 512.
  - A planted `whitelist.json` at gen N+1 with `alice` at a lookalike address while the session
    signs under gen N: the picker offers nothing and says the file is not the list this session
    signs under.

## U6 — whitelist screens and edits; the pty run across T2, T6 and T7

- **Closes:** T7, T11.
- **Entry points added:** `wallet::whitelist_view`, `wallet::whitelist_verify`,
  `wallet::whitelist_plan`, `wallet::whitelist_apply`, and `Change` made `pub` with
  `Change::add`, `Change::remove`, `Change::fee_cap`, `Change::reset`. The CLI's `whitelist`
  commands call them (commands/whitelist.rs:51) and keep their own prompt and printing.
- **Files:** `src/wallet.rs`, `src/commands/whitelist.rs`, `src/tui/app.rs`, `ui.rs`,
  `tests/tui.rs`.
- **Depends on:** U5 (the end-to-end run goes through a send), U2 (the password field).
- **RED tests:**
  - T7 (pty, `common::exclusive()`): warm, then `w`, `a`, `bob` and an address, no duration: the
    frame shows the summary above the password field; after the password, `zwallet whitelist list
    --verify --password-stdin` shows `bob` at gen+1 and `zwallet status --json` has `locked:
    true`. The same with a duration of 10m: unlocked until about now+10m.
  - T11 (pty, the run of 03-tui.md §Tests: T2 → T6 → T7): no echo; the password is in no captured
    frame; Esc in the middle of the password, then a retry, works.
- **Counter-examples:**
  - `whitelist.json` with a 600-character nickname, or a nickname of `▀▄█`, or one holding ESC:
    not a list at parse (`check_shapes`, core/whitelist.rs:689); T7 shows the "cannot be read"
    line with the parser's message cut at 80; the cell rule holds.
  - The password in a frame, as in U2, across the edit's summary, password and outcome frames.
