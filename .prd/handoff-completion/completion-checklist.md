# Original-goal completion checklist

Status: in-progress, 2026-10-29. This checklist records evidence and open work, not a
claim that all of issue #34 or the product specification has shipped.

## Original requests and preserved decisions

- The intent-first skills are installed at the personal skills location; personal
  AGENTS.md loads using-intentfirst and using-ssot before every request and records
  the required Co-authored-by: Z <z@2lab.ai> trailer.
- Repository review findings became issues #21–#28 and the implemented hardening
  became PR #29. Its latest head was verified on Linux/macOS before the owner merged.
- PR #29 was rebased over the README/docs split in PR #30; later changes retain that
  division and distinguish reachable behavior from WIP.
- The active request is to finish the entire issue #34 handoff, including original
  drafts #31/#32/#33 and TUI/Tron successors. No routine user clarification is needed.
- The explicit cumulative-budget/joint-rollback exception stays design-only.
- Use the ai-insightquest fork. Do not infer permission for upstream merges,
  public visibility changes, releases, stable tags, permission expansion or live funds.
- Marketing PR #36 owns its README/video work. Do not overwrite that branch, its
  assets or its task directory. Reconcile any shared README lines against its final
  merged result before claiming final integration.

## Issue inventory

GitHub issue states below were fetched individually on 2026-10-29.

