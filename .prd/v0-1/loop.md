# v0.1 — loop

Status: **in-progress**, 2026-10-28. Target = [`ssot.md`](ssot.md), [`../01-spec.md`](../01-spec.md)
acceptance A1–A12. This file holds only open work.

## Round 8 — plan (WU1–WU5, WU1b, R6 and R7 hardening PRs #6–#15 closed: see [`loop.done.md`](loop.done.md))

| WU | Repo · branch · worktree | Scope | Owner | State |
|---|---|---|---|---|
| WU6 | live | user runs create and unlock on fable-m5max, then the agent reads address and balance (A12) | user TTY + dispatcher | the wallet exists (key `zbrain`, created 2026-10-21); waits for the user's `zwallet unlock 1y` on the newest preview; a watcher reads the address and balance as soon as the user's daemon starts |

## Build facts

- Gate: `cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked`, the cli suite again on a pty ×3 (`script -q /dev/null cargo test --locked --test cli < /dev/null`), live `cargo test --locked -- --ignored`, Linux Docker `rust:1.99-bookworm` as a non-root user, and, for changes that touch macOS signing or the session check, the whole suite for `x86_64-apple-darwin` under Rosetta (the rustup toolchain's own binaries: `TC=~/.rustup/toolchains/stable-aarch64-apple-darwin/bin; PATH=$TC:$PATH RUSTC=$TC/rustc $TC/cargo test --locked --target x86_64-apple-darwin`; the Homebrew cargo and rustc first in PATH have no x86_64 std, and `rustup run` still picks up that rustc). Dispatcher on b1626cc (PR #15), 2026-10-28: lib 269, cli 111 (111/111 ×3 on a pty), evm_offline 65, whitelist 5+3, live 11/11; Linux Docker lib 263, cli 112.
- The gate leaves the user's real session running (PR #13). The suite scopes every stop to its own test homes with the debug-only `ZWALLET_TEST_DAEMON_SCOPE=/tmp/zwt-` (`common::zwallet_at`); release builds carry no knob, and a release-profile test run refuses to start a zwallet. The `ps` check stays as a belt-and-braces step.
- Every macOS binary zwallet ships is ad-hoc signed in CI with the hardened runtime (`codesign --force --sign - --options runtime`, PR #15); a CI step shows dyld then ignores `DYLD_INSERT_LIBRARIES` where System Integrity Protection is on (GitHub's macOS runners have it off: there the step is a notice, and it fails when the SIP state cannot be read). A build without a readable code signature refuses `unlock`. The test harness signs an unsigned debug zwallet.
- CI (`ci.yml`) runs fmt, clippy `-D warnings --locked`, `cargo test --locked` and a version-line smoke on macos-latest and ubuntu-latest for every PR and main push; it is the merge gate together with the dispatcher's local rerun.
- Every main push publishes a `preview-*` prerelease (4 binaries + SHA256SUMS). The tap's `bump-zwallet-preview` job picks it up on its 6 h schedule (`23 */6 * * *`); render by hand when it cannot wait. A merge with no user-facing change does not get a hand-rendered tap bump.
- After a preview upgrade the user must `zwallet lock` and unlock again: session verification compares the daemon's cdhash, which changes with every build.
- Reviews: per change set, an evidence-file review (`gpt6-reviewer`) and a worktree review (`zworkflow:code-reviewer`); specs and decisions go to trinity (grok-4.7, gpt-6-astra, fable). `gh pr review --approve` on our own PRs is refused (same account), so merges rest on these receipts.
- Still open: the plaintext metadata (`active`, names, `nextIndex`) is unauthenticated. While unlocked, address reads are cross-checked against the daemon (on macOS the peer's cdhash); while locked they say `verified:false`.
- Live tests: `cargo test -- --ignored` (public mainnet RPC). Unit tests never touch the real `~/.zwallet`. Daemon tests use a short `/tmp/zwt-*` home because of `sun_path`.

## Open gaps

| ID | Acceptance | Gap | Source | Status |
|---|---|---|---|---|
| G3 | A11 | `xbrew install zwallet` measured on the xbrew preview build (macOS formula, Arch pinned binary); stable xbrew users need an xbrew release, pending the owner (it also ships xbrew PRs #1–#3, on preview since 2026-08-26) | xbrew release | 🚪 |
| G4 | A12 | the wallet exists; the agent's read of the session-verified address and the balance waits for the user's unlock at a TTY | WU6 | 🚪 |

## Next

The user unlocks on the newest preview (`zwallet unlock 1y`); the agent then reads the
session-verified address and the balance (A12, closes G4). Funding comes after that. v0.2 (the
TUI) runs in [`../v0-2-tui/loop.md`](../v0-2-tui/loop.md).
