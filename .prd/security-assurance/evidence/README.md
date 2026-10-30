# Verification evidence — 2026-10-30

This snapshot contains local observations for the source manifest in
[`local-validation.json`](local-validation.json). It does not infer final GitHub CI
from the earlier PR head. The [current draft checks](https://github.com/2lab-ai/zwallet/pull/59/checks)
and [delivery receipt](https://github.com/2lab-ai/zwallet/issues/57) identify the exact
submitted commit, merge checkout and platform outcomes after publication.

## Local gate

Rust 1.95.0: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo test --no-fail-fast`. Complete suite: **964 passed, 0 failed, 15 ignored**.
Ignored cases require external/live prerequisites and are not counted as passes.
All executed signing/broadcast fixtures used isolated test homes and mock/test data;
no owner wallet or real funds were used. The JSON lists every test binary's count.
Input-manifest SHA256: `c10573c2ec7304baea3c152b2a2a0eb52370fb1a50c1200e7f6ef7a39646d167`. Documentation-only edits are outside that manifest.

The first integration attempt exposed an overbroad `verified"` substring assertion
that matched the new `local-file-unverified` provenance value. It now checks the
injected JSON key token while retaining the planted uppercase marker and bounded
line checks. Focused output/navigation validation passed 17 tests. A subsequent integration run
caught an overlong retry warning overlapping the error/wait row and a stale duplicate
test that still equated different calldata. The warning was shortened without losing
its uncertainty/advisory meaning; four review cases now distinguish identical and
different calldata in signature and raw forms. All 28 replacement tests passed. The
complete final gate above was rerun after these fixes. Static integration review also caught a malformed action-table
merge and the lost Tools menu hint. Final UI checks restored the EVM advisory
range label and kept both provenance and the pay-twice warning on Tron; all 24
transfer-review and 20 Tron UI tests passed. All corrections precede this final gate.

## Real terminal observations

`tests/tui_tasks.rs::task_create_list_report_real_terminal_both_chains_and_sizes`
starts the actual terminal runtime, creates a task from the selected account,
checks offline and online reports, default redaction, exact units, back navigation
and terminal restoration. State tests separately cover reveal and read cancellation. Each file contains the observed create/offline/online frames, decoded
by the terminal parser; these are test fixtures, not an owner's wallet or a product
mockup. Four scenarios are exercised in one test:

- [EVM 80×24](zwallet-task-tui-ethereum-80x24.txt)
- [EVM 120×40](zwallet-task-tui-ethereum-120x40.txt)
- [Tron 80×24](zwallet-task-tui-tron-80x24.txt)
- [Tron 120×40](zwallet-task-tui-tron-120x40.txt)

The full gate also observes actual SIGWINCH shrink/grow with refresh ticks disabled
and the exact failed-lock/panic stream through EOF. The test-only controls are not
present in ordinary builds.

## Independent source review

Separate custody, signing/report and terminal reviewers inspected the final relevant
changes. Corrections verified: missing receipt/fee evidence stays unknown; conflicting
mined replacement identities stay uncertain; source/time provenance and default
address redaction persist; sanitized credential-bearing origins can be saved safely;
duplicate refusals retain their advisory source; task-write warnings survive receipt
updates; all 60 menu actions are registered and routed. No confirmed finding remained.
These were source reviews, not independent penetration tests or cryptographic audits.
Executed acceptance comes from the gate and frames above, followed by the PR checks.

## Requirement-to-evidence mapping

| Requirement | Evidence |
|---|---|
| T1/T2 original handoff | Existing H1–H24 ledger, carried #55 fixes, final source gate and PR #59 / issue #57 receipt |
| T3 remaining authorized plan | S3 terminal tests; S4 pinned Tron fee/contract rules; S6 CLI/backend/TUI report and replacement tests; S7 signed-policy and duplicate/calldata tests; complete gate and final CI |
| T4 security explanation | `docs/security-threat-model.md` covers each required attack class with code/named tests, assumptions, residual risk and unsupported claims; local-link/test-name/table checks and independent factual review |

The referenced optional `local:ztrace` skill was not available in the installed skills
or the source skill repository. No ztrace execution is claimed; the consolidated
mapping above uses directly executed tests, observed terminal frames and source review.

## Linux portability follow-up

Initial CI [41058794545](https://github.com/2lab-ai/zwallet/actions/runs/41058794545)
rejected the new test harness's mutable-reference argument to Linux `openpty`, whose
libc declaration uses a const pointer. macOS declares the same argument mutable.
The harness now uses the repository's existing `&raw mut window` convention, which
fits both declarations without suppressing lint. The local complete gate and source
manifest above were refreshed after this test-only correction. Final Linux/macOS
results still come from the exact-head PR checks and delivery receipt.
