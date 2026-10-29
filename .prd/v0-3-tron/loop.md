# v0.3 Tron — loop

Status: done (shipped to preview 2026.10.29.2134, 2026-10-30), except TR23, which the owner runs.
A, B1, B2, C and the Tron TUI merged in PR #51 (599bde9); closed rows, build facts and receipts
are in [`loop.done.md`](loop.done.md) R1. Target = [`ssot.md`](ssot.md),
[`../05-tron.md`](../05-tron.md) acceptance TR1–TR25. This file holds only what is still open.

## Open gaps

| ID | Acceptance | Gap | Source | Status |
|---|---|---|---|---|
| TR23 | owner-gated mainnet TRX and USDT send | TRON-7 is shipped (PR #51, preview 2026.10.29.2134); the live send needs the owner's funds and decision; it also closes Q2 | owner | 🚪 |

## Open doc notes (fable, spec review round 3, 2026-10-28; non-blocking)

Written for C; C shipped without either, and 05-tron.md does not answer them yet:
- §Fees names the `fees` fields `activationSun` and `bandwidthSun` but not their values in each
  branch (05-tron.md:479); define them in one line, so the TUI's "activation fee" and "bandwidth
  burn" rows are fixed.
- Whether java-tron refuses a TRX `TransferContract` to a contract account
  (`TransferActuator.validate`) is unverified; read it and note it at TR17's contract case
  (no change to the numbers).

## Next

The owner decides TR23 (funds, timing) on preview 2026.10.29.2134 or a later build; the agent
records its receipt here and closes Q2 with it. Release publication (a `v*` tag) is the owner's.
The doc notes above go with the next 05-tron.md change.
