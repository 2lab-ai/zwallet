# v0.1 — loop

Status: **in-progress**, 2026-10-21. Target = [`ssot.md`](ssot.md), [`../01-spec.md`](../01-spec.md)
acceptance A1–A12. This file holds only open work.

## Round 5 — plan (WU1, WU2, WU3, WU1b closed: see [`loop.done.md`](loop.done.md))

| WU | Repo · branch · worktree | Scope | Owner | State |
|---|---|---|---|---|
| WU4 | xbrew · branch per repo convention | `recipes/zwallet.toml` (macos formula + arch pinned binaries from the v0.1.0 release) | after v0.1.0 release | planned |
| WU5 | all | tag `v0.1.0` (the repo is already public), release green, render the stable formula by hand, `brew install 2lab-ai/tap/zwallet` + `brew test`, a preview at the same version, `xbrew install zwallet` (A10, A11). The preview channel is already live (see loop.done.md R3) | dispatcher | planned |
| WU6 | live | user runs create and unlock on fable-m5max, then the agent reads address and balance (A12) | user TTY + dispatcher | planned |

## Build facts

- Gate: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`. Dispatcher measured it on d1ef24d, 2026-10-20: rc=0, 77 lib + 25 `tests/cli.rs` (~10–14 s; scrypt log_n=18 ≈ 0.5 s per op) + 10 `tests/evm_offline.rs` (~10 s).
- CI (`ci.yml`, since PR #2) runs fmt, clippy `-D warnings --locked`, `cargo test --locked` and an exact version-line smoke on macos-latest and ubuntu-latest for every PR and main push; it is the merge gate together with the dispatcher's local rerun. Linux Docker `rust:1.97-bookworm` uid 1001 stays the local way to reproduce a Linux failure.
- Every main push publishes a `preview-*` prerelease (4 binaries + SHA256SUMS). The tap's `bump-zwallet-preview` job picks it up on its 6 h schedule (`23 */6 * * *`); render by hand when it cannot wait (the steps in `bump.yml`; note that the Bash tool runs zsh, where `BASH_REMATCH` is empty, so derive the version with `sed`).
- Still open after WU1b: the plaintext metadata (`active`, names, `nextIndex`) is unauthenticated. While unlocked, address reads are cross-checked against the daemon; while locked they say `verified:false`. (Settled by WU1b: the policy is viewable while locked; `increaseAllowance`, `setApprovalForAll` and Permit2 are refused unless a human lists that contract and selector; fee caps are absolute, in the signed list.)
- Gate since WU1b (dispatcher, f0745b3, 2026-10-21): 215 lib + 62 cli + 54 evm_offline + 4 whitelist_daemon + 3 whitelist_seed, live 10/10. The cli suite takes 3–5 min under load (scrypt per edit).
- llmux GPT engines (gpt-6-astra, gpt-5.6-sol) refuse "with a ChatGPT account" since 2026-10-21; reviews use opus (zworkflow:reviewer) and grok-4.7 until they return (zbrain open-loops).
- c3 (concurrent unlocks) failed once in the custody coder's run (root cause not established). In the dispatcher's 5 repeats it passed 5/5. Watch for it in CI.
- Live tests: `cargo test -- --ignored` (public mainnet RPC). Unit tests never touch the real `~/.zwallet`. Daemon tests use a short `/tmp/zwt-*` home because of `sun_path`.

## Open gaps

| ID | Acceptance | Gap | Source | Status |
|---|---|---|---|---|
| G2 | A10 | preview channel live; stable needs the v0.1.0 release and its formula | WU5 | 🟡 |
| G3 | A11 | no xbrew recipe | WU4, WU5 | 🔴 |
| G4 | A12 | no real wallet; needs the user's password at a TTY | WU6 | 🚪 |

## Next

WU5: tag `v0.1.0` (the user's release request, 2026-10-20), release green, stable formula,
`brew install` + `brew test`, then a preview at the same version. Then WU4 (xbrew) and WU6 (the
user creates the wallet at a TTY, `backup`, first `whitelist add`; the agent reports the address).
