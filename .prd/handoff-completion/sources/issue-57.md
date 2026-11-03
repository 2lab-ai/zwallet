# GitHub issue #57 source snapshot

Source: https://github.com/2lab-ai/zwallet/issues/57
Fetched: 2026-10-30
State at fetch: open
Title: Handoff 2026-10-30: #52 fixed (#54 → 6f1a2c9); #55 in draft PR #56; real-chain Tron unverified

The source body below is preserved verbatim. It records the handoff author's
observations and requests at that time; current execution status and authorization
are recorded in the driver and completion checklist.

---

# zwallet handoff — 2026-10-30 (work stopped at the owner's request)

This continues [#50](https://github.com/2lab-ai/zwallet/issues/50). The goal is unchanged:
- every CLI function usable in the TUI;
- a Tron address from the same seed;
- TRX and TRC20 USDT balances and sends.

That shipped to preview `2026.10.29.2315` (`d6fb99d`): PR #51 + #53, [completion checklist](https://github.com/2lab-ai/zwallet/blob/main/.prd/handoff-completion/completion-checklist.md). This session fixed #52 and found #55. This issue holds only what `.prd/`, the PRs and the issues do not.

## 자신 없는 곳 & 사각지대

### 1. Least confident, and how to close it

1. **Tron and USDT have never touched a real chain.**
   - Every Tron receipt so far ran against loopback mocks: unit tests, pty tests, and the real-terminal QA on the installed preview. That includes the TRX send (1,500,000 sun, fee limit 0) and the USDT send (2.5 USDT, `feeLimit` 7,800,000 sun).
   - The 7.8 TRX `feeLimit` in QA came from the mock. In code, a USDT `feeLimit` is derived live:
     - it uses the node's simulated `energy_used` for this exact transfer, times `getEnergyFee` from `getchainparameters`, times 1.2;
     - the floor is 1 TRX;
     - it is refused above `getMaxFeeLimit` (`src/chains/tron/send.rs:348-360`).

     The simulation runs the same transfer, so a recipient with no USDT yet should show its higher energy cost.
   - **Unverified:** a real public node's replies (field shapes, rate limits without an API key, energy price moving between the estimate and the block) have never been seen.
   - **To close it:** at TR23, read the review screen's fee before signing, and send the smallest real amount first.
2. **The tr4 failure on Ubuntu (#55) is attributed to the #52 mechanism by inference, not by a repro.**
   - The printed address equals `TRON_POISON`, which `home.edit_wallet` plants right after `home.unlock("10m")`, and `verified: false` means no session answered. That fits a session ended by a torn in-place write, which #54 removed.
   - **To close it:** run `tr4_a_tampered_tron_cache_is_refused_and_shown_escaped` ×30 under Linux load at `6f1a2c9` or later. If it ever fails again, reapply the daemon diagnostics from `b96cc01` (branch history of PR #54) locally to read the daemon's own end reason.
3. **#52's a_cache half was never reproduced as a single edit.**
   - #54 closes the one mechanism found: an in-place write giving the daemon a torn file. It was shown by a churn probe.
   - If a `locked: true` flake recurs after `6f1a2c9`, the cause is something else, possibly the 2 s inspection deadline.
   - **To close it:** same as above. Reapply `b96cc01` locally. Do not merge it: the daemon-writes-no-file rule (CLAUDE.md:23) is the owner's to change.
4. **The #55 fix (draft PR, see State) did not finish its gates** before the stop. Its stop note in the PR body says what ran.

### 2. What we are missing

**(a) What the owner may not have seen:**
- **Each new preview means unlocking again.** A new build has a new cdhash.
  - `preview-2026-10-30-0233-6f1a2c91ab3a` was published automatically after #54. Its product code is identical to `2026.10.29.2315`, because #54 changed tests only.
  - The tap was **not** bumped by hand. The tap's scheduled bump will still offer it.
  - Upgrading to it gains nothing but a forced re-unlock. Skip it until a preview with product changes.
- **No acceptance on the real wallet yet.** After `zwallet unlock 1y` on the installed preview, the read-only checks are still pending: `zwallet address --chain tron`, `zwallet balance --chain tron`, and `zwallet tui` → Tron. They are the cheapest real-world proof of the goal.

**(b) What the dispatcher only just noticed:**
- **Main's CI after a merge was not checked.** Main CI at `d6fb99d` failed on both runners (run 40971807359), and the preview was still declared shipped. The Preview workflow runs its own validation, which happened to pass, so a red main can still publish a preview.
  - **Rule from now on:** after every merge, read `gh run list --branch main --workflow CI --limit 1` before calling it shipped.
- **`.prd/` still lists #52 as open**, but #52 is closed:
  - `.prd/v0-2-tui/loop.md:41-42`;
  - `.prd/handoff-completion/loop.md:19,24`;
  - `.prd/handoff-completion/completion-checklist.md:50,84`.

  It needs a docs closeout PR: move those items to `loop.done.md` with PR #54 / `6f1a2c9`, then `loop-lint.py` with 0 FAIL.

## State (2026-10-30)

| Item | State | Next steps, in order |
|---|---|---|
| #52 | **Closed** by PR #54, merged `6f1a2c9` (2026-10-30T02:33Z). | Both reviewers approved it in round 3 (gpt6 + code-reviewer). PR CI and main CI passed. src/ diff is empty. |
| #55 | Open. **Draft PR [#56](https://github.com/2lab-ai/zwallet/pull/56)** @ `1cba293`, branch `fix/55-whitelist-test-random-path`. It fixes 3 sites of one defect class (a "never echoed" check on a short needle that a random temp path, tx hash or address can contain): `src/core/whitelist.rs:3246`, `src/chains/mod.rs:2353`, `tests/evm_offline.rs:275`. RED on main (TMPDIR containing `2J`), then GREEN; echo proofs E1–E4 still fail as they should. The sweep covered 478 checks. **Not run:** clippy, the full test run, the unleaked `dry_run_prints_no_broadcastable_raw_tx`, and poison round R2. tr4's half is closed by #54. | Follow §5 of #56's body: 1. gates; 2. gpt6-reviewer and code-reviewer; 3. CI; 4. mark ready, then merge with `--match-head-commit`; 5. check main CI. |
| `.prd` #52 closeout | Not started. | A docs PR as above. Merge after #55 so it can record both. |
| Preview | Installed and on the tap: `2026.10.29.2315` (`d6fb99d`). Newest release: `preview-2026-10-30-0233-6f1a2c91ab3a` (test-only change). | Bump the tap only for a product change. |

**Owner-only:**
- `zwallet unlock 1y`;
- TR23, the funded mainnet send: read item 1.1 first;
- the open trade-offs: Q (QR on receive; Q1 shipped), B (recovery phrase in the TUI; B1 shipped), BSC USDT, and the stable release or `v*` tag;
- #26 and #27 stay design-only.

## Constraints that bind every session

- **Never touch the owner's wallet:** never touch `~/.zwallet` or the owner's daemon. Test homes go under `/tmp/zwt-*` only.
- **Secrets:** never print a mnemonic, password or private key.
- **No personal data in this public repo.**
- **Merging:** never approve our own PRs. Merge only with `gh pr merge <n> --merge --match-head-commit <sha>`.
- **Rust 1.95.0 is pinned.** Use the rustup toolchain explicitly; the Homebrew cargo ignores `rust-toolchain.toml`.
- **One cli test suite at a time on a shared machine.** A daemon's readiness wait fails under heavy load.

## Suggested skills

- `using-dotprd`: `.prd` closeout and `loop-lint.py`.
- `using-epic-issue`, entered through `zworkflow:using-epic-tasks` (epic-drive §0), to take over the drive.
- `receipt-loop`: for #55 and any new flake. RED first, ship to receipt, which includes main CI after the merge.
- Agents: `gpt6-reviewer` on an evidence diff, and `zworkflow:code-reviewer` in the worktree, one PR each.


