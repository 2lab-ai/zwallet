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
| TUI-1c | zwallet · `feat/tui-1c-pty` · `.worktrees/feat-tui-1c-pty` | the pty runs for T9 and T10, the argv-listed fake, the attacker moved into `tests/common`, the pty helpers (T9, T10) | coding agent + dispatcher gate | PR #31, 2026-10-29: both reviewers approved round 5 (gpt6, code-reviewer); `origin/main` (51b4a51) merged in at ba59846; the A12 ledger (`.prd/v0-1`) rides here. Gate on 17e92a6: Linux fmt, clippy, cli 112, tui 25 green; macOS cli on a pty 111 of 111 (a run beside two other cli suites at load 100-170 failed 12 of 111 on daemon-readiness timeouts). From TUI-1b at 0c248cf: eae51a2 (the attacker and five helpers moved, `tests/cli.rs` imports only), 1948f20 (the pty helpers and the three pty tests; they pass on TUI-1b's code, no `src` change; the assertions were seen failing on a scratch build with the restore, the refresh and the expiry budget broken), 94e5be8 (T10's bound written as 10 s; the fake's expiry read up to its separator), e3cd07c and 016ddcc (docs); 65be4dd (merge of TUI-1b round 5, 5cd6512); d1f8101 (flow.md's reference); review round 1: 6f0c5fe (RED: a valid stream cut inside a sequence counted as stray; the 79x24 check passed on a live snapshot), f5a60cf (stray bytes and the 79x24 screen judged on the last frame, the whole output up to leaving the alternate screen; at 79x24 after both reads are drawn; a scratch build drawing more at 79x24 once the status lands fails it), b53292b (docs); review round 2: 917b311 (RED: T9 reaches the loaded 79x24 screen by a resize instead of beating a refresh; an exited child is reported before a wait's limit), 8175f48 (`PtyChild::resize`; `wait_output` reports an exited child once the reader has every byte, which held on macOS only: on Linux the reader never ended while the test held a slave, fixed in round 3; the 79x24 fixture ends as the real stream does), 6697e57 (docs); review round 3: 268333d (a full-screen child's pty keeps no slave in the test, so the reader ends after the last byte on Linux too; RED: the early-exit test failed 3 of 3 in rust:1.99-bookworm at 6697e57; a macOS probe lost no byte in 200 runs with the parent's slave closed), 08e5d8b (docs); review round 4: 526162d (docs: the stderr-join line for TUI-2 closed); 3b7c548 and a0c644f (merges of TUI-1b: main with TUI-1a merged, then TUI-1b's approval); round 5: fb4fb93 (the tests pass clippy on Linux: `ATTACKER_C` is named in the macOS-only s7 test that uses it, and openpty gets the window as `&raw mut`, since libc declares it `*const` on Linux and `*mut` on macOS; no assertion changes; RED: `cargo clippy --all-targets --locked -- -D warnings` exited 101 in rust:1.99-bookworm at a0c644f on exactly those two, an unused import and `clippy::unnecessary_mut_passed`), then this docs commit; gate on fb4fb93: macOS fmt and clippy clean, the tui suite on a pty 25 of 25; Linux clippy clean, the tui suite 3 of 3. T10 measured 3.1-4.9 s from `zwallet lock` returning, over 22 runs to round 3 (round 3's gate: macOS on a pty 4.68, 4.80, 4.85 s; Linux 4.88, 4.88, 4.87 s) and round 5's gate (macOS on a pty 4.83 s; Linux 4.90, 4.90, 4.91 s). Changed lines against TUI-1b's 5cd6512 (counting rule): tests 1,003 raw at 268333d and 1,006 at fb4fb93, about 814 with the 192 moved lines counted once; `.prd` 48 at 268333d |
| TUI-2a | zwallet · `feat/tui-2-warmup` · `.worktrees/feat-tui-2-warmup` | the TUI-2 entry points and the CLI on them, `s` (T2b), A1 ([`units.md`](units.md) §TUI-2 split) | coding agent + dispatcher gate | in review, 2026-10-28, from TUI-1c at a0c644f: 4f4d54f (RED), af10704, e47b593; 773aa03 (RED: a refused `s` cut in the notice line), 3711370; fbaab35 (docs); 39af5df (RED: Ctrl-C while `s` asks did not quit), 9b2ab25; 700a721 (A1's impostor case keeps its warning line); merges of TUI-1c round 5 (56ac866: fb4fb93, the Linux clippy fix; a1ed46c: 17e92a6); then a docs commit (the measured size). 790 changed lines by the counting rule |
| TUI-2b, TUI-2c | zwallet · branches from TUI-2a | the warm-up, password field and check (T2); pinned addresses on later reads and the open key's trust | coding agent + dispatcher gate | planned, after TUI-2a |
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
  and Linux/macOS CI on the pinned Rust 1.95.0 toolchain.
- Tests start debug binaries with an empty environment and the `/tmp/zwt-` daemon scope;
  production sessions are outside that scope. `lock`, whitelist edits and `L` tests hold
  `common::exclusive()`. Never stop a real session to run tests. Run one full CLI suite at a
  time; the cross-process fixture lock does not make concurrent heavy suites efficient.
- Acceptance frames: tmux at 120×40 and 80×24, `capture-pane`, read by the dispatcher
  (03-tui.md §Acceptance); T3 and T3b on in-test `TestBackend` frames. Every `TestBackend` frame
  goes through `FilteredBackend` with zero replaced cells. No frame of a real wallet's phrase.

## Open gaps

| ID | Acceptance | Gap | Source | Status |
|---|---|---|---|---|
| T2 | warm-up ≈ 1h, open key verified by password, global active key unchanged | no password field or warm-up (the entry points landed in TUI-2a) | TUI-2b | 🚪 |
| T2b | `s` makes the key active | closes when TUI-2a merges: `t2b_on_a_pty_s_then_y_makes_the_selected_key_active` (tests/tui.rs) | TUI-2a | 🚪 |
| T3 | 10 rows, merged, pending first, coverage note | no journal reader, explorer client or merge | TUI-7+8 | 🚪 |
| T3b | explorer unavailable: journal only, note says so | no explorer client | TUI-7+8 | 🚪 |
| T4 | detail fields equal the fixture's, receipt token movements included | `lookup_tx` has no fee, confirmations or token movements | TUI-7+8 | 🚪 |
| T5 | native + non-zero registry tokens | no balances pane | TUI-3+4 | 🚪 |
| T6 | send to `alice`, hash at once, mined, journal line | send path private and prints; no exact-intent check | TUI-5 | 🚪 |
| T6b | picker: the checked list's address entries on this chain only | no picker | TUI-5 | 🚪 |
| T6c | `dropped/replaced` | no code tells it | TUI-7+8 | 🚪 |
| T7 | add `bob` while warm, summary above the password, locked after | edit path reads `/dev/tty` itself | TUI-6 | 🚪 |
| T8 | `c` → base reloads every pane, host redacted | no chain picker | TUI-3+4 | 🚪 |
| T9 | 80×24 fits, smaller says so, terminal restored after `q` and a panic | closes when TUI-1b and TUI-1c merge: the frames and `q` in TUI-1b, the panic restore in TUI-1a's `term.rs` child tests, the pty run `t9_on_a_pty_q_gives_the_terminal_back_in_canonical_mode_with_echo` (tests/tui.rs) in TUI-1c | TUI-1c | 🚪 |
| T10 | `zwallet lock` elsewhere shows `locked` within two refreshes (10 s) | closes when TUI-1b and TUI-1c merge: the `Msg` test in TUI-1b, the pty run `t10_on_a_pty_zwallet_lock_elsewhere_shows_locked_within_two_refreshes` (tests/tui.rs) in TUI-1c | TUI-1c | 🚪 |
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

Merge TUI-1c and TUI-2 (each with RED tests first, the gate above, an outside review). Then the
TUI-3+4 PR, and preview M1: the owner unlocks, sees money on every network, receives and finds
every function from `?`. Use the existing Q1 and B1 pointers; the handoff does not authorize phrase display or a new QR implementation.
