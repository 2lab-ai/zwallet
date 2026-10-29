# H15 / TUI-11 tools handoff — 2026-10-29

The tools tab now uses shared, terminal-free wallet operations for contract reads, arbitrary
ERC-20 balances, new contract invokes, and reviewed EIP-191 messages. CLI output remains on its
existing interface; `call --json` retains complete decoded values and raw bytes. TUI results
are escaped and bounded to 512 characters, and arbitrary token symbols to 32 characters.

A message review owns its exact printable-ASCII bytes (0–512), password-derived key, chain and
checked-list fingerprint. It draws 76 bytes per bracketed row, with leading/trailing-space
counts, and requires the literal `sign` in a separate field before a signature request.
Editing discards the review. Both the fingerprint and policy are checked again, and the
returned digest and recovered signer must match the held bytes and key. CLI sign-message
continues accepting its existing UTF-8/multiline input.

Invokes select only checked contract entries and allowed selectors. Review uses independently
decoded unsigned bytes; confirm refreshes under the account lock and compares chain, sender,
contract, calldata, value, nonce, funds verdict and integer maximum total fee. A change produces
another review with no signature. The exact contract nickname is required before send. The
shared send tail checks the signed payload, journals before broadcast and drives the existing
send status/receipt UI. The `w` control on that status extends receipt waiting. New invokes do
not accept arbitrary nonce/replacement overrides: H19 owns replacement reconstruction and UI.

`RetainedInvoke` has private fields and no public arbitrary constructor. The tools state keeps
that snapshot only beside a locally computed hash observed after journal fsync, with the
uncertain flag. It survives form and context resets for H19 to consume.

## Evidence

- Backend API tests were added before the APIs existed (compile RED). Runtime regressions then
  demonstrated denied selectors leaving the form, missing typed `sign` confirmation, a lost
  deferred lock after an invoke re-review, and opening a message while a lock was pending.
  Each failed before its corresponding correction.
- `cargo +1.95.0 test --locked --test tool_wallet`: 23 focused tests. Uses isolated `/tmp/zwt-*`
  directories, fake Unix socket replies, local HTTP mocks and FilteredBackend/TestBackend.
  Covers exact 512 bytes and the last byte, spaces, whole-paste refusal, malformed/wrong-key/
  wrong-message signatures, checked policy over daemon claims, fingerprint and metadata-index
  changes, ABI/raw invokes, held input, fee/funds/nonce changes, journal kind, typed confirmation,
  queued quit/lock, and same-key altered signed bytes refusing broadcast.
- Strict all-target Clippy and formatting were checked. The focused suite does not run PTYs or
  start a real key daemon.
- `tests/tools_pty.rs::t36_real_daemon_signs_the_512_bytes_shown_by_the_tui` is added for the
  parent-controlled serialized PTY gate. It uses an isolated real daemon, checks all seven
  review rows, types `sign`, verifies the resulting EIP-191 digest/recovered address, and checks
  terminal restoration. This agent has not run it; no real-daemon T36 result is claimed here.

The unit is not declared fully closed until the parent runs the serialized PTY/full gate and
integrates H19's invoke replacement flows. No funded send, public RPC call, commit, or GitHub
approval was performed by this work unit.

## H15 + H19 integration receipt — 2026-10-29

The isolated integration worktree is `.worktrees/handoff-tui-tools-pr`, branch
`handoff/tui-tools-pr`, based at H13 `685a363`. It reconstructs the tested H17
`9addb25` merge baseline and H19 standalone delta, then adds H15 and the typed
RetainedInvoke adapters. `/tmp/zwallet-h15-pr-baseline` records the dependencies
before H15 edits. The H19 PR worktree remains untouched by this unit.

H13's receipt deadline, terminal mutation drain, queued-quit error preservation
and deferred LockAll return path are retained. H17's test server 200ms timeouts
and H7 history frame style audit are retained. Tools signing extends the common
`mutation_pending` predicate; its input continues to use the existing Key/Paste
classification, so a failed terminal drops queued input while worker results
and deferred locks still complete. A signature failure after queued q now keeps
the original error code, message and details, even if a deferred lock also fails.
An existing terminal failure keeps precedence over the signature error.

T34(g–l) now run through one App's real forms and worker jobs. The initial invoke
uses the `exactInputSingle` signature with selector `0x414bf389`, all tuple
arguments and nonzero native value. The test does not inject retained authority:
the actual locally computed, fsynced Progress event creates it. A FakeDaemon
wire proxy counts every SignTx while a separate deterministic test signer returns
valid signatures; neither is a real wallet daemon.

| Acceptance | Observed extra SignTx after the original invoke | Assertion |
|---|---:|---|
| T34(g), same session | 1 | All calldata/value/to/nonce retained despite a deleted journal; both fee fields at least 1.25x |
| T34(g), restarted | 1 | Node's reconstructed bytes/hash/recovered owner and matching journal prove the same full calldata |
| T34(h) | 0 | Re-signed list without router or its selector rejects both speed-up and retry |
| T34(i) | 0 | Rewritten journal recipient or amount disables both s/x after restart |
| T34(j) | 1 | Cancel keeps nonce, pinned self, zero value and empty input |
| T34(k) | 1 in session; 0 after restart | Unknown-node retry uses the retained review after journal deletion; restarted UI warns and has no R authority |
| T34(l) | 0 | Claimed owner with another recovered signer, or altered arguments retaining the selector, disables both s/x |

