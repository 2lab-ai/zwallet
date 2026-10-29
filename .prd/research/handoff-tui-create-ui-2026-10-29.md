# H16 / TUI-10 creation UI receipt

Status: in-progress — implementation and isolated gates passed 2026-10-29; parent-owned
process acceptance, integration and GitHub delivery remain pending. B1 is retained.

Worktree: `.worktrees/handoff-tui-create`. Baseline UI source is preserved under
`/tmp/zwallet-create-ui-start`; pre-navigation backend source is separately preserved under
`/tmp/zwallet-create-pre-navigation`. The UI baseline incorporates H8 navigation; its matching
README/command/feature documentation is the documentation comparison baseline. The existing
`wallet/create.rs` and CLI creation implementation were retained.

## Scope and observed behavior

- Empty-home discovery opens the first-wallet form. `n` on keys opens a named-key form.
  Fields cover the name, 12/15/18/21/24 words for a new master, optional unlock duration,
  and an exact name confirmation. The form explains the active-key change. An existing wallet
  disappearing cannot silently turn a named-key request into a new-master creation.
- `prepare_create` completes before secret mode starts. Its private plan stays in a worker
  slot, outside messages. An existing wallet asks once; a new wallet asks twice. The input
  thread owns both fixed zeroizing buffers and only sends phase/count messages. Mismatch,
  rejected confirmation and cancellation clear both; success moves one owner into
  `wallet::create` and clears the confirmation. No password or phrase enters a Msg/Effect.
- Submitted creation cannot be cancelled by Esc or PasswordCancelled. Quit waits for the
  result; `L` is sequenced after optional unlock. Failure re-reads keys/status because a
  failed session start can follow a successful wallet write. The form warns that a key may
  already exist and offers the refreshed list.
- Successful local derivations pin each family address and immutable key index. Creation
  refuses an index conflicting with an existing verified key. `verified_indices` and
  `verified_key()` mirror the shared TUI-5 implementation and must be merged only once.
- The success view shows the public addresses and backup reminder. `B` opens only the shell
  pointer to `zwallet backup`; no recovery-phrase read effect exists.

## Gates observed

Rust 1.95.0, `CARGO_TARGET_DIR=target-handoff-tui-create` except the explicitly labelled negative
mutation build, which used the worktree's default target directory:

- `cargo +1.95.0 fmt --check`: passed.
- `cargo +1.95.0 clippy --all-targets --locked -- -D warnings`: passed.
- `cargo +1.95.0 test --locked --test tui creation:: -- --test-threads=1`: 8 passed.
  Every frame uses the existing 80x24 cell/style/budget/placement assertions.
- Existing navigation Msg/TestBackend cases: 13 passed; existing pure baseline cases: 25 passed.
- Password buffer/confirmation unit cases: 3 passed.
- Input-thread rejected-confirmation regression: 1 passed.
- RED → GREEN: removing only the in-flight `PasswordCancelled` guard made
  `create_submission_cannot_be_cancelled_and_quit_waits_for_failure_and_refresh` fail at the
  assertion requiring `CreateStage::Submitting`. The source was restored in `finally`, and
  the focused creation gate passed again. The default-target binary retains this deliberately
  failing build; normal integration must rebuild from source and use the coordinated target.
- `git diff --check`: passed.

## Pending parent gate

Two real-process tests were added and not run by H16:
`t30_on_a_pty_create_key_activates_it_and_optional_unlock_uses_typed_duration` and
`t31_on_a_pty_first_wallet_confirms_password_and_never_displays_recovery_words`.
They submit raw-mode TUI passwords with CR; only the canonical backup CLI uses LF.
Every RPC points to loopback. T31 reads the generated recovery phrase only through the
terminal-only backup CLI after TUI exit and tests the capture for three consecutive recovery
words without formatting private data on assertion failure.

No commit was made. The parent owns the coordinated process/full gates and integration with
other modal workflows; preserve the shared derivation-index pins and wait/lock sequencing.

## Independent-review fixes — 2026-10-29

Review baseline: `/tmp/zwallet-create-review-start`. Delta:
`/tmp/zwallet-tui-create-review-fixes.patch`.

