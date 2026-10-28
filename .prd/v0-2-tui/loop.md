# v0.2 TUI — loop

Status: **in-progress**, 2026-10-28 (v2: every CLI function; revised after review rounds 1 and 2 of v2).
Target = [`ssot.md`](ssot.md), [`../03-tui.md`](../03-tui.md) §Coverage matrix and acceptance
T1–T44. Current code, function by
function: [`flow.md`](flow.md). Units: [`units.md`](units.md). This file holds only open work;
closed rows are in [`loop.done.md`](loop.done.md).

## Start condition

Met: TUI-1a (PR #17) and TUI-1b (PR #18, 24f4b5b) are merged ([`loop.done.md`](loop.done.md)).
G3 and G4 of [`../v0-1/loop.md`](../v0-1/loop.md) wait only on owner actions and do not block TUI
code.

## Entry conditions

- **Satisfied 2026-10-28: the explorer re-measure that gated TUI-7+8.** Receipt: the dispatcher's record `explorer-remeasure/record.md` with the raw bodies, in the session scratchpad, outside the repo
  (04:50:25Z–04:50:46Z), copied into 03-tui.md §History (re-measured 2026-10-28). Decision recorded
  there: no extra parser for base, arbitrum or polygon in v0.2; avalanche through Routescan stays
  out; the coverage per chain is unchanged. Nothing gates TUI-7+8 now but TUI-5.
- **Trade-offs Q and B** (03-tui.md §Open trade-offs) go to the owner. TUI-3+4 and TUI-10 build
  the CLI pointers (Q1, B1) and do not wait; Q3 lands as a delta of T18 if picked; B2 is built
  only if the owner picks it (recommended: B1).
- **TUI-12 starts after 05-tron.md** (branch `docs/tron-spec`) and its library units.

## PRs

One risk boundary per PR ([`units.md`](units.md) §PRs and previews, the dispatcher's plan after
review round 1 of v2, 2026-10-28).

| WU | Repo · branch · worktree | Scope | Owner | State |
|---|---|---|---|---|
| TUI-1c | zwallet · `feat/tui-1c-pty` · `.worktrees/feat-tui-1c-pty` | pty runs for T9 and T10, the argv-listed fake, the pty helpers | coding agent + dispatcher gate | in progress (branch at a0c644f, TUI-1b merged in) |
| TUI-2 | zwallet · `feat/tui-2-warmup` · `.worktrees/feat-tui-2-warmup` | warm-up, password field, password check, make active, address trust (T2, T2b) | coding agent + dispatcher gate | in progress: RED 4f4d54f (the TUI-2a slice: `open_with_password`, `check_list`, `s` then `y`, the not-verified note) |
| TUI-3+4 | zwallet · at dispatch | "see and receive": `actions.rs` and the form helpers first; tabs, menu, hints; balances, every network, chain switch and session RPC; receive, `V` on every key, `A`, session, `u`, the start flag `--allow-ptrace-scope-0` (T5, T8, T12–T20, T42) | coding agent + dispatcher gate | planned, after TUI-2 |
| TUI-5 | zwallet · at dispatch | send: payee, amount, review with overrides and fee ceiling, confirm, status, `w` (T6, T6b, T21, T22, T43) | coding agent + dispatcher gate | planned, after TUI-3+4 |
| TUI-6 | zwallet · at dispatch | whitelist view and every change, the zero-address refusal at add; pty run across T2, T6, T7 (T7, T11, T23–T25, T44) | coding agent + dispatcher gate | planned, right after TUI-5, its own review |
| TUI-7+8 | zwallet · at dispatch | history (journal, explorer, merge); transaction detail, `dropped/replaced`, `/` lookup (T3, T3b, T4, T6c, T26) | coding agent + dispatcher gate | planned, after TUI-5 (the explorer re-measure is done, 2026-10-28) |
| TUI-9 | zwallet · at dispatch | speed up, cancel, retry at nonce N (T27–T29) | coding agent + dispatcher gate | planned, after TUI-7+8 |
| TUI-10 | zwallet · at dispatch | new key, first-run create, `B` as B1 (T30–T32) | coding agent + dispatcher gate | planned, after TUI-3+4 |
| TUI-11 | zwallet · at dispatch | call, invoke and its replacements, sign a message, a token by contract (T33–T37) | coding agent + dispatcher gate | planned, after TUI-9 |
| TUI-12 | zwallet · at dispatch | Tron on every tab (T38–T41) | coding agent + dispatcher gate | planned, after TUI-5 and the Tron library PRs |

## Previews

| Preview | After | State |
|---|---|---|
| M1 | TUI-2 and TUI-3+4 | planned |
| M2 | TUI-5 and TUI-6 | planned |
| M3 | TUI-7+8 and TUI-9 | planned |
| M4 | TUI-10 and TUI-11 | planned |
| M5 | TUI-12 | planned |

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
| T3 | 10 rows, merged, pending first, coverage note | no journal reader, explorer client or merge | TUI-7+8 | 🚪 |
| T3b | explorer unavailable: journal only, note says so | no explorer client | TUI-7+8 | 🚪 |
| T4 | detail fields equal the fixture's, receipt token movements included | `lookup_tx` has no fee, confirmations or token movements | TUI-7+8 | 🚪 |
| T5 | native + non-zero registry tokens | no balances pane | TUI-3+4 | 🚪 |
| T6 | send to `alice`, hash at once, mined, journal line | send path private and prints; no exact-intent check | TUI-5 | 🚪 |
| T6b | picker: the checked list's address entries on this chain only | no picker | TUI-5 | 🚪 |
| T6c | `dropped/replaced` | no code tells it | TUI-7+8 | 🚪 |
| T7 | add `bob` while warm, summary above the password, locked after | edit path reads `/dev/tty` itself | TUI-6 | 🚪 |
| T8 | `c` → base reloads every pane, host redacted | no chain picker | TUI-3+4 | 🚪 |
| T9 | 80×24 fits, smaller says so, terminal restored after `q` and a panic | pty run missing (`TestBackend` frames landed in TUI-1b) | TUI-1c | 🚪 |
| T10 | `zwallet lock` elsewhere shows `locked` within two refreshes | pty run missing (the `Msg` test landed in TUI-1b) | TUI-1c | 🚪 |
| T11 | pty across T2, T6, T7: no echo, no password in a frame, cancel and retry | needs TUI-2, TUI-5, TUI-6 | TUI-6 | 🚪 |
| T12 | the menu's 52 rows equal the Alias column of rows 1–52; each runs or says why not | no menu | TUI-3+4 | 🚪 |
| T13 | tabs, the hint rule, Esc, the PR's frames at 80×24 | no tabs | TUI-3+4 | 🚪 |
| T14 | unlock for a typed duration | the warm-up only (TUI-2) | TUI-3+4 | 🚪 |
| T15 | every network's balances, all 8 RPCs on mocks | `every_balance` is private | TUI-3+4 | 🚪 |
| T16 | the session in full | the bar only | TUI-3+4 | 🚪 |
| T17 | receive: full address and trust | no receive tab | TUI-3+4 | 🚪 |
| T18 | the QR code (trade-off Q) | Q not decided; Q1 line missing | TUI-3+4 | 🚪 |
| T19 | `V` verifies every key; `A` repairs the cache | `address --verify` reads the password itself | TUI-3+4 | 🚪 |
| T20 | the start flag only; `ptrace_scope 0 accepted` shown; no key sets it | no such flag | TUI-3+4 | 🚪 |
| T21 | allow-pending and allow-duplicate on the review | no review | TUI-5 | 🚪 |
| T22 | fee ceiling on the review | no review | TUI-5 | 🚪 |
| T23 | remove, fee cap, reset | edit path reads `/dev/tty` itself | TUI-6 | 🚪 |
| T24 | contract entry with selectors | as T23 | TUI-6 | 🚪 |
| T25 | message signing on or off | as T23 | TUI-6 | 🚪 |
| T26 | look a hash up | no field | TUI-7+8 | 🚪 |
| T27 | speed up at nonce N: pinned address, node-confirmed intent, cap | no replacement in a library entry point | TUI-9 | 🚪 |
| T28 | cancel at nonce N | as T27 | TUI-9 | 🚪 |
| T29 | retry after `BROADCAST_UNCERTAIN`, from this session's review only | as T27 | TUI-9 | 🚪 |
| T30 | new key, verified, active | `create` reads the password and prints | TUI-10 | 🚪 |
| T31 | first-run create, no phrase or 3-word run in a frame | as T30 | TUI-10 | 🚪 |
| T32 | the phrase: B1's line (B2 only if picked) | B1 line missing | TUI-10 | 🚪 |
| T33 | `call` and its bounded result | `call` is private | TUI-11 | 🚪 |
| T34 | invoke a listed contract and selector; its replacements judged as invoke | `invoke` is private | TUI-11 | 🚪 |
| T35 | invoke with raw calldata | as T34 | TUI-11 | 🚪 |
| T36 | sign a message: the message block, bytes shown = bytes signed | `sign_message` is private | TUI-11 | 🚪 |
| T37 | a token by contract, symbol bounded | `balance --token` is private | TUI-11 | 🚪 |
| T38 | Tron address on 4 receive | no Tron family (05-tron.md) | TUI-12 | 🚪 |
| T39 | TRX and TRC20 USDT balances | as T38 | TUI-12 | 🚪 |
| T40 | send to a listed Tron payee | as T38; how a Tron payee is listed is open | TUI-12 | 🚪 |
| T41 | look a Tron hash up | as T38 | TUI-12 | 🚪 |
| T42 | an RPC URL for this session, host shown, chain id checked | `wallet::net` takes no override | TUI-3+4 | 🚪 |
| T43 | keep waiting for a typed duration | the TUI has no send status | TUI-5 | 🚪 |
| T44 | zero address refused at add; an old entry flagged, removable | nothing refuses it at add | TUI-6 | 🚪 |

## Next

Merge TUI-1c and TUI-2 (each with RED tests first, the gate above, an outside review). Then the
TUI-3+4 PR, and preview M1: the owner unlocks, sees money on every network, receives and finds
every function from `?`. Take trade-offs Q and B to the owner in parallel.
