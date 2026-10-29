# Handoff completion — driver

Status: in-progress, 2026-10-29.

## R1 — recover and finish the foundations

| ID | Scope | Owner | Status | Evidence |
|---|---|---|---|---|
| H1 | Integrate PR31/32 with main; fix activation ordering and password lifetime | root | 🟡 | PR35 fa79ca7 ready; local521 pass; CI40832716457 Linux/macOS pass |
| H2 | Reconcile Tron A PR33 with main, review reads and update documentation | tron_a | 🟡 | PR37 95a61be ready; local543 pass; CI40836777320 Linux/macOS pass after ETXTBSY repair |
| H3 | Tron strict transaction codec and signing primitive (B1/TRON-4) | tron_codec | 🟡 | 11 codec tests pass; independent review/integration pending |
| H4 | TUI warm-up/password/pinned trust (TUI-2b/2c) | tui_unlock | 🟡 | PR38 f70924d ready; local534 pass; CI40835576772 Linux/macOS pass |
| H5 | Complete custody-preserving design decisions for issues26/27 | design | 🟡 | 08/09 docs published in PR35; issue26/27 comments linked; runtime deliberately deferred |
| H6 | Remaining TUI3–12 and Tron B2/C units | root dispatch | 🟡 | Tron B2 policy, TUI forms/actions and history backend dispatched; other units follow dependencies |
| H7 | Independent reviews, integrated gates, fork PRs and latest-head CI | root/reviewers | 🟡 | H1 and Tron A reviewed; H4 fixes in progress; no completion at pending checks |
| H8 | TUI3+4 navigation, balances, receive, verify/repair and session | tui_forms | 🟡 | Integrated navigation; full566 pass incl60TUI; fork PR/CI next |
| H9 | TUI7+8 history, explorer and transaction detail backend | tui_history_backend | 🟡 | Backend and pending/merge review corrections pass; UI integration follows |
| H10 | TUI10 shared wallet/key creation backend | tui_create_backend | 🟡 | Owned password API, CLI reuse; focused validation in progress |
| H11 | Tron B2 family-aware signed policy and daemon validation | tron_policy | 🟡 | Focused policy/library tests pass; daemon rejection corpus added |
| H12 | Tron C fees, signed send, receipts and transaction lookup | tron_codec | 🟡 | H2/B1/B2 integrated in handoff-tron-send; mocks before funded acceptance |
| H13 | TUI5 exact reviewed transfer intent and shared send API | tui_create_backend | 🟡 | 10 send API mocks pass; transfer UI in progress |
| H14 | TUI6 shared verified whitelist edit API and zero-payee guard | tron_policy | 🟡 | 33 API tests pass; whitelist UI in progress |

| H15 | TUI11 shared contract read, arbitrary token and exact message API | tron_codec | 🟡 | Isolated handoff-tui-tools; invoke follows send/replacement APIs |

| H16 | TUI10 creation forms and end-to-end seed/key workflow | tui_forms | 🟡 | Shared create backend plus navigation; B1 phrase pointer retained |
| H17 | TUI7+8 history and detail screens | tui_history_backend | 🟡 | Reviewed history API plus navigation; chain/owner generations pinned |

| H18 | TUI12 family integration: selected-chain address, Tron history/send adapters | root | 🟡 | Review found primary EVM address must not appear under a Tron label before opening a key |

| H19 | TUI9 trusted original intent and fixed-nonce replacements | tui_foundation_review | 🟡 | Separate replacement boundary on reviewed send/history APIs; no journal-only authority |

Run only one full CLI suite at once. Agent worktrees are under .worktrees; each unit
owns its isolated tree and reports changes for integration before heavy gates.

H1 independent review found stale Keys overwriting activation failure and a blocking metadata
lock on the UI runtime. Both have observed RED regressions and fixes; fresh full gate follows.
