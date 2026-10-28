# v0.3 Tron — loop

Status: **planned**, 2026-10-28. Target = [`ssot.md`](ssot.md), [`../05-tron.md`](../05-tron.md)
acceptance TR1–TR25. Units: [`units.md`](units.md). This file holds only open work.

## Round 1 — plan

PRs per [`units.md`](units.md) (the dispatcher's plan after review round 1, 2026-10-28; no line
cap, each PR reviewable as one argument). Worktrees at `.worktrees/<branch with / as ->`.

| PR | Branch | Work items | Closes | Owner | State |
|---|---|---|---|---|---|
| A, see it | `feat/tron-a-see` | TRON-1, TRON-2 (no `tron` default fee cap) | TR1–TR8, TR24 | coding agent + dispatcher gate | planned; preview after merge |
| B1, codec | `feat/tron-b1-codec` | TRON-4 | TR12, TR13 | coding agent + dispatcher gate | planned, after A |
| B2, guard | `feat/tron-b2-guard` | TRON-3, TRON-5, the `tron` default fee cap; review evidence carries B1's refusal list | TR9–TR11, TR14, TR15 | coding agent + dispatcher gate | planned, after B1 |
| C, send it | `feat/tron-c-send` | TRON-6, TRON-7 | TR16–TR21, TR25 | coding agent + dispatcher gate | planned, after B2 and Q3 (units.md §Before C starts); preview after merge |
| TR23 | — | the live mainnet send; closes Q2 | TR23 | owner (funds, decision) + dispatcher | waits for C and its preview |

## Build facts

- Gate (as [`../v0-2-tui/loop.md`](../v0-2-tui/loop.md) §Build facts): `cargo fmt --check &&
  cargo clippy --all-targets --locked -- -D warnings && cargo test --locked`; the cli suite again
  on a pty ×3 (`script -q /dev/null cargo test --locked --test cli < /dev/null`); the tui suite
  the same way; live `cargo test --locked -- --ignored`; Linux Docker `rust:1.99-bookworm` as a
  non-root user; an x86_64 macOS build (`unlock` needs it signed).
- From A on, `tests/tron_offline.rs` (the local mock node) runs in `cargo test --locked`.
- Before a gate, `ps` shows no live `zwallet daemon` of the user: the gate stops every daemon.
- Live Tron tests use `https://tron-rpc.publicnode.com`. TronGrid without a key allowed 3
  requests/s on 2026-10-28 (05 §Network access), too few for a parallel test run.
- `bs58` 0.5.1 and `sha2` 0.10.9 are already in `Cargo.lock`; TRON-1 adds no crate.

## Open gaps

| ID | Acceptance | Gap | Source | Status |
|---|---|---|---|---|
| TR1 | the A1 mnemonic gives `TWer2…` (i=0), `TPjjv…` (i=1) | no Tron family | A | 🚪 |
| TR2 | `addresses.tron` cached; session-verified after unlock, no prompt | no Tron address | A | 🚪 |
| TR3 | an old key: `WALLET_LOCKED` locked, `--verify` adds the address | `cache_repairs` only checks entries that exist | A | 🚪 |
| TR4 | case-flipped, bad-checksum, `0x` caches refused | compare is case-insensitive | A | 🚪 |
| TR5 | a Nile genesis is `RPC_ERROR` before any read | no chain identity check | A | 🚪 |
| TR6 | owner-permission warning from the M7 fixture; live balance equals the node, nothing pinned | no Tron RPC | A | 🚪 |
| TR7 | USDT balance, 6 decimals; a token error is the row's | no Tron registry | A | 🚪 |
| TR8 | `balance` has a `tron` row, Sepolia last | no Tron arm in `every_balance` | A | 🚪 |
| TR9 | `whitelist add bob T…` stores `0x…` + `["tron"]`, shows `T…` | `add_change` takes `0x` only | B2 | 🚪 |
| TR10 | six bad `add` inputs refused before the password; `validate` refuses mixed and contract `tron` entries | no Tron parse rules | B2 | 🚪 |
| TR11 | old lists verify unchanged; `*` is no `tron` payee; the EVM twin of a `tron` payee is refused | `covers_chain` treats `*` as every chain | B2 | 🚪 |
| TR12 | golden vectors decode, re-encode, hash, sign, recover | no codec | B1 | 🚪 |
| TR13 | every non-canonical or foreign input refused | no strict decoder | B1 | 🚪 |
| TR14 | the action table, zero signatures per refusal | no Tron policy | B2 | 🚪 |
| TR15 | fee cap, missing cap, expiration window | no Tron policy | B2 | 🚪 |
| TR16 | a forged daemon reply is refused, nothing broadcast | no `inspect_signed` for Tron | C | 🚪 |
| TR17 | dry-run with `fees`; a refused dry-run asks no signature; activation at the prices read, staked bandwidth assumed gone in the maximum | no `prepare_transfer` for Tron | C | 🚪 |
| TR18 | `INSUFFICIENT_FUNDS` ×4 (one above the estimate, below the maximum), revert `RPC_ERROR`, before signing | no preflight | C | 🚪 |
| TR19 | nonce flags refused on `tron` | flags apply to every chain | C | 🚪 |
| TR20 | TRX and USDT sent and confirmed against the mock; fee above the estimate stays within the maximum | no broadcast | C | 🚪 |
| TR21 | each broadcast outcome; uncertain and duplicate messages name `expiresAt` and the second-payment risk | no outcome mapping, no `expiresAt` | C | 🚪 |
| TR25 | `expired-unseen` only by the node's head and a node within 60 s of the local clock; never a safe-resend message | no Tron lookup | C | 🚪 |
| TR22 | every row of 05 §TUI is in the TUI coverage matrix | `03-tui.md` is being rewritten | TUI task | 🚪 |
| TR23 | owner-gated mainnet TRX and USDT send | needs TRON-7 and the owner's funds and decision | owner | 🚪 |
| TR24 | `sign-message`, `call`, `invoke` on `tron` → `CAPABILITY_UNSUPPORTED` | no `tron` chain | A | 🚪 |

## Next

Dispatch A with RED tests first, the gate above, an outside review, merge, preview. Then B1, B2
and C in that order; Q3 closes before C (units.md §Before C starts).
