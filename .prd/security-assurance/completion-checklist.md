# Planned development and security — completion checklist

Status: implementation and local acceptance complete, 2026-10-30. Final post-commit
CI/delivery state is maintained by the linked PR checks and issue receipt. [Contract](../10-security-assurance.md),
[raw instructions](ssot.md), [live driver](loop.md).

## Scope reconciliation

| Plan / request | Disposition | Implementation / evidence / remaining gate |
|---|---|---|
| Original review #21–25, #28 | Already delivered in #29 | Keep current regression coverage; no duplicate implementation. |
| Original full TUI/Tron handoff | Already delivered in #51 | Existing acceptance/preview receipts remain in v0.2/v0.3 ledgers. |
| #55/#57 assertion follow-up | Existing draft #59, rebased on main 0994f9a | Original assertion fixes retained; final local gate below supersedes historical local results. Final-head CI is the linked external gate. |
| Comprehensive security explanation (T4/S2) | Complete, independently reviewed | `docs/security-threat-model.md`: capabilities, implemented controls, source/named tests, assumptions, residual risks and unsupported claims. |
| Resize and post-exit observation (T3/S3) | Complete, final local gate passed | Actual runtime with ticks disabled; shrink/grow; complete EOF stream; exact lock/panic tail. Unit receipt: 26 targeted tests passed. |
| Tron fee/contract documentation (T3/S4) | Source clarification and independent factual review complete | Actual fee branches and pinned official java-tron validation conditions; no funded send. |
| AE-1 native reports (T3/S6) | Complete, final local gate passed | Both chain families, six outcome states, exact accounting and provenance, advisory local task identity, CLI and TUI, shell reference. |
| Calldata duplicate identity / configurable window (T3/S7) | Complete, final local gate passed | Optional authenticated policy field, legacy canonical compatibility, CLI/TUI path, explicit local advisory semantics. |
| AE-2 cumulative budget / policy rollback | Explicit design-only exception retained | [#26](https://github.com/2lab-ai/zwallet/issues/26), [#27](https://github.com/2lab-ai/zwallet/issues/27). No daemon writes or external authority selected. |
| AE-3 investment, AE-4 service payments, AE-5 checkout | Dependent future scope, not shipped | Depend on AE-2 plus protocol-specific allowance/signature/merchant policy; bypassing that prerequisite would contradict the retained decision. |
| AE-6 integrations | Shell reference follows AE-1 | Optional framework/MCP/adapters have no selected contract; no new platform or credentials inferred. |
| Solana/TON | Future family architecture, not unfinished original acceptance | Original SSOT says later; derivation/decoder/network/asset/UI contracts must be fixed before a new family can be called complete. |
| User-added tokens | Conflicting historical plan, registry-only retained | Later TUI SSOT removed user-added token kind; metadata/issuer/policy authority is not silently expanded. Arbitrary read-only token lookup already exists. |
| Hardware policy keys / import / password change | Unselected future custody contract | No device migration, password store, restore command or second-factor claim introduced. |
| TUI QR / phrase alternatives | Owner choices remain open | Accepted Q1/B1 CLI pointers remain; no unrequested TUI phrase exposure. |
| Funded Tron TR23 / owner's real wallet / stable distribution | Owner-only | Requires actual funds/owner decision or release authority. Mock/test evidence does not stand in for these actions. |

This is the bounded interpretation of the latest broad continuation. It activates
the written report/duplicate plans rather than treating every WIP as excluded, while
preserving the user's explicit budget/freshness exception and completed work.

## Final verification gates

| Gate | Status / evidence |
|---|---|
| Integrated focused tests | 89 regression passes after integration fixes; all remaining integration binaries were exercised, then the complete final gate rerun. |
| Independent factual/security review | Custody, signing/report, document and TUI reviews complete; all confirmed findings corrected; final executed evidence below |
| Rust 1.95.0 fmt, strict all-target Clippy, complete test suite | Passed: **964 passed, 0 failed, 15 ignored**; per-suite counts and input manifest in [local validation](evidence/local-validation.json). |
| Relative links, Markdown table shape, loop lint, append-only ledgers | Checked before commit; source/named-test references present; old ledgers append-only. |
| Existing ai-insightquest fork draft #59 | [Current draft](https://github.com/2lab-ai/zwallet/pull/59), continued on the existing branch; exact delivery revision is in its final receipt. |
| Linux + macOS CI on final source | [Current-head checks](https://github.com/2lab-ai/zwallet/pull/59/checks) and [delivery receipt](https://github.com/2lab-ai/zwallet/issues/57) are authoritative; this source snapshot cannot predict its post-commit result. Old CI 41038434180 is historical only. |
| Exact head/tree, test counts, CI and artifact links | [Local source/terminal evidence](evidence/README.md); exact submitted commit/merge tree and final CI are recorded on the PR/issue after publication. |

No PR merge/approval, stable release, real funds, new credentials or live-wallet
security change is authorized by this checklist. Those boundaries do not block
independent local implementation, review, testing or the already authorized draft update.
