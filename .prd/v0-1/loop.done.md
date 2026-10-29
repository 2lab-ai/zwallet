# v0.1 — loop (done)

Append-only ledger of closed rounds, WUs and gap rows moved out of [`loop.md`](loop.md).

## R1–R6 — closed (WU1, PR #1)

WU1 (EVM wallet core) went through six external review rounds and merged as PR #1; G1 (A1–A9) closed with it.

### Closed rows

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
- **Review round 4** (gpt6 + opus, split by area, on the round-3 delta): send and custody CHANGES_REQUESTED.
- **Fixed:** custody 73d7a20 (lock reads no home; status lists other daemons), 2de4df6 (a `/proc` listing error fails closed), 80bc94c (hidepid hosts refused); send 5f00028 (journal tail repair, duplicate key on base units), cba257a (pending reported bounded), 5ade2b9 (`zwallet tx` lookup), merged at 88c34bc.
- **Dispatcher rerun** on 80bc94c: gate rc=0 (126 lib + 47 CLI + 31 evm_offline), cli+evm_offline 3/3 repeats, live 9/9; Linux Docker uid 1001: 127 lib + 49 CLI + 31 evm_offline.
- **Review round 5** (gpt6 ×2 on the round-4 delta): send CHANGES_REQUESTED (S-A a malformed receipt status read as `success:false`; S-B "went through" asserted from mining alone); custody conditional (C-A a denied `/proc` read of an own-uid process was skipped).
- **Fixed:** c3e3ed0 (S-A, S-B: outcome only from status `0x0`/`0x1`, `--wait` gains `mined` + `success`, mined-nonce text points at the receipt), 2438a5c (C-A: `kill(pid,0)` tells own from foreign, own + denied fails closed), bec63f2 (blockNumber without receipt = outcome unknown). Linux Docker: 130 lib + 49 CLI + 35 evm_offline.
- **Review round 6** (gpt6 ×2, confirmation of the round-5 delta): send APPROVE, custody APPROVE, no blocking item. Non-blocking notes fixed: 75ffaa1 (tests: a mismatched or unreadable receipt never reports an outcome, mutation-checked), 8f6a37f (README: `minedTxHash`/`success` only when the receipt can be read), 75f5528 (custody wording: `kill(pid,0)` proves signal permission, not ownership).
- **Dispatcher rerun** on bec63f2: gate rc=0 (128 lib + 47 CLI + 35 evm_offline), cli+evm_offline 3/3 repeats, live 9/9; on 75f5528: gate rc=0 (128 lib + 47 CLI + 38 evm_offline). Linux Docker uid 1001 on 75f5528 (coder): 130 lib + 49 CLI + 38 evm_offline.
- **Closed:** merged to main as PR #1. |

| ID | Acceptance | Gap | Source | Status |
|---|---|---|---|---|
| G1 | A1–A9 | closed: code merged as PR #1 (offline tests + `#[ignore]` live tests cover A1–A9, A2b) | WU1 | ✅ |

## R3 — closed (WU2 distribution, WU3 tap)

Distribution and the tap landed; the preview channel installs through Homebrew end to end.

### Closed rows

| WU | Repo · branch · worktree | Scope | Owner | State |
|---|---|---|---|---|
| WU2 | zwallet · `feat/2-distribution` · `.worktrees/feat-2-distribution` | `ci.yml`/`preview.yml`/`release.yml` from harn's, version line `zwallet <ver> (<channel> <rev12>)` (the preview formula test greps `preview`), MIT `LICENSE` file | opus-coder; dispatcher rebased onto main 2026-10-20 (dropped 6abd466: main's process-table lookup already finds the Linux daemon) | in review: PR open, CI is the merge gate |
| WU3 | homebrew-tap · `feat/zwallet-formulae` · `.worktrees/feat-zwallet-formulae` | `Formula/zwallet.rb.tmpl` and `zwallet-preview.rb.tmpl`, `bump-zwallet` and `bump-zwallet-preview` jobs, README rows (mirror of harn commit 2a3af89) | opus-coder | committed 5468a0a + ed9150e (desc ≤80). The diff is name-only against the harn jobs; tap `scripts/test-*.sh` pass 8/8; actionlint clean. The hold "until zwallet is public" is void: zwallet has been public since it was created (GitHub PublicEvent 2026-10-20T04:04:01Z, same second as creation). Merge after WU2 so the preview job has releases to read |