Observed RED→GREEN: without the RetainedInvoke adapters, the same-session test
fails with `no matching local send`; with the adapters it passes. The queued-q
signature failure initially lost its error (`None`); the new completion regression
passes with exact error equality, including details, and stale replies remain ignored.

Final focused validation in this tree:

- `cargo test --test replacement_review`: 22 passed (the five new T34 integration tests included).
- `cargo test --test tool_wallet -- --skip call_matches_cli_and_contract_text_is_inert_and_bounded`:
  22 passed; the one test invoking the CLI was deliberately excluded from this unit's gate.
- `cargo test --lib completion_tests`: 2 passed; both queued-quit variants and terminal-error
  drain/precedence are exercised without I/O.
- `cargo test --test tui_history`: 10 passed with the H7 style audit.
- `cargo test --test tools_pty --no-run`: compiled only; no PTY process was run.
- `cargo fmt --check` and strict `cargo clippy --all-targets -- -D warnings`: passed.

The reused target is `target-handoff-tui-unlock`. Independent integration review
and the parent's serialized full/PTY gate remain separate completion requirements.
No commit was created here. The final scoped H15 patch excludes the already-supplied
H17 frame audit; it requires that dependency to remain in the parent integration.

The parent subsequently found an H19 integration regression: `x` typed into the
wait-duration form was intercepted as cancel. The narrow status-shortcut guard
and the explicit Wait/replacement-Idle assertions were adopted here. After this
dependency update, all 21 transfer mocks and all 22 replacement mocks passed again;
the exact Wait regression with the new assertions and strict all-target Clippy
also passed. This dependency is excluded from the scoped H15 patch because the
parent's H19 commit owns it. No additional full or PTY gate was run here.

Independent review reproduced one further display defect: two invokes with the
same selector and calldata length but different argument bytes had identical
review screens. The review now shows every decoded calldata byte and exact
quantity in 72-character scrollable rows; j/k scroll and Home returns to the top.
New or changed reviews reset the scroll; the separate confirmation field retains
ownership of Home/End. Policy errors and the exact 512-byte message block remain
intact. The permanent TestBackend regression reconstructs raw calldata through
the 4096-character limit and the ABI signature path, distinguishes last-byte
changes, checks p/d/g and changed-review resets, and confirms scrolling signs
nothing. Independent RED evidence: `/tmp/h15-independent-invoke-calldata-red.rs`.

After the display fix, tool_wallet 23 (CLI one excluded), replacement_review 22
and strict all-target Clippy passed again. Total distinct focused tests observed
for this integration are 78 (23 tools + 22 replacement + 21 transfer + 10 history
+ 2 completion); no real CLI/PTY/full gate was run by this unit. Parent H19's
repaired PTY fixture and final status text expectation are present in both the
tree and dependency snapshot, with no production daemon socket interception.

Final independent integration review: CLEAR, no remaining blocker. The reviewer
directly reran tool_wallet 23 (CLI one excluded), T34 invoke/replacement 5 and
pure completion 2 after the calldata fix. Report:
`/tmp/h15-h19-independent-review.md`; reviewed source hashes:
`/tmp/h15-independent-reviewed-sha256.json`. The parent's full/PTY gate and commits
remain pending, and are not implied by this focused result.

## Final-gate follow-ups — 2026-10-29

On the final H19 ancestor `049921e`, the parent's whole gate found the idle Tools
hint omitted `? actions`. A dedicated 80x24 Tools frame/action-menu regression
failed before the correction. The hint is now 79 ASCII characters including
all five tool shortcuts and `? actions`. The existing every-tab suffix assertion
is unchanged; only Tools was removed from the obsolete future-tab expectation.
Observed focused GREEN: all 14 navigation tests and 23 Tools mocks (CLI one
explicitly excluded), plus strict all-target Clippy and format checks.

The H7 reviewer also reproduced an accepted late Keys/Status read overwriting
the original send/signature error while a queued quit waited for deferred LockAll.
The minimal common fix makes fail_pane preserve the first error with all recovery
details. Reviewer-owned public-App regressions (send/Keys, send/Status, signature/Keys,
signature/Status, stale-generation/no-lock control) were adopted unchanged and all
five passed directly in this final tree. The reviewer independently observed four
RED failures and one control pass before the fix, then five GREEN on its isolated
snapshot. No actual CLI/PTY/full suite was executed by this unit for these fixes.

Compared with the reviewed frozen source/tests, the only changes are
`src/tui/app/tools.rs` (idle hint), `tests/tui.rs` (navigation expectations/regression),
`src/tui/app.rs` (first quit error), and the two new late-read regression files.
Verification: `/tmp/zwallet-h15-final-followup-verification.json`. The parent owns
the renewed whole gate and commit; the prior full-gate failure is not counted as
a passing gate.

## Parent serialized final gate — 2026-10-29

The final publish tree is `.worktrees/handoff-tui-tools-final`, based on the
validated H19 commit `049921e60417db9ef19778877a15f3dc66f3a041`. It retains
H19 ancestry, the history server write bound and frame-style assertions.

Observed after both final-gate follow-ups: Rust 1.95.0 formatting, strict
all-target Clippy, and the complete locked debug test suite passed: **688 passed,
0 failed, 12 ignored**. The suite includes the real-daemon 512-byte Tools PTY,
both replacement PTYs, all transfer PTYs, and all five late-read error regressions.
Log: `/tmp/zwallet-h15-final-gate-repaired.log`. Only this process-heavy gate ran
at a time. All signing/broadcast fixtures use temporary wallets and local mocks.
The independent review remains clear. GitHub final-head CI is the next gate;
no merge, release or funded public broadcast is claimed.
