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
  - C CLI and docs: seven review rounds. The summary before the password shows the change, both files' old values, every outcome and what happens to signing; edits leave the wallet locked unless `--unlock`; an edit lands on the fingerprinted bytes shown or nothing changes; no edit rebuilds an unverifiable or unsealed list (only `reset`, which states its cost); promotion and publish write verified bytes from memory; the action is decided before the fee; the rollback diagnosis is gated on the highest generation the human remembers within one seal lineage.
- **Reviews**: gpt6 (gpt-6-astra) through round 2; from round 3 the llmux GPT engines refused ("not supported when using Codex with a ChatGPT account"), so opus (zworkflow:reviewer) and grok-4.7 reviewed, plus gpt-5.6-terra once. Final: code APPROVE (opus, terra), docs findings fixed in the last text pass (no logic change, dispatcher-verified diff).
- **Dispatcher gate** on every round head (62fa40c, f308bbc, 51ceac9, b64bf32, f5e7670) green, with a second cli + evm_offline run; on f0745b3: fmt, clippy `-D warnings`, 215 lib + 62 cli + 54 evm_offline + 4 whitelist_daemon + 3 whitelist_seed, live 10/10 (all 19 registry rows' `symbol()`/`decimals()`). Coders' Linux Docker uid 1001: 217 + 64 + 54 + 4 + 3.
- **Closed**: merged to main as PR #4; `.prd/04-whitelist.md` Status shipped.
