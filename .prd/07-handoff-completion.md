# Zwallet handoff completion

Status: **shipped** (preview 2026.10.29.2134, PR #51, 2026-10-30). PR #51 merged #35 and
#37–#49, superseded the drafts #31–#33 and closed issue #34. This page is the contract the
handoff was delivered under, not a future target. What shipped, its receipts and what stays
with the owner (TR23, trade-offs Q and B, a stable release; #26 and #27 design-only):
[`handoff-completion/completion-checklist.md`](handoff-completion/completion-checklist.md).

## Goal

Complete the remaining TUI v2 and Tron work handed off in issue #34 and draft PRs
#31–#33, preserving the merged review hardening in main 99fff4e. Use the acceptance
contracts in 03-tui.md, 05-tron.md and their unit plans. Design issues #26 and #27
retain their explicit local-only custody boundary and require honest design decisions,
not claims of strong protection from mutable same-user files.

## Delivery and scope

- Preserve and integrate the draft work, fix confirmed review findings, then implement
  the remaining units with regression tests and independent review.
- Use Rust 1.95.0 and the current signed transaction-limit format. Keep user-facing
  documentation under README and docs, and keep feature status code-backed.
- Run one full CLI suite at a time on this host. Never touch the real wallet or stop
  a real daemon; isolate test and QA homes. Review test subprocess guards before gates.
- Current ai-insightquest permissions on upstream: pull only, no push/merge/triage.
  Continue in the user's authorized fork and publish successor PRs with provenance
  to the draft PRs. Upstream merges and preview installation cannot be claimed until
  an authorized maintainer actually performs them.
- Latest-head macOS and Linux CI success is required for every completed delivery.
  Every new commit includes Co-authored-by: Z <z@2lab.ai>.
- Keep one signing/custody risk boundary per reviewable PR. Do not self-approve PRs.
- Default to the existing safe B1 recovery-phrase pointer; no phrase in the TUI.
  Preserve Q1's CLI QR pointer until a bounded QR design is independently verified.
  Stable tags, funding and real-money TR23 sends require their explicit owner action;
  implement and test all non-funded paths without inventing live receipts.

## Acceptance

Map every open TUI and Tron acceptance row to code, observed tests and its delivery PR.
Unimplemented rows stay open. No placeholder screen may be described as complete.
Document design decisions for #26/#27 and distinguish design completion from a new
runtime guarantee. Finish all authorized code and review work even where upstream
publication permissions are unavailable.
