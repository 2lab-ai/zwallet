# v0.1 — loop

Status: **in-progress**, 2026-10-20. Target = [`ssot.md`](ssot.md), [`../01-spec.md`](../01-spec.md)
acceptance A1–A12. This file holds only open work.

## Round 1 — plan

| WU | Repo · branch · worktree | Scope | Owner | State |
|---|---|---|---|---|
| WU1 | zwallet · `feat/1-evm-wallet-core` · `.worktrees/feat-1-evm-wallet-core` | core (store, seed, amounts, durations, errors), EVM family, `call`/`invoke`/`transfer`/`balance`, unlock daemon, HD named keys + flat CLI, offline + `#[ignore]` live tests (A1–A9, A2b) | opus-coder | PR [#1](https://github.com/2lab-ai/zwallet/pull/1).
- **Review round 1** (3× gpt6 + 2× opus): all CHANGES_REQUESTED. Critical: a same-user fake daemon captured the unlock password.
- **Fixed:** custody C1–C14 (b210eb9) and EVM E1–E12 (e581711, b733d25), merged at d1ef24d. RED was reproduced first for C1/C4/C5/C6/C7abc/C8–C11/C13 and E1–E10. C3 and C7(d) did not reproduce.
- **Dispatcher rerun** on the merged tree, 2026-10-20: gate rc=0 (77 lib + 25 CLI + 10 evm_offline), CLI suite 5/5 repeats green, live 7/7.
- **Follow-ups F1–F4:** c4d0575, 3d1e2a9.
- **Review round 2** (4× gpt6 + 2× opus): all CHANGES_REQUESTED. Main items: `verified:true` was forgeable; `lock` could be defeated by a socket-identity race; retry messages nudged a double send; the legacy fee ceiling was vacuous; ERC-721 selector collisions.
- **Fixed:** custody K1–K12 (2597115) and EVM V1–V11 (fix/2-evm, merged at 5b98d8d), then P1 fail-fast send lock (fba95ef), P2 index < 2^31 (39daf9f), P3 README (53fa390). V4 (the `uint` alias) did not reproduce on alloy 1.7.3 and is pinned by a test.
- **Dispatcher rerun** on 5b98d8d: gate rc=0 (99 lib + 37 CLI + 16 evm_offline), CLI 5/5 repeats, live 8/8.
- **Review round 3** (4× gpt6 + 2× opus, on the round-2 delta): tx/abi and store were conditional-approve. Blockers:
  - Linux pipe reopen via `/proc`, which exposed the password and allowed a fake `verified` readiness line;
  - the nonce-taken message pointed at the refused hash and said "next nonce";
  - no record of a broadcast if the process died after it;
  - the serde error leaked an escaped key echo.
- **The macOS exe check→exec race** was judged the same class as the documented binary-swap limit and documented, not fixed (no `fexecve`).
- **Fixed:** custody R1–R8 (fcf9e66 … a90b4f8) and send S1–S10 (fix/3-evm, merged at 6a1f934).
  - Linux tests ran in Docker `rust:1.97-bookworm` as uid 1001: 108 lib + 46 CLI + 16 evm_offline.
  - Only the Linux run covers the `cfg(linux)` code with clippy.
- **Dispatcher rerun** on 6a1f934: gate rc=0 (114 lib + 44 CLI + 24 evm_offline), cli+evm_offline 3/3 repeats, live 8/8.
- **Next:** review round 4 on the round-3 delta. |
| WU1b | zwallet · `feat/3-signing-policy` (after WU1 merge) | daemon-side signing policy per 02-architecture §Resistance: authenticated policy in the encrypted store, rolling-24h native cap, approve/permit deny unless the spender is allowlisted, per-token caps with unknown tokens denied, `zwallet policy` view (locked) and edit (password), defaults shown at `create` | opus-coder, brief written after WU1's gate rerun (protocol/store as landed) | planned |
| WU2 | zwallet · `feat/2-distribution` | `ci.yml`/`preview.yml`/`release.yml` from harn's, version line `zwallet <ver> (<channel> <rev12>)` (the preview formula test greps `preview`), MIT `LICENSE` file | after WU1 merge | planned |
| WU3 | homebrew-tap · `feat/zwallet-formulae` · `.worktrees/feat-zwallet-formulae` | `Formula/zwallet.rb.tmpl` and `zwallet-preview.rb.tmpl`, `bump-zwallet` and `bump-zwallet-preview` jobs, README rows (mirror of harn commit 2a3af89) | opus-coder | committed 5468a0a + ed9150e (desc ≤80). The diff is name-only against the harn jobs; tap `scripts/test-*.sh` pass 8/8; actionlint clean. **Hold the merge until zwallet is public (WU5)**: the preview job's `gh release list` has no `\|\| true`, so it goes red on a private repo |
| WU4 | xbrew · branch per repo convention | `recipes/zwallet.toml` (macos formula + arch pinned binaries from the v0.1.0 release) | after v0.1.0 release | planned |
| WU5 | all | make the repo public, tag `v0.1.0`, release green, render the tap by hand, run `brew install` for stable and preview, `xbrew install zwallet` (A10, A11) | dispatcher | planned |
| WU6 | live | user runs create and unlock on fable-m5max, then the agent reads address and balance (A12) | user TTY + dispatcher | planned |

## Build facts

- Gate: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`. Dispatcher measured it on d1ef24d, 2026-10-20: rc=0, 77 lib + 25 `tests/cli.rs` (~10–14 s; scrypt log_n=18 ≈ 0.5 s per op) + 10 `tests/evm_offline.rs` (~10 s).
- Linux-only code (`prctl`, the `/proc` scan in tests) has not been compiled yet. CI on ubuntu (WU2) is its first build.
- Open for WU1b:
  - The plaintext metadata (`active`, names, `nextIndex`) is still unauthenticated. While unlocked, address reads are cross-checked against the daemon; while locked they say `verified:false`.
  - "Policy viewable while locked" conflicts with the policy living inside the seal, so the view needs the password or the daemon.
  - The calldata classifier lacks `increaseAllowance`, `setApprovalForAll` and Permit2.
  - An absolute fee limit belongs in the policy, because RPC-relative fee caps trust the RPC.
- c3 (concurrent unlocks) failed once in the custody coder's run (root cause not established). In the dispatcher's 5 repeats it passed 5/5. Watch for it in CI.
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
