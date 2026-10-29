# H18B — Tron history/detail adapter

Status: in-progress (local implementation and focused gates complete; parent integration/full gate pending)
Last measured: 2026-10-29

## Contract and scope

This unit implements the history/detail part of TUI-12 / T41 from `03-tui.md`, `05-tron.md`, and `v0-2-tui/units.md`. The parent delegated an isolated adapter on validated Tron C commit `deda723d4f6575b23f0084715b619f5b40f9be8d`, preserving the existing H17 EVM model and screens. The parent subsequently assigned the shared footer/key-row family-address defect to this unit. There is no signing, replacement, background indexer or new screen in this unit.

The implementation now:

- Shapes only the selected owner's Tron journal rows: exact base58check addresses, bare 64-hex txID, no nonce, parsed millisecond expiration, bounded raw amount, registry-selected symbol/decimals. Invalid rows are omitted. The ten-row history explicitly says local sends only and incoming transfers not indexed; it performs no explorer or node request.
- Dispatches details to the C hash-checked Tron lookup after fresh owner and journal reads. Arbitrary caller-supplied journal rows do not supply expiration. Network genesis, receipt id and transaction raw-data hash checks remain in C. Unknown or unsupported historical bytes do not receive invented transfer semantics.
- Preserves pending, not found, mined success/failure/unknown execution and expired-unseen separately. Fees remain sun; energy and bandwidth are labelled with Tron units. Expired-unseen carries the node's observed block/time/origin and second-payment caution. Confirmations remain explicitly unknown; this lookup does not count them.
- Uses a separate bounded Tron hash input accepting 64 hex digits with optional `0x`; EVM hash syntax is unchanged. Detail responses remain pinned to key, chain, endpoint, owner, generation and hash/request identity.
- Resolves checked-list Tron nicknames only through an exact Tron-only entry's canonical 20-byte body, with the unchanged checked fingerprint. Node names/symbols do not become verified identities.
- Displays addresses through `App::displayed_address` / `display_bar`. Primary EVM key metadata cannot be presented as a Tron address. Family observations expire with the UI generation; password pins remain family-specific. Cached, session, password, lock and impostor states retain distinct trust words.

## Evidence

Observed RED then GREEN:

1. Four initial regressions: Tron journal rows were absent, the slash field rejected bare txIDs, and a stale EVM owner could schedule a Tron read. Initial result: 0 passed / 4 failed; corrected result: 4 passed.
2. Tron nickname canonical-body mapping: 0 passed / 1 failed, then GREEN.
3. Prefixed lookup IDs: a direct `0x`-prefixed Tron detail request initially missed the fresh bare-hash journal row and returned not found instead of expired-unseen. The regression failed, then passed after canonicalizing the journal match.
4. Header/key-row family confusion: the 80x24 frame showed `tron` with the primary EVM address; a new zero-EVM-exposure assertion failed, then passed after the shared accessor fix.

Focused gates, using `target-handoff-tui-history` after `cargo clean -p zwallet`:

- `cargo test --offline --test tron_history -- --nocapture`: **19 passed**. Pure state/TestBackend and isolated `127.0.0.1` HTTP fixtures only; no daemon, CLI send or public traffic.
- `cargo test --offline --test history --test tui_history -- --nocapture`: **19 + 10 passed**, **1 live test ignored**. Existing H17 tests were preserved.
- `cargo test --offline --lib tui:: -- --nocapture`: **35 passed** (forms, state, TestBackend and existing fake-terminal restoration unit tests).
- `cargo test --offline --test tron_send --no-run`: compiled successfully. C's CLI tests were not executed; the C test file and `tests/common/tron.rs` are byte-identical to the captured source.
- `cargo clippy --offline --lib --test tron_history --test history --test tui_history -- -D warnings`: passed.
- `cargo fmt --all --check`: passed after formatting the two prerequisite compatibility files.

The default sandbox initially denied loopback listener creation. The same focused HTTP fixture tests ran successfully with loopback-capable execution; this was an environment restriction, not a product failure.

The 80x24 TestBackend artifact `/tmp/h18b-tron-detail-80x24.txt` was read directly: Tron address in the footer, unknown execution, `12345 sun`, millisecond expiry, explicit unknown confirmations, second-payment caution and unavailable replacement controls all remain visible. Width 79 produces the existing terminal-too-small view. Placement tests require every outside value to remain ASCII, bounded by its source budget and within the terminal rectangle.

## Independent review fixes (2026-10-29)

The independent reviewer identified two P2 state/trust regressions in the new shared family-address hooks. Both were reproduced before correction:

- Primary-address observation failure returned before replacing the status bar, leaving an old Open state after a newly locked observation, or an old Locked state after a newly open observation. Two tests failed. The observation error is now retained until the fresh lock state, expiry and session metadata are installed, then the existing failure path runs. Tests cover both a different valid primary address and malformed primary address.
- A fresh unlocked status with `Trust::Unverified(None)` retained an earlier Tron `Trust::Session` display. One test failed. Fresh negative session evidence now downgrades cached family-read trust while password-derived lifetime pins remain separate and intact.

Post-fix local verification: `tron_history` **19/19**, `tui_history` **10/10**, clippy with warnings denied, format and diff checks. The independent reviewer also read the revised paths and directly reran the existing test binaries: **19/19 + 10/10**, no additional blocker. Original C and H17 worktrees remain unchanged; the parent's separate H17 explorer test-server write-timeout fix must be retained during integration.

The three-file follow-up delta from the previously delivered adapter is `/tmp/h18b-status-review-delta.patch`. The full adapter/full patch and manifest are regenerated to include it; do not apply the follow-up delta again after using the regenerated adapter.

## Integration boundaries

- Worktree: `.worktrees/handoff-tron-history`, base C `deda723d4f6575b23f0084715b619f5b40f9be8d`.
- Exact pre-adapter source snapshot: `/tmp/h18b-before-adapter/manifest.json`. The generated adapter baseline normalizes formatting only and is recorded separately.
- H17 prerequisites include history/explorer/detail backend, common navigation/forms and small CLI/wallet visibility/Tron read-dispatch compatibility. They are a dependency baseline, not a request to overwrite newer parent integration work.
- New adapter modules: `chains/tron/journal.rs`, `history/tron.rs`, `tui/app/addresses.rs`. Hooks are in `SendJournal::recent`, the existing history worker/view, hash field, navigation state and shared key/footer drawing.
- `app::history::TronDetail` re-exports `history::tron::Detail`; added fields are `chain`, `token`, `as_of` and `node_behind_secs`. EVM replacement APIs are unchanged. H19 and H18A owners were notified.
- Parent owns reconciliation with newer common UI changes, independent review, full CLI/PTY/Linux gates, commit, PR and merge. No commit was created by this unit.
