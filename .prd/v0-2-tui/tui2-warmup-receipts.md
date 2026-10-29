# TUI-2b / TUI-2c — implementation receipts

Status: in-progress (2026-10-29; local integration and independent review passed; remote CI and upstream merge pending)

Scope: issue #34 handoff, TUI-2b/TUI-2c in `units.md`, based on PR #32
`428e7a00a9343b8aa4743a210b1a00413250961e`. This record does not claim the other
TUI screens or the upstream merge are complete.

## Implementation

- Enter on a locked key prepares and pins the daemon executable before accepting input.
  An input-thread handshake prevents typeahead after Enter entering `Msg::Key`. A preparation
  failure discards typeahead until Enter/Esc dismisses the error.
- `PasswordBuf` owns one fixed 1024-byte zeroizing allocation, no Debug/Display/Clone. Only
  character counts cross the UI message channel. Paste copies are wiped immediately. Input
  is masked, rejected in full on overflow/newline/NUL characters, and cleared on cancellation.
- The worker starts the timed session, then decrypts locally and checks the selected and
  global active key, session readiness, and signed list. The local KDF runs in `spawn_blocking`;
  the UI draws `checking...`. The password drops immediately after its last use.
- Opening a key does not change the global active key. Its checked address is pinned and
  shown as verified by password. Keys/status/open-key refreshes reject contradictory answers.
  Failure invalidates older reads and drops the checked list while retaining pinned addresses.
- `q` waits for the password worker. `L` during the check queues the stop after it, preventing a
  late session start from undoing the stop. Bracketed paste is disabled on normal/panic restore.
- `--warm` defaults to one hour. `--allow-ptrace-scope-0` is available only at TUI start, like
  CLI unlock; no UI key changes it.

## Observed verification

- RED: `enter_on_a_locked_key_starts_warmup_without_changing_global_active` failed on the
  pre-implementation binary: Enter produced no effect. GREEN after the staged workflow.
- T2 real PTY passed: immediate Enter+typeahead, bracketed paste, all 17 mask characters,
  no password or marked prefix in terminal output, approximately one-hour expiry, verified
  open `a` while the global active key remains `b`. Initial sandbox run refused a local Unix
  socket with EPERM; the same isolated test passed outside the sandbox (3.84 s).
- Exact-order counterexample passed: unlock first, edit cache, local password check refuses
  with `zwallet address --verify`; the existing session remains, with no pinned address/list.
- Invalid socket address (`▀▄` repeated 600 times) passed: wallet layer refuses; filtered
  80x24 frame shows a bounded escaped error while retaining the password-pinned address.
- Password Msg/frame and later conflicting-address counterexample passed. Short one-line
  error placement was corrected to report the text's actual rectangle.
- All 20 `cargo +1.95.0 test --locked --lib tui:: -- --test-threads=1` tests passed, including
  panic/terminal restoration and checking the paste allocation's original bytes are zero.
- `cargo +1.95.0 clippy --locked --all-targets -- -D warnings`, `cargo +1.95.0 fmt --check`,
  and `git diff --check` passed. The parent owns the whole-repository gate and remote CI.

## Integration notes

- Preserve the parent's PR32 activation serialization and quit/error handling. Gate Enter
  on `!activating` as well as `!locking`; this branch predates that fix.
- `invalidate_reads` and the single-row `error_view` rectangle fix overlap the parent's H1.
- Later payee/whitelist screens must call `check_unchanged` before consuming `CheckedList`.
  A cached list exposed by `App::checked_list()` alone is not permission to use it.
- Current pinned addresses use `PRIMARY_CHAIN` (EVM), as TUI-2 specifies. Tron-aware views
  remain TUI-12 and must use family-aware address comparison and chain-specific derivation.

## Independent review corrections (2026-10-29)

- Repeated password checks validate all previously pinned addresses before inserting any new
  pin. A different wallet/key index cannot replace a lifetime pin or partially add new pins.
- Every key-list read error or mismatch uses the same fail path: clear the checked list,
  invalidate pending reads, keep original pinned addresses. Old Status/Address replies are ignored.
- An address refusal preserves the pin but applies the fresh lock state and expiry. A failed
  status read reports unknown lock state, retaining only independently password-pinned addresses.
- The secret buffer accepts the same newline/NUL exclusions as CLI password validation,
  including tab via typing or bracketed paste. The other-keys hint names the existing CLI verify
  command; TUI-3/4 will replace it with its implemented V action.
- All three pin/Keys/lock-state regressions were observed RED before correction; focused pure
  Msg regressions and password-buffer tests are the follow-up gate. The final pre-review
  38-test TUI suite passed in 54.61 s; no further process suite is run during parent integration.

## Integrated gate (2026-10-29)

Based on validated TUI foundation `fa79ca7` (PR #35), including serialized activation
and quit handling. The integrated gate passed: `cargo +1.95.0 fmt --check`,
`cargo +1.95.0 clippy --all-targets --locked -- -D warnings`, and
`cargo +1.95.0 test --locked`: **534 passed, 0 failed, 11 live tests ignored**.
The 45-test TUI target includes the real PTY no-echo, terminal restore, activation
and lock-refresh checks. The password worker also waits before quit; the fixed
secret buffer implements `ZeroizeOnDrop` for the upcoming owned-password APIs.

Independent review rechecked all four corrections above with no remaining blockers.
Remote Linux/macOS checks and upstream merge are tracked in the successor PR.
