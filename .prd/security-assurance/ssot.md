# Security assurance — fixed user instructions

Date: 2026-10-30.

Trusted user message from the main conversation, verbatim:

> 문서 머지했고 zwallet 계획한것까지 개발 끝까지 진행해주 저까ㅜ 멈추지말고 
> 그리고 문서는 추가로 왜 해킹에 안전한지 모든 공격 시나리오에 대해서 왜 안전할 수 있는지 명확히 설명하는 문서 만들어줘

Earlier instructions and approved exceptions are preserved in
[`../handoff-completion/ssot.md`](../handoff-completion/ssot.md).

## Task and intent mapping

- T3: continue development through the actual approved plan and final validation.
- T4: create the comprehensive attack-scenario security explanation.
- I3: finish useful outstanding work without recreating shipped features (T3).
- I4: make the security boundary assessable from code, tests and stated limitations (T4).
- I3.1 ← T3 [reuse, rev]: reconcile main, PR #59, plan issues and current acceptance rows.
- I3.2 ← T3 [build, rev]: implement and validate identified authorized gaps.
- I4.1 ← T4 [glue, rev]: assemble the threat model and evidence matrix from primary code/tests.
- I4.2 ← T4 [build, rev]: close concrete evidence gaps needed to support the document.
- I3.3 ← T3 [reuse, irrev]: update the existing fork draft and verify final CI, within the
  user's earlier authorization to publish fixes as fork draft PRs.

Diff: added T3/T4; original T1/T2 completed work is retained. The request to explain
why all attacks are safe is interpreted as comprehensive coverage, not a guarantee:
host/root compromise, password/seed theft, allowed-but-harmful actions, cumulative
spending and rollback limitations must remain explicit. No security authority is
expanded to make a planned feature appear complete.

Scope inference on 2026-10-30: T3 activates the existing AE-1 native report and
calldata-aware/configurable duplicate guard plans. I3.2 includes those reversible
implementation units. AE-2 and its dependent adapters remain blocked by the explicit
#26/#27 choice; future hardware/new-chain choices are not silently selected. The
scope matrix records these distinctions and preserves completed original work.
