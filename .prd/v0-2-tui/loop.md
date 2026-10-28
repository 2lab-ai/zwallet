# v0.2 TUI — loop

Status: **in-progress**, 2026-10-27. Target = [`ssot.md`](ssot.md), [`../03-tui.md`](../03-tui.md)
acceptance T1–T11. Current code, function by function: [`flow.md`](flow.md). Units:
[`units.md`](units.md). This file holds only open work.

## Start condition

TUI-1 starts once the agent-side v0.1 work is merged: the test-scope fix and the PR #11
follow-ups N5 and N6 ([`../v0-1/loop.md`](../v0-1/loop.md) §Next names the PR #11 follow-ups).
G3 (the stable xbrew release) and G4 (the owner's unlock and the agent's read of the address) wait
only on owner actions and do not block TUI code; they stay open gaps in
[`../v0-1/loop.md`](../v0-1/loop.md). Reason: the owner's instruction of 2026-10-27, "/using-dotprd
랑 /using-intentfirst /using-epic-issue 사용해서 작업 끝까지 진행" (carry the work through to the
end); the dispatcher's decision in review round 1 the same day.

## Entry conditions

- **TUI-3 merges only after the explorer table is measured again** (03-tui.md §History): the
  Blockscout hosts, and Routescan's paths for base, arbitrum and polygon, where Blockscout answers
  403. On 2026-10-27 Routescan's v2 address path answered 400 for all three (03-tui.md §History);
  its other paths were not tried. That data decides whether one more parser opens those chains;
  the decision is recorded in 03-tui.md before TUI-3 merges.

## Round 1 — plan

| WU | Repo · branch · worktree | Scope | Owner | State |
|---|---|---|---|---|
| TUI-1b | zwallet · `feat/tui-1b-screens` · `.worktrees/feat-tui-1b-screens` | `zwallet tui`, the screens, the cell/style/budget helpers, T1 frames, the T9 frames and `q`, the T10 `Msg` test, the cached-address counter-example, README (T1) | coding agent + dispatcher gate | in review (round 4), 2026-10-28: d7c8323 (RED), fd7e9c9, 75754ae, fd104cc, f8a9fd9, 249c5c7; review round 1: e02db59 (RED), 606bd40, d3668d4; round 2: f5b1640 (RED), 80b8845, 0c248cf; round 3: ab914b3 (RED), 8221618, then this docs commit; merges of TUI-1a (d7b91bc, 694eac5, 8ad1d1a, e49f8f1, 6e70102); merges after TUI-1a |
| TUI-1c | zwallet · `feat/tui-1c-pty` · `.worktrees/feat-tui-1c-pty` | the pty runs for T9 and T10, the argv-listed fake, the attacker moved into `tests/common`, the pty helpers (T9, T10) | coding agent + dispatcher gate | in review (round 4), 2026-10-28, from TUI-1b at 0c248cf: eae51a2 (the attacker and five helpers moved, `tests/cli.rs` imports only), 1948f20 (the pty helpers and the three pty tests; they pass on TUI-1b's code, no `src` change; the assertions were seen failing on a scratch build with the restore, the refresh and the expiry budget broken), 94e5be8 (T10's bound written as 10 s; the fake's expiry read up to its separator), e3cd07c and 016ddcc (docs); 65be4dd (merge of TUI-1b round 5, 5cd6512); d1f8101 (flow.md's reference); review round 1: 6f0c5fe (RED: a valid stream cut inside a sequence counted as stray; the 79x24 check passed on a live snapshot), f5a60cf (stray bytes and the 79x24 screen judged on the last frame, the whole output up to leaving the alternate screen; at 79x24 after both reads are drawn; a scratch build drawing more at 79x24 once the status lands fails it), b53292b (docs); review round 2: 917b311 (RED: T9 reaches the loaded 79x24 screen by a resize instead of beating a refresh; an exited child is reported before a wait's limit), 8175f48 (`PtyChild::resize`; `wait_output` reports an exited child once the reader has every byte, which held on macOS only: on Linux the reader never ended while the test held a slave, fixed in round 3; the 79x24 fixture ends as the real stream does), 6697e57 (docs); review round 3: 268333d (a full-screen child's pty keeps no slave in the test, so the reader ends after the last byte on Linux too; RED: the early-exit test failed 3 of 3 in rust:1.99-bookworm at 6697e57; a macOS probe lost no byte in 200 runs with the parent's slave closed), then this docs commit. T10 measured 3.1-4.9 s from `zwallet lock` returning, over 22 runs (round 3's gate: macOS on a pty 4.68, 4.80, 4.85 s; Linux 4.88, 4.88, 4.87 s). Changed lines against TUI-1b's 5cd6512 (counting rule, before this commit): tests 1,003 raw, about 811 with the 192 moved lines counted once; `.prd` 48 |
| TUI-2 | zwallet · `feat/tui-2-warmup` · `.worktrees/feat-tui-2-warmup` | warm-up, password field, password check (checked list, pinned addresses), make active, address trust (T2, T2b) | coding agent + dispatcher gate | planned, after TUI-1c |
| TUI-3 | zwallet · `feat/tui-3-history` · `.worktrees/feat-tui-3-history` | balance, journal rows, explorer client, merge (T3, T3b) | coding agent + dispatcher gate | planned, after TUI-2 and the explorer re-measure |
| TUI-4 | zwallet · `feat/tui-4-detail` · `.worktrees/feat-tui-4-detail` | tx detail with receipt token movements, `dropped/replaced`, chain switch (T4, T6c, T8) | coding agent + dispatcher gate | planned, after TUI-3 |
| TUI-5 | zwallet · `feat/tui-5-transfer` · `.worktrees/feat-tui-5-transfer` | assets, transfer from the checked list, exact-intent confirm, send status (T5, T6, T6b) | coding agent + dispatcher gate | planned, after TUI-4 |
| TUI-6 | zwallet · `feat/tui-6-whitelist` · `.worktrees/feat-tui-6-whitelist` | whitelist screens and edits, the pty run across T2, T6 and T7 (T7, T11) | coding agent + dispatcher gate | planned, after TUI-5 |

