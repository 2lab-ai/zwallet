# Handoff: TUI v2 and Tron drive — #31, #32, #33 in flight; a parallel review-hardening stream (#29, #30)

Source: https://github.com/2lab-ai/zwallet/issues/34

Handoff for the session that drove the TUI v2 and Tron work (2026-10-27 → 2026-10-29). The next session picks it up from here. Three branches are in flight as draft PRs: **#31** (TUI-1c), **#32** (TUI-2a, stacked on #31), and **#33** (Tron A). Specs, units and open gaps live in `.prd/`. This issue holds only what `.prd/` and the PRs do not.

## 자신 없는 곳 & 사각지대

### 1. Least confident: what would disprove it, and how to close it
1. **The 12 macOS failures on #31 (TUI-1c, `17e92a6`) are explained as load, not as a defect, but no quiet macOS run has proved it yet.**
   - The failing run: the full macOS `cargo test --locked` failed 12 of 111 cli tests in 14,315 s. Two other cli suites ran at the same time, at load 100–170.
   - What the 12 were: 7 are `the key daemon did not report readiness within 60s`. The other 5 are a command still running after 120 s, a daemon read that would block, an address not yet verified by the session, an empty `decimals()` reply, and empty outputs.
   - Evidence for load: the cli suite on a pty, same HEAD, right after: 111/111. Linux: cli 112/112. Round 5 changed only a test import and a pointer form.
   - **What is missing:** one macOS `cargo test --locked` on that HEAD with no other `cargo test` running (`pgrep -fl "cargo test"` empty, load under ~20).
   - **To close it:** run it in `.worktrees/feat-tui-1c-pty`, or let CI's macOS job run once the conflicts are resolved.
2. **#32 (TUI-2a) and #33 (Tron A) were never reviewed, and their gates were never re-run by the dispatcher.** Both coding agents stopped on an API rate limit (2026-10-28 18:47 KST) before they handed back. The gate lines in the PR bodies are read from their log files. What the agents knew about residual risk was never written down. **To close it:**
   1. Run gpt6-reviewer on an evidence file of each diff.
   2. Run zworkflow:code-reviewer in each worktree.
   3. Re-run the gate yourself.
   4. For #33, also run the two steps missing from its logs: the tui suite on a pty ×3, and the x86_64 macOS build.
3. **The cli suite's timing depends on machine load.** One cli suite alone takes about 4–6 minutes (TUI-2a runs: 231–261 s). Three at once took up to 4 hours, and the 60 s daemon-readiness wait starts failing. The shared lock `/tmp/zwt-serial-<uid>.lock` does not serialize whole suites. **To close it:** run one cli suite at a time on this machine. Other projects' `cargo test` runs share the machine too.

### 2. What we are missing
**(a) What the owner may not have seen:**
- **A second stream is changing zwallet in parallel.** Account `ai-insightquest`, working from a fork, did this today:
  - opened issues #21–#28;
  - **merged #30** into `main` (`51b4a51`), which moves most of `README.md` into `docs/commands.md`, `docs/features.md`, `docs/security.md` and `docs/development.md`, and edits `.prd`;
  - opened **#29**: Rust pinned to **1.95.0** (`rust-toolchain.toml`), CI replaced by `validate.yml`, new signed whitelist fields (`whitelist native-cap`, `token-cap`, `calldata-cap`; "older binaries reject new fields"), and a daemon rewrite. Its Linux check fails.

  Neither stream's `loop.md` knows about the other. Consequences:
  - **Merge order is a decision.** #29 changes the signed whitelist format. Tron B2 (TRON-3: `["tron"]` chains on whitelist entries) must be designed on top of whichever format lands first.
  - **The TUI coverage matrix grows.** `.prd/03-tui.md` has 59 rows and none for the three cap commands. "Every CLI function in the TUI" now means three more rows.
  - **The toolchain differs.** Our gates use Rust 1.99 (`rust:1.99-bookworm`, macOS stable). After #29, code must also build on 1.95.0, so gate on 1.95 then.
  - **README edits move.** The README parts of #32 and #33 must go into the new `docs/*.md` files.
- **The TUI does nothing new yet.** The owner's complaint was that the TUI cannot do anything. Since then, two specs (#19, #20) and test infrastructure (#31, #32) have landed. The first PR that adds actions the owner can use is TUI-3+4 (balances, chain picker, receive), with preview M1.
  - TUI-3 depends on "TUI-2 (the open key)" (`units.md` §TUI-3). #32 already adds `wallet::address`.
  - [unverified] If TUI-2c's pinning is not needed for TUI-3, then TUI-3+4 can start as soon as #32 merges, in parallel with TUI-2b, and M1 arrives sooner.
