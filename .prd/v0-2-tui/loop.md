# v0.2 TUI — loop

Status: done (shipped to preview 2026.10.29.2134, 2026-10-30). Every v2 unit (TUI-1c … TUI-12)
merged in PR #51 (599bde9); closed rows, units, build facts and receipts are in
[`loop.done.md`](loop.done.md) R1. Target = [`ssot.md`](ssot.md), [`../03-tui.md`](../03-tui.md)
§Coverage matrix and acceptance T1–T44. Post-delivery fixes #52, #55 and #63 merged and shipped
to preview 2026.11.03.1955: [`loop.done.md`](loop.done.md) R2. This file holds only what is still open.

## Open gaps

| ID | Acceptance | Gap | Source | Status |
|---|---|---|---|---|
| T18 | the QR code (trade-off Q) | the owner picked Q5 (2026-11-06); built in PR #66 (`Q` on 4 receive draws the verified address's code, Tron included; real-terminal decode passes at 120x40 and 80x24 on a debug build). Closes with the merge and the preview's real-terminal receipt | TUI-3+4 | 🟡 |

## Waiting on the owner

- **Trade-off B** (03-tui.md §Open trade-offs): the recovery phrase in the TUI. B1, the CLI
  pointer, is shipped (T32); B2 is built only if the owner picks it (recommended: B1). The
  handoff does not authorize phrase display.
- **The owner's acceptance on preview 2026.11.03.1955** (or later): upgrade, `zwallet lock` and
  unlock again (session verification compares the daemon's cdhash, which changes with every
  build), then accept the TUI on it. The dispatcher's real-terminal acceptance passed on preview
  2026.10.29.2134 (loop.done.md R1) and on the merged code 8eaf051 (R2); neither stands in for
  the owner's.

## Open test notes (TUI-1c review rounds 1 and 2, 2026-10-28)

For the next unit whose pty runs resize or print after the TUI. Both still hold at 599bde9:
- T9 shrinks a running terminal (120x40 to 79x24) and sees the warning alone, but does not tell
  whether SIGWINCH's `Msg::Resize` or the next refresh tick redrew it (a probe on 2026-10-28 saw
  the redraw 5 ms after the resize), and no test grows the window back
  (`t9_on_a_pty_q_gives_the_terminal_back_in_canonical_mode_with_echo`, tests/tui.rs:950-973,
  only shrinks).
- `quit()`'s stray and pending check (tests/tui.rs:858) covers only the bytes before the first
  `CSI ?1049l` (`Frame::last`, tests/common/mod.rs:1645-1650). What is written after it goes
  unchecked: the line for a failed `L` stop, and the panic hook's fixed line.

## Next

The owner: the three items above. The agent: nothing queued; pick up the test notes with the next
TUI change that touches resize or post-exit output.