## Build facts

- Gate (copied from [`../v0-1/loop.md`](../v0-1/loop.md) §Build facts): `cargo fmt --check &&
  cargo clippy --all-targets --locked -- -D warnings && cargo test --locked`, the cli suite again
  on a pty ×3 (`script -q /dev/null cargo test --locked --test cli < /dev/null`), live `cargo
  test --locked -- --ignored`, and Linux Docker `rust:1.99-bookworm` as a non-root user. From
  TUI-1 on, the tui suite runs on a pty ×3 too (`script -q /dev/null cargo test --locked --test
  tui < /dev/null`).
- The gate stops the user's real session. `lock`, every CLI whitelist edit and the `create
  --unlock` mismatch call `stop_all_daemons`, which finds every zwallet daemon of the user through
  the process table, whatever its home. The TUI's `L` key and its whitelist edits do the same, so
  their tests hold `common::exclusive()`. Before a gate, `ps` must show no live `zwallet daemon`
  of the user.
- Acceptance frames: tmux at 120×40, `capture-pane`, read by the dispatcher (03-tui.md
  §Acceptance); T3 and T3b on in-test `TestBackend` frames, since the explorer URL is fixed. Every
  `TestBackend` frame goes through `FilteredBackend` with zero replaced cells.
- Explorer reachability was measured 2026-10-27 11:53Z (03-tui.md §History).
- `ratatui` and `crossterm` are not dependencies yet; TUI-1 adds them. `reqwest` is already in
  the tree through alloy (flow.md Fact 4).

## Open gaps

| ID | Acceptance | Gap | Source | Status |
|---|---|---|---|---|
| T1 | key list, active marked, lock state in the bar only | no `zwallet tui` (`wallet::keys` landed in TUI-1a) | TUI-1b | 🚪 |
| T2 | warm-up ≈ 1h, open key verified by password, global active key unchanged | no `wallet::prepare_unlock` / `wallet::unlock` / `wallet::open_with_password` / `wallet::check_list`, no password field | TUI-2 | 🚪 |
| T2b | `s` makes the key active | no `wallet::set_active` | TUI-2 | 🚪 |
| T3 | 10 rows, merged, pending first, coverage note | no journal reader, explorer client or merge | TUI-3 | 🚪 |
| T3b | explorer unavailable: journal only, note says so | no explorer client | TUI-3 | 🚪 |
| T4 | detail fields equal the fixture's, receipt token movements included | `lookup_tx` has no fee, confirmations or token movements | TUI-4 | 🚪 |
| T5 | native + non-zero registry tokens | no assets screen | TUI-5 | 🚪 |
| T6 | send to `alice`, hash at once, mined, journal line | send path private and prints to stderr; no exact-intent check | TUI-5 | 🚪 |
| T6b | picker: the checked list's address entries on this chain only | no picker, no checked list | TUI-5 | 🚪 |
| T6c | `dropped/replaced` | no code tells it | TUI-4 | 🚪 |
| T7 | add `bob` while warm, summary above the password, locked after | edit path reads `/dev/tty` itself | TUI-6 | 🚪 |
| T8 | `c` → base reloads every pane, host redacted | no chain picker | TUI-4 | 🚪 |
| T9 | 80×24 fits, smaller says so, terminal restored after `q` and a panic | closes when TUI-1b and TUI-1c merge: the frames and `q` in TUI-1b, the panic restore in TUI-1a's `term.rs` child tests, the pty run `t9_on_a_pty_q_gives_the_terminal_back_in_canonical_mode_with_echo` (tests/tui.rs) in TUI-1c | TUI-1c | 🚪 |
| T10 | `zwallet lock` elsewhere shows `locked` within two refreshes (10 s) | closes when TUI-1b and TUI-1c merge: the `Msg` test in TUI-1b, the pty run `t10_on_a_pty_zwallet_lock_elsewhere_shows_locked_within_two_refreshes` (tests/tui.rs) in TUI-1c | TUI-1c | 🚪 |
| T11 | pty across T2 and T7: no echo, no password in a frame, cancel and retry | needs TUI-2 and TUI-6 | TUI-6 | 🚪 |

## Open for TUI-2

From TUI-1c's review rounds 1 and 2 (2026-10-28), for the first unit whose pty runs resize or
print after the TUI. (Round 4 closed a third line, the stderr join in `PtyChild::stderr()`: a daemon
the warm-up starts inherits neither that pipe nor the pty slave, since `spawn_daemon` gives it
`Stdio::null()` for stderr and marks every fd from 3 up close-on-exec, src/core/protocol.rs:1354,
:1363.)
- T9 shrinks a running terminal (120x40 to 79x24) and sees the warning alone, but does not tell
  whether SIGWINCH's `Msg::Resize` (src/tui/app.rs:160) or the next refresh tick redrew it (a
  probe on 2026-10-28 saw the redraw 5 ms after the resize), and no test grows the window back.
- `quit()`'s stray and pending check (tests/tui.rs) covers only the bytes before the first
  `CSI ?1049l` (`Frame::last`, tests/common/mod.rs:1535). What is written after it goes
  unchecked: the line for a failed `L` stop, and the panic hook's fixed line.

## Next

TUI-1 is three PRs ([`units.md`](units.md) §TUI-1 split, with the dispatcher's six decisions of
2026-10-28). Gate and review TUI-1a on `feat/tui-1-shell`, merge; then TUI-1b, then TUI-1c, each
with RED tests first, the gate above, an outside review, merge.
