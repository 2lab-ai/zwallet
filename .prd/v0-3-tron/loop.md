# v0.3 Tron — loop

Status: **in-progress**, 2026-10-29. Target = [`ssot.md`](ssot.md), [`../05-tron.md`](../05-tron.md)
acceptance TR1–TR25. Units: [`units.md`](units.md). This file holds only open work.

## Round 1 — plan

PRs per [`units.md`](units.md) (the dispatcher's plan after review round 1, 2026-10-28; no line
cap, each PR reviewable as one argument). Worktrees at `.worktrees/<branch with / as ->`.

| PR | Branch | Work items | Closes | Owner | State |
|---|---|---|---|---|---|
| A, see it | `handoff/tron-a` (continues PR #33) | TRON-1, TRON-2 (no `tron` default fee cap) | TR1–TR8, TR24 | coding agent + dispatcher gate | PR #37 95a61be: local543, independent review and Linux/macOS CI40836777320 pass; merge pending |
| B1, codec | `handoff/tron-codec` | TRON-4 | TR12, TR13 | coding agent + dispatcher gate | PR #40 8df04af: local557, independent review and CI40840314282 both pass; merge pending |
| B2, guard | `handoff/tron-policy` | TRON-3, TRON-5, the `tron` default fee cap; review evidence carries B1's refusal list | TR9–TR11, TR14, TR15 | coding agent + dispatcher gate | PR #41 6f7f18b: local570, independent review and CI40841286260 both pass; merge pending |
| C, send it | `handoff/tron-send` | TRON-6, TRON-7 | TR16–TR21, TR25 | coding agent + dispatcher gate | PR #42 deda723: local591 and macOS CI pass; Linux observation-timing repair passes local595; new exact-head CI pending; Q3 closed |
| TR23 | — | the live mainnet send; closes Q2 | TR23 | owner (funds, decision) + dispatcher | waits for C and its preview |

## Build facts

- Gate (as [`../v0-2-tui/loop.md`](../v0-2-tui/loop.md) §Build facts): `cargo fmt --check &&
  cargo clippy --all-targets --locked -- -D warnings && cargo test --locked`; the cli suite again
  on a pty ×3 (`script -q /dev/null cargo test --locked --test cli < /dev/null`); the tui suite
  the same way; live `cargo test --locked -- --ignored`; Linux with Rust 1.95 (the repository toolchain) as a
  non-root user; an x86_64 macOS build (`unlock` needs it signed).
- From A on, `tests/tron_offline.rs` (the local mock node) runs in `cargo test --locked`.
- Tests use `/tmp/zwt-*` homes and the debug-only `ZWALLET_TEST_DAEMON_SCOPE` guard.
  Confirm the guard before running tests; never stop the user's real wallet daemon.
  Run only one complete CLI suite at a time. Focused tests that can stop test daemons
  hold `common::exclusive()` and serialize with other suites.
- Live Tron tests use `https://tron-rpc.publicnode.com`. TronGrid without a key allowed 3
  requests/s on 2026-10-28 (05 §Network access), too few for a parallel test run.
- `bs58` 0.5.1 and `sha2` 0.10.9 are already in `Cargo.lock`; TRON-1 adds no crate.

## Open gaps

| ID | Acceptance | Gap | Source | Status |
|---|---|---|---|---|
| TR1 | the A1 mnemonic gives `TWer2…` (i=0), `TPjjv…` (i=1) | implemented and reviewed; local543 and PR #37 exact-head CI both pass; merge pending | A | 🟡 |
| TR2 | `addresses.tron` cached; session-verified after unlock, no prompt | implemented and reviewed; local543 and PR #37 exact-head CI both pass; merge pending | A | 🟡 |
| TR3 | an old key: `WALLET_LOCKED` locked, `--verify` adds the address | implemented and reviewed; local543 and PR #37 exact-head CI both pass; merge pending | A | 🟡 |
| TR4 | case-flipped, bad-checksum, `0x` caches refused | implemented and reviewed; local543 and PR #37 exact-head CI both pass; merge pending | A | 🟡 |
| TR5 | a Nile genesis is `RPC_ERROR` before any read | implemented and reviewed; local543 and PR #37 exact-head CI both pass; merge pending | A | 🟡 |
| TR6 | owner-permission warning from the M7 fixture; live balance equals the node, nothing pinned | implemented and reviewed; local543 and PR #37 exact-head CI both pass; merge pending | A | 🟡 |
| TR7 | USDT balance, 6 decimals; a token error is the row's | implemented and reviewed; local543 and PR #37 exact-head CI both pass; merge pending | A | 🟡 |
| TR8 | `balance` has a `tron` row, Sepolia last | implemented and reviewed; local543 and PR #37 exact-head CI both pass; merge pending | A | 🟡 |
| TR9 | `whitelist add bob T…` stores `0x…` + `["tron"]`, shows `T…` | implemented and reviewed; PR #41 local570 and CI both pass; merge pending | B2 | 🟡 |
| TR10 | six bad `add` inputs refused before the password; `validate` refuses mixed and contract `tron` entries | implemented and reviewed; PR #41 local570 and CI both pass; merge pending | B2 | 🟡 |
| TR11 | old lists verify unchanged; `*` is no `tron` payee; the EVM twin of a `tron` payee is refused | implemented and reviewed; PR #41 local570 and CI both pass; merge pending | B2 | 🟡 |
| TR12 | golden vectors decode, re-encode, hash, sign, recover | implemented and reviewed; PR #40 local557 and CI both pass; merge pending | B1 | 🟡 |
| TR13 | every non-canonical or foreign input refused | implemented and reviewed; PR #40 local557 and CI both pass; merge pending | B1 | 🟡 |
| TR14 | the action table, zero signatures per refusal | implemented and reviewed; PR #41 local570 and CI both pass; merge pending | B2 | 🟡 |
| TR15 | fee cap, missing cap, expiration window | implemented and reviewed; PR #41 local570 and CI both pass; merge pending | B2 | 🟡 |
| TR16 | a forged daemon reply is refused, nothing broadcast | implemented and reviewed; local591 passes; PR #42 Linux timing repair passes local595; new exact-head CI pending | C | 🟡 |
| TR17 | dry-run with `fees`; a refused dry-run asks no signature; activation at the prices read, staked bandwidth assumed gone in the maximum | implemented and reviewed; local591 passes; PR #42 Linux timing repair passes local595; new exact-head CI pending | C | 🟡 |
| TR18 | `INSUFFICIENT_FUNDS` ×4 (one above the estimate, below the maximum), revert `RPC_ERROR`, before signing | implemented and reviewed; local591 passes; PR #42 Linux timing repair passes local595; new exact-head CI pending | C | 🟡 |
| TR19 | nonce flags refused on `tron` | implemented and reviewed; local591 passes; PR #42 Linux timing repair passes local595; new exact-head CI pending | C | 🟡 |
| TR20 | TRX and USDT sent and confirmed against the mock; fee above the estimate stays within the maximum | implemented and reviewed; local591 passes; PR #42 Linux timing repair passes local595; new exact-head CI pending | C | 🟡 |
| TR21 | each broadcast outcome; uncertain and duplicate messages name `expiresAt` and the second-payment risk | implemented and reviewed; local591 passes; PR #42 Linux timing repair passes local595; new exact-head CI pending | C | 🟡 |
| TR25 | `expired-unseen` only by the node's head and a node within 60 s of the local clock; never a safe-resend message | implemented and reviewed; local591 passes; PR #42 Linux timing repair passes local595; new exact-head CI pending | C | 🟡 |
| TR22 | every row of 05 §TUI is in the TUI coverage matrix | T38–T41 implementation is in H18A/B plus family UI integration; shared address display and receipt/status gates remain | TUI task | 🟡 |
| TR23 | owner-gated mainnet TRX and USDT send | needs TRON-7 and the owner's funds and decision | owner | 🚪 |
| TR24 | `sign-message`, `call`, `invoke` on `tron` → `CAPABILITY_UNSUPPORTED` | implemented and reviewed; local543 and PR #37 exact-head CI both pass; merge pending | A | 🟡 |

C focused results, the production-client Q3 receipt and the remaining owner-only evidence are
recorded in [`c-evidence.md`](c-evidence.md).

## Next

Finish PR #42's deterministic observation/deadline repair and verify its new commit on both
CI platforms. A/B1/B2 are implemented, independently reviewed and green at the linked commits;
they await upstream merge permission. Complete TUI12 integration and its tests, then record final
stress/live-read/Intel/preview evidence explicitly. Owner-funded TR23 and release publication are
not authorized by the handoff. Completion is tracked in ../handoff-completion/completion-checklist.md.

The spec passed the three-engine review in round 3 (2026-10-28: grok-4.7, gpt-6-astra and fable all
APPROVE; rounds 1 and 2 are summarised in 05-tron.md). Two non-blocking remarks from fable's round 3,
for C:
- §Fees names the `fees` fields `activationSun` and `bandwidthSun` but not their values in each
  branch (05-tron.md:465-466); define them in one line, so the TUI's "activation fee" and "bandwidth
  burn" rows are fixed.
- Whether java-tron refuses a TRX `TransferContract` to a contract account (`TransferActuator.validate`)
  is unverified; read it while building C and note it at TR17's contract case (no change to the numbers).
