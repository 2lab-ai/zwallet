# v0.3 Tron — loop

Status: done (shipped to preview 2026.10.29.2134, 2026-10-30), except TR23, which the owner runs.
A, B1, B2, C and the Tron TUI merged in PR #51 (599bde9); closed rows, build facts and receipts
are in [`loop.done.md`](loop.done.md) R1. Target = [`ssot.md`](ssot.md),
[`../05-tron.md`](../05-tron.md) acceptance TR1–TR25. This file holds only what is still open.

## Open gaps

| ID | Acceptance | Gap | Source | Status |
|---|---|---|---|---|
| TR23 | owner-gated mainnet TRX and USDT send | TRON-7 is shipped (PR #51; current preview 2026.11.06.2001); the live send needs the owner's funds and decision; the owner's Tron address is not activated yet (0 TRX), so the first step is a TRX deposit; it also closes Q2 | owner | 🚪 |

## Round 2 (2026-11-04)

re-verified ✅: TR2, TR6, TR7, TR8 · read-only on the owner's own unlocked wallet with the installed preview 2026.10.29.2315: `address --chain tron` is session-verified with no prompt; `balance --chain tron` and `--token TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t` read Tron mainnet through `tron-rpc.publicnode.com` (0 TRX, not activated; 0 USDT, 6 decimals); the all-networks `balance` has the `tron` row; the TUI's wallet and receive tabs show the same · 8eaf051 is the current main
re-verified ✅: TR2, TR6, TR7, TR8 · 2026-11-06, read-only on the owner's own wallet with the installed preview 2026.11.03.2208: the key, made before Tron support, had no saved Tron address, so while locked `address --chain tron` said `WALLET_LOCKED` until the owner ran `address --chain tron --verify`; then, unlocked, the Tron address equals the 2026-11-04 read, 0 TRX (not activated) and 0 USDT, and the TUI's wallet, all-networks and receive tabs agree · the Tron receive QR (T18) was verified separately on preview 2026.11.06.1138 (36e8483) with loopback mocks, not on the owner's wallet ([`../v0-2-tui/loop.done.md`](../v0-2-tui/loop.done.md) R3)

## Next

The owner decides TR23 (funds, timing) on preview 2026.10.29.2134 or a later build; the agent
records its receipt here and closes Q2 with it. Release publication (a `v*` tag) is the owner's.
The two round-3 doc notes are closed in [`loop.done.md`](loop.done.md) (doc notes, 2026-11-06).
