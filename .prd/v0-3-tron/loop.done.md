# v0.3 Tron — loop (done)

Append-only ledger of closed rounds and rows moved out of [`loop.md`](loop.md). Each round close
appends one block at the bottom; nothing above it changes (loop-lint C2 compares against git).
Rows here are snapshots taken when they moved.

## R1 — closed 2026-10-30 (A, B1, B2, C and the Tron TUI, PR #51)

- result: A, B1, B2 and C merged in [PR #51](https://github.com/2lab-ai/zwallet/pull/51) (599bde9, merged 2026-10-29T21:34Z = 2026-10-30 KST) with the Tron TUI (#49), and shipped to preview 2026.10.29.2134 · closed rows: TR1–TR22, TR24, TR25 · still open in [`loop.md`](loop.md): TR23, the owner's funded mainnet send. (The whole round 1 of the single-file driver, split here at its close.)

### Closed rows

| PR | Branch | Work items | Closes | Owner | State |
|---|---|---|---|---|---|
| A, see it | `handoff/tron-a` (continues PR #33) | TRON-1, TRON-2 (no `tron` default fee cap) | TR1–TR8, TR24 | coding agent + dispatcher gate | PR #37 95a61be: local543, independent review and Linux/macOS CI40836777320 pass; merged in PR #51 (599bde9), 2026-10-30 |
| B1, codec | `handoff/tron-codec` | TRON-4 | TR12, TR13 | coding agent + dispatcher gate | PR #40 8df04af: local557, independent review and CI40840314282 both pass; merged in PR #51 (599bde9), 2026-10-30 |
| B2, guard | `handoff/tron-policy` | TRON-3, TRON-5, the `tron` default fee cap; review evidence carries B1's refusal list | TR9–TR11, TR14, TR15 | coding agent + dispatcher gate | PR #41 6f7f18b: local570, independent review and CI40841286260 both pass; merged in PR #51 (599bde9), 2026-10-30 |
| C, send it | `handoff/tron-send` | TRON-6, TRON-7 | TR16–TR21, TR25 | coding agent + dispatcher gate | PR #42 1a4f36f: local595 with the Linux observation-timing repair; CI40847935216 Linux/macOS pass; Q3 closed; merged in PR #51 (599bde9), 2026-10-30 |

| ID | Acceptance | Gap | Source | Status |
|---|---|---|---|---|
| TR1 | the A1 mnemonic gives `TWer2…` (i=0), `TPjjv…` (i=1) | merged in PR #51 (599bde9), 2026-10-30, as PR #37 (local543, exact-head CI pass) | A | ✅ |
| TR2 | `addresses.tron` cached; session-verified after unlock, no prompt | merged in PR #51 (599bde9), 2026-10-30, as PR #37 (local543, exact-head CI pass) | A | ✅ |
| TR3 | an old key: `WALLET_LOCKED` locked, `--verify` adds the address | merged in PR #51 (599bde9), 2026-10-30, as PR #37 (local543, exact-head CI pass) | A | ✅ |
| TR4 | case-flipped, bad-checksum, `0x` caches refused | merged in PR #51 (599bde9), 2026-10-30, as PR #37 (local543, exact-head CI pass) | A | ✅ |
| TR5 | a Nile genesis is `RPC_ERROR` before any read | merged in PR #51 (599bde9), 2026-10-30, as PR #37 (local543, exact-head CI pass) | A | ✅ |
| TR6 | owner-permission warning from the M7 fixture; live balance equals the node, nothing pinned | merged in PR #51 (599bde9), 2026-10-30, as PR #37 (local543, exact-head CI pass) | A | ✅ |
| TR7 | USDT balance, 6 decimals; a token error is the row's | merged in PR #51 (599bde9), 2026-10-30, as PR #37 (local543, exact-head CI pass) | A | ✅ |
| TR8 | `balance` has a `tron` row, Sepolia last | merged in PR #51 (599bde9), 2026-10-30, as PR #37 (local543, exact-head CI pass) | A | ✅ |
| TR9 | `whitelist add bob T…` stores `0x…` + `["tron"]`, shows `T…` | merged in PR #51 (599bde9), 2026-10-30, as PR #41 (local570, CI pass) | B2 | ✅ |
| TR10 | six bad `add` inputs refused before the password; `validate` refuses mixed and contract `tron` entries | merged in PR #51 (599bde9), 2026-10-30, as PR #41 (local570, CI pass) | B2 | ✅ |
| TR11 | old lists verify unchanged; `*` is no `tron` payee; the EVM twin of a `tron` payee is refused | merged in PR #51 (599bde9), 2026-10-30, as PR #41 (local570, CI pass); PR #51's fix round 1 also refuses `*` payees and `0x` bodies on Tron in the TUI | B2 | ✅ |
| TR12 | golden vectors decode, re-encode, hash, sign, recover | merged in PR #51 (599bde9), 2026-10-30, as PR #40 (local557, CI pass) | B1 | ✅ |
| TR13 | every non-canonical or foreign input refused | merged in PR #51 (599bde9), 2026-10-30, as PR #40 (local557, CI pass) | B1 | ✅ |
| TR14 | the action table, zero signatures per refusal | merged in PR #51 (599bde9), 2026-10-30, as PR #41 (local570, CI pass) | B2 | ✅ |
| TR15 | fee cap, missing cap, expiration window | merged in PR #51 (599bde9), 2026-10-30, as PR #41 (local570, CI pass) | B2 | ✅ |
| TR16 | a forged daemon reply is refused, nothing broadcast | merged in PR #51 (599bde9), 2026-10-30, as PR #42 (local595, CI40847935216 pass); [C evidence](c-evidence.md) | C | ✅ |
| TR17 | dry-run with `fees`; a refused dry-run asks no signature; activation at the prices read, staked bandwidth assumed gone in the maximum | merged in PR #51 (599bde9), 2026-10-30, as PR #42 (local595, CI40847935216 pass); [C evidence](c-evidence.md) | C | ✅ |
| TR18 | `INSUFFICIENT_FUNDS` ×4 (one above the estimate, below the maximum), revert `RPC_ERROR`, before signing | merged in PR #51 (599bde9), 2026-10-30, as PR #42 (local595, CI40847935216 pass); [C evidence](c-evidence.md) | C | ✅ |
| TR19 | nonce flags refused on `tron` | merged in PR #51 (599bde9), 2026-10-30, as PR #42 (local595, CI40847935216 pass); [C evidence](c-evidence.md) | C | ✅ |
| TR20 | TRX and USDT sent and confirmed against the mock; fee above the estimate stays within the maximum | merged in PR #51 (599bde9), 2026-10-30, as PR #42 (local595, CI40847935216 pass); [C evidence](c-evidence.md); real-terminal acceptance: each Tron broadcast equals its review | C | ✅ |
| TR21 | each broadcast outcome; uncertain and duplicate messages name `expiresAt` and the second-payment risk | merged in PR #51 (599bde9), 2026-10-30, as PR #42 (local595, CI40847935216 pass); [C evidence](c-evidence.md) | C | ✅ |
| TR22 | every row of 05 §TUI is in the TUI coverage matrix | T38–T41 merged in PR #51 (599bde9), 2026-10-30, as #49; evidence: [h18](../v0-2-tui/h18-tron-tui-receipt.md), [h18b](../v0-2-tui/h18b-tron-history-receipt.md); real-terminal acceptance: coverage matrix 59 of 59 PASS at each size | TUI task | ✅ |
| TR24 | `sign-message`, `call`, `invoke` on `tron` → `CAPABILITY_UNSUPPORTED` | merged in PR #51 (599bde9), 2026-10-30, as PR #37 (local543, exact-head CI pass) | A | ✅ |
| TR25 | `expired-unseen` only by the node's head and a node within 60 s of the local clock; never a safe-resend message | merged in PR #51 (599bde9), 2026-10-30, as PR #42 (local595, CI40847935216 pass); [C evidence](c-evidence.md) | C | ✅ |

### Round section (as it was in loop.md, with the result)

**Result, 2026-10-30.**
- **Merge.** PR #51 merged 2026-10-29T21:34Z as 599bde9; it marks #37, #40, #41, #42 and #49 merged and supersedes the draft #33 (Tron A). It closed #34.
- **Preview.** `preview-2026-10-29-2134-599bde9ee211` = Homebrew `zwallet-preview 2026.10.29.2134` ([tap PR #18](https://github.com/2lab-ai/homebrew-tap/pull/18)); installed, `brew test` passes, codesign verifies, the binary's sha256 equals the release asset digest.
- **Gate at 2a1ee64** (the PR head; the merge commit adds no change): macOS 923 passed, 0 failed; Linux Docker 916 passed on the second full run (the first run's one failure was the pty test tracked as #52); [CI run 40955576162](https://github.com/2lab-ai/zwallet/actions/runs/40955576162) success on macOS and ubuntu.
- **Real-terminal acceptance** (tmux, release build, loopback mock nodes, sandboxed, no non-loopback connection): six scenarios PASS at 120x40 and 80x24, 152 checks on the 2a1ee64 build and 154 on the installed preview. Scenario 6 receives the same-seed T-address (equal to `zwallet address --chain tron`), reads TRX and USDT from the mock, adds a Tron payee in the TUI, sends TRX and USDT, and looks the txid up; every Tron broadcast equals its review, from the mock node's decoded log. Evidence: the QA run's logs are kept off-repo; the results are in [PR #51's body](https://github.com/2lab-ai/zwallet/pull/51).
- **Not done:** TR23, the funded mainnet send; it stays with the owner ([`loop.md`](loop.md)).

**Round 1 — plan.** PRs per [`units.md`](units.md) (the dispatcher's plan after review round 1, 2026-10-28; no line cap, each PR reviewable as one argument). Worktrees at `.worktrees/<branch with / as ->`. Rows A, B1, B2, C: Closed rows above. TR23's row (the live mainnet send; closes Q2; owner funds and decision + dispatcher; "waits for C and its preview") stays open in [`loop.md`](loop.md).

**Build facts.**
- Gate (as `../v0-2-tui/loop.md` §Build facts, now in [`../v0-2-tui/loop.done.md`](../v0-2-tui/loop.done.md) R1): `cargo fmt --check &&
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

**Open gaps (intro).** As of 2026-10-30, A, B1, B2, C and the Tron TUI were combined on `integ/tui-tron-final` (#35 and #37–#49 merged with main). The 🟡 rows closed when that PR merged; TR23 stays with the owner. C focused results, the production-client Q3 receipt and the remaining owner-only evidence are recorded in [`c-evidence.md`](c-evidence.md).

**Next (as it was).** Finish PR #42's deterministic observation/deadline repair and verify its new commit on both CI platforms. A/B1/B2 are implemented, independently reviewed and green at the linked commits; they await upstream merge permission. Complete TUI12 integration and its tests, then record final stress/live-read/Intel/preview evidence explicitly. Owner-funded TR23 and release publication are not authorized by the handoff. Completion is tracked in ../handoff-completion/completion-checklist.md. Done 2026-10-30 (PR #42 at 1a4f36f, CI40847935216; TUI-12 as #49; preview and real-terminal acceptance above), except TR23.

**Spec review.** The spec passed the three-engine review in round 3 (2026-10-28: grok-4.7, gpt-6-astra and fable all APPROVE; rounds 1 and 2 are summarised in 05-tron.md). fable's two non-blocking round-3 remarks for C stay open in [`loop.md`](loop.md).


## R2 — closed 2026-10-30 (remaining evidence notes, draft PR #59)

Resolved in `05-tron.md` Fee field branches and contract recipients: exact
`activationSun`/`bandwidthSun` branch values match `send::fees`; official java-tron
b33eed89a6a424c498d4fb1b03ca2c86eddf4840 makes native contract-recipient rejection
conditional on governance flags. No TR17 number changed and no funded transaction
was performed. `pricesAtBlock` is qualified as the later observed reference block,
not an atomic price snapshot. Independent source review found the clarification
accurate; final documentation/gate receipt is tracked in security-assurance.

### Original note snapshot

## Open doc notes (fable, spec review round 3, 2026-10-28; non-blocking)

Written for C; C shipped without either, and 05-tron.md does not answer them yet:
- §Fees names the `fees` fields `activationSun` and `bandwidthSun` but not their values in each
  branch (05-tron.md:479); define them in one line, so the TUI's "activation fee" and "bandwidth
  burn" rows are fixed.
- Whether java-tron refuses a TRX `TransferContract` to a contract account
  (`TransferActuator.validate`) is unverified; read it and note it at TR17's contract case
  (no change to the numbers).