- **Each preview costs the owner a re-unlock.** A new build changes the cdhash, so the owner must lock and unlock again. Seven previews are planned (M1–M5, Tron A, Tron C). If the owner prefers fewer unlocks, batch them.

**(b) What the dispatcher only just noticed:**
- **The owner has not answered the open questions.** They are now on the owner's page and were asked in chat on 2026-10-29. There is no answer yet:
  - Q: a QR code on receive. Recommend Q3.
  - B: the recovery phrase in the TUI. Recommend B1, the safe default.
  - G3: the stable xbrew release.
  - BSC USDT.
  - The final `v*` tag.

  TUI-10 and TUI-11, and the scope of Tron C, depend on these answers.
- **The deferred REDs file is now named by a durable path.** `units.md` §TUI-2 split used to send readers to the "dispatcher scratchpad", which a reboot wipes. As of 428e7a0 (#32) it names `~/.local/share/zwallet-epic/deferred/tui2b-deferred-tests.rs`.
- **Nothing ran for about 17 hours.** The rate limit said "retry in ~13,900 s", but no one resumed the agents. A drive that uses long-running agents needs a check after the retry time.
- **A preview release was published without us.** The Preview workflow ran on `main` after #30 (2026-10-29 03:18Z) and published a `preview-*` release. The tap still points at 2026.10.28.0227, because the tap bump is a separate, manual step.

## State (2026-10-29 12:50 KST; work stopped at the owner's request)

| PR | Branch @ head | Base | State | Next steps, in order |
|---|---|---|---|---|
| #31 TUI-1c (tests only; closes T9, T10) | `feat/tui-1c-pty` @ `8223b79` | `main` | Round 5: gpt-6 APPROVE, code-reviewer APPROVE. **`origin/main` merged** (ba59846); `.prd/v0-2-tui` keeps the v2 plan with TUI-1c's rows. **The A12 ledger is committed** (8223b79, `.prd/v0-1`); the owner's address is not written in the repo. CI run 40812894695: ubuntu **success**, macOS still running when work stopped. | 1. Read CI's macOS result.<br>2. Mark ready.<br>3. Merge with `--match-head-commit 8223b79`. |
| #32 TUI-2a (closes T2b) | `feat/tui-2-warmup` @ `428e7a0` | `feat/tui-1c-pty` | **Main merged via #31** (428e7a0). TUI-2a's README text moved to `docs/commands.md`. At 428e7a0, macOS lib 289 and tui 30 pass. **gpt-6 review: REJECT.** The MUST-FIX: two confirmed `s`/`y` presses spawn independent tasks, and these can finish in reverse order, leaving the wrong key active (src/tui/mod.rs:108, src/tui/app.rs:230). Also, `q` should wait for an activation in flight, as it waits for `L`. Two non-blocking items. The worktree reviewer was stopped before it reported. Review text: `~/.local/share/zwallet-epic/logs/rev2a/gpt6-review-428e7a0.md`. | 1. Fix round, RED first: a `Msg` test with reversed completion, and a successful activation on the second row.<br>2. Run both reviews again.<br>3. Gate, running the cli suite alone.<br>4. After #31 merges, CI.<br>5. Merge. |
| #33 Tron A (closes TR1–TR8, TR24) | `feat/tron-a-see` @ `1e8864c` | `main` | Unchanged and not reviewed. A finishing agent was halfway through merging main (moving README parts into `docs/*.md`) when it was stopped. The merge was aborted; its partial edits are saved at `~/.local/share/zwallet-epic/deferred/tron-a-merge-wip/`. | 1. Merge main, reusing that patch.<br>2. The docs commit (TR rows).<br>3. The missing gate steps.<br>4. Reviews.<br>5. CI.<br>6. Merge.<br>7. Preview. |
| TUI-2b (closes T2) | `feat/tui-2b-password` @ `428e7a0`, local only | — | Brief written (`~/.local/share/zwallet-epic/epic/tui2b-brief.md`). The coding agent was stopped before its first commit; the worktree is clean. | Start it from #32's tip after #32's fix round. |

- **Worktrees:** `.worktrees/{feat-tui-1c-pty,feat-tui-2-warmup,feat-tron-a-see}` are clean and pushed. `.worktrees/feat-tui-2b-password` is clean and unpushed, with no commits of its own. The local `main` checkout is stale (`24f4b5b`).
- **The owner's daemon:** one runs on `~/.zwallet`, from preview 2026.10.28.0227. Gates must leave it alive; the gate scripts print `USER_DAEMON_BEFORE` and `USER_DAEMON_AFTER`.
- **After these three:** follow `.prd/v0-2-tui/units.md` and `loop.md`:
  - TUI-2b, TUI-2c;
  - TUI-3+4, then preview M1;
  - TUI-5, TUI-6, then M2;
  - TUI-7+8, TUI-9, then M3;
  - TUI-10, TUI-11, then M4;
  - TUI-12, then M5.
- **Tron, per `.prd/v0-3-tron/units.md` and `loop.md`:**
  1. B1, then B2.
  2. Q3: POST an unsigned transaction.
  3. C, then a preview.
  4. TR23: the live send, which only the owner can approve.

## Rules that bind the work (owner decisions)
- No OS keychain. Nothing from Trust Wallet's code. After `unlock`, `zwallet address` asks for no confirmation.
- Never print the mnemonic, the password or a private key. The password is typed by a human only: no file, no environment variable. `backup` runs only at a TTY.
- Tests never touch `~/.zwallet`.
  - Test homes are `/tmp/zwt-*`. QA homes are `/tmp/zqa-*`, because the test suite stops every daemon under `/tmp/zwt-`.
  - The real wallet may be read only read-only, and only its non-secret fields.
- Never approve our own PRs with `gh pr review --approve`. Merge with `gh pr merge <n> --merge --match-head-commit <sha>`.
- No personal data, such as an email address, in any HTTP request or header.
- Only the owner decides:
  - the stable release (G3) and any `v*` tag;
  - BSC USDT;
  - funding, and the live Tron send (TR23);
  - trade-offs Q and B.
- Merges and preview deploys with green receipts (gate, outside review, CI) need no new question.

## Preview chain
1. Push to `main`. The Preview workflow publishes a `preview-*` release.
2. Render the tap formula from that release's assets.
3. Open a PR in `2lab-ai/homebrew-tap` and merge it.
4. Run `brew upgrade 2lab-ai/tap/zwallet-preview`, then `brew test`, then a smoke test.
5. Run the tmux QA in a `/tmp/zqa-*` home.
6. Ask the owner to unlock again (the cdhash changed).

## Tooling
The dispatcher's tooling is not in the repo. A durable local copy is at `~/.local/share/zwallet-epic/`:
- **`gates/gate3.sh <worktree> <outdir>`:** the full gate. It runs fmt, clippy, the full test suite, the cli suite on a pty ×3, the live tests, the x86_64 lib, and Linux Docker. It prints `LINUX_OK`.
- **Other gates:**
  - `gates/gate-1c.sh`: gate3, plus the tui suite on a pty ×3 and on Linux ×3.
  - `gates/gate-delta.sh`: fmt, clippy, the full test suite, the x86_64 lib, and Linux.
  - `gates/linux-gate199.sh` and `gates/linux-tui.sh`.
- **`tools/`:**
  - `render-formula.py <TAG> <assets.json> <formula>`: renders the tap formula.
  - `qa-tui/run.sh`: the tmux QA.
  - `call.py` and `call2.py`: direct grok-4.7 and gpt-6-astra review calls. Start them in the background as `( python3 call2.py … & )`.
- **`epic/`:** the owner page source, briefs, earlier PR bodies, review rounds, and `ledger-v01.py <worktree>`.
- **`deferred/`:** the REDs for TUI-2b and TUI-2c.
- **`logs/`:** the gate logs cited in #31–#33.

## Suggested skills
- `using-epic-issue`: the owner page and the question loop. It enters through `zworkflow:using-epic-tasks` (epic-drive §0).
- `using-dotprd`: `loop.md` holds open items only; `loop.done.md` is append-only; `loop-lint.py` must report 0 FAIL.
- `using-intentfirst`: the owner's words, then their intent, then the task tree.
- `receipt-loop`: a receipt before any "done".
- `gpt6-reviewer` with `zworkflow:code-reviewer` for each PR. Use `trinity` for spec changes, e.g. reconciling Tron B2 with #29's whitelist format.
- `superpowers:systematic-debugging`, if a quiet macOS run still fails.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
