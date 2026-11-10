# v0.2 TUI — loop

Status: done (shipped to preview 2026.10.29.2134, 2026-10-30). Every v2 unit (TUI-1c … TUI-12)
merged in PR #51 (599bde9); closed rows, units, build facts and receipts are in
[`loop.done.md`](loop.done.md) R1. Target = [`ssot.md`](ssot.md), [`../03-tui.md`](../03-tui.md)
§Coverage matrix and acceptance T1–T44. Post-delivery fixes #52, #55 and #63 merged and shipped
to preview 2026.11.03.1955: [`loop.done.md`](loop.done.md) R2. T18 (trade-off Q, decided Q5) merged in
PR #66 and shipped to preview 2026.11.06.1138: [`loop.done.md`](loop.done.md) R3. Post-delivery fix #67
merged in PR #70 and shipped to preview 2026.11.06.2001: [`loop.done.md`](loop.done.md) R4. This file
holds only what is still open.

## Open gaps

None. Every acceptance row T1–T44 is closed ([`loop.done.md`](loop.done.md) R1–R4).

## Waiting on the owner

- **Trade-off B** (03-tui.md §Open trade-offs): the recovery phrase in the TUI. B1, the CLI
  pointer, is shipped (T32); B2 is built only if the owner picks it (recommended: B1). The
  handoff does not authorize phrase display.
- **The owner's acceptance on preview 2026.11.06.2001** (or later): upgrade, `zwallet lock` and
  unlock again (session verification compares the daemon's cdhash, which changes with every
  build), then accept the TUI on it. The dispatcher's real-terminal acceptance passed on preview
  2026.10.29.2134 (loop.done.md R1), on the merged code 8eaf051 (R2) and on the installed previews
  2026.11.06.1138 (R3) and 2026.11.06.2001 (R4); none stands in for the owner's.

## Next

The owner: the two items above. The agent: nothing queued. The two open test notes closed with
v0.4 L1 (PR #72): [`loop.done.md`](loop.done.md) R5.
