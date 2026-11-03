# Source and intent

## SSOT-LIST
1. 2026-11-02 user, verbatim: "트론 qr 주소 같은 라인에 별도 표시해주고 솔라나랑 톤까지 같은 시드로 모두 처리 가능하도록 계획하고 끝까지 진행해줘 "

Prior instructions and completed work remain in `.prd/security-assurance/ssot.md`; no completed scope is reopened. ai-insightquest fork, draft PR, Z coauthor, no owner wallet/real funds and #26/#27 design-only constraints carry forward.

## SSOT-TASK-TREE
- T5 — Tron QR and separately readable address together (source: "트론 qr 주소 같은 라인에 별도 표시해주고").
- T6 — Solana and TON with the same seed (source: "솔라나랑 톤까지 같은 시드로 모두 처리 가능하도록").
- T7 — Plan and complete implementation (source: "계획하고 끝까지 진행해줘").

## INTENT-LIST and execution mapping
- I5 — readable, scannable receive identity: I5.1 QR/address presentation ← T5 [build][rev].
- I6 — one recovery secret for chain-specific accounts: I6.1 derivation/migration [glue], I6.2 strict Solana family [build], I6.3 strict TON family [build], I6.4 native CLI/TUI flows [glue], all ← T6 [rev].
- I7 — reviewable verified result: I7.1 plan/docs/tests [build][rev], I7.2 existing draft PR/CI [reuse][irrev], both ← T7 under continuing explicit PR authorization.

Diff: T5/T6/T7 added; old completed tasks kept. Assumption: “all” means native wallet lifecycle on both named chains, not every arbitrary smart-contract/token protocol. This explicit native SOL/TON scope is a reshaping [암묵지] disclosed before work. TON native-mnemonic import is not equivalent to BIP39: record the multichain scheme/path and V4R2 identity. Full safety against malicious RPC/compromised host remains unsupported.
