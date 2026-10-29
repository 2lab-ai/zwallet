# Review hardening — done (append-only)

## R1 — closed 2026-10-29

Local implementation converged: 505 tests pass, 11 ignored; remote delivery follows this snapshot.

### Closed rows

| ID | Scope | Status | Verification Evidence | Last Measured |
|---|---|---|---|---|
| WU1 | receipt deadline | ✅ | 4 regression cases; full gate | 2026-10-29 |
| WU2 | bounded daemon filesystem inspection | ✅ | 15 daemon tests; full gate | 2026-10-29 |
| WU3 | signed stateless limits | ✅ | 2 integration cases, signature fixture; full gate | 2026-10-29 |
| WU4 | CI/Clippy, issues and PRD preparation | ✅ | actionlint, strict Clippy, #21–#28, full gate | 2026-10-29 |
| WU5 | protocol module boundary | ✅ | 33 existing protocol cases; full gate | 2026-10-29 |

### Round section (as recorded at close)

## R1 observations (2026-10-29)

- Issue tracking created as ai-insightquest: #21–#28. Fork confirmed: ai-insightquest/zwallet.
- Receipt regression observed RED (late receipt) then GREEN (4 cases); integrated test queued.
- Daemon focused tests: 15 pass, including real server-loop expiry and bounded 12-request/4-worker stall.
- Protocol extraction: original 33 protocol tests pass; integrated Clippy required moving two test-only imports.
- Policy focused tests: 2 pass plus original pinned signature fixture. Independent review found a reset warning gap when policy JSON is unreadable; fixed with an unconditional disclosure and regression test.
- actionlint and loop-lint: 0 failures. Integrated fmt and strict Clippy pass; full tests completed: 505 passed, 11 live RPC tests ignored, exit 0.
- User chose design-only tracking for #26 and #27. No merge or deployment is permitted.
- Personal co-author rule recorded; commit trailer will be verified before push.

## R2 — closed 2026-10-29

Local documentation reconciliation converged on main `51b4a51`; PR #29 publication follows this snapshot.

### Closed rows

| ID | Scope | Status | Verification Evidence | Last Measured |
|---|---|---|---|---|
| WU6 | Integrate latest main and reconcile README, docs and PRD | ✅ | Full gate: 505 passed, 11 ignored; 113 local links; three public CLI help paths | 2026-10-29 |

### Round section (as recorded at close)

### R2 observations (2026-10-29)

- Upstream PR #30 is merged at main `51b4a51`. Rebase conflicts in README and the
  whitelist spec are resolved while preserving the new docs split and WIP labels.
- Source, tests, Cargo inputs, toolchain and workflows match the prior PR head `3c745ce`.
  The TUI Clippy fix is already in upstream main.
- README stays concise; usage/JSON, security boundaries, feature status and release
  validation are synchronized in `docs/`. Local links: 113 checked, 0 missing paths/anchors.
- All three limit commands are reachable through the built CLI's `--help` in a clean
  environment; no wallet is opened. fmt, strict Clippy and actionlint pass.
- Full offline test suite: 505 passed, 11 live-RPC tests ignored, 0 failed (exit 0).
- re-verified ✅: WU1–WU5 · full gate on the resolved main `51b4a51` rebase tree.
- Upstream main and fork head were rechecked before publication: `51b4a51` and `3c745ce`.
  Remote publication follows this validated snapshot; the PR is the delivery receipt.

## R3 — closed 2026-10-29

Local Linux lint regression repaired. Remote CI remains the mandatory delivery gate.

### Closed rows

| ID | Source | Scope | Status | Verification Evidence | Last Measured |
|---|---|---|---|---|---|
| WU7 | regression:WU5 | Scope macOS ARM test-only Read import to its sole caller | ✅ | Linux ARM Rust 1.95.0 strict Clippy exit 0; macOS full gate: 505 passed, 11 ignored | 2026-10-29 |

### Round section (as recorded at close)

- CI run 40812668867, Ubuntu job 123246519143 failed strict Clippy at
  `src/core/protocol.rs:2174`: the shared test-module Read import was unused on Linux.
- The only synchronous Read caller is gated to macOS ARM. Move the import into that
  test function, preserving behavior and keeping warnings denied on both platforms.
- Direct Linux ARM validation used the existing Docker runtime and a source-only copy
  without `.git`, host credentials or a real wallet. Rust 1.95.0 Clippy: exit 0.
- macOS ARM fmt, strict Clippy and full tests pass: 505 passed, 11 live-RPC tests ignored.
- The user's correction makes remote CI success on the current PR head part of delivery.
  Push and inspect both platform results; the PR body is the final remote receipt.

## R4 — closed 2026-10-30 (delivery: PR #29 merged)

- result: the delivery gate passed and the owner merged [PR #29](https://github.com/2lab-ai/zwallet/pull/29) on 2026-10-29 (06:17Z) as 99fff4e; [CI 40814597506](https://github.com/2lab-ai/zwallet/actions/runs/40814597506) Linux and macOS success at its head 21b3a8c; #21–#25 and #28 closed completed; #26 and #27 stay open, design only. Recorded at the 2026-10-30 closeout of the TUI and Tron work.

### Closed rows

None: this round was the remote delivery gate, with no ID rows.

### Round section (as it was in loop.md)

Status: local validation complete, 2026-10-29.

Local repair evidence: `validation.txt` and `loop.done.md` R3. The macOS ARM-only
stdin helper owns its Read import; Rust 1.95.0 strict Clippy passes on both local
macOS ARM and Linux ARM, and the full macOS suite passes.

Delivery gate: Linux and macOS CI must both succeed for the current head of
[PR #29](https://github.com/2lab-ai/zwallet/pull/29). Inspect and resolve failures;
record the successful run and head in the PR body before reporting completion.
This local snapshot is not a substitute for that remote receipt.

Do not merge or deploy. Design issues #26 and #27 remain open by user decision.
