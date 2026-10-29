# Handoff completion — driver

Status: done (shipped to preview 2026.10.29.2134, 2026-10-30). H1–H22 closed with PR #51
(599bde9), which closed issue #34: [`loop.done.md`](loop.done.md) R1. Receipts and the
owner-only list: [`completion-checklist.md`](completion-checklist.md). This file holds only what
is still open; none of it is the handoff's to do.

## Open by design

- **TR23**, the owner's: the funded mainnet TRX and USDT send
  ([`../v0-3-tron/loop.md`](../v0-3-tron/loop.md)).
- **T18 / trade-off Q**, the owner's: a QR code on receive. Q1, the safe pointer line, is shipped
  ([`../v0-2-tui/loop.md`](../v0-2-tui/loop.md)).
- **Trade-off B**, the owner's: the recovery phrase in the TUI. B1, the CLI pointer, is shipped.
- **The owner's acceptance on preview 2026.10.29.2134** (unlock again: new cdhash).
- **#26 and #27**: design only ([`../08-cumulative-budget-design.md`](../08-cumulative-budget-design.md),
  [`../09-policy-freshness-design.md`](../09-policy-freshness-design.md)); runtime deferred by
  the owner.
- **#52**: a once-seen Linux pty flake (`t27_t28_pty_restart_original_proof_speedup_and_cancel_verify_signed_bytes`); tracked.

## Next

Nothing for the agent under this handoff. A stable release or `v*` tag, funding and the
trade-offs are the owner's; #52 is fixed when it recurs or is reproduced.
