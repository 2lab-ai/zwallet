# v0.1 — loop

Status: **in-progress**, 2026-10-27. Target = [`ssot.md`](ssot.md), [`../01-spec.md`](../01-spec.md)
acceptance A1–A12. This file holds only open work.

## Round 7 — plan (WU1–WU5, WU1b and the R6 hardening PRs #6–#12 closed: see [`loop.done.md`](loop.done.md))

| WU | Repo · branch · worktree | Scope | Owner | State |
|---|---|---|---|---|
| WU6 | live | user runs create and unlock on fable-m5max, then the agent reads address and balance (A12) | user TTY + dispatcher | the wallet exists (key `zbrain`, created 2026-10-21); the user ran `whitelist reset` and `unlock 1y` on 2026-10-23; the gate runs since then ended that session. Waits for the user's `zwallet unlock` on the PR #12 preview |

## Build facts

- Gate: `cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked`, the cli suite again on a pty ×3 (`script -q /dev/null cargo test --locked --test cli < /dev/null`), live `cargo test --locked -- --ignored`, and Linux Docker `rust:1.99-bookworm` as a non-root user. Dispatcher on 9df65bb (PR #12), 2026-10-27: lib 262, cli 108 (108/108 ×3 on a pty), evm_offline 65, whitelist 5+3, live 11/11; Linux Docker (rust 1.99.1, non-root) lib 262, cli 109, evm_offline 65, whitelist 5+3.
- The gate stops the user's real session. `lock`, every CLI whitelist edit and the `create --unlock` mismatch call `stop_all_daemons`, which finds every zwallet daemon of the user through the process table, whatever its home. Tests that reach it hold `common::exclusive()`. Before a gate, `ps` must show no live `zwallet daemon` of the user.
- CI (`ci.yml`) runs fmt, clippy `-D warnings --locked`, `cargo test --locked` and a version-line smoke on macos-latest and ubuntu-latest for every PR and main push; it is the merge gate together with the dispatcher's local rerun.
- Every main push publishes a `preview-*` prerelease (4 binaries + SHA256SUMS). The tap's `bump-zwallet-preview` job picks it up on its 6 h schedule (`23 */6 * * *`); render by hand when it cannot wait (the steps in `bump.yml`; the Bash tool runs zsh, where `BASH_REMATCH` is empty, so derive the version with `sed`).
- After a preview upgrade the user must `zwallet lock` and unlock again: session verification compares the daemon's cdhash, which changes with every build.
- Reviews: the llmux GPT engines answer again (gpt-6-astra reviewed PR #12 on 2026-10-27, by a direct llmux call). Trinity = grok-4.7, gpt-6-astra, fable.
- Still open: the plaintext metadata (`active`, names, `nextIndex`) is unauthenticated. While unlocked, address reads are cross-checked against the daemon (on macOS the peer's cdhash, since PR #10); while locked they say `verified:false`.
- Live tests: `cargo test -- --ignored` (public mainnet RPC). Unit tests never touch the real `~/.zwallet`. Daemon tests use a short `/tmp/zwt-*` home because of `sun_path`.

## Open gaps

| ID | Acceptance | Gap | Source | Status |
|---|---|---|---|---|
| G3 | A11 | `xbrew install zwallet` measured on the xbrew preview build (macOS formula, Arch pinned binary); stable xbrew users need an xbrew release, pending the owner (it also ships xbrew PRs #1–#3, on preview since 2026-08-26) | xbrew release | 🚪 |
| G4 | A12 | the wallet exists; the agent's read of the session-verified address and the balance waits for the user's unlock at a TTY | WU6 | 🚪 |

## Next

The user unlocks on the PR #12 preview (`zwallet unlock 1y`); the agent then reads the
session-verified address and the balance (A12, closes G4). Funding comes after that. Follow-ups
from PR #11's review: `list` can name an impostor whose answer matches a poisoned cache; check the
spawned daemon child's cdhash before the password is written. Then v0.2 (TUI,
[`../03-tui.md`](../03-tui.md)).
