# v0.2 TUI — loop

Status: **in-progress**, 2026-10-28 (v2: every CLI function). Target = [`ssot.md`](ssot.md),
[`../03-tui.md`](../03-tui.md) §Coverage matrix and acceptance T1–T41. Current code, function by
function: [`flow.md`](flow.md). Units: [`units.md`](units.md). This file holds only open work;
closed rows are in [`loop.done.md`](loop.done.md).

## Start condition

Met: TUI-1a (PR #17) and TUI-1b (PR #18, 24f4b5b) are merged ([`loop.done.md`](loop.done.md)).
G3 and G4 of [`../v0-1/loop.md`](../v0-1/loop.md) wait only on owner actions and do not block TUI
code.

## Entry conditions

- **TUI-7 merges only after the explorer table is measured again** (03-tui.md §History): the
  Blockscout hosts, and Routescan's paths for base, arbitrum and polygon. That data decides
  whether one more parser opens those chains; the decision is recorded in 03-tui.md first.
- **Trade-offs Q and B** (03-tui.md §Open trade-offs) go to review, then to the owner. TUI-4 and
  TUI-10 ship the CLI pointers (Q1, B1) and do not wait; Q3 or B2 lands as a delta once picked.
- **TUI-12 starts after 05-tron.md** (branch `docs/tron-spec`) and its library units.

## Units

| WU | Repo · branch · worktree | Scope | Owner | State |
|---|---|---|---|---|
| TUI-1c | zwallet · `feat/tui-1c-pty` · `.worktrees/feat-tui-1c-pty` | pty runs for T9 and T10, the argv-listed fake, the pty helpers | coding agent + dispatcher gate | in progress (branch at a0c644f, TUI-1b merged in) |
| TUI-2 | zwallet · `feat/tui-2-warmup` · `.worktrees/feat-tui-2-warmup` | warm-up, password field, password check, make active, address trust (T2, T2b) | coding agent + dispatcher gate | in progress: RED 4f4d54f (the TUI-2a slice: `open_with_password`, `check_list`, `s` then `y`, the not-verified note) |
| TUI-3 | zwallet · at dispatch | tabs, action menu, hints, forms; balances, every network, chain switch (T5, T8, T12, T13, T15) | coding agent + dispatcher gate | planned, after TUI-2 |
| TUI-4 | zwallet · at dispatch | receive, `V` on every key, `A` repair, session, `u`, `--allow-ptrace-scope-0` (T14, T16–T20) | coding agent + dispatcher gate | planned, after TUI-3 |
| TUI-5 | zwallet · at dispatch | send: payee, amount, review with overrides and fee ceiling, confirm, status (T6, T6b, T21, T22) | coding agent + dispatcher gate | planned, after TUI-3 |
| TUI-6 | zwallet · at dispatch | whitelist view and every change; pty run across T2, T6, T7 (T7, T11, T23–T25) | coding agent + dispatcher gate | planned, after TUI-5 |
| TUI-7 | zwallet · at dispatch | history: journal, explorer, merge (T3, T3b) | coding agent + dispatcher gate | planned, after TUI-3 and the re-measure |
| TUI-8 | zwallet · at dispatch | transaction detail, `dropped/replaced`, `/` lookup (T4, T6c, T26) | coding agent + dispatcher gate | planned, after TUI-7 and TUI-5 |
| TUI-9 | zwallet · at dispatch | speed up, cancel, retry at nonce N (T27–T29) | coding agent + dispatcher gate | planned, after TUI-8 |
| TUI-10 | zwallet · at dispatch | new key, first-run create, the phrase (T30–T32) | coding agent + dispatcher gate | planned, after TUI-3 |
| TUI-11 | zwallet · at dispatch | call, invoke, sign a message, a token by contract (T33–T37) | coding agent + dispatcher gate | planned, after TUI-5 |
| TUI-12 | zwallet · at dispatch | Tron on every tab (T38–T41) | coding agent + dispatcher gate | planned, after TUI-5 and 05-tron.md |

## Build facts

- Gate: `cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings && cargo test
  --locked`, the cli and tui suites again on a pty ×3 (`script -q /dev/null cargo test --locked
  --test cli < /dev/null`, the same with `--test tui`), live `cargo test --locked -- --ignored`,
  and Linux Docker `rust:1.99-bookworm` as a non-root user.
- The gate stops the user's real session: `lock`, every whitelist change, `L` and the TUI's
  whitelist changes call `stop_all_daemons` for every zwallet daemon of the user; their tests
  hold `common::exclusive()`. Before a gate, `ps` must show no live `zwallet daemon` of the user.
- Acceptance frames: tmux at 120×40 and 80×24, `capture-pane`, read by the dispatcher
  (03-tui.md §Acceptance); T3 and T3b on in-test `TestBackend` frames. Every `TestBackend` frame
  goes through `FilteredBackend` with zero replaced cells. No frame of a real wallet's phrase.

## Open gaps

| ID | Acceptance | Gap | Source | Status |
|---|---|---|---|---|
| T2 | warm-up ≈ 1h, open key verified by password, global active key unchanged | no warm-up, password field or password check in the TUI | TUI-2 | 🚪 |
| T2b | `s` makes the key active | no `wallet::set_active` | TUI-2 | 🚪 |
| T3 | 10 rows, merged, pending first, coverage note | no journal reader, explorer client or merge | TUI-7 | 🚪 |
| T3b | explorer unavailable: journal only, note says so | no explorer client | TUI-7 | 🚪 |
| T4 | detail fields equal the fixture's, receipt token movements included | `lookup_tx` has no fee, confirmations or token movements | TUI-8 | 🚪 |
| T5 | native + non-zero registry tokens | no balances pane | TUI-3 | 🚪 |
| T6 | send to `alice`, hash at once, mined, journal line | send path private and prints; no exact-intent check | TUI-5 | 🚪 |
| T6b | picker: the checked list's address entries on this chain only | no picker | TUI-5 | 🚪 |
| T6c | `dropped/replaced` | no code tells it | TUI-8 | 🚪 |
| T7 | add `bob` while warm, summary above the password, locked after | edit path reads `/dev/tty` itself | TUI-6 | 🚪 |
| T8 | `c` → base reloads every pane, host redacted | no chain picker | TUI-3 | 🚪 |
| T9 | 80×24 fits, smaller says so, terminal restored after `q` and a panic | pty run missing (`TestBackend` frames landed in TUI-1b) | TUI-1c | 🚪 |
| T10 | `zwallet lock` elsewhere shows `locked` within two refreshes | pty run missing (the `Msg` test landed in TUI-1b) | TUI-1c | 🚪 |
| T11 | pty across T2, T6, T7: no echo, no password in a frame, cancel and retry | needs TUI-2, TUI-5, TUI-6 | TUI-6 | 🚪 |
| T12 | the action menu lists every function of rows 1–52 and runs it | no menu | TUI-3 | 🚪 |
| T13 | tabs, hint lines, Esc, every screen at 80×24 | no tabs | TUI-3 | 🚪 |
| T14 | unlock for a typed duration | the warm-up only (TUI-2) | TUI-4 | 🚪 |
| T15 | every network's balances | `every_balance` is private | TUI-3 | 🚪 |
| T16 | the session in full | the bar only | TUI-4 | 🚪 |
| T17 | receive: full address and trust | no receive tab | TUI-4 | 🚪 |
| T18 | the QR code (trade-off Q) | Q not decided; Q1 line missing | TUI-4 | 🚪 |
| T19 | `V` verifies every key; `A` repairs the cache | `address --verify` reads the password itself | TUI-4 | 🚪 |
| T20 | `zwallet tui --allow-ptrace-scope-0` | no such flag | TUI-4 | 🚪 |
| T21 | allow-pending and allow-duplicate on the review | no review | TUI-5 | 🚪 |
| T22 | fee ceiling on the review | no review | TUI-5 | 🚪 |
| T23 | remove, fee cap, reset | edit path reads `/dev/tty` itself | TUI-6 | 🚪 |
| T24 | contract entry with selectors | as T23 | TUI-6 | 🚪 |
| T25 | message signing on or off | as T23 | TUI-6 | 🚪 |
| T26 | look a hash up | no field | TUI-8 | 🚪 |
| T27 | speed up at nonce N | no replacement in a library entry point | TUI-9 | 🚪 |
| T28 | cancel at nonce N | as T27 | TUI-9 | 🚪 |
| T29 | retry after `BROADCAST_UNCERTAIN` | as T27 | TUI-9 | 🚪 |
| T30 | new key, verified, active | `create` reads the password and prints | TUI-10 | 🚪 |
| T31 | first-run create, no word in a frame | as T30 | TUI-10 | 🚪 |
| T32 | the phrase (trade-off B) | B not decided; B1 line missing | TUI-10 | 🚪 |
| T33 | `call` and its bounded result | `call` is private | TUI-11 | 🚪 |
| T34 | invoke a listed contract with a listed selector | `invoke` is private | TUI-11 | 🚪 |
| T35 | invoke with raw calldata | as T34 | TUI-11 | 🚪 |
| T36 | sign a message; refusals at the field | `sign_message` is private | TUI-11 | 🚪 |
| T37 | a token by contract, symbol bounded | `balance --token` is private | TUI-11 | 🚪 |
| T38 | Tron address on 4 receive | no Tron family (05-tron.md) | TUI-12 | 🚪 |
| T39 | TRX and TRC20 USDT balances | as T38 | TUI-12 | 🚪 |
| T40 | send to a listed Tron payee | as T38; how a Tron payee is listed is open | TUI-12 | 🚪 |
| T41 | look a Tron hash up | as T38 | TUI-12 | 🚪 |

## Next

Merge TUI-1c and TUI-2 (each with RED tests first, the gate above, an outside review). Then
TUI-3, the unit after which the owner sees money and finds every function from `?`. Take
trade-offs Q and B to review in parallel.
