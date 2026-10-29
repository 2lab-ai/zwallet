# H14 whitelist integration receipt — 2026-10-29

The requested whitelist editor now shares the CLI's authenticated plan/apply/view boundary.
This branch integrates the reviewed H13 send state and B2 Tron policy ancestry before adding
H14; it does not claim H18 Tron UI or C Tron transfer/lookup support.

## Source and boundaries

- Branch: `handoff/tui-whitelist-pr`.
- First parent: `685a363` (H13); pending merge parent: `6f7f18b` (Tron codec/policy).
- Merge is intentionally uncommitted for the parent-owned final gate and PR publication.
- Owned backend/UI source from handoff-tui-whitelist, including deferred L/q fixes, H7 frame
  style/source-budget assertions and independent lock regression.
- H13 common terminal failure drain, receipt deadline and first-error-preserving quit behavior
  retained. `mutation_pending` also checks whitelist mutation state.
- H2 repair additions, per-family addresses and Tron activation/owner-permission metadata are
  retained through the shared read adapter independently reviewed in H18.
- EVM-only transfer config callers propagate the new fallible family check. Tron send UI remains
  a later integration; this branch adds no C send/lookup handler.
- T46 now shows registry symbol and decimals alongside the contract and exact smallest base
  units, for EVM and Tron. The signing payload/limit units are unchanged.
- All password owners remain zeroizing and move into the blocking mutation worker. No new
  password clone, RPC, or signer operation belongs to whitelist mutation.

## Verification actually performed

- `cargo check --locked --all-targets`: PASS.
- `cargo clippy --locked --all-targets -- -D warnings`: PASS.
- `cargo fmt --all -- --check`, `git diff --check`: PASS.
- `cargo test --locked --lib wallet::whitelist::`: 35 PASS.
- `cargo test --locked --lib tui::app::whitelist`: 14 PASS.
- `cargo test --locked --test review_whitelist_independent`: 5 PASS.
- `cargo test --locked --test tui nav_`: 13 PASS.
- H13 `terminal_send_failure_tests`: 5 PASS; `quit_error_tests`: 2 PASS.
- `tui::actions::`: 6 PASS; all 55 aliases and inert future actions remain covered.
- T46 symbol/decimals regression was RED before the fix (missing USDC), then GREEN in the 35
  backend tests. `/tmp/zwallet-h14-token-decimals-red.log` retains the failing output.
- Compile only: `cargo test --locked --test tui_whitelist --test tui_whitelist_caps
  --test tui_transfer --no-run`: PASS.

## Pending parent serialized process gate

No PTY, CLI suite or daemon process test was executed by this integration unit. Actual runtime
results must be recorded after the parent runs the new cases and full gate on the final SHA.

- Existing whitelist add/optional unlock and password cancellation/retry PTYs (2).
- New T7 warm-before-password → V read-only, then T23 remove/fee/reset (1).
- New T24 contract/selector/warning, warm registry-token refusal and individual removal (1).
- New T25 message signing off/on/off (1).
- New T44 zero-address add rejected before password/stop/write (1).
- T45–47 PTYs in `tests/tui_whitelist_caps.rs` (3): exact limits and explicit none removal,
  precision/non-registry/overflow refusal, CLI signed verification.
- Combined T2→T6→T7 and T44 legacy zero PTYs from `/tmp/zwallet-tui-combined-pty.patch` in
  `tests/tui_transfer.rs` (2).

Each new mutation PTY reviews all summary pages before CR password submission, compares
files/session before password and signed CLI verification after saving, and checks no echo.
All use debug-scoped disposable homes and `common::exclusive()`; no real wallet/funds.

Independent integration review by tui_forms: CLEAR after all three repairs. The reviewer
reran standalone3, permanent regression5, backend35, UI14, drain5, nav13 and pin2 directly;
no additional blocker. `/tmp/zwallet-h14-independent-review-receipt.md` and
`/tmp/zwallet-h14-independent-review-manifest.json` retain the independent evidence.
No commits, pushes, issue updates, merges or funds were performed by this subagent.

## Independent review repairs

Three findings were reproduced before their fixes, then covered by the permanent public
regressions in `tests/review_whitelist_independent.rs`:

1. A list read begun before Keys/Status invalidated trust could restore checked authority.
   Each read now carries a separate trust-generation snapshot and consumes it once. The edit
   request id is unchanged, so a running mutation's result is never invalidated by this fix.
2. The human list labeled Tron native limits as wei. It now uses sun for Tron, wei for EVM,
   and neutral base units for an unknown chain. Historical JSON field names remain compatible.
3. `q` on a non-input summary Review was swallowed. It now discards the unsubmitted plan and
   quits. Input fields still own their characters; submitted writes still drain before quit.

Direct RED logs: `/tmp/zwallet-h14-pr-reviewfix-red.log`,
`/tmp/zwallet-h14-pr-q-review-red.log`. Final regression5/UI14/backend35 and strict all-target
Clippy/fmt/diff are GREEN after these repairs. Incremental fix patch:
`/tmp/zwallet-h14-independent-fixes.patch`.

## Parent process acceptance and common shutdown correction, 2026-10-29

The parent executed all 13 real PTY cases across tui_transfer (4), tui_whitelist
(6) and tui_whitelist_caps (3). Combined warm/send/edit/global-test-lock and
legacy-zero removal passed, as did T23–25 and UI→worker→signed CLI verification
for T45–47. T45 initially expected an error-code label in a field-validation
notice; the actual correct 19-fractional-digit refusal was observed. Only the
expected wording was corrected, and that complete PTY then passed. No-write,
no-password, exact saved-unit and locked-state checks remain intact.

The shared App now preserves the submitted operation's original exit error if
a same-generation Keys/Status read fails during deferred LockAll. Three public
App regressions (two late-read failures and stale/no-lock controls) passed here;
independent reproduction failed before the one-line preservation change. This
changes neither policy enforcement nor the daemon. The whole gate is repeated
after all changes before the parent publishes this branch.

Evidence: /tmp/zwallet-h14-pty-first-run.log, /tmp/zwallet-h14-t45-second-run.log,
/tmp/zwallet-h14-late-read-green.log; all homes are isolated /tmp/zwt-* fixtures.

Final parent gate passed: Rust 1.95.0 fmt, locked all-target strict Clippy and
full suite **709 passed, 0 failed, 13 explicit live-test skips**. All 13 PTY
cases passed again within this whole gate. Full log: /tmp/zwallet-h14-final-full-gate.log.

## Final macOS CI warm-up synchronization repair — 2026-10-29

CI 40857743153 passed Linux but failed the macOS combined warm/send/edit/re-warm
PTY. Its old wait accepted password-verified and signing bars before the password
worker completed: a previous password pin persists through locking, and a fresh
Status can observe the daemon before Checked. Input during Checking is correctly
ignored, leaving the fixture waiting on the wrong screen after its premature key.

A deterministic public-App event-order regression reproduced the false readiness
condition, then passed with the same predicate now used by the PTY: actual Wallet
panel, fresh balances, normal input hint and both verification/signing bars. Product
code and timeouts are unchanged. The failed actual PTY passed directly in 11.41 s,
and the renewed Rust 1.95.0 fmt, strict locked all-target Clippy and complete debug
suite passed: **710 passed, 0 failed, 13 ignored**. All 13 actual PTY cases passed
again. Logs: `/tmp/zwallet-h14-warm-sync-pty.log` and
`/tmp/zwallet-h14-warm-sync-full-gate.log`. The final combined tree uses its
history-enabled Wallet hint with the same event-order regression. Fresh final-head
CI follows this commit; the earlier failed CI is not counted as green.
