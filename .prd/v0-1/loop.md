# v0.1 — loop

Status: **in-progress**, 2026-10-20. Target = [`ssot.md`](ssot.md), [`../01-spec.md`](../01-spec.md)
acceptance A1–A12. This file holds only open work.

## Round 3 — plan (WU1, WU2, WU3 closed: see [`loop.done.md`](loop.done.md))

| WU | Repo · branch · worktree | Scope | Owner | State |
|---|---|---|---|---|
| WU1b | zwallet · `feat/3-whitelist` (after the PR #1 merge) | the signed whitelist per [`../04-whitelist.md`](../04-whitelist.md) (W1–W12): sealed payload v2 (pub + gen); `whitelist.key` envelope; signed `whitelist.json`; daemon enforcement and `FEE_CAP_EXCEEDED`; the edit protocol (stop daemons → staged write → re-warm); CLI `whitelist add/remove/list/fee-cap/allow-message-signing/reset`; `@nick`; dry-run pre-check; registry tokens. Must merge **before the v0.1.0 tag and before WU6 funding** | opus-coder ×3 by file ownership: A core (`core/whitelist.rs`, sealed carrier in the existing `policy` object, key envelope, canonical bytes, edit primitives, token registry, test seeding helper) on `feat/3-whitelist`, then B enforcement (`daemon/`, `chains/`, error codes; fixes every existing test it breaks) on `feat/3b-whitelist-enforce` ‖ C CLI and docs (`commands.rs`, `cli.rs`, README, 04) on `feat/3c-whitelist-cli`; dispatcher merges B+C into `feat/3-whitelist` | planned. Registry addresses come only from issuer-published sources, cross-checked (librarian research, dispatcher spot-check) |
| WU4 | xbrew · branch per repo convention | `recipes/zwallet.toml` (macos formula + arch pinned binaries from the v0.1.0 release) | after v0.1.0 release | planned |
| WU5 | all | tag `v0.1.0` (the repo is already public), release green, render the stable formula by hand, `brew install 2lab-ai/tap/zwallet` + `brew test`, a preview at the same version, `xbrew install zwallet` (A10, A11). The preview channel is already live (see loop.done.md R3) | dispatcher | planned |
| WU6 | live | user runs create and unlock on fable-m5max, then the agent reads address and balance (A12) | user TTY + dispatcher | planned |

## Build facts

- Gate: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`. Dispatcher measured it on d1ef24d, 2026-10-20: rc=0, 77 lib + 25 `tests/cli.rs` (~10–14 s; scrypt log_n=18 ≈ 0.5 s per op) + 10 `tests/evm_offline.rs` (~10 s).
- CI (`ci.yml`, since PR #2) runs fmt, clippy `-D warnings --locked`, `cargo test --locked` and an exact version-line smoke on macos-latest and ubuntu-latest for every PR and main push; it is the merge gate together with the dispatcher's local rerun. Linux Docker `rust:1.97-bookworm` uid 1001 stays the local way to reproduce a Linux failure.
- Every main push publishes a `preview-*` prerelease (4 binaries + SHA256SUMS). The tap's `bump-zwallet-preview` job picks it up on its 6 h schedule (`23 */6 * * *`); render by hand when it cannot wait (the steps in `bump.yml`; note that the Bash tool runs zsh, where `BASH_REMATCH` is empty, so derive the version with `sed`).
- Open for WU1b:
  - The plaintext metadata (`active`, names, `nextIndex`) is still unauthenticated. While unlocked, address reads are cross-checked against the daemon; while locked they say `verified:false`.
  - "Policy viewable while locked": settled by 04-whitelist (`whitelist.json` is plaintext and signed; only pub + gen are sealed).
  - The calldata classifier lacks `increaseAllowance`, `setApprovalForAll` and Permit2.
  - An absolute fee limit belongs in the policy, because RPC-relative fee caps trust the RPC.
- c3 (concurrent unlocks) failed once in the custody coder's run (root cause not established). In the dispatcher's 5 repeats it passed 5/5. Watch for it in CI.
- Live tests: `cargo test -- --ignored` (public mainnet RPC). Unit tests never touch the real `~/.zwallet`. Daemon tests use a short `/tmp/zwt-*` home because of `sun_path`.

## Open gaps

| ID | Acceptance | Gap | Source | Status |
|---|---|---|---|---|
| G2 | A10 | preview channel live; stable needs the v0.1.0 release and its formula | WU5 | 🟡 |
| G3 | A11 | no xbrew recipe | WU4, WU5 | 🔴 |
| G4 | A12 | no real wallet; needs the user's password at a TTY | WU6 | 🚪 |

## Next

Dispatch WU1b part A (core) off main; when it is green, B and C in parallel; merge B+C, gate,
external review, merge. WU5 after WU1b.
