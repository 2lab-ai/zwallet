# v0.1 — loop

Status: **in-progress**, 2026-10-20. Target = [`ssot.md`](ssot.md), [`../01-spec.md`](../01-spec.md)
acceptance A1–A12. This file holds only open work.

## Round 1 — plan

| WU | Repo · branch · worktree | Scope | Owner | State |
|---|---|---|---|---|
| WU1 | zwallet · `feat/1-evm-wallet-core` · `.worktrees/feat-1-evm-wallet-core` | core (store, seed, amounts, durations, errors), EVM family, `call`/`invoke`/`transfer`/`balance`, unlock daemon, HD named keys + flat CLI, offline + `#[ignore]` live tests (A1–A9, A2b) | opus-coder | PR [#1](https://github.com/2lab-ai/zwallet/pull/1) (89ad1d7, 6cdd474, bf5c3eb). Dispatcher reran on 2026-10-20: gate rc=0 (52 lib + 10 CLI), live 6/6, manual end-to-end in a temp home. Review in flight: 3× gpt6 (daemon/protocol, store/seed, evm tx) + 2× opus (custody, EVM/CLI) |
| WU1b | zwallet · `feat/3-signing-policy` (after WU1 merge) | daemon-side signing policy per 02-architecture §Resistance: authenticated policy in the encrypted store, rolling-24h native cap, approve/permit deny unless the spender is allowlisted, per-token caps with unknown tokens denied, `zwallet policy` view (locked) and edit (password), defaults shown at `create` | opus-coder, brief written after WU1's gate rerun (protocol/store as landed) | planned |
| WU2 | zwallet · `feat/2-distribution` | `ci.yml`/`preview.yml`/`release.yml` from harn's, version line `zwallet <ver> (<channel> <rev12>)` (the preview formula test greps `preview`), MIT `LICENSE` file | after WU1 merge | planned |
| WU3 | homebrew-tap · `feat/zwallet-formulae` · `.worktrees/feat-zwallet-formulae` | `Formula/zwallet.rb.tmpl` and `zwallet-preview.rb.tmpl`, `bump-zwallet` and `bump-zwallet-preview` jobs, README rows (mirror of harn commit 2a3af89) | opus-coder | committed 5468a0a + ed9150e (desc ≤80). The diff is name-only against the harn jobs; tap `scripts/test-*.sh` pass 8/8; actionlint clean. **Hold the merge until zwallet is public (WU5)**: the preview job's `gh release list` has no `\|\| true`, so it goes red on a private repo |
| WU4 | xbrew · branch per repo convention | `recipes/zwallet.toml` (macos formula + arch pinned binaries from the v0.1.0 release) | after v0.1.0 release | planned |
| WU5 | all | make the repo public, tag `v0.1.0`, release green, render the tap by hand, run `brew install` for stable and preview, `xbrew install zwallet` (A10, A11) | dispatcher | planned |
| WU6 | live | user runs create and unlock on fable-m5max, then the agent reads address and balance (A12) | user TTY + dispatcher | planned |

## Build facts

- Gate: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`. Dispatcher measured it on 2026-10-20: rc=0, 52 lib + 10 integration tests (`tests/cli.rs`, ~4 s).
- Open from WU1's hand-back, for WU1b: the plaintext metadata (`active`, names, `nextIndex`) is not authenticated, so a hand-edited `active` goes undetected (A9b says it must). "Policy viewable while locked" conflicts with the policy living inside the seal, so the view needs the password or the daemon. The calldata classifier lacks `increaseAllowance`, `setApprovalForAll` and Permit2.
- Live tests: `cargo test -- --ignored` (public mainnet RPC). Unit tests never touch the real `~/.zwallet`. Daemon tests use a short `/tmp/zwt-*` home because of `sun_path`.

## Open gaps

| ID | Acceptance | Gap | Source |
|---|---|---|---|
| G1 | A1–A9 | no code merged yet | WU1 |
| G2 | A10 | no workflows, no tap formula | WU2, WU3, WU5 |
| G3 | A11 | no xbrew recipe | WU4, WU5 |
| G4 | A12 | no real wallet; needs the user's password at a TTY | WU6 |

## Next

Wait for the WU1 hand-back, then rerun the gate and the live tests in the worktree, then send it
for external review, merge, and start WU2. WU3 can start now; its bump jobs skip until a release
exists.