| Issue | Delivered behavior or contract | Verification and PR | State / remaining work |
|---|---|---|---|
| [#21](https://github.com/2lab-ai/zwallet/issues/21) | Exact-commit publishing gates | PR #29, reusable validate workflow; head 21b3a8c; CI 40814597506 Linux/macOS pass | Closed completed |
| [#22](https://github.com/2lab-ai/zwallet/issues/22) | Receipt RPC/sleep share one deadline | PR #29; four deadline regressions; latest successor full gates retain them | Closed completed |
| [#23](https://github.com/2lab-ai/zwallet/issues/23) | Strict Clippy restored and Rust 1.95 pinned | PR #29 and every successor all-target gate | Closed completed |
| [#24](https://github.com/2lab-ai/zwallet/issues/24) | Bounded secret-free inspection workers; responsive expiry/lock | PR #29; stalled-worker and late-result regressions | Closed completed |
| [#25](https://github.com/2lab-ai/zwallet/issues/25) | Signed native/token/calldata per-transaction caps | PR #29; daemon/protocol integration tests; TUI rows added to #34 work | Closed completed; TUI controls continue under #34 |
| [#26](https://github.com/2lab-ai/zwallet/issues/26) | Approved cumulative-budget design boundary | PR #35, 08-cumulative-budget-design.md; comment 6490908443 | Open deliberately; runtime enforcement deferred by owner |
| [#27](https://github.com/2lab-ai/zwallet/issues/27) | Approved trusted-freshness design boundary | PR #35, 09-policy-freshness-design.md; comment 6490909782 | Open deliberately; runtime protection deferred by owner |
| [#28](https://github.com/2lab-ai/zwallet/issues/28) | Unlock wire protocol separated without API change | PR #29; existing full protocol/integration suite | Closed completed |
| [#34](https://github.com/2lab-ai/zwallet/issues/34) | TUI v2 and Tron handoff | Successors and remaining boundaries below | Open; implementation, integration and final-head gates continue |

PR #29 is merged. Its current-head validation receipt is
[CI 40814597506](https://github.com/2lab-ai/zwallet/actions/runs/40814597506):
Ubuntu 499 and macOS 505 tests, zero failures. Completed fixes are not rebuilt from scratch.

## Draft successors and active units

| Source / boundary | Successor or worktree | Observed evidence | Remaining gate |
|---|---|---|---|
| #31 + #32 foundation | [PR #35](https://github.com/2lab-ai/zwallet/pull/35), fa79ca7 | Local521; independent activation review; CI40832716457 Linux/macOS pass | Upstream merge unavailable to fork account |
| #33 Tron A | [PR #37](https://github.com/2lab-ai/zwallet/pull/37), 95a61be | Local543; copied-executable race repaired; CI40836777320 both pass; earlier PTY19x3 and Intel build pass | Upstream merge unavailable |
| TUI2 password/pins | [PR #38](https://github.com/2lab-ai/zwallet/pull/38), f70924d | Local534, TUI45; independent re-review; CI40835576772 both pass | Upstream merge unavailable |
| TUI3+4 navigation/read/receive | [PR #39](https://github.com/2lab-ai/zwallet/pull/39), f0b2b25 | Local566, TUI60; actual T14/T19 PTY; independent review | CI40838788105 Linux/macOS pass; upstream merge unavailable |
| Tron B1 codec | [PR #40](https://github.com/2lab-ai/zwallet/pull/40), 8df04af | Local557, independent codec review, Intel library-test build; CI40840314282 both pass | Upstream merge unavailable |
| Tron B2 policy | [PR #41](https://github.com/2lab-ai/zwallet/pull/41), 6f7f18b | Local570; decoded-byte/daemon corpus and CLI whitelist tests; CI40841286260 both pass | Upstream merge unavailable |
| Tron C lifecycle | [PR #42](https://github.com/2lab-ai/zwallet/pull/42), deda723 | Local591;19 Tron send tests; real daemon+CLI+local node TRX/USDT passes | CI40842932300 macOS pass/Linux timing regression failed; repair in progress |
| TUI5 transfer | parent integration | Local597; independent reviews, malicious token decimals, deadline/quit regressions and actual PTY2 pass | Fork PR and exact-head CI next |
| TUI6 whitelist | handoff-tui-whitelist | Reviewed API, generic owned password, forms under validation | UI integration, real process gates, review/CI |
| TUI7+8 history/detail | handoff-tui-history | Pending-null and metadata-conflict fixes independently reviewed; ten UI scenarios | Family adapter, whole integration gates/review/CI |
| TUI9 replacements | handoff-tui-replacement | Two cryptographic proof tests and nine focused mocks | UI wiring, independent signing-boundary review, full gates/CI |
| TUI10 creation | [PR #43](https://github.com/2lab-ai/zwallet/pull/43), 54fe8c1 | Local591; independent reviews; actual T30/T31 PTY pass | Exact-head CI pending |
| TUI11 tools | handoff-tui-tools | 13 exact-message/invoke/read mocks | Tools UI, replacement binding, full gates/review/CI |
| TUI12 Tron UI | parent integration | Shared family pins; adapters identified | Complete selected-family status, send/history adapters and T38–T41 gates |

These counts are distinct receipts, not additive coverage claims. Agent-reported focused
checks do not replace the parent's whole gate. Every publishable commit requires fmt,
all-target strict Clippy and full tests; delivery also verifies CI on that exact SHA.

## Permission and environment boundaries

- The connected ai-insightquest account has pull access but no upstream push/merge
  permission. Original upstream drafts remain preserved; fork successors are attached.
- No stable release, tap publication, repo visibility change or permission expansion.
- TR23 real funded Tron transfers remain owner-only. All send acceptance uses
  temporary wallets and loopback mock nodes; the real wallet daemon is left running.
- Q3 was tested without signatures or funds: both configured public endpoints returned
  SIGERROR for the unsigned envelope using the production client User-Agent.
- Q1 rendered Tronscan confirmation is unverified: web returned 403 and computer-use
  reported no browser available. RPC historical lookup does not substitute for that proof.
- Safe Q1 receive/QR and B1 recovery-phrase CLI pointers remain the implemented defaults;
  no on-screen recovery-phrase popup, BSC USDT addition or final v* tag is inferred.
- Full CLI suites run one at a time. Test homes and the debug-only daemon scope isolate
  the real ~/.zwallet. Final live-read/PTY stress and preview requirements must be
  explicitly recorded as executed, pending or blocked; never inferred from ordinary tests.

