# Original-goal completion checklist

Status: done, 2026-10-30. Issue #34's handoff shipped in PR #51 and preview 2026.10.29.2134.
This checklist records what shipped, with its receipts, and what stays with the owner; it does
not claim that the whole product specification has shipped (01-spec.md keeps its own status).

## What shipped

- **PR [#51](https://github.com/2lab-ai/zwallet/pull/51)**, `integ/tui-tron-final`, merged
  2026-10-29T21:34Z (2026-10-30 KST) as 599bde9. It contains #35 and #37–#49, each now marked
  merged, and supersedes the drafts #31–#33. It closed #34 and #50.
- **Preview** `preview-2026-10-29-2134-599bde9ee211` (a prerelease at 599bde9: four binaries and
  SHA256SUMS) = Homebrew `zwallet-preview 2026.10.29.2134`
  ([tap PR #18](https://github.com/2lab-ai/homebrew-tap/pull/18), merged 2026-10-29T21:59Z).
  Installed on the dispatcher's Mac: `brew test` passes, codesign verifies, and the binary's
  sha256 equals the release asset digest.
- **Gate at 2a1ee64** (the PR head; the merge commit adds no change), Rust 1.95.0:
  - macOS: fmt and strict Clippy clean; 923 passed, 0 failed, 15 ignored; every pty-driven test
    binary again on a pty.
  - Linux Docker, non-root: the first full run 915 passed and 1 failed (the pty test
    `t27_t28_pty_restart_original_proof_speedup_and_cancel_verify_signed_bytes`, tracked as
    [#52](https://github.com/2lab-ai/zwallet/issues/52)); the second full run 916 passed.
  - [CI run 40955576162](https://github.com/2lab-ai/zwallet/actions/runs/40955576162): macOS and
    ubuntu success.
- **Real-terminal acceptance** (tmux, release build, loopback mock nodes, sandboxed, no
  non-loopback connection): all six scenarios PASS at 120x40 and 80x24.
  - 152 checks on the 2a1ee64 release build; 154 on the installed preview 2026.10.29.2134,
    including a tab round trip on the whitelist summary.
  - The [coverage matrix](../03-tui.md#coverage-matrix): 59 of 59 rows PASS at each size. 3 rows
    are not TUI features by design (row 8, a start flag; rows 57 and 58, n/a).
  - Every signed EVM transaction and every Tron broadcast equals its review, from the mock
    nodes' decoded logs.
  - Evidence: the QA run's logs are kept off-repo; the results are in [PR #51's body](https://github.com/2lab-ai/zwallet/pull/51).
- **Defects found and fixed before the merge**, each RED first (PR #51's body): the #46 policy
  category (now from the verified bytes, never the journal); #47's plan fingerprint, Tron payees
  and summary paging; the real-terminal D1–D4; the whitelist list wrapping; the explorer
  override (`ZWALLET_EXPLORER_<CHAIN>`); the tab-round-trip losses (whitelist plan, tools
  reviews); the guard labels.
- Ledgers: [`../v0-2-tui/loop.done.md`](../v0-2-tui/loop.done.md) R1,
  [`../v0-3-tron/loop.done.md`](../v0-3-tron/loop.done.md) R1, [`loop.done.md`](loop.done.md) R1.

## Owner-only (open by design)

- **TR23**: the funded mainnet TRX and USDT send.
- **T18 / trade-off Q**: a QR code on receive. Q1, the safe pointer line, is shipped.
- **Trade-off B**: the recovery phrase in the TUI. B1, the CLI pointer, is shipped.
- **The owner's own acceptance** on preview 2026.10.29.2134.
- **A stable release or `v*` tag**, and BSC USDT.
- **#26 and #27**: design only; runtime enforcement deferred by the owner.
- **#52**: the once-seen Linux pty flake, tracked.

## Original requests and preserved decisions

- The intent-first skills are installed at the personal skills location; personal
  AGENTS.md loads using-intentfirst and using-ssot before every request and records
  the required Co-authored-by: Z <z@2lab.ai> trailer.
- Repository review findings became issues #21–#28 and the implemented hardening
  became PR #29. Its latest head was verified on Linux/macOS before the owner merged.
- PR #29 was rebased over the README/docs split in PR #30; later changes retain that
  division and distinguish reachable behavior from WIP.
- The request was to finish the entire issue #34 handoff, including original drafts
  #31/#32/#33 and TUI/Tron successors, without routine user clarification. Done with PR #51.
- The explicit cumulative-budget/joint-rollback exception stays design-only.
- The handoff did not infer permission for public visibility changes, stable releases or tags,
  permission expansion or live funds.
- Marketing PR #36 owned its README/video work; it merged into main on 2026-10-29, and PR #51
  merged main (#36) last, after the handoff PRs (PR #51 body §How it was built).

## Issue inventory

GitHub issue states below were fetched individually on 2026-10-30.

| Issue | Delivered behavior or contract | Verification and PR | State |
|---|---|---|---|
| [#21](https://github.com/2lab-ai/zwallet/issues/21) | Exact-commit publishing gates | PR #29, reusable validate workflow; head 21b3a8c; CI 40814597506 Linux/macOS pass | Closed completed |
| [#22](https://github.com/2lab-ai/zwallet/issues/22) | Receipt RPC/sleep share one deadline | PR #29; four deadline regressions; later full gates retain them | Closed completed |
| [#23](https://github.com/2lab-ai/zwallet/issues/23) | Strict Clippy restored and Rust 1.95 pinned | PR #29 and every successor all-target gate | Closed completed |
| [#24](https://github.com/2lab-ai/zwallet/issues/24) | Bounded secret-free inspection workers; responsive expiry/lock | PR #29; stalled-worker and late-result regressions | Closed completed |
| [#25](https://github.com/2lab-ai/zwallet/issues/25) | Signed native/token/calldata per-transaction caps | PR #29; daemon/protocol integration tests; the TUI controls shipped in PR #51 (#47) | Closed completed |
| [#26](https://github.com/2lab-ai/zwallet/issues/26) | Approved cumulative-budget design boundary | PR #35 (in PR #51), 08-cumulative-budget-design.md; comment 6490908443 | Open deliberately; runtime enforcement deferred by owner |
| [#27](https://github.com/2lab-ai/zwallet/issues/27) | Approved trusted-freshness design boundary | PR #35 (in PR #51), 09-policy-freshness-design.md; comment 6490909782 | Open deliberately; runtime protection deferred by owner |
| [#28](https://github.com/2lab-ai/zwallet/issues/28) | Unlock wire protocol separated without API change | PR #29; existing full protocol/integration suite | Closed completed |
| [#34](https://github.com/2lab-ai/zwallet/issues/34) | TUI v2 and Tron handoff | PR #51 (599bde9); preview 2026.10.29.2134; §What shipped | Closed completed by PR #51, 2026-10-29T21:34Z |
| [#52](https://github.com/2lab-ai/zwallet/issues/52) | Once-seen Linux pty flake in `t27_t28_pty_restart_…` | Gate at 2a1ee64, Linux first full run | Open; tracked |

PR #29 merged 2026-10-29T06:17Z as 99fff4e. Its head validation receipt is
[CI 40814597506](https://github.com/2lab-ai/zwallet/actions/runs/40814597506):
Ubuntu 499 and macOS 505 tests, zero failures.

## Draft successors and units

Every row below merged in PR #51 (599bde9), 2026-10-30. The evidence column is each unit's
receipt at its own head; PR #51's gate and acceptance (§What shipped) cover the combined tree.

| Source / boundary | Successor | Observed evidence |
|---|---|---|
| #31 + #32 foundation | [PR #35](https://github.com/2lab-ai/zwallet/pull/35), fa79ca7 | Local521; independent activation review; CI40832716457 Linux/macOS pass |
| #33 Tron A | [PR #37](https://github.com/2lab-ai/zwallet/pull/37), 95a61be | Local543; copied-executable race repaired; CI40836777320 both pass; earlier PTY19x3 and Intel build pass |
| TUI2 password/pins | [PR #38](https://github.com/2lab-ai/zwallet/pull/38), f70924d | Local534, TUI45; independent re-review; CI40835576772 both pass |
| TUI3+4 navigation/read/receive | [PR #39](https://github.com/2lab-ai/zwallet/pull/39), f0b2b25 | Local566, TUI60; actual T14/T19 PTY; independent review; CI40838788105 Linux/macOS pass |
| Tron B1 codec | [PR #40](https://github.com/2lab-ai/zwallet/pull/40), 8df04af | Local557, independent codec review, Intel library-test build; CI40840314282 both pass |
| Tron B2 policy | [PR #41](https://github.com/2lab-ai/zwallet/pull/41), 6f7f18b | Local570; decoded-byte/daemon corpus and CLI whitelist tests; CI40841286260 both pass |
| Tron C lifecycle | [PR #42](https://github.com/2lab-ai/zwallet/pull/42), 1a4f36f | Local595; 19 Tron send tests; real isolated daemon+CLI+loopback TRX/USDT passes; deterministic deadline repair; CI40847935216 Linux/macOS pass |
| TUI10 creation | [PR #43](https://github.com/2lab-ai/zwallet/pull/43), 54fe8c1 | Local591; independent reviews; actual T30/T31 PTY pass; CI40845492673 Linux/macOS pass |
| TUI5 transfer | [PR #44](https://github.com/2lab-ai/zwallet/pull/44), 685a363 | Local597; independent reviews, malicious token decimals, deadline/quit regressions and actual PTY2 pass; CI40846478124 Linux/macOS pass |
| TUI7+8 history/detail | [PR #45](https://github.com/2lab-ai/zwallet/pull/45), 18b2f8e | Local598; independent review; ten UI scenarios; bounded test-server writes; frame-style assertions; CI40851168894 both pass |
| TUI9 replacements | [PR #46](https://github.com/2lab-ai/zwallet/pull/46) | Two cryptographic proof tests; 17 replacement tests; its MUST-FIX fixed in PR #51 (358b22b) |
| TUI6 whitelist | [PR #47](https://github.com/2lab-ai/zwallet/pull/47) | Review APPROVE; NB1–NB3 fixed in PR #51 fix round 1 |
| TUI11 tools | [PR #48](https://github.com/2lab-ai/zwallet/pull/48) | Review: no MUST-FIX |
| TUI12 Tron UI | [PR #49](https://github.com/2lab-ai/zwallet/pull/49) | [h18](../v0-2-tui/h18-tron-tui-receipt.md), [h18b](../v0-2-tui/h18b-tron-history-receipt.md) |
| All of the above + main #36 | [PR #51](https://github.com/2lab-ai/zwallet/pull/51), `integ/tui-tron-final` | One merge commit per handoff PR in the integration brief's order; each merge's conflict resolutions saved as a remerge diff and reviewed APPROVE; two fix rounds; §What shipped |

These counts are distinct receipts, not additive coverage claims. Agent-reported focused
checks do not replace the parent's whole gate.

## Permission and environment boundaries (as delivered)

- The ai-insightquest account had pull access only on upstream; it published fork successors
  with provenance to the drafts. The owner's account merged PR #51 and tap PR #18.
- The preview reached Homebrew through the repository's preview pipeline and tap PR #18. No
  stable release, `v*` tag, repo visibility change or permission expansion.
- TR23 real funded Tron transfers remain owner-only. All send acceptance used temporary
  wallets and loopback mock nodes; the real wallet was not touched (the acceptance runs record
  no owner daemon before or after).
- Q3 was tested without signatures or funds: both configured public endpoints returned
  SIGERROR for the unsigned envelope using the production client User-Agent.
- Q1 rendered Tronscan confirmation is unverified: web returned 403 and computer-use
  reported no browser available. RPC historical lookup does not substitute for that proof.
- Safe Q1 receive/QR and B1 recovery-phrase CLI pointers are the shipped defaults; no
  on-screen recovery-phrase popup, BSC USDT addition or final v* tag is inferred.
- Full CLI suites ran one at a time. Test homes and the debug-only daemon scope isolate
  the real ~/.zwallet. The real-terminal acceptance ran sandboxed with loopback-only network.