- **WU2** merged as PR #2 (814c4ae).
  - It was rebased onto main, and 6abd466 was dropped: main's process-table lookup already finds the Linux daemon.
  - External review (gpt6) APPROVE with no blockers. Its two notes were fixed before the merge:
    - every action is pinned to a full commit sha, on Node 24 majors (checkout v7.0.1, upload-artifact v7.0.2, download-artifact v8.0.2, action-gh-release v3.0.3, rust-cache v2.9.2, rust-toolchain stable); the dispatcher re-resolved every sha;
    - an exact version-line smoke runs in ci/preview/release.
  - CI passed on macos-latest and ubuntu-latest: the first hosted Linux build.
- **First preview:** `preview-2026-10-20-1315-814c4aed0978`. All 4 builds and publish succeeded; the assets match `SHA256SUMS`; the macOS arm64 binary prints `zwallet 0.1.0 (preview 814c4aed0978)`.
- **WU3** merged as tap PR #8.
  - It was rebased onto the tap master; the tap tests passed 8/8 and actionlint was clean.
  - External review (gpt6) APPROVE. Its note on formula tests was fixed: they now assert the channel, `zwallet <version> (release ` and `(preview `.
  - Its other note, the push race between sibling bump jobs, is tap-wide and predates this PR.
  - The first preview formula was rendered by hand (tap PR #9).
- **Receipt** on fable-m5max: `brew install 2lab-ai/tap/zwallet-preview` installed 2026.10.20.1315; `zwallet --version` printed `zwallet 0.1.0 (preview 814c4aed0978)`; `brew test` returned rc=0.

## R4 — closed (WU1b signed whitelist)

The daemon signs only what a human-signed whitelist allows; edits land on exactly the list shown or nothing changes. Merged as PR #4.

### Closed rows

| WU | Repo · branch · worktree | Scope | Owner | State |
|---|---|---|---|---|
| WU1b | zwallet · `feat/3-whitelist` (after the PR #1 merge) | the signed whitelist per [`../04-whitelist.md`](../04-whitelist.md) (W1–W12): sealed payload v2 (pub + gen); `whitelist.key` envelope; signed `whitelist.json`; daemon enforcement and `FEE_CAP_EXCEEDED`; the edit protocol (stop daemons → staged write → re-warm); CLI `whitelist add/remove/list/fee-cap/allow-message-signing/reset`; `@nick`; dry-run pre-check; registry tokens. Must merge **before the v0.1.0 tag and before WU6 funding** | opus-coder ×3 by file ownership: A core (`core/whitelist.rs`, sealed carrier in the existing `policy` object, key envelope, canonical bytes, edit primitives, token registry, test seeding helper) on `feat/3-whitelist`, then B enforcement (`daemon/`, `chains/`, error codes; fixes every existing test it breaks) on `feat/3b-whitelist-enforce` ‖ C CLI and docs (`commands.rs`, `cli.rs`, README, 04) on `feat/3c-whitelist-cli`; dispatcher merges B+C into `feat/3-whitelist` | planned. Registry addresses come only from issuer-published sources, cross-checked (librarian research, dispatcher spot-check) |

- **Build** (2026-10-20 → 21), three parts by file ownership, merged into `feat/3-whitelist`:
  - A core (`core/whitelist.rs`, sealed anchor, key envelope, edit primitives, 19-row token registry): reviewed in two rounds (M1 lost list after two interrupted edits, M2 reseal KDF downgrade), merged 97e0e92.
  - B enforcement (`chains/evm/policy.rs`, daemon): three review rounds. Fixed a self-transfer fee-cap bypass through a corrupted list (an untrusted list now signs nothing), list reads outside the session lock with a 2 s bound and at most 4 readers, and the typed anchor.
  - C CLI and docs: seven review rounds. The summary before the password shows the change, both files' old values, every outcome and what happens to signing; edits leave the wallet locked unless `--unlock`; an edit lands on the fingerprinted bytes shown or nothing changes; no edit rebuilds an unverifiable or unsealed list (only `reset`, which states its cost); promotion and publish write verified bytes from memory; the action is decided before the fee; zwallet never suggests restoring an older whitelist.json (round 8: the remedy was deleted).
- **Reviews**: gpt6 (gpt-6-astra) through round 2; from round 3 the llmux GPT engines refused ("not supported when using Codex with a ChatGPT account"), so opus (zworkflow:reviewer) and grok-4.7 reviewed, plus gpt-5.6-terra once. Final: code APPROVE (opus, terra), docs findings fixed in the last text pass (no logic change, dispatcher-verified diff).
- **Dispatcher gate** on every round head (62fa40c, f308bbc, 51ceac9, b64bf32, f5e7670) green, with a second cli + evm_offline run; on f0745b3: fmt, clippy `-D warnings`, 215 lib + 62 cli + 54 evm_offline + 4 whitelist_daemon + 3 whitelist_seed, live 10/10 (all 19 registry rows' `symbol()`/`decimals()`). Coders' Linux Docker uid 1001: 217 + 64 + 54 + 4 + 3.
- **Closed**: merged to main as PR #4; `.prd/04-whitelist.md` Status shipped.

## R5 — closed (WU5 release, WU4 xbrew recipe)

zwallet v0.1.0 is released; brew stable and preview install it; `xbrew install zwallet` installs it on macOS and Arch from the xbrew preview build.

### Closed rows

| WU | Repo · branch · worktree | Scope | Owner | State |
|---|---|---|---|---|
| WU5 | all | tag `v0.1.0` (the repo is already public), release green, render the stable formula by hand, `brew install 2lab-ai/tap/zwallet` + `brew test`, a preview at the same version, `xbrew install zwallet` (A10, A11). The preview channel is already live (see loop.done.md R3) | dispatcher | planned |
| WU4 | xbrew · branch per repo convention | `recipes/zwallet.toml` (macos formula + arch pinned binaries from the v0.1.0 release) | after v0.1.0 release | planned |

| ID | Acceptance | Gap | Source | Status |
|---|---|---|---|---|
| G2 | A10 | closed: v0.1.0 stable formula and the preview at the same commit installed and tested | WU5 | ✅ |

- **WU5** (2026-10-21): tag `v0.1.0` on e17b2be (PR #4's merge; the user's request 2026-10-20 "brew에도 올리고 xbrew에도 올려줘"). Release, CI and Preview runs on e17b2be all green; the release's 4 assets match SHA256SUMS; the preview `preview-2026-10-20-2311-e17b2bebb0c8` is the same commit. Tap PR #10 (4682f6b) rendered `Formula/zwallet.rb` 0.1.0 and `zwallet-preview.rb` 2026.10.20.2311 by hand. On fable-m5max: `brew install 2lab-ai/tap/zwallet` → `zwallet 0.1.0 (release e17b2bebb0c8)`, `brew test` rc=0; `brew upgrade 2lab-ai/tap/zwallet-preview` → `zwallet 0.1.0 (preview e17b2bebb0c8)`, `brew test` rc=0; `bin/zwallet` links the preview.
- **WU4** (2026-10-21): xbrew PR #4 (5978635) adds `recipes/zwallet.toml` (macOS `formula = "2lab-ai/tap/zwallet"`; Arch `[arch.binary]` pinned to the v0.1.0 linux assets) and `tests/zwallet_recipe_contract.rs` (RED 0/3 → GREEN 3/3; `just check` 76 tests; CI macOS + Ubuntu green; opus review APPROVE). With the xbrew preview build `preview-2026-10-20-2341-5978635097d6` (assets match SHA256SUMS): on fable-m5max, `xbrew uninstall zwallet` then `xbrew install zwallet` → brew formula, `zwallet 0.1.0 (release e17b2bebb0c8)`, tracked; on cachy-z13 (CachyOS x86_64), `xbrew install zwallet` → recipe-binary at `~/.xbrew/bin/zwallet`, `zwallet 0.1.0 (release e17b2bebb0c8)`, sha256 `131109e6…` = the pin. Stable xbrew (0.4.11) does not carry the recipe yet: G3 stays open on the owner's xbrew release.

## R6 — closed (hardening after v0.1.0, PRs #6–#12)

Seven PRs after the v0.1.0 release. The whitelist key comes from the password, a verified address shows a QR code, the session verifies addresses after unlock, and text from outside cannot make zwallet's output show a fundable address. Each code PR was followed by a preview the dispatcher installed and tested on fable-m5max.

### Closed rows

| WU | Repo · branch · worktree | Scope | Owner | State |
|---|---|---|---|---|
| H1 | zwallet · PR #6 | README: zwallet has no relation to twak | dispatcher | merged e5ae1d2 (2026-10-21) |
| H2 | zwallet · PR #7 | `balance` without `--chain` reads every network | opus-coder | merged 436d5dc (2026-10-21); preview 2026.10.21.0656 (tap PR #11) |
| H3 | zwallet · PR #8 | whitelist key derived from the password and a sealed salt; `whitelist.key` removed | opus-coder | merged fe03605 (2026-10-23) |
| H4 | zwallet · PR #9 | a verified address prints as a QR code on a terminal | opus-coder | merged 4f3b484 (2026-10-23); preview 2026.10.23.0722 (tap PR #12) |
| H5 | zwallet · PR #10 | after unlock, address views and `balance` are verified by the session (macOS peer cdhash) | opus-coder | merged 787a640 (2026-10-26); preview 2026.10.26.1119 (tap PR #13) |
| H6 | zwallet · PR #11 | display integrity: one write-point filter, parse-time shape checks, a total budget per outside source | opus-coder | merged f4c88b3 (2026-10-27); preview 2026.10.27.0200 (tap PR #14) |
| H7 | zwallet · PR #12 · `.worktrees/fix-display-budget-nits` | the display budgets left by #11's final review (N1–N5) and by its own review (B1, NB1–NB6) | opus-coder | merged with this ledger; its preview receipt goes in the next round |

- **PR #7.** RED first: lib 216 → 220, evm_offline 54 → 59, live 10 → 11.
- **PR #8.** scrypt over the password and a sealed salt (domain `zwallet/whitelist-key/v1\n`). A v0.1.0 anchor without a salt is refused, and the daemon signs nothing under it. Gate: lib 225, cli 63, evm_offline 60, whitelist 4+3, live 11/11.
- **PR #9.** The QR code is shown only for a verified address (`unlock`, `create` of a further key, `address --verify`) and only on a terminal. Gate on 9cb9b9f: lib 224, cli 65 (65/65 on a pty), evm_offline 60, whitelist 4+3; live 11/11.
- **PR #10.** The first version compared the executable by (dev, ino) plus `LOCAL_PEERUUID` and was bypassable (attacker code exec'd from a path later swapped for a symlink to zwallet, with a forged LC_UUID; test s7, RED at 3005dba). The peer is now identified by its kernel-validated cdhash (`LOCAL_PEERTOKEN` → `CS_OPS_CDHASH`) plus CS_VALID. Linux never session-verifies. Gate on f1d3c62: lib 233, cli 75 (75/75 on a pty), evm_offline 62, whitelist 4+3, live 11/11; Linux Docker lib 234, cli 76. Reviews: opus APPROVE, grok SAFE on the peer check.
- **PR #11.** Three rounds of code-point blacklists each missed glyphs that can draw a QR code, so the escaping became an allowlist (printable ASCII and `₮`) applied at the write point. whitelist.json is shape-checked when parsed, and every outside source has one 512-character budget. Gate on 0e6fb86: lib 258, cli 108 (×3 on a pty), evm_offline 64, whitelist 5+3, live 11/11; Linux Docker lib 258, cli 109. Review: opus 8 rounds, then APPROVE; grok SAFE on the write point and the shape check.
- **PR #12.** Gate on 9df65bb: lib 262, cli 108 (108/108 ×3 on a pty), evm_offline 65, whitelist 5+3, live 11/11; Linux Docker (rust 1.99.1, non-root) lib 262, cli 109, evm_offline 65, whitelist 5+3. Review: trinity (grok-4.7, gpt-6-astra, fable) R1 2 APPROVE + 1 REJECT on one test assertion that held only by where `brief()` cuts; fixed in 9df65bb; R2 unanimous APPROVE.

## R7 — closed (hardening after PR #12: PRs #13–#15)

The test suite stops no daemon outside its own test homes; the v0.2 TUI spec is re-anchored to today's code; `list` names an impostor whose answers match the cache, `unlock` checks the daemon it starts, and every macOS binary zwallet ships is signed with the hardened runtime.

### Closed rows

| WU | Repo · branch · worktree | Scope | Owner | State |
|---|---|---|---|---|
| H8 | zwallet · PR #13 | `cargo test` stops no zwallet daemon outside its own test homes (debug-only `ZWALLET_TEST_DAEMON_SCOPE`; a release-profile test run refuses to start) | opus-coder | merged 8ae9299 (2026-10-27); no user-facing change, so the tap was not bumped by hand |
| H9 | zwallet · PR #14 | v0.2 TUI spec refreshed against PRs #8–#12: `03-tui.md`, `v0-2-tui/flow.md`, `units.md`, `loop.md` | senior-software-engineer | merged 0ebe12c (2026-10-27) |
| H10 | zwallet · PR #15 | N5: `list` names a process on `d.sock` that is not this build's key daemon, also when its answers match the cache. N6: `unlock` checks the started daemon's code before the password (partial hardening). Every shipped macOS binary ad-hoc signed with the hardened runtime; an unsigned build refuses `unlock` | opus-coder | merged with this ledger; its preview receipt goes in the next round |

- **PR #13.** RED: `the_suite_stops_no_daemon_outside_its_test_homes` (lock and whitelist add both stopped a `/tmp/zwx-` daemon before the fix); mutation (the filter line removed) fails it; `a_release_profile_run_starts_no_zwallet` goes through the launcher. Gate on b64e2cd: lib 263, cli 110 (110/110 ×3 on a pty), evm_offline 65, whitelist 5+3, live 11/11; Linux Docker lib 263, cli 111. Reviews: gpt-6 evidence review and code-reviewer, round 1 both REJECT (a `cargo test --release` run was still destructive), round 2 both APPROVE. CI green on 403e15f (test-only delta after the gate).
- **PR #14.** Trinity (grok-4.7, gpt-6-astra, fable): R1 0/3 approve (payee address not authenticated, confirm not bound to the reviewed transaction, contract-routed token movements, no runtime write point), R2 2:1 (a test contract a correct backend could not pass), R3 unanimous APPROVE. `loop-lint` 0 FAIL.
- **PR #15.** RED: `list_names_a_listener_whose_answers_match_an_edited_cache`; `a_started_daemon_that_runs_other_code_is_given_nothing` (the real `drive_daemon` with `/bin/sh` as the child read the 22 password bytes); `the_started_code_verdict_refuses_when_this_build_has_no_cdhash`; `the_started_daemon_refusal_claims_no_lock_state`. Mutations: the cdhash comparison, `CS_VALID`, and accept-without-own-cdhash each fail a test. Gate on b1626cc: lib 269, cli 111 (111/111 ×3 on a pty), evm_offline 65, whitelist 5+3, live 11/11; Linux Docker lib 263, cli 112. The whole suite for `x86_64-apple-darwin` under Rosetta on 4ef17b3 (rustc 1.96.1, the rustup toolchain): lib 266, cli 111, evm_offline 65, whitelist 5+3. The round-2 delta (fde617f, 4ef17b3: messages, a quoting helper with its test, the CI control step, docs) was checked by targeted tests, a mutation and CI on both OS. Reviews: gpt-6 evidence review REJECT (fail-open without an own cdhash; N6 overclaimed) then APPROVE; code-reviewer REJECT (refusal message claimed a lock state; then a README sentence) then APPROVE.

## R8 — closed (WU6: the owner's unlock and the agent's read, A12)

The agent reads the owner's wallet without a password once the owner has unlocked it: the address comes
back verified by the session, and the balance of every network reads.

### Closed rows

| WU | Repo · branch · worktree | Scope | Owner | State |
|---|---|---|---|---|
| WU6 | live | user runs create and unlock on fable-m5max, then the agent reads address and balance (A12) | user TTY + dispatcher | done 2026-10-28: the owner unlocked at 12:01:03 KST (`unlock 1y`, preview 2026.10.28.0227); the dispatcher read `address`, `status` and `balance` (read-only, no password) |

| ID | Acceptance | Gap | Source | Status |
|---|---|---|---|---|
| G4 | A12 | the wallet exists; the agent's read of the session-verified address and the balance waits for the user's unlock at a TTY | WU6 | ✅ 2026-10-28 |

- **A12, 2026-10-28 12:02 KST**, with the preview 2026.10.28.0227 (24f4b5b), after the owner's `zwallet unlock 1y`
  at a TTY (the daemon started 12:01:03 KST):
  - `zwallet address --json`: key `zbrain`, the address in its cache (the owner's; not written in this public repo), `verified: true`,
    `verifiedBy: "session"`, no warning.
  - `zwallet status --json`: `locked: false`, `expiresAt` 2027-10-28T03:01:04Z, `verifiedBy: "session"`.
  - `zwallet balance --json`: 0 native and 0 of every registry token on ethereum, base, arbitrum, optimism,
    polygon, bsc, avalanche and sepolia; no network errored.
  - `zwallet tui` on the real home (read-only, `q`): the key, its shortened address and `verified by session`,
    `signing (agent too) until 2027-10-28 03:01Z`.
