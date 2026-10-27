# v0.2 TUI — loop

Status: **in-progress**, 2026-10-27. Target = [`ssot.md`](ssot.md), [`../03-tui.md`](../03-tui.md)
acceptance T1–T11. Current code, function by function: [`flow.md`](flow.md). Units:
[`units.md`](units.md). This file holds only open work. No code yet: U1 starts once v0.1 is
complete (its open gaps G3 and G4, [`../v0-1/loop.md`](../v0-1/loop.md)).

## Round 1 — plan

| WU | Repo · branch · worktree | Scope | Owner | State |
|---|---|---|---|---|
| U1 | zwallet · `feat/tui-u1-keys` · `.worktrees/feat-tui-u1-keys` | `zwallet tui`, ratatui/crossterm, cell rules, keys list, status bar, lock detection (T1, T9, T10) | coding agent + dispatcher gate | planned |
| U2 | zwallet · `feat/tui-u2-warmup` · `.worktrees/feat-tui-u2-warmup` | warm-up, password field, make active, address trust (T2, T2b) | coding agent + dispatcher gate | planned, after U1 |
| U3 | zwallet · `feat/tui-u3-history` · `.worktrees/feat-tui-u3-history` | balance, journal rows, explorer client, merge (T3, T3b) | coding agent + dispatcher gate | planned, after U2 |
| U4 | zwallet · `feat/tui-u4-detail` · `.worktrees/feat-tui-u4-detail` | tx detail, assets, chain switch (T4, T5, T8) | coding agent + dispatcher gate | planned, after U3 |
| U5 | zwallet · `feat/tui-u5-transfer` · `.worktrees/feat-tui-u5-transfer` | transfer, review, send status (T6, T6b, T6c) | coding agent + dispatcher gate | planned, after U4 |
| U6 | zwallet · `feat/tui-u6-whitelist` · `.worktrees/feat-tui-u6-whitelist` | whitelist screens and edits, the pty run across T2, T6 and T7 (T7, T11) | coding agent + dispatcher gate | planned, after U5 |

## Build facts

- Gate (copied from [`../v0-1/loop.md`](../v0-1/loop.md) §Build facts): `cargo fmt --check &&
  cargo clippy --all-targets --locked -- -D warnings && cargo test --locked`, the cli suite again
  on a pty ×3 (`script -q /dev/null cargo test --locked --test cli < /dev/null`), live `cargo
  test --locked -- --ignored`, and Linux Docker `rust:1.99-bookworm` as a non-root user. From U1
  on, the tui suite runs on a pty ×3 too (`script -q /dev/null cargo test --locked --test tui <
  /dev/null`).
- The gate stops the user's real session. `lock`, every CLI whitelist edit and the `create
  --unlock` mismatch call `stop_all_daemons`, which finds every zwallet daemon of the user through
  the process table, whatever its home. The TUI's `L` key and its whitelist edits do the same, so
  their tests hold `common::exclusive()`. Before a gate, `ps` must show no live `zwallet daemon`
  of the user.
- Acceptance frames: tmux at 120×40, `capture-pane`, read by the dispatcher (03-tui.md
  §Acceptance); T3 and T3b on in-test `TestBackend` frames, since the explorer URL is fixed.
- Explorer reachability was measured 2026-10-27 11:53Z (03-tui.md §History); measure again
  before U3 merges.
- `ratatui` and `crossterm` are not dependencies yet; U1 adds them. `reqwest` is already in the
  tree through alloy (flow.md Fact 4).

## Open gaps

| ID | Acceptance | Gap | Source | Status |
|---|---|---|---|---|
| T1 | key list, active marked, lock state in the bar only | no `zwallet tui`, no `wallet::keys` | U1 | 🚪 |
| T2 | warm-up ≈ 1h, global active key unchanged | no `wallet::prepare_unlock` / `wallet::unlock`, no password field | U2 | 🚪 |
| T2b | `s` makes the key active | no `wallet::set_active` | U2 | 🚪 |
| T3 | 10 rows, merged, pending first, coverage note | no journal reader, explorer client or merge | U3 | 🚪 |
| T3b | explorer unavailable: journal only, note says so | no explorer client | U3 | 🚪 |
| T4 | detail fields equal the fixture's | `lookup_tx` has no fee, confirmations or token amount | U4 | 🚪 |
| T5 | native + non-zero registry tokens | no assets screen | U4 | 🚪 |
| T6 | send to `alice`, hash at once, mined, journal line | send path private and prints to stderr | U5 | 🚪 |
| T6b | picker: address entries on this chain only | no picker | U5 | 🚪 |
| T6c | `dropped/replaced` | no code tells it | U5 | 🚪 |
| T7 | add `bob` while warm, summary above the password, locked after | edit path reads `/dev/tty` itself | U6 | 🚪 |
| T8 | `c` → base reloads every pane, host redacted | no chain picker | U4 | 🚪 |
| T9 | 80×24 fits, smaller says so, terminal restored after `q` and a panic | no terminal setup | U1 | 🚪 |
| T10 | `zwallet lock` elsewhere shows `locked` within 5 s | no `wallet::status` | U1 | 🚪 |
| T11 | pty across T2 and T7: no echo, no password in a frame, cancel and retry | needs U2 and U6 | U6 | 🚪 |

## Next

v0.1 first: the user's unlock closes G4, then the stable xbrew release G3
([`../v0-1/loop.md`](../v0-1/loop.md) §Next). Then U1 on `feat/tui-u1-keys`, one coding agent,
RED tests first as [`units.md`](units.md) lists them, the gate above, an outside review, merge.
