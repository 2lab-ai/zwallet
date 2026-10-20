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
