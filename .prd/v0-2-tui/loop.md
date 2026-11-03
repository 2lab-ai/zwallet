# v0.2 TUI — loop

Status: done (shipped to preview 2026.10.29.2134, 2026-10-30). Every v2 unit (TUI-1c … TUI-12)
merged in PR #51 (599bde9); closed rows, units, build facts and receipts are in
[`loop.done.md`](loop.done.md) R1. Target = [`ssot.md`](ssot.md), [`../03-tui.md`](../03-tui.md)
§Coverage matrix and acceptance T1–T44. This file holds only what is still open.

## Open gaps

| ID | Acceptance | Gap | Source | Status |
|---|---|---|---|---|
| T18 | the QR code (trade-off Q) | the Q1 pointer (`Q` names `zwallet address`) is shipped (PR #51, preview 2026.10.29.2134); Q3 waits on the owner's trade-off Q | TUI-3+4 | 🚪 |

## Waiting on the owner

- **Trade-off Q** (03-tui.md §Open trade-offs): a QR code on receive, T18 above. Q1, the CLI
  pointer, is the shipped default.
- **Trade-off B** (03-tui.md §Open trade-offs): the recovery phrase in the TUI. B1, the CLI
  pointer, is shipped (T32); B2 is built only if the owner picks it (recommended: B1). The
  handoff does not authorize phrase display or a new QR implementation.
- **The owner's acceptance on preview 2026.10.29.2134**: upgrade, `zwallet lock` and unlock
  again (session verification compares the daemon's cdhash, which changes with every build),
  then accept the TUI on it. The dispatcher's real-terminal acceptance on that preview passed
  (loop.done.md R1); it does not stand in for the owner's.

## Next

The owner: the three items above. The resize/post-exit test notes are resolved in
`loop.done.md` R3. Issue #52 is closed by PR #54; its receipt
is appended in [`loop.done.md`](loop.done.md) R2. The #55/#57 follow-up is tracked in
[`../handoff-completion/loop.md`](../handoff-completion/loop.md), without reopening shipped TUI units.
