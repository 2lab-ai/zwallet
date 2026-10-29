# TUI-5 transfer handoff — 2026-10-29

State: parent integration, independent reviews, actual PTY and local whole gate passed;
exact-head CI and upstream merge remain pending. No real wallet, password file, external RPC or funds used.
No commit or full CLI/PTY suite was run by this worker.

## Scope and baseline

This WU implements the Send tab through native/registry-token selection, checked payee selection,
amount, immutable review, typed nickname confirmation, durable-journal progress and receipt status.
The library boundary was previously delivered in `/tmp/zwallet-tui-send-backend.patch`.
The UI delta uses `/tmp/zwallet-tui-send-ui-baseline`, a snapshot of the stable TUI-3+4 navigation
source plus that backend. The worktree is `.worktrees/handoff-tui-send`, branch
`handoff/tui-send-api`. Imported navigation/read/CLI dependencies are excluded from the UI delta.

Apply order after the prior backend and navigation integration:

1. `/tmp/zwallet-tui-send-backend-delta.patch`: `RetainedTransfer` and registry-unit exact-intent check.
2. `/tmp/zwallet-tui-send-ui.patch`: UI, focused-test additions and this evidence/documentation.

The original backend patch remains separate; do not apply it twice. Shared `verified_indices` and
`verified_key` changes may already be present from creation UI: retain one common implementation.
Tron integration must preserve its signed raw-data verification, exact address comparison and
journal expiry hooks; this unit uses the EVM transfer/receipt backend.

## Protections and integration contracts

- A typed `CheckedPayee` is selected from the password-checked list. No raw-address field exists.
  Zero addresses, contract entries and other-chain entries are excluded. The fingerprint is
  checked before/after preparation and immediately before sending; stale lists lose their choices.
- The full recipient/from/token addresses and integer base-unit amount are shown. The integer
  maximum execution fee plus OP L1 estimate is compared at confirmation. Registry decimals form
  the expected decoded action independently of RPC metadata. An absent L1 estimate refuses review.
- The review sends no `SignTx` and takes no send lock. Confirm checks the fingerprint under the
  lock, prepares again, compares exact intent and fees, enforces pending/repeat/funds/policy, verifies
  the signed payload and recovered key, and journals before broadcast.
- `p` and `d` are separate explicit overrides with a second-payment warning. `g` reparses the
  numeric ceiling and reruns review. An invalid ceiling leaves the previous review unchanged.
- Each effect contains only an id; the runtime removes its owned job once. Replayed effects and
  repeated Enter cannot duplicate a request. `q` waits for send; `L` during send queues the stop.
- Receipt deadline starts at 90 seconds, includes RPC connection/in-flight work and reports unknown.
  `w` accepts CLI durations; a new wait cancels only the superseded poll, never the send. `n` starts
  a new payment. Replacement controls stay explicitly unavailable until TUI-9 is integrated.
- `App::verified_key()` returns the lifetime password-derived index and family address only when
  current metadata still agrees. Repeated password checks validate all old pins/indices before
  inserting any new authority. This API is shared with create/tools/replacement UI owners.
- `TransferReview::retained_intent()` creates an opaque public-metadata `RetainedTransfer` for
  fixed-nonce replacement. External workers can use `begin_external_send()` and emit
  `TransferMsg::{Progress,Sent}` with that id. Replacement retains its own OriginalIntent.

## RED → GREEN and checks

The corrected RED assertion required a real `Effect::Transfer` after opening the Send tab. It
failed before UI wiring. A weaker preliminary assertion for any effect was discarded because an
ordinary refresh also satisfied it. The 21-test final suite passed on the completed implementation.

```
CARGO_TARGET_DIR=.../target-handoff-tui-send cargo +1.95.0 test --locked --test transfer_review -- --test-threads=1
# 21 passed, 0 failed, 2.43 s
```

Tests count mock SignTx requests on same-generation list changes, metadata-index swaps, fee and
OP L1 rises (zero), fee decreases (one), registry-token decimals swaps (zero), pending/repeat
explicit overrides, wrong signed intent/key, and duplicate effect delivery. The production workers
are exercised with loopback RPC and a temporary fake Unix signer; no actual daemon is started.
Additional checks cover receipt timeout/extension/mining, stale poll cancellation, deferred lock
and quit, checked picker filtering and session-only authority refusal.

Every state frame in these tests passes through FilteredBackend at 80×24 with zero replaced cells,
only styles from `style::ALL`, independent per-source budgets, ASCII, placement bounds and exact
Outside text matching. Full addresses, base-unit amount, fees and transaction hashes are asserted.
The test budgets include WhitelistName=32, Quantity=78 and TransactionHash=66.

`cargo +1.95.0 fmt --check`, `git diff --check`, and
`cargo +1.95.0 clippy --locked --all-targets -- -D warnings` all passed. All-target clippy includes
the existing CLI/TUI test targets without running them. The dispatcher owns real PTY (CR Enter),
full gate, Linux/macOS CI, publication and issue closure.


