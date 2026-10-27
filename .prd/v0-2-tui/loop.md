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
| TUI-1a | zwallet · `feat/tui-1-shell` · `.worktrees/feat-tui-1-shell` | ratatui/crossterm, `FilteredBackend` and the terminal guard, `wallet::keys`, `wallet::status`, `wallet::net` with `list` and `status` on them (no acceptance ID) | coding agent + dispatcher gate | in review, 2026-10-28: 66f038c (RED), 4f527f5, 784cfa3, 5119e36 (c14's clock-step tolerance), this docs commit; about 610 counted lines |
| TUI-1b | zwallet · branch at dispatch | `zwallet tui`, the screens, the cell/style/budget helpers, T1 frames, the T9 frames and `q`, the T10 `Msg` test, the cached-address counter-example, README (T1) | coding agent + dispatcher gate | planned, after TUI-1a |
| TUI-1c | zwallet · branch at dispatch | the pty runs for T9 and T10, the argv-listed fake, the attacker moved into `tests/common`, the pty helpers (T9, T10) | coding agent + dispatcher gate | planned, after TUI-1b |
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
| T9 | 80×24 fits, smaller says so, terminal restored after `q` and a panic | no `zwallet tui` yet; the write point and the guard landed in TUI-1a | TUI-1c | 🚪 |
| T10 | `zwallet lock` elsewhere shows `locked` within two refreshes (10 s) | no `zwallet tui` yet; `wallet::status` landed in TUI-1a | TUI-1c | 🚪 |
| T11 | pty across T2 and T7: no echo, no password in a frame, cancel and retry | needs TUI-2 and TUI-6 | TUI-6 | 🚪 |

## Next

TUI-1 is three PRs ([`units.md`](units.md) §TUI-1 split, with the dispatcher's six decisions of
2026-10-28). Gate and review TUI-1a on `feat/tui-1-shell`, merge; then TUI-1b, then TUI-1c, each
with RED tests first, the gate above, an outside review, merge.
