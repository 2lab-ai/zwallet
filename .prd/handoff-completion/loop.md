# Handoff completion — driver

Status: in-progress, 2026-10-29.

## R1 — recover and finish the foundations

| ID | Scope | Owner | Status | Evidence |
|---|---|---|---|---|
| H1 | Integrate PR31/32 with main; fix activation ordering and password lifetime | root | 🟡 | PR35 fa79ca7 ready; local521 pass; CI40832716457 Linux/macOS pass |
| H2 | Reconcile Tron A PR33 with main, review reads and update documentation | tron_a | 🟡 | PR37 88a59b0; local541 pass, PTY19x3 and Intel build pass; Linux CI copy/spawn ETXTBSY under repair |
| H3 | Tron strict transaction codec and signing primitive (B1/TRON-4) | tron_codec | 🟡 | 11 codec tests pass; independent review/integration pending |
| H4 | TUI warm-up/password/pinned trust (TUI-2b/2c) | tui_unlock | 🟡 | Integrated H4; full534 pass, TUI45 included; fork PR/CI next |
| H5 | Complete custody-preserving design decisions for issues26/27 | design | 🟡 | 08/09 design docs reviewed; publication pending, runtime deliberately deferred |
| H6 | Remaining TUI3–12 and Tron B2/C units | root dispatch | 🟡 | Tron B2 policy, TUI forms/actions and history backend dispatched; other units follow dependencies |
| H7 | Independent reviews, integrated gates, fork PRs and latest-head CI | root/reviewers | 🟡 | H1 and Tron A reviewed; H4 fixes in progress; no completion at pending checks |
| H8 | TUI3+4 navigation, balances, receive, verify/repair and session | tui_forms | 🟡 | Dedicated handoff-tui-navigation tree with H4 and forms |
| H9 | TUI7+8 history, explorer and transaction detail backend | tui_history_backend | 🟡 | Backend and pending/merge review corrections pass; UI integration follows |
| H10 | TUI10 shared wallet/key creation backend | tui_create_backend | 🟡 | Owned password API, CLI reuse; focused validation in progress |
| H11 | Tron B2 family-aware signed policy and daemon validation | tron_policy | 🟡 | Focused policy/library tests pass; daemon rejection corpus added |
| H12 | Tron C fees, signed send, receipts and transaction lookup | tron_codec | 🟡 | H2/B1/B2 integrated in handoff-tron-send; mocks before funded acceptance |
| H13 | TUI5 exact reviewed transfer intent and shared send API | tui_create_backend | 🟡 | Dedicated handoff-tui-send tree from validated H1 |
| H14 | TUI6 shared verified whitelist edit API and zero-payee guard | tron_policy | 🟡 | Dedicated handoff-tui-whitelist tree; follows family policy contract |

| H15 | TUI11 shared contract read, arbitrary token and exact message API | tron_codec | 🟡 | Isolated handoff-tui-tools; invoke follows send/replacement APIs |

Run only one full CLI suite at once. Agent worktrees are under .worktrees; each unit
owns its isolated tree and reports changes for integration before heavy gates.

H1 independent review found stale Keys overwriting activation failure and a blocking metadata
lock on the UI runtime. Both have observed RED regressions and fixes; fresh full gate follows.