- T1: terminal write failure during submitted creation must not abandon its outcome or optional
  unlock. `terminal_frame` preserves the first error, never redraws the failed terminal, and
  waits on `App::mutation_pending` before returning it. The runtime stops the input thread,
  clears its secret owner via the discard acknowledgement, and ignores queued input while
  continuing to consume worker outcomes. A deferred `LockAll` is also awaited.
- T2: the completed creation view forwards `L` to the global lock handler while keeping the
  new addresses and backup reminder visible. Its hint names `L lock all`.
- Both terminal-failure tests failed before the mutation guard, then passed after it:
  `broken_terminal_waits_for_create_outcome_and_any_requested_lock` and
  `broken_terminal_releases_after_create_failure_and_refresh`. They inject BrokenPipe into
  the runtime's actual frame decision helper, drive public creation messages, and require the
  original error only after the success/failure and deferred lock outcomes. No secret or
  process is used.
- `create_completed_view_keeps_emergency_lock_available` failed with `[]` instead of
  `[LockAll]`, then passed after routing the key. All 9 creation Msg/TestBackend cases pass.
- The 2 terminal-failure unit tests, formatting, strict all-target Clippy and diff checks pass.
  No PTY or full test suite was run for this revision.
- Integration: use this terminal-failure loop once across H13/H15/H16/TUI6. The shared
  `mutation_pending` predicate must include send/sign activity in addition to locking,
  activation and `PasswordState::Checking` (the last includes create/whitelist/repair/unlock).
  H13 owns those additional send/sign predicates. Preserve the original error and input-stop
  behavior when merging. `tron_policy` independently re-reviewed both fixes and reran the
  9 creation cases plus 2 terminal-failure cases in an isolated target: all passed; no new
  blocker was found. The H13 owner accepted this API and is adding send/unlock/activate/lock
  regression coverage to the same shared predicate.

## Standalone PR39 integration — 2026-10-29

- Ready tree: `.worktrees/handoff-tui-create-pr`, branch `handoff/tui-create-pr`, based on
  PR39 commit `f0b2b25ffb73648a47886ec97f1db05abc1e8c43`. The H16 backend, final UI and both
  review fixes are applied. No H13 send or H15 tools workflow is included.
- The stable pre-integration source and patches remain at
  `/tmp/zwallet-create-stable-before-pr`. This tree preserves PR39 navigation, activation
  error reporting, CR submissions in T14/T19, and the single `PasswordBuf` ZeroizeOnDrop
  marker. README, CLI argument definitions, terminal primitives and wallet read code have
  no diff from PR39.
- Shared target reused: `/Users/dd/Documents/Codex/2026-10-29/task/zwallet/target-handoff-tui-create`.
  Ran `cargo +1.95.0 clean -p zwallet` there before rebuilding for the changed source path.
- Observed on this exact tree: strict all-target Clippy, fmt and diff checks passed; TUI
  library tests 37 passed (including existing terminal guard tests), creation Msg/TestBackend
  tests 9 passed, navigation Msg/TestBackend tests 13 passed, and direct creation backend
  tests 9 passed. The backend CLI-unlock case was explicitly filtered out.
- No CLI suite, PTY acceptance suite, or whole test suite was run in this integration task.
  The parent owns the full gate, commit, push and CI. All changes remain uncommitted.
- Complete standalone patch: `/tmp/zwallet-tui-create-pr-final.patch`. Its base is exactly the
  PR39 commit above; it includes the new files and does not require older H16 patches.

## Parent integration acceptance

The standalone PR tree is based on PR39 f0b2b25 and preserves its raw-terminal Enter,
quit-error and PasswordBuf lifetime fixes. No transfer/tools code or marketing README changes
are added by this unit. Independent review and re-review cleared both Done-screen L handling
and terminal-failure mutation draining; their RED-to-GREEN regressions remain included.

The final Rust 1.95.0 gate passed fmt --check, locked all-target Clippy with warnings denied,
and cargo test --locked: **591 passed, 0 failed, 11 explicit live skips**. This includes actual
PTY T30 (additional key activation and typed unlock duration) and T31 (first wallet password
confirmation and no recovery phrase on screen). The tests use isolated /tmp homes, never the
user's real wallet. Local log: /tmp/zwallet-tui-create-full-gate.log. Exact-head CI is pending.