## Independent-review follow-up: receipt deadline and process tests

The reviewer reproduced a ready receipt arriving after the connect-inclusive outer deadline:
chain-id response delayed 300 ms, overall wait 500 ms, consumer resumed at 550 ms. Before the fix,
`timeout_at` polled the ready inner result first and incorrectly returned success (author also
observed the RED). The Receipt worker now checks the original deadline again immediately after
awaiting the result. At or after that deadline it returns unknown, for both initial sends and typed
wait extensions. `tests/tui_transfer_deadline.rs` is the permanent local-HTTP regression.

After the fix, all 21 existing focused transfer tests plus the new deadline test passed. The
independent reviewer reran the original repro and confirmed RED→GREEN. Strict clippy for the
transfer, deadline and new PTY test targets passed.

`tests/tui_transfer.rs` adds two real-process cases using the existing common PTY/Home/RPC helpers:
80×24 locked `V` check then warm-up, native review, wrong nickname, fee rise/new review, fee fall,
actual daemon signature, journal before broadcast, exact hash/mined status; and native policy-cap
refusal with no journal or broadcast. They use CR Enter, the exclusive test lock and loopback RPCs.
Both compile with `cargo +1.95.0 test --locked --test tui_transfer --no-run`. The author did not run
those PTY/process tests; their runtime acceptance remains the dispatcher's scheduled gate.


## Common terminal-failure drain follow-up

A failed `terminal.draw` previously returned immediately and could tear down a submitted async
send, active-key write or unlock before its outcome. The shared `terminal_frame(app, failure,
draw)` helper now preserves the first terminal error, never redraws that failed screen and waits
while `App::mutation_pending()` is true. The H13 predicate covers send, lock, active-key change and
password Checking (unlock/repair); later units extend it for their submitted work. A completion
that schedules a previously requested lock keeps draining until `Locked` arrives.

On failure the input thread is stopped, an unsubmitted password owner is dropped, and
`terminal_update` ignores queued input while still applying worker completion messages. A later
worker/lock error does not replace the original terminal error. Read preparation or an unsubmitted
password does not keep the runtime alive. The same predicate governs normal quit waiting.

The new `tui::terminal_send_failure_tests` pure unit tests first observed three failures against
the old immediate-return behavior. All five now pass: send progress/outcome/deferred lock,
unlock/deferred lock, active-key write/existing lock, immediate release without submission, and
queued confirmation/password events unable to authorize new work while completions still drain.
`cargo +1.95.0 test --locked --lib terminal_send_failure_tests -- --test-threads=1` passed 5/5.
Strict all-target Clippy, format and diff checks passed. No PTY, process or network test was run
for this follow-up. H16 shares the helper/API; its input classifier also includes creation's two
password handshake messages. TUI6 extends the pending predicate with its apply state.


## Quit during failed send follow-up

The independent H19 review found that q during send could finish with exit success after the
worker failed. The accepted `TransferMsg::Sent(Err)` now records its original error in quit_error
when quit is already requested, before either pre-progress or post-progress UI handling. A queued
lock's later error does not overwrite the send's hash/nonce recovery details. The shared external
send path therefore covers H19 replacements too. Existing terminal failure remains the primary
result through the runtime's preserved first-error return.

The new pure `quit_error_tests` observed RED (`None` instead of the original RpcError), then GREEN
for all progress/no-progress × deferred-lock/no-lock × RpcError/BroadcastUncertain combinations.
Stale worker results and failures while staying in the UI do not manufacture quit errors. Both
new tests and all five terminal failure regressions passed. Strict all-target Clippy, fmt and
diff checks passed; no PTY or process test was executed for this follow-up. H19's author and the
independent reviewer were asked to re-review the shared-path fix.

## Parent final acceptance

On 2026-10-29 the integrated source passed the full Rust 1.95.0 gate: fmt --check,
locked all-target Clippy with warnings denied, and cargo test --locked:
**597 passed, 0 failed, 11 explicit live-test skips**. This includes both actual native-send
PTYs, the receipt-deadline regression, five terminal-drain tests and two queued-quit error tests.
The independent reviewers reproduced the deadline and lost quit error before their fixes,
reran the corrected regressions, and found no remaining blocker in their reviewed boundaries.

One earlier whole gate exposed the old navigation assertion that the Send tab must say
'in a later version'. The expectation now covers only the still-pending tabs; the focused
navigation test and whole suite were rerun successfully. No runtime test was disabled.
The first passing whole gate preceded the later queued-quit finding; the final gate above
includes that correction and preserves the original WalletError and recovery details.
Logs: /tmp/zwallet-tui-send-pty.log and /tmp/zwallet-tui-send-final-quit-gate.log.
No public RPC or actual funds were used. Tests use disposable isolated wallets and loopback nodes.
