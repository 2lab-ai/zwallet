# v0.2 TUI — loop

Status: **in-progress**, 2026-10-29 (v2: every CLI function; revised after review rounds 1 and 2 of v2).
Target = [`ssot.md`](ssot.md), [`../03-tui.md`](../03-tui.md) §Coverage matrix and acceptance
T1–T44. Current code, function by
function: [`flow.md`](flow.md). Units: [`units.md`](units.md). This file holds only open work;
closed rows are in [`loop.done.md`](loop.done.md).

## Start condition

Met: TUI-1a (PR #17) and TUI-1b (PR #18, 24f4b5b) are merged ([`loop.done.md`](loop.done.md)).
G3 and G4 of [`../v0-1/loop.md`](../v0-1/loop.md) wait only on owner actions and do not block TUI
code.

## Entry conditions

- **Satisfied 2026-10-28: the explorer re-measure that gated TUI-7+8.** Receipt: [`../research/explorer-remeasure-2026-10-28.md`](../research/explorer-remeasure-2026-10-28.md), every status line and the decision
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
| TUI-1c | zwallet · `feat/tui-1c-pty` · `.worktrees/feat-tui-1c-pty` | the pty runs for T9 and T10, the argv-listed fake, the attacker moved into `tests/common`, the pty helpers (T9, T10) | coding agent + dispatcher gate | implemented (handoff #31 via #35); on `integ/tui-tron-final`, merge pending |
| TUI-2a | zwallet · `feat/tui-2-warmup` · `.worktrees/feat-tui-2-warmup` | the TUI-2 entry points and the CLI on them, `s` (T2b), A1 ([`units.md`](units.md) §TUI-2 split) | coding agent + dispatcher gate | implemented (handoff #32 via #35); on `integ/tui-tron-final`, merge pending |
| TUI-2b, TUI-2c | zwallet · branches from TUI-2a | the warm-up, password field and check (T2); pinned addresses on later reads and the open key's trust | coding agent + dispatcher gate | implemented (handoff #38); on `integ/tui-tron-final`, merge pending |
| TUI-3+4 | zwallet · at dispatch | "see and receive": `actions.rs` and the form helpers first; tabs, menu, hints; balances, every network, chain switch and session RPC; receive, `V` on every key, `A`, session, `u`, the start flag `--allow-ptrace-scope-0` (T5, T8, T12–T20, T42) | coding agent + dispatcher gate | implemented (handoff #39); on `integ/tui-tron-final`, merge pending |
| TUI-5 | zwallet · at dispatch | send: payee, amount, review with overrides and fee ceiling, confirm, status, `w` (T6, T6b, T21, T22, T43) | coding agent + dispatcher gate | implemented (handoff #44); on `integ/tui-tron-final`, merge pending |
| TUI-6 | zwallet · at dispatch | whitelist view and every change, the zero-address refusal at add; pty run across T2, T6, T7 (T7, T11, T23–T25, T44) | coding agent + dispatcher gate | implemented (handoff #47); on `integ/tui-tron-final`, merge pending |
| TUI-7+8 | zwallet · at dispatch | history (journal, explorer, merge); transaction detail, `dropped/replaced`, `/` lookup (T3, T3b, T4, T6c, T26) | coding agent + dispatcher gate | implemented (handoff #45); on `integ/tui-tron-final`, merge pending |
| TUI-9 | zwallet · at dispatch | speed up, cancel, retry at nonce N (T27–T29) | coding agent + dispatcher gate | implemented (handoff #46); on `integ/tui-tron-final`, merge pending |
| TUI-10 | zwallet · at dispatch | new key, first-run create, `B` as B1 (T30–T32) | coding agent + dispatcher gate | implemented (handoff #43); on `integ/tui-tron-final`, merge pending |
| TUI-11 | zwallet · at dispatch | call, invoke and its replacements, sign a message, a token by contract (T33–T37) | coding agent + dispatcher gate | implemented (handoff #48); on `integ/tui-tron-final`, merge pending |
| TUI-12 | zwallet · at dispatch | Tron on every tab (T38–T41) | coding agent + dispatcher gate | implemented (handoff #49); on `integ/tui-tron-final`, merge pending |

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
  and Linux/macOS CI on the pinned Rust 1.95.0 toolchain.
- Tests start debug binaries with an empty environment and the `/tmp/zwt-` daemon scope;
  production sessions are outside that scope. `lock`, whitelist edits and `L` tests hold
  `common::exclusive()`. Never stop a real session to run tests. Run one full CLI suite at a
  time; the cross-process fixture lock does not make concurrent heavy suites efficient.
- Acceptance frames: tmux at 120×40 and 80×24, `capture-pane`, read by the dispatcher
  (03-tui.md §Acceptance); T3 and T3b on in-test `TestBackend` frames. Every `TestBackend` frame
  goes through `FilteredBackend` with zero replaced cells. No frame of a real wallet's phrase.

## Open gaps

As of 2026-10-30 every unit below is implemented on `integ/tui-tron-final`, which merges
the handoff PRs #35 and #37–#49 with main; each row stays open until that PR merges, then moves
to [`loop.done.md`](loop.done.md). Only T18's owner trade-off remains open after that.

| ID | Acceptance | Gap | Source | Status |
|---|---|---|---|---|
| T2 | warm-up ≈ 1h, open key verified by password, global active key unchanged | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [tui2-warmup-receipts.md](tui2-warmup-receipts.md); closes when that PR merges | TUI-2b | 🟡 |
| T2b | `s` makes the key active | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [tui2-warmup-receipts.md](tui2-warmup-receipts.md); closes when that PR merges | TUI-2a | 🟡 |
| T3 | 10 rows, merged, pending first, coverage note | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [h17-history-receipt.md](h17-history-receipt.md); closes when that PR merges | TUI-7+8 | 🟡 |
| T3b | explorer unavailable: journal only, note says so | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [h17-history-receipt.md](h17-history-receipt.md); closes when that PR merges | TUI-7+8 | 🟡 |
| T4 | detail fields equal the fixture's, receipt token movements included | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [h17-history-receipt.md](h17-history-receipt.md); closes when that PR merges | TUI-7+8 | 🟡 |
| T5 | native + non-zero registry tokens | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [navigation receipt](../research/handoff-tui-navigation-2026-10-29.md); closes when that PR merges | TUI-3+4 | 🟡 |
| T6 | send to `alice`, hash at once, mined, journal line | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [send receipt](../research/handoff-tui-send-2026-10-29.md); closes when that PR merges | TUI-5 | 🟡 |
| T6b | picker: the checked list's address entries on this chain only | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [send receipt](../research/handoff-tui-send-2026-10-29.md); closes when that PR merges | TUI-5 | 🟡 |
| T6c | `dropped/replaced` | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [h17-history-receipt.md](h17-history-receipt.md); closes when that PR merges | TUI-7+8 | 🟡 |
| T7 | add `bob` while warm, summary above the password, locked after | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [whitelist receipt](../research/handoff-tui-whitelist-2026-10-29.md); closes when that PR merges | TUI-6 | 🟡 |
| T8 | `c` → base reloads every pane, host redacted | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [navigation receipt](../research/handoff-tui-navigation-2026-10-29.md); closes when that PR merges | TUI-3+4 | 🟡 |
| T9 | 80×24 fits, smaller says so, terminal restored after `q` and a panic | closes when TUI-1b and TUI-1c merge: the frames and `q` in TUI-1b, the panic restore in TUI-1a's `term.rs` child tests, the pty run `t9_on_a_pty_q_gives_the_terminal_back_in_canonical_mode_with_echo` (tests/tui.rs) in TUI-1c; implemented on `integ/tui-tron-final` (#35) | TUI-1c | 🟡 |
| T10 | `zwallet lock` elsewhere shows `locked` within two refreshes (10 s) | closes when TUI-1b and TUI-1c merge: the `Msg` test in TUI-1b, the pty run `t10_on_a_pty_zwallet_lock_elsewhere_shows_locked_within_two_refreshes` (tests/tui.rs) in TUI-1c; implemented on `integ/tui-tron-final` (#35) | TUI-1c | 🟡 |
| T11 | pty across T2, T6, T7: no echo, no password in a frame, cancel and retry | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [whitelist receipt](../research/handoff-tui-whitelist-2026-10-29.md); closes when that PR merges | TUI-6 | 🟡 |
| T12 | the menu's 52 rows equal the Alias column of rows 1–52; each runs or says why not | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [navigation receipt](../research/handoff-tui-navigation-2026-10-29.md); closes when that PR merges | TUI-3+4 | 🟡 |
| T13 | tabs, the hint rule, Esc, the PR's frames at 80×24 | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [navigation receipt](../research/handoff-tui-navigation-2026-10-29.md); closes when that PR merges | TUI-3+4 | 🟡 |
| T14 | unlock for a typed duration | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [navigation receipt](../research/handoff-tui-navigation-2026-10-29.md); closes when that PR merges | TUI-3+4 | 🟡 |
| T15 | every network's balances, all 8 RPCs on mocks | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [navigation receipt](../research/handoff-tui-navigation-2026-10-29.md); closes when that PR merges | TUI-3+4 | 🟡 |
| T16 | the session in full | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [navigation receipt](../research/handoff-tui-navigation-2026-10-29.md); closes when that PR merges | TUI-3+4 | 🟡 |
| T17 | receive: full address and trust | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [navigation receipt](../research/handoff-tui-navigation-2026-10-29.md); closes when that PR merges | TUI-3+4 | 🟡 |
| T18 | the QR code (trade-off Q) | the Q1 pointer (`Q` names `zwallet address`) is implemented on `integ/tui-tron-final`; Q3 waits on the owner's trade-off Q | TUI-3+4 | 🚪 |
| T19 | `V` verifies every key; `A` repairs the cache | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [navigation receipt](../research/handoff-tui-navigation-2026-10-29.md); closes when that PR merges | TUI-3+4 | 🟡 |
| T20 | the start flag only; `ptrace_scope 0 accepted` shown; no key sets it | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [navigation receipt](../research/handoff-tui-navigation-2026-10-29.md); closes when that PR merges | TUI-3+4 | 🟡 |
| T21 | allow-pending and allow-duplicate on the review | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [send receipt](../research/handoff-tui-send-2026-10-29.md); closes when that PR merges | TUI-5 | 🟡 |
| T22 | fee ceiling on the review | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [send receipt](../research/handoff-tui-send-2026-10-29.md); closes when that PR merges | TUI-5 | 🟡 |
| T23 | remove, fee cap, reset | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [whitelist receipt](../research/handoff-tui-whitelist-2026-10-29.md); closes when that PR merges | TUI-6 | 🟡 |
| T24 | contract entry with selectors | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [whitelist receipt](../research/handoff-tui-whitelist-2026-10-29.md); closes when that PR merges | TUI-6 | 🟡 |
| T25 | message signing on or off | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [whitelist receipt](../research/handoff-tui-whitelist-2026-10-29.md); closes when that PR merges | TUI-6 | 🟡 |
| T26 | look a hash up | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [h17-history-receipt.md](h17-history-receipt.md); closes when that PR merges | TUI-7+8 | 🟡 |
| T27 | speed up at nonce N: pinned address, node-confirmed intent, cap | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [replacement receipt](../research/handoff-tui-replacement-2026-10-29.md); closes when that PR merges | TUI-9 | 🟡 |
| T28 | cancel at nonce N | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [replacement receipt](../research/handoff-tui-replacement-2026-10-29.md); closes when that PR merges | TUI-9 | 🟡 |
| T29 | retry after `BROADCAST_UNCERTAIN`, from this session's review only | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [replacement receipt](../research/handoff-tui-replacement-2026-10-29.md); closes when that PR merges | TUI-9 | 🟡 |
| T30 | new key, verified, active | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [creation receipt](../research/handoff-tui-create-ui-2026-10-29.md); closes when that PR merges | TUI-10 | 🟡 |
| T31 | first-run create, no phrase or 3-word run in a frame | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [creation receipt](../research/handoff-tui-create-ui-2026-10-29.md); closes when that PR merges | TUI-10 | 🟡 |
| T32 | the phrase: B1's line (B2 only if picked) | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [creation receipt](../research/handoff-tui-create-ui-2026-10-29.md); closes when that PR merges | TUI-10 | 🟡 |
| T33 | `call` and its bounded result | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [tools receipt](../research/handoff-tui-tools-2026-10-29.md); closes when that PR merges | TUI-11 | 🟡 |
| T34 | invoke a listed contract and selector; its replacements judged as invoke | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [tools receipt](../research/handoff-tui-tools-2026-10-29.md); closes when that PR merges | TUI-11 | 🟡 |
| T35 | invoke with raw calldata | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [tools receipt](../research/handoff-tui-tools-2026-10-29.md); closes when that PR merges | TUI-11 | 🟡 |
| T36 | sign a message: the message block, bytes shown = bytes signed | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [tools receipt](../research/handoff-tui-tools-2026-10-29.md); closes when that PR merges | TUI-11 | 🟡 |
| T37 | a token by contract, symbol bounded | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [tools receipt](../research/handoff-tui-tools-2026-10-29.md); closes when that PR merges | TUI-11 | 🟡 |
| T38 | Tron address on 4 receive | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [h18-tron-tui-receipt.md](h18-tron-tui-receipt.md), [h18b](h18b-tron-history-receipt.md); closes when that PR merges | TUI-12 | 🟡 |
| T39 | TRX and TRC20 USDT balances | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [h18-tron-tui-receipt.md](h18-tron-tui-receipt.md), [h18b](h18b-tron-history-receipt.md); closes when that PR merges | TUI-12 | 🟡 |
| T40 | send to a listed Tron payee | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [h18-tron-tui-receipt.md](h18-tron-tui-receipt.md), [h18b](h18b-tron-history-receipt.md); closes when that PR merges | TUI-12 | 🟡 |
| T41 | look a Tron hash up | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [h18-tron-tui-receipt.md](h18-tron-tui-receipt.md), [h18b](h18b-tron-history-receipt.md); closes when that PR merges | TUI-12 | 🟡 |
| T42 | an RPC URL for this session, host shown, chain id checked | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [navigation receipt](../research/handoff-tui-navigation-2026-10-29.md); closes when that PR merges | TUI-3+4 | 🟡 |
| T43 | keep waiting for a typed duration | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [send receipt](../research/handoff-tui-send-2026-10-29.md); closes when that PR merges | TUI-5 | 🟡 |
| T44 | zero address refused at add; an old entry flagged, removable | implemented on `integ/tui-tron-final` (#35, #37–#49 with main); evidence: [whitelist receipt](../research/handoff-tui-whitelist-2026-10-29.md); closes when that PR merges | TUI-6 | 🟡 |

## Open for TUI-2b

From TUI-1c's review rounds 1 and 2 (2026-10-28), for the first unit whose pty runs resize or
print after the TUI. Narrowed by TUI-2a, whose only pty run (T2b) does neither: it adds no code on
the resize path (the input thread is unchanged), and writes nothing after the terminal is given
back (a refused `s` is drawn in the pane). TUI-2b rewrites the input thread for password mode. (Round 4 closed a third line, the stderr join in `PtyChild::stderr()`: a daemon
the warm-up starts inherits neither that pipe nor the pty slave, since `spawn_daemon` gives it
`Stdio::null()` for stderr and marks every fd from 3 up close-on-exec, src/core/protocol.rs:1354,
:1363.)
- T9 shrinks a running terminal (120x40 to 79x24) and sees the warning alone, but does not tell
  whether SIGWINCH's `Msg::Resize` (src/tui/app.rs:160) or the next refresh tick redrew it (a
  probe on 2026-10-28 saw the redraw 5 ms after the resize), and no test grows the window back.
- `quit()`'s stray and pending check (tests/tui.rs) covers only the bytes before the first
  `CSI ?1049l` (`Frame::last`, tests/common/mod.rs:1536). What is written after it goes
  unchecked: the line for a failed `L` stop, and the panic hook's fixed line.

## Next

Merge `integ/tui-tron-final` as one PR (it marks #35 and #37–#49 merged), then move the 🟡 rows
above to [`loop.done.md`](loop.done.md). The previews M1–M5 can collapse into one preview of that
merge: the owner unlocks again (new cdhash) and accepts the TUI on it. T18's trade-off Q stays
with the owner; the existing Q1 and B1 pointers remain the defaults, and the handoff does not
authorize phrase display or a new QR implementation.
