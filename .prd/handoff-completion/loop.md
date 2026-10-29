# Handoff completion — driver

Status: in-progress, 2026-10-29.

## R1 — recover and finish the foundations

| ID | Scope | Owner | Status | Evidence |
|---|---|---|---|---|
| H1 | Integrate PR31/32 with main; fix activation ordering and password lifetime | root | 🟡 | PR31 CI success; PR32 MUST-FIX reproduced next |
| H2 | Reconcile Tron A PR33 with main, review reads and update documentation | tron_a | 🟡 | Focused 17 unit + 13 RPC + 6 CLI pass; integrated gate/CI pending |
| H3 | Tron strict transaction codec and signing primitive (B1/TRON-4) | tron_codec | 🟡 | 11 codec tests pass; independent review/integration pending |
| H4 | TUI warm-up/password/pinned trust (TUI-2b/2c) | tui_unlock | 🟡 | Password input and pinned address implementation in progress |
| H5 | Complete custody-preserving design decisions for issues26/27 | design | 🟡 | 08/09 design docs reviewed; publication pending, runtime deliberately deferred |
| H6 | Remaining TUI3–12 and Tron B2/C units | root dispatch | 🟡 | Tron B2 policy, TUI forms/actions and history backend dispatched; other units follow dependencies |
| H7 | Independent reviews, integrated gates, fork PRs and latest-head CI | root/reviewers | 🟡 | H1 and Tron A reviewed; H4 fixes in progress; no completion at pending checks |
| H8 | TUI3+4 navigation, balances, receive, verify/repair and session | tui_forms | 🟡 | Dedicated handoff-tui-navigation tree with H4 and forms |
| H9 | TUI7+8 history, explorer and transaction detail backend | tui_history_backend | 🟡 | 13 history + 2 HTTP tests; UI integration follows |
| H10 | TUI10 shared wallet/key creation backend | tui_create_backend | 🟡 | Owned password API, CLI reuse; focused validation in progress |
| H11 | Tron B2 family-aware signed policy and daemon validation | tron_policy | 🟡 | Focused policy/library tests pass; daemon rejection corpus added |
| H12 | Tron C fees, signed send, receipts and transaction lookup | tron_codec | 🟡 | H2/B1/B2 integrated in handoff-tron-send; mocks before funded acceptance |

Run only one full CLI suite at once. Agent worktrees are under .worktrees; each unit
owns its isolated tree and reports changes for integration before heavy gates.

H1 independent review found stale Keys overwriting activation failure and a blocking metadata
lock on the UI runtime. Both have observed RED regressions and fixes; fresh full gate follows.
