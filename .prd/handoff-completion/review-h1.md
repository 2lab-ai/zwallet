# TUI foundation review — 2026-10-29

Scope: PR31/32 integrated on main 99fff4e, with activation ordering and password lifetime fixes.
The independent reviewer examined the shared wallet operations, UI state machine and test isolation.

## Findings and changes

- Two confirmed activations could finish backwards. Only one confirmed activation now runs;
  q/Ctrl-C waits for its outcome, and a failed write is returned after terminal restoration.
- An old Keys response could replace an activation-refusal pane. Failure now invalidates
  pending read generations. A regression sends the old result after the refusal.
- The synchronous metadata lock could block the current-thread UI runtime. The write runs on
  spawn_blocking; a held-lock test verifies a timer still runs before the write completes.
- The new short-error regression exposed an inaccurate one-line Placed rectangle. Its width
  now describes the actual text; the normal filtered frame checks verify it.
- list --verify explicitly drops its zeroizing password immediately after opening the wallet.

The first four behavioral regressions were observed failing before their fixes. Independent
follow-up review found no remaining blocker in those fixes. This is local code review, not a
GitHub approval. Final full-gate and exact-head CI receipts remain required for delivery.
