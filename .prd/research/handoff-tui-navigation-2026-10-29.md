# TUI-3+4 handoff implementation receipt

Status: in-progress — implementation and isolated gates passed 2026-10-29; coordinated
process acceptance and integration remain pending. This is not a shipped-v0.2 claim.

Worktree: `.worktrees/handoff-tui-navigation`, branch `handoff/tui-navigation`. Scope follows
`v0-2-tui/units.md` TUI-3+4 and `03-tui.md` T5, T8, T12–T20 and T42. Q1 is retained.

## Implemented boundary

- Seven tabs and cycling, context hints, all 55 action rows (the original 52 aliases plus
  rows 60–62), exact alias filtering, disabled reasons, popup/field focus, printable-ASCII
  rejection and bounded fields. Future signing/editing/history actions remain disabled.
- Shared `wallet::read` APIs for one/all-network balances, all-network addresses, brief errors,
  endpoint construction and chain-id validation. CLI balance/addresses reuse them and retain
  their existing output model. All-network reads retain partial failures and put testnets last.
- Session-only RPC endpoints, validated before adoption. Input hides credentials, suffixes and
  key-like host labels. Cancelled/stale endpoint checks cannot change a session endpoint.
- Full receive addresses and family-specific lifetime password pins. Cache/node/session text
  cannot create a password pin. A conflicting result fails the pane and drops the checked list.
- `V` checks all keys on the key tab and the open/active keys elsewhere, without starting or
  stopping a daemon or writing files. `A` shares CLI repair, preserves sealed contents and
  displays an inert, scrollable old/new cache report. `Q` points to `zwallet address`.
- `u` takes a typed duration before password/unlock preparation; the startup ptrace flag is
  immutable from keys and visible on the session/password panes. The session pane labels the
  daemon's whitelist generation/count as reports, never as a checked list.
- H4's final review fixes and H1's activation serialization/quit wait/read invalidation are
  preserved. Active-key metadata writes use `spawn_blocking`.

## Observed gates

Rust 1.95.0, target directory `target-handoff-tui-navigation`:

- `cargo +1.95.0 fmt --check`: passed.
- `cargo +1.95.0 clippy --all-targets --locked -- -D warnings`: passed.
- `cargo +1.95.0 test --locked --test tui navigation:: -- --test-threads=1`: 13 passed.
  Each rendered 80x24 frame passes the existing cell/style/source-budget/placement gate;
  tests cover menu focus, one/all-network balances, hostile token errors, stale responses,
  RPC redaction/adoption/cancel, receive mismatch, V, duration input, session pids/home text,
  index-1000 verified key widths, inconsistent derivation results and repair scrolling.
- Existing pure Msg/TestBackend cases selected without Home/PTY/process setup: 25 passed.
  The old verification-hint assertion now checks the implemented `V` hint.
- `cargo +1.95.0 test --locked --test wallet_read -- --test-threads=1`: 3 passed, using
  only temporary fixture wallets and loopback RPCs. Checks mismatched chain identity,
  redacted RPC errors, one/all-network partial failures, testnet ordering, read immutability,
  and cache-only repair with an unchanged crypto envelope. No CLI or daemon starts.
- `cargo +1.95.0 test --locked --lib tui:: -- --test-threads=1`: 33 passed, including the
  12 action/form tests, password tests and existing terminal/filter/guard library tests.
- `git diff --check`: passed.

## Open acceptance gates

T14 and T19 real-process tests were added but not run in this work unit:
`t14_on_a_pty_duration_unlock_matches_status_expiry` and
`t19_on_a_pty_verify_all_refuses_bad_cache_then_repair_agrees_with_cli`.
They and the existing warm-up process test explicitly point every balance RPC at loopback;
none should reach a public endpoint. The parent coordinates their execution and the complete
CLI/PTY gate to avoid concurrent daemon lock-all tests from other worktrees. No commit was made.

This worktree starts from the EVM-only H4 source. The parent must merge the Tron family dispatch
into `wallet::read::{validate_rpc, read_network, balance}` from the separate Tron unit before
building the combined branch. Family pins and receive/address rows already use registry/family
identities rather than EVM-only address trust. The parent retains ownership of the combined
review, process gates, GitHub delivery and completion ledger.

## Parent integration (2026-10-29)

Merged onto TUI warm-up `f70924d` (PR #38), preserving serialized activation,
quit errors, password `ZeroizeOnDrop` and prior lifetime pins. All-target strict
Clippy and fmt passed; the complete local gate passed **566 tests, 0 failed,
11 live tests ignored**, including all 60 TUI tests and 3 shared-read tests.

The first integrated run exposed a test-input error in new T14/T19: the canonical
CLI helper sends LF, while the raw TUI Enter is CR. Both tests showed the masked
password field without submitting. Sending actual CR made both pass (2.51s and
4.47s); the whole gate was then rerun successfully. No production workaround
was added for this test issue. The inherited executable-copy/fork fixture fix
from PR #37 is included, with its two deterministic regressions.

Independent review found no standalone EVM blocker. A required Tron integration
follow-up is recorded as H18: a primary EVM status address must never retain
its address/trust under a newly selected Tron label before a key is opened.
This EVM-only unit does not claim that later family integration is complete.
Exact-head remote Linux/macOS CI and upstream merge remain the PR gates.
