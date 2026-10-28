# v0.3 Tron — loop

Status: **planned**, 2026-10-28. Target = [`ssot.md`](ssot.md), [`../05-tron.md`](../05-tron.md)
acceptance TR1–TR24. Units: [`units.md`](units.md). This file holds only open work.

## Round 1 — plan

| WU | Branch (worktree `.worktrees/<branch with / as ->`) | Scope | Owner | State |
|---|---|---|---|---|
| TRON-1 | `feat/tron-1-family` | family, derivation, address, chain identity, TRX balance (TR1, TR2, TR4, TR5, TR24) | coding agent + dispatcher gate | planned |
| TRON-2 | `feat/tron-2-usdt-balance` | USDT, owner-permission warning, cache fill (TR3, TR6, TR7, TR8) | coding agent + dispatcher gate | planned, after TRON-1 |
| TRON-3 | `feat/tron-3-whitelist` | whitelist Tron entries (TR9, TR10, TR11) | coding agent + dispatcher gate | planned, after TRON-1 |
| TRON-4 | `feat/tron-4-codec` | protobuf codec, txID, signature (TR12, TR13) | coding agent + dispatcher gate | planned, after TRON-1 |
| TRON-5 | `feat/tron-5-daemon-policy` | the daemon's judgement (TR14, TR15) | coding agent + dispatcher gate | planned, after TRON-3 and TRON-4 |
| TRON-6 | `feat/tron-6-transfer-build` | build, fee preflight, dry-run (TR16–TR19); closes Q4 | coding agent + dispatcher gate | planned, after TRON-2 and TRON-5 |
| TRON-7 | `feat/tron-7-broadcast` | broadcast, `--wait`, journal, `tx` (TR20, TR21); closes Q1 | coding agent + dispatcher gate | planned, after TRON-6 |
| TR23 | — | the live mainnet send; closes Q2, Q3 | owner (funds, decision) + dispatcher | waits for TRON-7 and a preview |

## Build facts

- Gate (as [`../v0-2-tui/loop.md`](../v0-2-tui/loop.md) §Build facts): `cargo fmt --check &&
  cargo clippy --all-targets --locked -- -D warnings && cargo test --locked`; the cli suite again
  on a pty ×3 (`script -q /dev/null cargo test --locked --test cli < /dev/null`); the tui suite
  the same way; live `cargo test --locked -- --ignored`; Linux Docker `rust:1.99-bookworm` as a
  non-root user; an x86_64 macOS build (`unlock` needs it signed).
- From TRON-1 on, `tests/tron_offline.rs` (the local mock node) runs in `cargo test --locked`.
- Before a gate, `ps` shows no live `zwallet daemon` of the user: the gate stops every daemon.
- Live Tron tests use `https://tron-rpc.publicnode.com`. TronGrid without a key allowed 3
  requests/s on 2026-10-28 (05 §Network access), too few for a parallel test run.
- `bs58` 0.5.1 and `sha2` 0.10.9 are already in `Cargo.lock`; TRON-1 adds no crate.

## Open gaps

| ID | Acceptance | Gap | Source | Status |
|---|---|---|---|---|
| TR1 | the A1 mnemonic gives `TWer2…` (i=0), `TPjjv…` (i=1) | no Tron family | TRON-1 | 🚪 |
| TR2 | `addresses.tron` cached; session-verified after unlock, no prompt | no Tron address | TRON-1 | 🚪 |
| TR3 | an old key: `WALLET_LOCKED` locked, `--verify` adds the address | `cache_repairs` only checks entries that exist | TRON-2 | 🚪 |
| TR4 | case-flipped, bad-checksum, `0x` caches refused | compare is case-insensitive | TRON-1 | 🚪 |
| TR5 | a Nile genesis is `RPC_ERROR` before any read | no chain identity check | TRON-1 | 🚪 |
| TR6 | live TRX balance; owner-permission warning on `TWer2…` | no Tron RPC | TRON-2 | 🚪 |
| TR7 | USDT balance, 6 decimals; a token error is the row's | no Tron registry | TRON-2 | 🚪 |
| TR8 | `balance` has a `tron` row, Sepolia last | no Tron arm in `every_balance` | TRON-2 | 🚪 |
| TR9 | `whitelist add bob T…` stores `0x…` + `["tron"]`, shows `T…` | `add_change` takes `0x` only | TRON-3 | 🚪 |
| TR10 | five bad `add` inputs refused before the password | no Tron parse rules | TRON-3 | 🚪 |
| TR11 | old lists verify unchanged; `*` is no `tron` payee | `covers_chain` treats `*` as every chain | TRON-3 | 🚪 |
| TR12 | golden vectors decode, re-encode, hash, sign, recover | no codec | TRON-4 | 🚪 |
| TR13 | every non-canonical or foreign input refused | no strict decoder | TRON-4 | 🚪 |
| TR14 | the action table, zero signatures per refusal | no Tron policy | TRON-5 | 🚪 |
| TR15 | fee cap, missing cap, expiration window | no Tron policy | TRON-5 | 🚪 |
| TR16 | a forged daemon reply is refused, nothing broadcast | no `inspect_signed` for Tron | TRON-6 | 🚪 |
| TR17 | dry-run with `fees`; a refused dry-run asks no signature | no `prepare_transfer` for Tron | TRON-6 | 🚪 |
| TR18 | `INSUFFICIENT_FUNDS` ×3, revert `RPC_ERROR`, before signing | no preflight | TRON-6 | 🚪 |
| TR19 | nonce flags refused on `tron` | flags apply to every chain | TRON-6 | 🚪 |
| TR20 | TRX and USDT sent and confirmed against the mock | no broadcast | TRON-7 | 🚪 |
| TR21 | each broadcast outcome; `expired`; duplicate names `expiresAt` | no outcome mapping, no `expiresAt` | TRON-7 | 🚪 |
| TR22 | every row of 05 §TUI is in the TUI coverage matrix | `03-tui.md` is being rewritten | TUI task | 🚪 |
| TR23 | owner-gated mainnet TRX and USDT send | needs TRON-7 and the owner's funds and decision | owner | 🚪 |
| TR24 | `sign-message`, `call`, `invoke` on `tron` → `CAPABILITY_UNSUPPORTED` | no `tron` chain | TRON-1 | 🚪 |

## Next

Dispatch TRON-1 with RED tests first, the gate above, an outside review, merge. Then TRON-2,
TRON-3 and TRON-4 in parallel.
