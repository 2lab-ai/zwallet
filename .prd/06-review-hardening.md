# Review hardening

Status: in-progress, 2026-10-29. Implemented on the review branch; not shipped or merged.

## Problem and goals

Release workflows must validate the exact source they publish. Receipt deadlines and daemon expiry must remain effective when external I/O stalls. Optional signed limits must constrain individual transfers and contract calldata without placing policy authority in the CLI.

## Contract and acceptance

- R1: preview and stable publishing depend on reusable fmt/clippy/test gates for the same checked-out commit. Pull requests from forks run read-only CI without publish credentials.
- R2: the receipt polling deadline bounds RPC requests and sleeps; expiry returns pending/unknown rather than a false failure or receipt success after the deadline. A delayed mock response cannot exceed a 100ms test budget by more than scheduling tolerance.
- R3: Rust 1.95.0 fmt, clippy --all-targets --locked -- -D warnings, and cargo test --locked pass. CI and local toolchains agree.
- R4: filesystem inspection never runs under the daemon's session mutex or on its async event-loop thread. Bounded workers carry no seed. On timeout, no signature escapes; lock and expiry remain responsive; late results cannot sign after session invalidation. Include wallet AND socket checks.
- R5: optional signed per-transaction native/token amount and calldata limits are enforced on decoded payload bytes by the daemon, with explicit units and backward-compatible absence. CLI and tests must exercise set/display/denial semantics. No claim of aggregate spend or L1 fee enforcement.

- R6: isolate unlock channel framing/progress parsing behind existing protocol exports; preserve handshake bytes and public behavior. Existing protocol tests remain green.

## Issue mapping

| Requirement | Issue | Disposition |
|---|---|---|
| R1 | #21 | implementation |
| R2 | #22 | implementation |
| R3 | #23 | implementation |
| R4 | #24 | implementation |
| R5 | #25 | incremental stateless controls |
| Cumulative budgets | #26 | design only, user decision |
| Joint rollback | #27 | design only, user decision |
| R6 | #28 | implementation |

## Architecture and constraints

Reuse the existing chain-specific decoder and signed whitelist; do not trust CLI intent fields. Use bounded secret-free filesystem worker threads and recheck session liveness after await. Do not move the seed into worker threads. Preserve no daemon RPC/file writes, no OS keychain, no external account requirement. Joint rollback and cumulative budgets need a different trusted-state boundary and remain planned, by explicit user direction. Contract argument semantic policies and strong L1 total-cost enforcement also require separately specified trust inputs; calldata limits are only a bounded-exposure mitigation.

## Delivery

Fork: ai-insightquest/zwallet. PR base: 2lab-ai/zwallet main. Do not merge or deploy; user direction overrides using-dotprd Phase 3.

PR delivery is complete only after the Linux and macOS CI jobs succeed for the current
PR head. Inspect failed job logs, repair the cause and repeat that gate; local passes,
a successful push or queued checks alone do not complete delivery. The PR body records
the final run and commit as the remote receipt.

## Validation and operational limits

The gate uses Rust 1.95.0; `.github/workflows/validate.yml` checks the caller-supplied exact
revision on macOS and Linux. Publish jobs depend on validation as well as their build matrix.
Local validation is on macOS arm64; remote Linux results are recorded on the pull request.
The reusable workflow changes the CI check hierarchy; maintainers should confirm required-check
settings refer to the intended validation job before merging.

The filesystem regression tests use a stalled inspector seam and the actual daemon server loop;
no real FUSE mount or live wallet is needed. Inspection workers are capped at four, including
abandoned syscalls, and get only paths/inodes/ciphertext markers. A timeout erases the session.
The existing password/decrypt startup sequence remains outside this session-serving guarantee.

Receipt tests use loopback JSON-RPC only. Policy tests sign test-wallet payloads without broadcast,
including limit edges, wrong-chain tokens, duplicate/malformed fields and tampered signed caps.
Reset discloses removal of all optional limits even when the prior list cannot be parsed.
